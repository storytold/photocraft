//! The compositor's render pipelines: one table per compositor and its siblings, each pipeline
//! built the first time a draw needs it, or all of them at once where a caller wants every
//! shader compiled up front (the app's background build).
//!
//! On DX12 the shader compiler is FXC, which compiles one shader at a time; the ~30 pipelines
//! took about 8 s on one core (#6419), and 3 to 7.5 s per test process on the Windows CI
//! runners' software adapter (#15439). A compositor that draws one plain layer needs two or three.

use std::sync::OnceLock;

use super::consts::{MAP16, MAP32};
use super::kit::SHADER;
use super::{Kernel, Unsupported, first_error};

/// Why documents with layer effects can't use the GPU compositor on this device.
pub(crate) const NO_EFFECT_MAPS: &str = "layer effects need float effect-map render targets, which this GPU lacks";

/// Every pipeline of one compositor, shared by its siblings.
///
/// No shader is used by two threads at once (#9173): wgpu-hal's D3D12 backend caches each
/// compiled shader and hands every pipeline a reference to it, but FXC's blobs count references
/// without atomics (gfx-rs/wgpu#10547), so threads sharing one could free it while the cache still
/// held it, and dropping the device then aborted the process. So each kernel has its own vertex
/// entry point ([`vertex_shader`]), a kernel's pipelines for its target formats are built
/// together on one thread, and every build in the process holds [`building`]: the tables of
/// several compositors on one device compile the same shaders.
pub(crate) struct Pipelines {
    module: wgpu::ShaderModule,
    vertex: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    kernels: Vec<KernelPipelines>,
}

/// One kernel's pipelines, one per target format, built together.
pub(crate) struct KernelPipelines {
    pub(crate) kernel: Kernel,
    entry: &'static str,
    /// Its own vertex entry point in [`Pipelines::vertex`].
    pub(crate) vertex: String,
    pub(crate) formats: Vec<wgpu::TextureFormat>,
    /// Per format, the pipeline or the error building it raised.
    built: OnceLock<Vec<Result<wgpu::RenderPipeline, String>>>,
}

/// Held by every pipeline build in the process (see [`Pipelines`]).
fn building() -> std::sync::MutexGuard<'static, ()> {
    static BUILDING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Pipelines {
    /// The shaders and layout of every pipeline of [`table`]. Builds no pipeline.
    pub(crate) fn new(device: &wgpu::Device, acc_format: wgpu::TextureFormat, bgl0: &wgpu::BindGroupLayout, bgl1: &wgpu::BindGroupLayout) -> Self {
        let kernels = table(acc_format);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_compose"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pc_compose_vertex"),
            source: wgpu::ShaderSource::Wgsl(vertex_shader(kernels.iter().map(|k| k.vertex.as_str())).into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_compose"),
            bind_group_layouts: &[Some(bgl0), Some(bgl1)],
            immediate_size: 0,
        });
        Self { module, vertex, layout, kernels }
    }

    /// The pipeline drawing `kernel` into `format`, built now if no draw needed it before. A
    /// pipeline the device can't build is an [`Unsupported`] for every draw that needs it, so
    /// that refresh uses the CPU compositor.
    pub(crate) fn get(&self, device: &wgpu::Device, kernel: Kernel, format: wgpu::TextureFormat) -> Result<&wgpu::RenderPipeline, Unsupported> {
        let missing = || Unsupported(format!("the compositor has no {kernel:?} pipeline for {format:?}"));
        let k = self.kernels.iter().find(|k| k.kernel == kernel).ok_or_else(missing)?;
        let i = k.formats.iter().position(|&f| f == format).ok_or_else(missing)?;
        let built = match k.built.get() {
            Some(built) => built,
            None => {
                let _building = building();
                k.built.get_or_init(|| self.build(device, k))
            }
        };
        built[i].as_ref().map_err(|e| {
            Unsupported(if kernel.is_map() {
                format!("{NO_EFFECT_MAPS} ({e})")
            } else {
                format!("couldn't build the compositor's `{}` pipeline: {e}", k.entry)
            })
        })
    }

    /// Build every pipeline no draw has built yet, one kernel per thread of the current Rayon
    /// pool (or one after another), and return the first error a non-map pipeline raised. Effect
    /// maps render to R32Float / R16Float, which some adapters (e.g. GL without float render
    /// targets) can't: a failed map pipeline only turns layer effects to the CPU compositor
    /// ([`Self::effect_maps`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn build_all(&self, device: &wgpu::Device, sequential: bool) -> Option<String> {
        let _building = building();
        let missing: Vec<&KernelPipelines> = self.kernels.iter().filter(|k| k.built.get().is_none()).collect();
        let build = |k: &KernelPipelines| {
            k.built.get_or_init(|| self.build(device, k));
        };
        if sequential {
            missing.into_iter().for_each(build);
        } else {
            super::fx::par_map(missing, build);
        }
        let first_error = |maps: bool| {
            self.kernels.iter().filter(|k| k.kernel.is_map() == maps).flat_map(|k| k.built.get().into_iter().flatten()).find_map(|b| b.as_ref().err().cloned())
        };
        if let Some(e) = first_error(true) {
            log::info!("GPU compositor: no effect-map pipelines ({e}); layer effects use the CPU");
        }
        first_error(false)
    }

    /// `k`'s pipelines, each inside its own error scope on the calling thread (wgpu's scopes are
    /// per thread). The caller holds [`building`].
    fn build(&self, device: &wgpu::Device, k: &KernelPipelines) -> Vec<Result<wgpu::RenderPipeline, String>> {
        k.formats
            .iter()
            .map(|&format| {
                let (pipeline, error) = first_error(device, || {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(k.entry),
                        layout: Some(&self.layout),
                        vertex: wgpu::VertexState { module: &self.vertex, entry_point: Some(&k.vertex), buffers: &[], compilation_options: Default::default() },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &self.module,
                            entry_point: Some(k.entry),
                            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                            compilation_options: Default::default(),
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                });
                match error {
                    Some(e) => Err(e.to_string()),
                    None => Ok(pipeline),
                }
            })
            .collect()
    }

    /// Whether no effect-map pipeline has failed to build (one not built yet is assumed to work).
    pub(crate) fn effect_maps(&self) -> bool {
        self.kernels.iter().filter(|k| k.kernel.is_map()).all(|k| k.built.get().is_none_or(|b| b.iter().all(Result::is_ok)))
    }

    /// How many pipelines have been built (or failed to build) so far, and how many there are.
    pub(crate) fn counts(&self) -> (usize, usize) {
        let built = self.kernels.iter().filter(|k| k.built.get().is_some()).map(|k| k.formats.len()).sum();
        (built, self.kernels.iter().map(|k| k.formats.len()).sum())
    }
}

/// Every drawn kernel, none built yet: plain kernels draw into `acc_format`, effect-map kernels
/// into R32F and R16F maps.
pub(crate) fn table(acc_format: wgpu::TextureFormat) -> Vec<KernelPipelines> {
    let (plain, maps): (Vec<Kernel>, Vec<Kernel>) = Kernel::DRAWN.iter().partition(|k| !k.is_map());
    plain
        .into_iter()
        .chain(maps)
        .filter_map(|kernel| Some((kernel, kernel.entry()?)))
        .enumerate()
        .map(|(i, (kernel, entry))| KernelPipelines {
            kernel,
            entry,
            vertex: format!("vs_{i}"),
            formats: if kernel.is_map() { vec![MAP32, MAP16] } else { vec![acc_format] },
            built: OnceLock::new(),
        })
        .collect()
}

/// The full-chunk triangle's vertex stage, once under each of `names` (`vs_0`, `vs_1`, ...): the
/// same code under distinct names, so each kernel compiles its own vertex shader.
///
/// A workaround, not a design: wgpu-hal's D3D12 shader cache shares a compiled FXC blob between
/// threads, and FXC's blobs don't count references atomically, so a shared one can be freed
/// while still cached (gfx-rs/wgpu#10547, #9173). Each copy costs one compile of a three-line
/// shader. Once a wgpu release fixes the cache, go back to one `vs` in `compose.wgsl` and build
/// each pipeline on its own.
pub(crate) fn vertex_shader<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut s = String::from(
        "struct VOut { @builtin(position) pos: vec4<f32> };
",
    );
    for name in names {
        s.push_str(&format!(
            "@vertex fn {name}(@builtin(vertex_index) vi: u32) -> VOut {{
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}}
"
        ));
    }
    s
}
