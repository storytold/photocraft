use super::resources::{CanvasStyle, Resources, ViewGpu, f32_bytes};
use super::{CallbackResources, CallbackTrait, ScreenDescriptor, VIEW_FLOATS, VIEW_UNIFORM_SIZE, ViewParams, wgpu};

// ---------------------------------------------------------------------------------------------
// Per-frame callback

pub(super) struct CanvasCallback {
    pub(super) rect: egui::Rect,
    pub(super) params: ViewParams,
}

/// Sampling mode: 0 trilinear, 1 bilinear, 2 anti-aliased nearest.
pub(super) fn filter_mode(scale: f32) -> (f32, f32) {
    if scale < 0.999 {
        (0.0, (-scale.log2()).max(0.0))
    } else if (scale - scale.round()).abs() < 1e-3 || scale >= 2.0 {
        (2.0, 0.0)
    } else {
        (1.0, 0.0)
    }
}

/// Source-pixel support for an unbounded, full-resolution texture. The sharp branch uses
/// exactly the shader's positive weights; reduced views include the contributing mip blocks.
/// `scale` is device pixels per document pixel.
#[cfg(test)]
pub(crate) fn sampled_doc_rect(x: f64, y: f64, scale: f32) -> photocraft_geom::Rect {
    if !valid_sample(x, y, scale) {
        return photocraft_geom::Rect::EMPTY;
    }
    let (mode, lod) = filter_mode(scale);
    let axis = |v| {
        if mode != 0.0 {
            return weighted_axis(v, mode, f64::from(scale));
        }
        let level = f64::from(lod).ceil().min(31.0) as i32;
        let block = 2.0_f64.powi(level);
        let (lo, hi) = weighted_axis(v / block, 1.0, 1.0);
        (lo * block, hi * block)
    };
    let (x0, x1) = axis(x);
    let (y0, y1) = axis(y);
    pixel_rect([x0, y0, x1, y1])
}

fn valid_sample(x: f64, y: f64, scale: f32) -> bool {
    x.is_finite() && y.is_finite() && scale.is_finite() && scale > 0.0
}

/// Half-open texel indices whose bilinear weights are strictly positive. In sharp mode
/// the shader compresses the ramp to one device pixel, leaving most neighbours at zero.
fn weighted_axis(v: f64, mode: f32, scale: f64) -> (f64, f64) {
    let t = v - 0.5;
    let i = t.floor();
    let f = if mode == 2.0 { ((t - i - 0.5) * scale + 0.5).clamp(0.0, 1.0) } else { t - i };
    (i + if f == 1.0 { 1.0 } else { 0.0 }, i + if f > 0.0 { 2.0 } else { 1.0 })
}

fn pixel_rect([x0, y0, x1, y1]: [f64; 4]) -> photocraft_geom::Rect {
    let bounded = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    photocraft_geom::Rect::new(bounded(x0.floor()), bounded(y0.floor()), bounded(x1.ceil()), bounded(y1.ceil()))
}

/// The source grid and drawn core of an actual resident texture, in picture coordinates.
#[derive(Clone)]
pub(super) struct SamplingGrid {
    pub(super) rect: [u32; 4],
    pub(super) core: photocraft_geom::Rect,
    pub(super) texels: [u32; 2],
    pub(super) levels: u32,
    pub(super) opaque: Option<OpaqueTexels>,
}

/// Bounded per-texel opacity bits shared by GPU pages and uploaded CPU textures.
/// Copy-on-write patches preserve the opacity of the picture the user actually pressed.
#[derive(Clone)]
pub(crate) struct OpaqueTexels {
    bits: std::sync::Arc<Vec<u64>>,
    width: u32,
    height: u32,
}

impl OpaqueTexels {
    pub(crate) fn new(width: u32, height: u32, samples: impl IntoIterator<Item = bool>) -> Option<Self> {
        let count = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return None;
        }
        let words = usize::try_from(count.div_ceil(64)).ok()?;
        let mut out = Self { bits: std::sync::Arc::new(vec![0; words]), width, height };
        out.update([0, 0], width, samples);
        Some(out)
    }

    pub(crate) fn update(&mut self, [x, y]: [u32; 2], width: u32, samples: impl IntoIterator<Item = bool>) {
        if width == 0 || x >= self.width || width > self.width - x {
            return;
        }
        let bits = std::sync::Arc::make_mut(&mut self.bits);
        let capacity = u64::from(self.width) * u64::from(self.height);
        for (i, opaque) in samples.into_iter().enumerate() {
            let index = (u64::from(y) + i as u64 / u64::from(width)) * u64::from(self.width) + u64::from(x) + i as u64 % u64::from(width);
            if index >= capacity {
                break;
            }
            if let Some(word) = bits.get_mut((index / 64) as usize) {
                let mask = 1u64 << (index % 64);
                if opaque {
                    *word |= mask;
                } else {
                    *word &= !mask;
                }
            }
        }
    }

    pub(crate) fn all(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
        if x0 >= x1 || y0 >= y1 || x1 > self.width || y1 > self.height {
            return false;
        }
        (y0..y1).all(|y| {
            (x0..x1).all(|x| {
                let index = u64::from(y) * u64::from(self.width) + u64::from(x);
                self.bits.get((index / 64) as usize).is_some_and(|word| word & (1u64 << (index % 64)) != 0)
            })
        })
    }
}

/// Cheap snapshot of the source grids that a GPU picture actually draws. Page metadata is
/// shared until residency changes; pointer queries never allocate or consult a later frame.
#[derive(Clone)]
pub(crate) struct GpuSampling {
    pub(super) grids: std::sync::Arc<[SamplingGrid]>,
    pub(super) scale: f32,
    pub(super) factor: u32,
    pub(super) growth: [f32; 2],
}

impl GpuSampling {
    pub(crate) fn empty(scale: f32) -> Self {
        Self { grids: Default::default(), scale, factor: 1, growth: [0.0; 2] }
    }

    pub(crate) fn any_region(&self, x: f64, y: f64, mut test: impl FnMut(photocraft_geom::Rect) -> bool) -> bool {
        if !valid_sample(x, y, self.scale) || self.factor == 0 {
            return false;
        }
        let factor = f64::from(self.factor);
        let p = [x / factor + f64::from(self.growth[0]), y / factor + f64::from(self.growth[1])];
        let (mode, lod) = filter_mode(self.scale);
        // Reverse the callback's paint order: opaque finer pages hide earlier grids.
        for grid in self.grids.iter().rev() {
            let [tx, ty, w, h] = grid.rect.map(f64::from);
            let core = grid.core;
            if p[0] < tx
                || p[1] < ty
                || p[0] >= tx + w
                || p[1] >= ty + h
                || p[0] < f64::from(core.x0)
                || p[1] < f64::from(core.y0)
                || p[0] >= f64::from(core.x1)
                || p[1] >= f64::from(core.y1)
            {
                continue;
            }
            let last = grid.levels.saturating_sub(1).min(31);
            let lod = if mode == 0.0 { lod.clamp(0.0, last as f32) } else { 0.0 };
            let mut all_levels_opaque = true;
            for level in lod.floor() as u32..=lod.ceil() as u32 {
                let axis = |v: f64, origin: f64, span: f64, n: u32| {
                    let reduced = (n >> level).max(1);
                    let q = (v - origin) / span * f64::from(reduced);
                    let (lo, hi) = weighted_axis(q, mode, f64::from(self.scale) * span / f64::from(n));
                    let clamp = |i: f64, size: u32| i.clamp(0.0, f64::from(size.saturating_sub(1)));
                    let mut lo = clamp(lo, reduced);
                    let mut hi = clamp(hi - 1.0, reduced);
                    // Reverse the normalized bilinear mip taps. Endpoint bounds include all
                    // source pixels, but may bridge gaps in odd-sized reduction kernels;
                    // minification support remains conservative, unlike exact base/sharp taps.
                    for child in (1..=level).rev() {
                        let child_n = (n >> child).max(1);
                        let parent_n = (n >> (child - 1)).max(1);
                        let ratio = f64::from(parent_n) / f64::from(child_n);
                        let (a, _) = weighted_axis((lo + 0.5) * ratio, 1.0, 1.0);
                        let (_, b) = weighted_axis((hi + 0.5) * ratio, 1.0, 1.0);
                        lo = clamp(a, parent_n);
                        hi = clamp(b - 1.0, parent_n);
                    }
                    (lo, hi + 1.0)
                };
                let (x0, x1) = axis(p[0], tx, w, grid.texels[0]);
                let (y0, y1) = axis(p[1], ty, h, grid.texels[1]);
                all_levels_opaque &= grid.opaque.as_ref().is_some_and(|bits| bits.all(x0 as u32, y0 as u32, x1 as u32, y1 as u32));
                let to_doc = |v: f64, origin: f64, span: f64, n: u32, growth: f32| (origin + v * span / f64::from(n) - f64::from(growth)) * factor;
                let region = pixel_rect([
                    to_doc(x0, tx, w, grid.texels[0], self.growth[0]),
                    to_doc(y0, ty, h, grid.texels[1], self.growth[1]),
                    to_doc(x1, tx, w, grid.texels[0], self.growth[0]),
                    to_doc(y1, ty, h, grid.texels[1], self.growth[1]),
                ]);
                if !region.is_empty() && test(region) {
                    return true;
                }
            }
            // Every positive-weight mip of this grid is visible before it can occlude
            // an earlier grid. One opaque mip alone does not make a trilinear sample opaque.
            if all_levels_opaque {
                return false;
            }
        }
        false
    }
}

pub(super) fn map_doc_px(origin: [f32; 2], scale: f32, center: [f32; 2], cos: f32, sin: f32, x: f32, y: f32) -> [f32; 2] {
    let dx = x - center[0];
    let dy = y - center[1];
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    [origin[0] + center[0] * scale + rx * scale, origin[1] + center[1] * scale + ry * scale]
}

pub(super) fn tile_screen_aabb(origin: [f32; 2], scale: f32, center: [f32; 2], cos: f32, sin: f32, tile: [f32; 4]) -> (f32, f32, f32, f32) {
    let [tx, ty, tw, th] = tile;
    if sin.abs() < 1e-8 && (cos - 1.0).abs() < 1e-8 {
        let x0 = origin[0] + tx * scale;
        let y0 = origin[1] + ty * scale;
        return (x0, y0, x0 + tw * scale, y0 + th * scale);
    }
    let pts = [
        map_doc_px(origin, scale, center, cos, sin, tx, ty),
        map_doc_px(origin, scale, center, cos, sin, tx + tw, ty),
        map_doc_px(origin, scale, center, cos, sin, tx + tw, ty + th),
        map_doc_px(origin, scale, center, cos, sin, tx, ty + th),
    ];
    let mut x0 = pts[0][0];
    let mut y0 = pts[0][1];
    let mut x1 = x0;
    let mut y1 = y0;
    for p in pts {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    (x0, y0, x1, y1)
}

impl CanvasCallback {
    /// Screen-pixel origin of document (0, 0) (snapped to whole pixels) and device px per doc px.
    fn placement(&self, ppp: f32) -> ([f32; 2], f32) {
        let ([ox, oy], scale) = self.exact_placement(ppp);
        ([ox.round(), oy.round()], scale)
    }

    /// The origin before snapping: where the canvas draws the checkerboard under the picture and the edge around it.
    fn exact_placement(&self, ppp: f32) -> ([f32; 2], f32) {
        let p = &self.params;
        let scale = p.zoom * ppp;
        let c = self.rect.center();
        ([c.x * ppp - p.center[0] * scale, c.y * ppp - p.center[1] * scale], scale)
    }

    fn uniforms(&self, screen: [u32; 2], ppp: f32, out_linear: bool, style: &CanvasStyle) -> [f32; VIEW_FLOATS] {
        let p = &self.params;
        let (origin, scale) = self.placement(ppp);
        // How far the exact place is from the snapped texels, at most half a pixel each way: the picture's outer edge
        // goes there, so it covers just the pixels the checkerboard does (#17099).
        let (exact, _) = self.exact_placement(ppp);
        let shift = [exact[0] - origin[0], exact[1] - origin[1]].map(|v| if v.is_finite() { v.clamp(-0.5, 0.5) } else { 0.0 });
        let (mode, lod) = filter_mode(scale);
        let g = style.gamut_color;
        // 32-bit preview: linear-light gain 2^exposure (0 = off) and 1 / gamma.
        let (gain, inv_gamma) = match p.hdr {
            Some([e, gm]) if e.is_finite() && gm.is_finite() && gm > 0.0 => (2f32.powf(e.clamp(-20.0, 20.0)), 1.0 / gm.max(0.01)),
            _ => (0.0, 1.0),
        };
        [
            screen[0] as f32,
            screen[1] as f32,
            scale,
            ppp,
            origin[0],
            origin[1],
            p.doc_size[0] as f32,
            p.doc_size[1] as f32,
            mode,
            lod,
            shift[0],
            shift[1],
            0.0,
            0.0,
            p.display as f32,
            if out_linear { 1.0 } else { 0.0 },
            0.0,
            0.0,
            0.0,
            style.gamut_opacity,
            0.0,
            0.0,
            0.0,
            gain,
            g[0],
            g[1],
            g[2],
            inv_gamma,
            {
                let r = if p.rotation.is_finite() { p.rotation } else { 0.0 };
                if r.abs() < 1e-8 { 1.0 } else { r.cos() }
            },
            {
                let r = if p.rotation.is_finite() { p.rotation } else { 0.0 };
                if r.abs() < 1e-8 { 0.0 } else { r.sin() }
            },
            p.center[0],
            p.center[1],
        ]
    }
}

impl CallbackTrait for CanvasCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else { return Vec::new() };
        if !res.health.is_ok() {
            return Vec::new();
        }
        let data = self.uniforms(screen.size_in_pixels, screen.pixels_per_point, res.out_linear, &res.style);
        let (bgl, sampler) = (&res.view_bgl, &res.sampler);
        let v = res.views.entry(self.params.view_key).or_insert_with(|| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_canvas_view"),
                size: VIEW_UNIFORM_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pc_canvas_view"),
                layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
            });
            ViewGpu { buffer, bind_group, frame_data: [f32::NAN; VIEW_FLOATS] }
        });
        if v.frame_data != data {
            queue.write_buffer(&v.buffer, 0, &f32_bytes(&data));
            v.frame_data = data;
        }
        Vec::new()
    }

    /// Draw this view's visible tiles inside the clipped canvas.
    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(res) = resources.get::<Resources>() else { return };
        if !res.health.is_ok() {
            return;
        }
        let Some(view) = res.views.get(&self.params.view_key) else { return };
        let [sw, sh] = info.screen_size_px;
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 || sw == 0 || sh == 0 {
            return;
        }
        // Work in whole-screen pixel coordinates; the scissor does the clipping.
        pass.set_viewport(0.0, 0.0, sw as f32, sh as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        pass.set_bind_group(0, &view.bind_group, &[0]);
        let Some(doc) = res.docs.get(&self.params.doc) else { return };
        if doc.size != self.params.doc_size {
            return;
        }
        let (origin, scale) = self.placement(info.pixels_per_point);
        let (cx0, cy0) = (clip.left_px as f32, clip.top_px as f32);
        let (cx1, cy1) = (cx0 + clip.width_px as f32, cy0 + clip.height_px as f32);
        let rot = if self.params.rotation.is_finite() { self.params.rotation } else { 0.0 };
        let (cos, sin) = if rot.abs() < 1e-8 { (1.0, 0.0) } else { (rot.cos(), rot.sin()) };
        let cen = self.params.center;
        pass.set_pipeline(&res.tile_pipeline);
        pass.set_bind_group(2, res.luts.get(&(self.params.doc, self.params.output)).unwrap_or(&res.identity_lut), &[]);
        for t in &doc.tiles {
            let [tx, ty, tw, th] = t.rect.map(|v| v as f32);
            let (x0, y0, x1, y1) = tile_screen_aabb(origin, scale, cen, cos, sin, [tx, ty, tw, th]);
            if x1 < cx0 || y1 < cy0 || x0 > cx1 || y0 > cy1 {
                continue;
            }
            pass.set_bind_group(1, &t.bind_group, &[0]);
            pass.draw(0..6, 0..1);
        }
    }
}
