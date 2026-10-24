//! Shadows and Highlights for Camera Raw's GPU preview (#6916): the local Laplacian filter of
//! `photocraft_algo::camera_raw::local_tone` as fragment passes (`camera_raw_tone.wgsl`), with no
//! compute shaders, so the same passes can run on WebGL2.
//!
//! Each [`TonePass`](photocraft_algo::camera_raw::TonePass) builds the Gaussian pyramid of its
//! input, the remapped pyramids of up to [`MAX_REFS`] references (four per `Rgba32Float` tile, all
//! tiles of a level in one atlas), and the output from coarse to fine, each coefficient
//! interpolated between the two references around the input there. The quantiles the filter and
//! Whites and Blacks read are exact order statistics, found on the GPU by narrowing an interval of
//! keys in [`ROUNDS`] rounds of [`SPLITS`] thresholds, so nothing is read back before the
//! per-pixel pass (`camera_raw.wgsl`) applies the change.
//!
//! The textures and the list of passes are built once per frame size and set of passes
//! ([`ToneFrame`]); a develop only writes the settings uniform and records the passes.

use std::collections::HashMap;

use photocraft_algo::camera_raw::local_tone::{KNOTS, MAX_REFS, RANGE_QUANTILES, RANGE_SIDE, REF_LOW_QUANTILE, REF_STEP, TAPS};
use photocraft_algo::camera_raw::{LocalTone, ToneKind, WhitesBlacksModel, quantile_rank};

use crate::Unsupported;

/// The passes' WGSL source.
pub const CAMERA_RAW_TONE_SHADER: &str = include_str!("camera_raw_tone.wgsl");

const PLANE: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const RGBA: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const UINT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
/// `Step` in the shader: five rows of 16 bytes, one per uniform slot of [`SLOT`] bytes.
const STEP_SIZE: u64 = 5 * 16;
const SLOT: u64 = 256;
/// `Tone` in the shader.
const TONE_SIZE: u64 = 16 * 16;
/// Thresholds per round of a selection: each round splits the key interval into `SPLITS + 1`.
const SPLITS: u32 = 16;
/// Rounds until the interval of every key (2^32) is down to one key.
const ROUNDS: usize = rounds();
/// Values per tile edge counted by one fragment; counts are summed in tiles of the same edge until
/// at most [`SUM_MAX`] tiles remain, which a round's update sums.
const COUNT_TILE: u32 = 16;
const SUM_MAX: u32 = 256;

/// [`ROUNDS`]: the widest part `fs_update` can leave of an interval `span` keys wide (less one),
/// until it is 0.
const fn rounds() -> usize {
    let mut span: u64 = u32::MAX as u64;
    let mut n = 0;
    while span > 0 {
        let q = span / (SPLITS as u64 + 1) + 1;
        let last = span.saturating_sub(SPLITS as u64 * q);
        span = if q - 1 > last { q - 1 } else { last };
        n += 1;
    }
    n
}

/// Reference groups (four per tile).
const GROUPS: u32 = MAX_REFS.div_ceil(4) as u32;
/// Statistics slots: pass `i`'s input and output are `2i` and `2i + 1`; Whites and Blacks' points.
const WB_SLOT: u32 = 4;
const SLOTS: u32 = 5;
/// Rows of `camera_raw.wgsl`'s table texture that hold the Whites and Blacks curves.
pub(crate) const WB_ROW: u32 = 5;

// The shader's knot interpolation and taps are written for these sizes.
const _: () = assert!(KNOTS == 5 && TAPS.len() == 5);

/// A texture and its view.
struct Tex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Tex {
    fn new(device: &wgpu::Device, label: &str, [w, h]: [u32; 2], format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Tex { texture, view }
    }

    fn target(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat) -> Self {
        Tex::new(device, label, size, format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
    }
}

fn tex_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { multisampled: false, sample_type, view_dimension: wgpu::TextureViewDimension::D2 },
        count: None,
    }
}

/// A `u32` texture binding.
pub(crate) fn uint_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    tex_entry(binding, wgpu::TextureSampleType::Uint)
}

/// The passes' entry points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    Down,
    DownRemap,
    DownAtlas,
    Lap,
    Shift,
    Cells,
    Init,
    Count,
    Reduce,
    Update,
    WbRows,
}

impl Pipe {
    const ALL: [Pipe; 11] =
        [Pipe::Down, Pipe::DownRemap, Pipe::DownAtlas, Pipe::Lap, Pipe::Shift, Pipe::Cells, Pipe::Init, Pipe::Count, Pipe::Reduce, Pipe::Update, Pipe::WbRows];

    fn entry(self) -> &'static str {
        match self {
            Pipe::Down => "fs_down",
            Pipe::DownRemap => "fs_down_remap",
            Pipe::DownAtlas => "fs_down_atlas",
            Pipe::Lap => "fs_lap",
            Pipe::Shift => "fs_shift",
            Pipe::Cells => "fs_cells",
            Pipe::Init => "fs_init",
            Pipe::Count => "fs_count",
            Pipe::Reduce => "fs_reduce",
            Pipe::Update => "fs_update",
            Pipe::WbRows => "fs_wb_rows",
        }
    }

    fn targets(self) -> &'static [wgpu::TextureFormat] {
        match self {
            Pipe::DownRemap | Pipe::DownAtlas => &[RGBA],
            Pipe::Init | Pipe::Update => &[UINT],
            Pipe::Count | Pipe::Reduce => &[UINT, UINT],
            _ => &[PLANE],
        }
    }
}

/// The passes' pipelines, the settings uniform and the Whites and Blacks tables.
pub(crate) struct TonePipelines {
    layout: wgpu::BindGroupLayout,
    pipelines: HashMap<Pipe, wgpu::RenderPipeline>,
    settings: wgpu::Buffer,
    model: Tex,
    model_written: bool,
    /// Bound where a pass reads nothing.
    dummy: Tex,
    dummy_uint: Tex,
}

/// Rows of the Whites and Blacks model texture, in order: Whites curves, Blacks curves, the two
/// adaptation tables, the black points, the white points.
struct ModelRows {
    whites: u32,
    blacks: u32,
    whites_adapt: u32,
    blacks_adapt: u32,
    black_points: u32,
    white_points: u32,
    rows: u32,
    width: u32,
}

fn model_rows(m: &WhitesBlacksModel) -> ModelRows {
    let whites = 0;
    let blacks = whites + m.whites.len() as u32;
    let whites_adapt = blacks + m.blacks.len() as u32;
    let blacks_adapt = whites_adapt + m.whites_adapt.len() as u32;
    let black_points = blacks_adapt + m.blacks_adapt.len() as u32;
    let white_points = black_points + 1;
    let width = [m.whites.first().map_or(0, |r| r.len()), m.black_points.len(), m.white_points.len(), m.whites_adapt[0].len()].into_iter().max().unwrap_or(1);
    ModelRows { whites, blacks, whites_adapt, blacks_adapt, black_points, white_points, rows: white_points + 1, width: width as u32 }
}

impl TonePipelines {
    pub(crate) fn build(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pc_camera_raw_tone"),
            source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_TONE_SHADER.into()),
        });
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let uniform = |binding, dynamic, size| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: wgpu::BufferSize::new(size) },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw_tone"),
            entries: &[
                uniform(0, true, STEP_SIZE),
                uniform(1, false, TONE_SIZE),
                tex_entry(2, float),
                tex_entry(3, float),
                tex_entry(4, float),
                tex_entry(5, float),
                uint_entry(6),
                uint_entry(7),
                uint_entry(8),
                uint_entry(9),
                tex_entry(10, float),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw_tone"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = Pipe::ALL
            .iter()
            .map(|&pipe| {
                let targets: Vec<Option<wgpu::ColorTargetState>> =
                    pipe.targets().iter().map(|&format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })).collect();
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(pipe.entry()),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(pipe.entry()),
                        targets: &targets,
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                (pipe, pipeline)
            })
            .collect();
        let settings = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_tone_settings"),
            size: TONE_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let rows = model_rows(&photocraft_algo::camera_raw::whites_blacks_model());
        let model =
            Tex::new(device, "pc_camera_raw_wb_model", [rows.width, rows.rows], PLANE, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        TonePipelines {
            layout,
            pipelines,
            settings,
            model,
            model_written: false,
            dummy: Tex::new(device, "pc_camera_raw_tone_dummy", [1, 1], PLANE, sampled),
            dummy_uint: Tex::new(device, "pc_camera_raw_tone_dummy_uint", [1, 1], UINT, sampled),
        }
    }

    /// A plane bound where a pass reads nothing (for `camera_raw.wgsl`'s bindings too).
    pub(crate) fn dummy(&self) -> &wgpu::TextureView {
        &self.dummy.view
    }

    pub(crate) fn dummy_uint(&self) -> &wgpu::TextureView {
        &self.dummy_uint.view
    }

    /// Writes the settings uniform for `tone` and Whites and Blacks' sliders, and the model once.
    pub(crate) fn write(&mut self, queue: &wgpu::Queue, tone: &LocalTone, whites_blacks: Option<[f32; 2]>) {
        let m = photocraft_algo::camera_raw::whites_blacks_model();
        let rows = model_rows(&m);
        if !self.model_written {
            let width = rows.width as usize;
            let mut data = vec![0.0f32; width * rows.rows as usize];
            let mut put = |row: u32, v: &[f32]| {
                let start = row as usize * width;
                for (d, s) in data.iter_mut().skip(start).take(width).zip(v) {
                    *d = *s;
                }
            };
            for (k, r) in m.whites.iter().enumerate() {
                put(rows.whites + k as u32, r);
            }
            for (k, r) in m.blacks.iter().enumerate() {
                put(rows.blacks + k as u32, r);
            }
            for (k, r) in m.whites_adapt.iter().enumerate() {
                put(rows.whites_adapt + k as u32, r);
            }
            for (k, r) in m.blacks_adapt.iter().enumerate() {
                put(rows.blacks_adapt + k as u32, r);
            }
            put(rows.black_points, m.black_points);
            put(rows.white_points, m.white_points);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.model.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&data),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rows.width * 4), rows_per_image: Some(rows.rows) },
                wgpu::Extent3d { width: rows.width, height: rows.rows, depth_or_array_layers: 1 },
            );
            self.model_written = true;
        }
        let mut f: Vec<f32> = Vec::with_capacity(TONE_SIZE as usize / 4);
        for i in 0..2 {
            f.extend(tone.passes.get(i).map_or([0.0; 4], |p| [if p.kind == ToneKind::Highlights { 0.0 } else { 1.0 }, p.amount, p.sigma, 0.0]));
        }
        let mut knots = [0.0f32; 20];
        for (i, p) in tone.passes.iter().take(2).enumerate() {
            for (r, row) in p.knots.iter().enumerate() {
                for (k, v) in row.iter().enumerate() {
                    if let Some(slot) = knots.get_mut(i * 10 + r * 5 + k) {
                        *slot = *v;
                    }
                }
            }
        }
        f.extend(knots);
        f.extend([REF_STEP, MAX_REFS as f32, 0.0, 0.0]);
        f.extend([TAPS[0], TAPS[1], TAPS[2], TAPS[3], TAPS[4], 0.0, 0.0, 0.0]);
        let [whites, blacks] = whites_blacks.unwrap_or_default();
        f.extend([whites, blacks, m.row_step, m.photo_blacks]);
        f.extend([m.whites_from, m.blacks_from, 0.0, 0.0]);
        f.extend([m.whites_range[0], m.whites_range[1], m.blacks_range[0], m.blacks_range[1]]);
        let mut bytes: Vec<u8> = bytemuck::cast_slice(&f).to_vec();
        let u = [
            rows.whites,
            rows.blacks,
            rows.whites_adapt,
            rows.blacks_adapt,
            rows.black_points,
            rows.white_points,
            m.black_points.len() as u32,
            m.white_points.len() as u32,
            m.whites.len() as u32,
            m.blacks.len() as u32,
            0,
            0,
        ];
        bytes.extend(u.iter().flat_map(|v| v.to_le_bytes()));
        bytes.resize(TONE_SIZE as usize, 0);
        queue.write_buffer(&self.settings, 0, &bytes);
    }
}

/// One pass's uniform slot (`Step` in the shader).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
struct Step {
    sizes: [u32; 4],
    sizes2: [u32; 4],
    misc: [u32; 4],
    ranks: [[u32; 4]; 2],
}

impl Step {
    fn sized(dst: [u32; 2], src: [u32; 2]) -> Self {
        Step { sizes: [dst[0], dst[1], src[0], src[1]], ..Default::default() }
    }
    fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        [self.sizes, self.sizes2, self.misc, self.ranks[0], self.ranks[1]].into_iter().flatten().flat_map(u32::to_le_bytes)
    }
}

/// The textures one pass reads (unset ones are dummies).
#[derive(Clone, Default, PartialEq, Eq, Hash)]
struct Reads {
    a: Option<wgpu::TextureView>,
    b: Option<wgpu::TextureView>,
    fine: Option<wgpu::TextureView>,
    coarse: Option<wgpu::TextureView>,
    ua: Option<wgpu::TextureView>,
    ub: Option<wgpu::TextureView>,
    uc: Option<wgpu::TextureView>,
}

/// A recorded pass, or the copy of a bisection's result (column 0 of its state) into a slot.
enum Op {
    Draw { pipe: Pipe, group: wgpu::BindGroup, offset: u32, targets: Vec<wgpu::TextureView>, viewport: Option<[f32; 4]> },
    Result { state: wgpu::Texture, slot: u32 },
}

/// A pass before its uniform slot and bind group exist (a list built once per plan).
#[allow(clippy::large_enum_variant)]
enum Pending {
    Draw { pipe: Pipe, step: Step, reads: Reads, targets: Vec<wgpu::TextureView>, viewport: Option<[f32; 4]> },
    Result { state: wgpu::Texture, slot: u32 },
}

/// Count textures for selections over a source of one size: per stage, both banks and the size.
struct Chain {
    stages: Vec<([Tex; 2], [u32; 2])>,
}

impl Chain {
    fn new(device: &wgpu::Device, [w, h]: [u32; 2]) -> Self {
        let mut stages = Vec::new();
        let mut size = [w.div_ceil(COUNT_TILE), h.div_ceil(COUNT_TILE)];
        loop {
            // Each tile has a column per threshold.
            let texels = [size[0] * SPLITS, size[1]];
            stages.push(([Tex::target(device, "pc_camera_raw_counts_a", texels, UINT), Tex::target(device, "pc_camera_raw_counts_b", texels, UINT)], size));
            if size[0] * size[1] <= SUM_MAX {
                break;
            }
            size = [size[0].div_ceil(COUNT_TILE), size[1].div_ceil(COUNT_TILE)];
        }
        Chain { stages }
    }
}

/// Collects a plan's passes.
#[derive(Default)]
struct Builder {
    ops: Vec<Pending>,
}

impl Builder {
    fn draw(&mut self, pipe: Pipe, step: Step, reads: Reads, targets: &[&wgpu::TextureView], viewport: Option<[f32; 4]>) {
        self.ops.push(Pending::Draw { pipe, step, reads, targets: targets.iter().map(|v| (*v).clone()).collect(), viewport });
    }

    /// Order statistics by bisection, into statistics slot `slot`: bank 0 of `source` (`size`
    /// texels; its red channel in all four, or with `channels` its four channels), bank 1 of the
    /// red channel of `second`, at `ranks` (0-based, below the counts).
    #[allow(clippy::too_many_arguments)]
    fn select(
        &mut self,
        source: &wgpu::TextureView,
        size: [u32; 2],
        channels: bool,
        second: Option<(&wgpu::TextureView, [u32; 2])>,
        ranks: [[u32; 4]; 2],
        slot: u32,
        chain: &Chain,
        state: &[Tex; 2],
    ) {
        let (Some(first), Some(last)) = (chain.stages.first(), chain.stages.last()) else { return };
        self.draw(Pipe::Init, Step::default(), Reads::default(), &[&state[0].view], None);
        let (second, second_size) = second.map_or((None, [0, 0]), |(v, s)| (Some(v.clone()), s));
        let mut count = Step::sized(first.1, size);
        count.sizes2 = [second_size[0], second_size[1], 0, COUNT_TILE];
        count.misc = [0, 0, u32::from(channels), SPLITS];
        let mut update = Step::sized([2, 2], last.1);
        update.misc[3] = SPLITS;
        update.ranks = ranks;
        let mut cur = 0;
        for _ in 0..ROUNDS {
            let reads = Reads { a: Some(source.clone()), b: second.clone(), ua: Some(state[cur].view.clone()), ..Default::default() };
            self.draw(Pipe::Count, count, reads, &[&first.0[0].view, &first.0[1].view], None);
            for w in chain.stages.windows(2) {
                let (from, to) = (&w[0], &w[1]);
                let mut step = Step::sized(to.1, from.1);
                step.sizes2[3] = COUNT_TILE;
                step.misc[3] = SPLITS;
                let reads = Reads { ua: Some(from.0[0].view.clone()), ub: Some(from.0[1].view.clone()), ..Default::default() };
                self.draw(Pipe::Reduce, step, reads, &[&to.0[0].view, &to.0[1].view], None);
            }
            let reads = Reads { ua: Some(state[cur].view.clone()), ub: Some(last.0[0].view.clone()), uc: Some(last.0[1].view.clone()), ..Default::default() };
            self.draw(Pipe::Update, update, reads, &[&state[1 - cur].view], None);
            cur = 1 - cur;
        }
        self.ops.push(Pending::Result { state: state[cur].texture.clone(), slot });
    }

    /// The passes with their uniform slots in `steps` (written here) and their bind groups.
    fn finish(self, device: &wgpu::Device, queue: &wgpu::Queue, pipes: &TonePipelines, stats: &wgpu::TextureView) -> (wgpu::Buffer, Vec<Op>) {
        let mut steps: Vec<Step> = Vec::new();
        let mut slots: HashMap<Step, u32> = HashMap::new();
        for op in &self.ops {
            if let Pending::Draw { step, .. } = op {
                slots.entry(*step).or_insert_with(|| {
                    steps.push(*step);
                    steps.len() as u32 - 1
                });
            }
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_tone_steps"),
            size: steps.len().max(1) as u64 * SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut data = vec![0u8; steps.len() * SLOT as usize];
        for (chunk, step) in data.chunks_mut(SLOT as usize).zip(&steps) {
            for (d, v) in chunk.iter_mut().zip(step.bytes()) {
                *d = v;
            }
        }
        queue.write_buffer(&buffer, 0, &data);
        // wgpu resources hash and compare by identity, which never changes.
        #[allow(clippy::mutable_key_type)]
        let mut groups: HashMap<Reads, wgpu::BindGroup> = HashMap::new();
        let ops = self
            .ops
            .into_iter()
            .map(|op| match op {
                Pending::Draw { pipe, step, reads, targets, viewport } => {
                    let group = groups.entry(reads.clone()).or_insert_with(|| pipes.group(device, &buffer, &reads, stats)).clone();
                    let offset = slots.get(&step).copied().unwrap_or(0) * SLOT as u32;
                    Op::Draw { pipe, group, offset, targets, viewport }
                }
                Pending::Result { state, slot } => Op::Result { state, slot },
            })
            .collect();
        (buffer, ops)
    }
}

impl TonePipelines {
    fn group(&self, device: &wgpu::Device, steps: &wgpu::Buffer, r: &Reads, stats: &wgpu::TextureView) -> wgpu::BindGroup {
        fn or<'a>(v: &'a Option<wgpu::TextureView>, dummy: &'a wgpu::TextureView) -> wgpu::BindingResource<'a> {
            wgpu::BindingResource::TextureView(v.as_ref().unwrap_or(dummy))
        }
        let step = wgpu::BufferBinding { buffer: steps, offset: 0, size: wgpu::BufferSize::new(STEP_SIZE) };
        let entries = [
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(step) },
            wgpu::BindGroupEntry { binding: 1, resource: self.settings.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: or(&r.a, &self.dummy.view) },
            wgpu::BindGroupEntry { binding: 3, resource: or(&r.b, &self.dummy.view) },
            wgpu::BindGroupEntry { binding: 4, resource: or(&r.fine, &self.dummy.view) },
            wgpu::BindGroupEntry { binding: 5, resource: or(&r.coarse, &self.dummy.view) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(stats) },
            wgpu::BindGroupEntry { binding: 7, resource: or(&r.ua, &self.dummy_uint.view) },
            wgpu::BindGroupEntry { binding: 8, resource: or(&r.ub, &self.dummy_uint.view) },
            wgpu::BindGroupEntry { binding: 9, resource: or(&r.uc, &self.dummy_uint.view) },
            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.model.view) },
        ];
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pc_camera_raw_tone"), layout: &self.layout, entries: &entries })
    }
}

/// What a plan runs: the passes' kept quantiles (their bits), and whether Whites and Blacks
/// measure the points.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PlanKey {
    keeps: Vec<u32>,
    whites_blacks: bool,
}

impl PlanKey {
    fn new(tone: &LocalTone, whites_blacks: bool) -> Self {
        PlanKey { keeps: tone.passes.iter().map(|p| p.keep.to_bits()).collect(), whites_blacks }
    }
}

/// The recorded passes for one [`PlanKey`].
struct Plan {
    /// Holds the passes' uniform slots.
    _steps: wgpu::Buffer,
    /// Shadows and Highlights.
    tone: Vec<Op>,
    /// Whites and Blacks' points and curves, after `camera_raw.wgsl`'s `fs_points`.
    whites_blacks: Vec<Op>,
}

/// Pyramid level sizes down to about 2 pixels on the short side (`local_tone::depth`).
fn levels([w, h]: [u32; 2]) -> Vec<[u32; 2]> {
    let mut out = vec![[w, h]];
    let (mut s, mut level) = (w.min(h), [w, h]);
    while s > 2 {
        s = s.div_ceil(2);
        level = level.map(|v| v.div_ceil(2));
        out.push(level);
    }
    out
}

/// The filter's textures for one frame size, and its plans.
pub(crate) struct ToneFrame {
    size: [u32; 2],
    /// Pyramid level sizes, finest first (at least two).
    levels: Vec<[u32; 2]>,
    /// Atlas columns.
    cols: u32,
    /// The first pass's input, which `fs_log` renders.
    log_in: Tex,
    /// The second pass's input.
    input: Tex,
    /// The input's Gaussian pyramid (index 0 unused: level 0 is the input).
    gauss: Vec<Tex>,
    /// The remapped pyramids (index 0 unused).
    atlas: Vec<Tex>,
    /// The output, level by level (0 … n − 2).
    out: Vec<Tex>,
    /// The range's cells.
    cells: Tex,
    cells_size: [u32; 2],
    /// Whites and Blacks' cells, which `fs_points` renders.
    points: Tex,
    points_size: [u32; 2],
    points_edge: u32,
    full: Chain,
    small: Chain,
    state: [Tex; 2],
    stats: Tex,
    plans: HashMap<PlanKey, Plan>,
}

impl ToneFrame {
    /// The textures for a `size` frame, or `Unsupported` where they don't fit the device.
    pub(crate) fn new(device: &wgpu::Device, size: [u32; 2]) -> Result<Self, Unsupported> {
        let levels = levels(size);
        let n = levels.len();
        let Some(&[w1, h1]) = levels.get(1) else {
            return Err(Unsupported(format!("a {}×{} frame is too small for the Shadows and Highlights pyramid", size[0], size[1])));
        };
        let max = device.limits().max_texture_dimension_2d;
        // The squarest atlas of the reference groups.
        let cols = (1..=GROUPS).min_by_key(|c| (c * w1).max(GROUPS.div_ceil(*c) * h1)).unwrap_or(GROUPS);
        let rows = GROUPS.div_ceil(cols);
        if cols * w1 > max || rows * h1 > max {
            return Err(Unsupported(format!("Shadows and Highlights' {}×{} atlas doesn't fit the GPU's {max} px textures", cols * w1, rows * h1)));
        }
        let cells_of = |side: usize| {
            let f = size[0].max(size[1]).div_ceil(side as u32).max(1);
            ([size[0].div_ceil(f), size[1].div_ceil(f)], f)
        };
        let (cells_size, _) = cells_of(RANGE_SIDE);
        let (points_size, points_edge) = cells_of(photocraft_algo::camera_raw::whites_blacks_model().points_side);
        let plane = |label: &str, s: [u32; 2]| Tex::target(device, label, s, PLANE);
        Ok(ToneFrame {
            size,
            cols,
            log_in: plane("pc_camera_raw_log_in", size),
            input: plane("pc_camera_raw_tone_input", size),
            gauss: levels.iter().enumerate().map(|(l, &s)| plane("pc_camera_raw_gauss", if l == 0 { [1, 1] } else { s })).collect(),
            atlas: levels
                .iter()
                .enumerate()
                .map(|(l, &[lw, lh])| Tex::target(device, "pc_camera_raw_atlas", if l == 0 { [1, 1] } else { [cols * lw, rows * lh] }, RGBA))
                .collect(),
            out: levels.iter().take(n - 1).map(|&s| plane("pc_camera_raw_tone_out", s)).collect(),
            cells: plane("pc_camera_raw_tone_cells", cells_size),
            cells_size,
            points: Tex::target(device, "pc_camera_raw_wb_cells", points_size, RGBA),
            points_size,
            points_edge,
            full: Chain::new(device, size),
            small: Chain::new(device, points_size),
            state: [0, 1].map(|_| {
                let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
                Tex::new(device, "pc_camera_raw_select", [2, 2], UINT, usage)
            }),
            stats: Tex::new(device, "pc_camera_raw_stats", [SLOTS, 2], UINT, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST),
            levels,
            plans: HashMap::new(),
        })
    }

    pub(crate) fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The log luminance the first pass filters (`camera_raw.wgsl`'s `fs_log` renders it).
    pub(crate) fn log_in(&self) -> &wgpu::TextureView {
        &self.log_in.view
    }

    /// The last pass's output, before its kept quantile is put back.
    pub(crate) fn log_out(&self) -> &wgpu::TextureView {
        &self.out[0].view
    }

    pub(crate) fn stats(&self) -> &wgpu::TextureView {
        &self.stats.view
    }

    /// Whites and Blacks' cells (rendered by `fs_points`), their size and edge in pixels.
    pub(crate) fn points(&self) -> (&wgpu::TextureView, [u32; 2], u32) {
        (&self.points.view, self.points_size, self.points_edge)
    }

    /// The statistics slots of the last pass of `tone`: its input's and its output's.
    pub(crate) fn last_slots(tone: &LocalTone) -> [u32; 2] {
        let i = tone.passes.len().saturating_sub(1) as u32;
        [2 * i, 2 * i + 1]
    }

    /// Records Shadows and Highlights on `log_in`: then `log_out` and `stats` hold the result.
    /// `luts` is `camera_raw.wgsl`'s table texture, where Whites and Blacks' curves go.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_tone(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipes: &TonePipelines,
        encoder: &mut wgpu::CommandEncoder,
        tone: &LocalTone,
        whites_blacks: bool,
        luts: &wgpu::TextureView,
    ) {
        let key = PlanKey::new(tone, whites_blacks);
        if !self.plans.contains_key(&key) {
            let plan = self.plan(device, queue, pipes, &key, luts);
            self.plans.insert(key.clone(), plan);
        }
        if let Some(plan) = self.plans.get(&key) {
            record(encoder, &plan.tone, pipes, &self.stats.texture);
        }
    }

    /// Records Whites and Blacks' points and curves (rows [`WB_ROW`] and the next of `luts`),
    /// after `fs_points` rendered the cells.
    pub(crate) fn encode_whites_blacks(&self, pipes: &TonePipelines, encoder: &mut wgpu::CommandEncoder, tone: &LocalTone) {
        if let Some(plan) = self.plans.get(&PlanKey::new(tone, true)) {
            record(encoder, &plan.whites_blacks, pipes, &self.stats.texture);
        }
    }

    fn plan(&self, device: &wgpu::Device, queue: &wgpu::Queue, pipes: &TonePipelines, key: &PlanKey, luts: &wgpu::TextureView) -> Plan {
        let mut b = Builder::default();
        let n = self.levels.len();
        let full = self.size;
        let count = full[0] * full[1];
        let cells = self.cells_size[0] * self.cells_size[1];
        let rank = |len: u32, q: f32| (quantile_rank(len as usize, q) as u32).min(len.saturating_sub(1));
        let v = |t: &Tex| Some(t.view.clone());
        for (i, keep) in key.keeps.iter().enumerate() {
            let keep = f32::from_bits(*keep);
            let i = i as u32;
            let (slot_in, slot_out) = (2 * i, 2 * i + 1);
            let input = if i == 0 { &self.log_in } else { &self.input };
            let level = |l: usize| if l == 0 { input } else { &self.gauss[l] };
            // The input's Gaussian pyramid.
            for l in 0..n - 1 {
                b.draw(
                    Pipe::Down,
                    Step::sized(self.levels[l + 1], self.levels[l]),
                    Reads { a: v(level(l)), ..Default::default() },
                    &[&self.gauss[l + 1].view],
                    None,
                );
            }
            // Its range, the lowest reference, the kept quantile and the highest reference.
            let mut step = Step::sized(self.cells_size, full);
            step.sizes2[3] = full[0].max(full[1]).div_ceil(RANGE_SIDE as u32).max(1);
            b.draw(Pipe::Cells, step, Reads { a: v(input), ..Default::default() }, &[&self.cells.view], None);
            let ranks = [
                [rank(count, REF_LOW_QUANTILE), rank(count, keep), count - 1, count - 1],
                [rank(cells, RANGE_QUANTILES[0]), rank(cells, RANGE_QUANTILES[1]), 0, 0],
            ];
            b.select(&input.view, full, false, Some((&self.cells.view, self.cells_size)), ranks, slot_in, &self.full, &self.state);
            // The remapped pyramids.
            let mut step = Step::sized(self.levels[1], full);
            step.sizes2[2] = self.cols;
            step.misc = [i, slot_in, 0, 0];
            b.draw(Pipe::DownRemap, step, Reads { a: v(input), ..Default::default() }, &[&self.atlas[1].view], None);
            for l in 1..n - 1 {
                let mut step = Step::sized(self.levels[l + 1], self.levels[l]);
                step.sizes2[2] = self.cols;
                step.misc = [i, slot_in, 0, 0];
                b.draw(Pipe::DownAtlas, step, Reads { fine: v(&self.atlas[l]), ..Default::default() }, &[&self.atlas[l + 1].view], None);
            }
            // The output, coarse to fine.
            for l in (0..n - 1).rev() {
                let mut step = Step::sized(self.levels[l], self.levels[l + 1]);
                step.sizes2[2] = self.cols;
                step.misc = [i, slot_in, u32::from(l == 0), 0];
                let coarse = if l == n - 2 { &self.gauss[n - 1] } else { &self.out[l + 1] };
                let reads = Reads {
                    a: v(level(l)),
                    b: v(coarse),
                    fine: if l > 0 { v(&self.atlas[l]) } else { None },
                    coarse: v(&self.atlas[l + 1]),
                    ..Default::default()
                };
                b.draw(Pipe::Lap, step, reads, &[&self.out[l].view], None);
            }
            b.select(&self.out[0].view, full, false, None, [[rank(count, keep), 0, 0, 0], [0; 4]], slot_out, &self.full, &self.state);
            if (i as usize) + 1 < key.keeps.len() {
                let mut step = Step::sized(full, full);
                step.misc = [i, slot_in, 0, slot_out];
                b.draw(Pipe::Shift, step, Reads { a: v(&self.out[0]), ..Default::default() }, &[&self.input.view], None);
            }
        }
        let split = b.ops.len();
        if key.whites_blacks {
            let points = self.points_size[0] * self.points_size[1];
            let q = photocraft_algo::camera_raw::whites_blacks_model().point_quantiles;
            b.select(
                &self.points.view,
                self.points_size,
                true,
                None,
                [[rank(points, q[0]), rank(points, q[1]), 0, 0], [0; 4]],
                WB_SLOT,
                &self.small,
                &self.state,
            );
            let step = Step { misc: [0, WB_SLOT, WB_ROW, 0], ..Default::default() };
            let rows = Some([0.0, WB_ROW as f32, photocraft_algo::camera_raw::WB_STEPS as f32, 2.0]);
            b.draw(Pipe::WbRows, step, Reads::default(), &[luts], rows);
        }
        let (steps, mut tone) = b.finish(device, queue, pipes, &self.stats.view);
        let whites_blacks = tone.split_off(split.min(tone.len()));
        Plan { _steps: steps, tone, whites_blacks }
    }
}

/// Records `ops`; results go into `stats`.
fn record(encoder: &mut wgpu::CommandEncoder, ops: &[Op], pipes: &TonePipelines, stats: &wgpu::Texture) {
    for op in ops {
        match op {
            Op::Draw { pipe, group, offset, targets, viewport } => {
                let Some(pipeline) = pipes.pipelines.get(pipe) else { continue };
                // Only Whites and Blacks' rows draw into part of a target, which keeps the rest.
                let load = if viewport.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) };
                let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = targets
                    .iter()
                    .map(|view| {
                        Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                        })
                    })
                    .collect();
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(pipe.entry()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[*offset]);
                if let Some([x, y, w, h]) = *viewport {
                    pass.set_viewport(x, y, w, h, 0.0, 1.0);
                    pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
                }
                pass.draw(0..3, 0..1);
            }
            Op::Result { state, slot } => encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: state, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: stats, mip_level: 0, origin: wgpu::Origin3d { x: *slot, y: 0, z: 0 }, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: 1, height: 2, depth_or_array_layers: 1 },
            ),
        }
    }
}
