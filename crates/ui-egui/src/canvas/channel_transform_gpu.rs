//! Cached native RGB samples for channel previews. Only geometry/view uniforms change while
//! dragging. Unsupported stacks, monitor transforms and devices use the CPU preview oracle.

use std::sync::Arc;

use eframe::{egui_wgpu::RenderState, wgpu};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerId};
use photocraft_engine::channel_cmds::ChannelView;

use crate::state::TransformSession;

const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FLOAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const UNIFORM_BYTES: u64 = 128;

struct Pipelines {
    inputs: wgpu::BindGroupLayout,
    moved: wgpu::BindGroupLayout,
    affine: wgpu::RenderPipeline,
    warp: wgpu::RenderPipeline,
    finish: wgpu::RenderPipeline,
}

pub(crate) struct Preview {
    rs: RenderState,
    pipes: Arc<Pipelines>,
    uniform: wgpu::Buffer,
    inputs: wgpu::BindGroup,
    moved: wgpu::BindGroup,
    moved_view: wgpu::TextureView,
    output_view: wgpu::TextureView,
    _output: wgpu::Texture,
    id: egui::TextureId,
    size: [u32; 2],
    opacity: f32,
    steps: f32,
    fill: [f32; 4],
    content: [photocraft_geom::Rect; 3],
    lifted: [photocraft_geom::Rect; 3],
    selection: Option<photocraft_geom::Rect>,
    selected: bool,
    last: Option<(TransformSession, ChannelView, bool, bool, Option<photocraft_geom::Rect>)>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}

/// Deliberately narrow: no flattened RGB shortcut may change a mask, blending or ICC result.
fn source(doc: &Document, layer: Option<LayerId>) -> Option<(&Layer, bool)> {
    if doc.mode != ColorMode::Rgb {
        return None;
    }
    let mut visible = doc.layers.iter().filter(|l| l.visible);
    // Document layers are stored bottom to top. An opaque active layer at the top hides
    // lower layers and can be displayed by the same single-layer shader.
    let l = visible.next_back()?;
    let beneath = visible.next_back().is_some();
    (Some(l.id) == layer
        && doc.walk().iter().filter(|(_, _, other)| other.id == l.id).count() == 1
        && matches!(l.content, photocraft_doc::LayerContent::Raster(_))
        && l.surface().is_some_and(|s| s.format().mode == ColorMode::Rgb && s.format().sample == doc.depth)
        && l.blend == BlendMode::Normal
        && !l.clipped
        && l.mask.is_none()
        && l.vector_mask.is_none()
        && !photocraft_compose::effects::has_effects(l)
        && l.excluded_channels == 0
        && l.blend_if == Default::default()
        && l.advanced == Default::default()
        && l.video.is_none()
        && (0.0..=1.0).contains(&l.opacity)
        && (0.0..=1.0).contains(&l.fill_opacity)
        && (!beneath || (l.opacity == 1.0 && l.fill_opacity == 1.0 && l.surface().is_some_and(|s| doc.bounds().contains_rect(&s.content_bounds())))))
    .then_some((l, beneath))
}

impl Preview {
    pub(crate) fn new(gpu: &crate::gpu_canvas::GpuCanvas, doc: &Document, layer: Option<LayerId>, fill: [f32; 4]) -> Option<Self> {
        let (l, beneath) = source(doc, layer)?;
        let surf = l.surface()?;
        let selection_bounds = doc.selection.as_ref().map(photocraft_raster::Surface::content_bounds);
        if doc.selection.as_ref().is_some_and(|s| s.default_pixel().first().copied().unwrap_or(0.0) != 0.0)
            || selection_bounds.is_some_and(|b| !doc.bounds().contains_rect(&b))
        {
            // The CPU can lift default-valued selected samples outside the canvas. They are
            // not in this document-sized upload and must not disappear from its preview.
            return None;
        }
        let size = [doc.size.width, doc.size.height];
        let rs = gpu.render_state();
        let limit = rs.device.limits().max_texture_dimension_2d;
        // Source, selection, moved component and display: checked before GPU/CPU allocations.
        let bytes = doc.size.area().checked_mul(40)?;
        if !gpu.health().is_ok()
            || size.contains(&0)
            || size.into_iter().any(|n| n > limit)
            || bytes > gpu.memory_budget().unwrap_or(256 * 1024 * 1024).min(512 * 1024 * 1024)
            || fill.iter().any(|v| !v.is_finite() || v.abs() > 65504.0)
            || !rs.adapter.get_texture_format_features(FLOAT).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return None;
        }
        let pipes = {
            let mut renderer = rs.renderer.write();
            if let Some(p) = renderer.callback_resources.get::<Arc<Pipelines>>() {
                p.clone()
            } else {
                let p = Arc::new(Pipelines::new(&rs.device)?);
                renderer.callback_resources.insert(p.clone());
                p
            }
        };
        let texture = |label, format, usage| {
            rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let input_usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let rgb = texture("channel source", FLOAT, input_usage);
        let selection = texture("channel selection", wgpu::TextureFormat::R32Float, input_usage);
        let steps = match doc.depth {
            SampleType::U8 => 255.0,
            SampleType::U16 => 65535.0,
            SampleType::F32 => 0.0,
        };
        let fill = fill.map(|v| if steps > 0.0 { (v.clamp(0.0, 1.0) * steps + 0.5).floor() / steps } else { v });
        let mut content = [photocraft_geom::Rect::EMPTY; 3];
        let mut lifted = [photocraft_geom::Rect::EMPTY; 3];
        // Upload bounded bands at native precision, once. No RGBA8 conversion of HDR samples.
        for y in (0..size[1]).step_by(32) {
            let rows = (size[1] - y).min(32);
            let r = photocraft_geom::Rect::from_xywh(0, y as i32, size[0], rows);
            let native = surf.try_read_region(r).ok()?;
            let values = match &doc.selection {
                Some(s) => s.try_read_region(r).ok()?,
                None => vec![1.0; (size[0] as usize).checked_mul(rows as usize)?],
            };
            if values.len() != (size[0] as usize).checked_mul(rows as usize)? || values.iter().any(|v| !(0.0..=1.0).contains(v)) {
                return None;
            }
            let channels = surf.channels();
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(native.len().checked_div(channels)?.checked_mul(4)?).ok()?;
            for (i, p) in native.chunks_exact(channels).enumerate() {
                let colors = p.get(..3)?;
                rgba.extend_from_slice(colors);
                let x = (i % size[0] as usize) as i32;
                let y = (i / size[0] as usize) as i32 + y as i32;
                let pixel = photocraft_geom::Rect::new(x, y, x + 1, y + 1);
                let weight = *values.get(i)?;
                let coverage = if steps > 0.0 { (weight * steps + 0.5).floor() / steps } else { weight };
                // Match the extracted scalar plane's encoded default, including F32 signed
                // zero. Cache its bounds during upload rather than scanning while dragging.
                for (c, value) in colors.iter().enumerate() {
                    if value.to_bits() != fill.get(c)?.to_bits() {
                        let bounds = content.get_mut(c)?;
                        *bounds = bounds.union(&pixel);
                    }
                    if selection_bounds.is_some_and(|b| b.contains(x, y)) && (value.to_bits() != 0 || coverage.to_bits() != 0) {
                        let bounds = lifted.get_mut(c)?;
                        *bounds = bounds.union(&pixel);
                    }
                }
                let alpha = p.get(3).copied().unwrap_or(1.0);
                // A fully opaque top raster hides every layer below it. That common unlocked-
                // image stack can use the same single-layer shader without compositing a base.
                if !(0.0..=1.0).contains(&alpha) || (beneath && alpha != 1.0) {
                    return None;
                }
                rgba.push(alpha);
            }
            if rgba.iter().any(|v| !v.is_finite() || v.abs() > 65504.0) {
                return None;
            }
            write_band(rs, &rgb, size[0], y, rows, 4, &rgba);
            write_band(rs, &selection, size[0], y, rows, 1, &values);
        }
        let render_usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let moved = texture("transformed channel", FLOAT, render_usage);
        let output = texture("channel display", OUTPUT, render_usage | wgpu::TextureUsages::COPY_SRC);
        let moved_view = moved.create_view(&Default::default());
        let output_view = output.create_view(&Default::default());
        let uniform = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("channel transform parameters"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let inputs = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("channel transform source"),
            layout: &pipes.inputs,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&rgb.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&selection.create_view(&Default::default())) },
            ],
        });
        let moved = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("channel transform result"),
            layout: &pipes.moved,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&moved_view) }],
        });
        let id = rs.renderer.write().register_native_texture(&rs.device, &output_view, wgpu::FilterMode::Linear);
        let buffer = |label, usage| {
            rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            rs: rs.clone(),
            pipes,
            uniform,
            inputs,
            moved,
            moved_view,
            output_view,
            _output: output,
            id,
            size,
            opacity: l.opacity.clamp(0.0, 1.0) * l.fill_opacity.clamp(0.0, 1.0),
            steps,
            fill,
            content,
            lifted,
            selection: selection_bounds,
            selected: doc.selection.is_some(),
            last: None,
            vertices: buffer("channel warp vertices", wgpu::BufferUsages::VERTEX),
            indices: buffer("channel warp triangles", wgpu::BufferUsages::INDEX),
        })
    }

    pub(crate) fn texture(&mut self, t: &TransformSession, view: &ChannelView, in_color: bool, encode: bool, factor: u32) -> Option<egui::TextureId> {
        self.texture_with_mode(t, view, in_color, encode, factor, None)
    }

    /// The Move tool uses the same cached source and affine shader, but changes only the
    /// channel's original content rectangle. Samples outside it retain their source value.
    pub(crate) fn texture_move(
        &mut self,
        t: &TransformSession,
        view: &ChannelView,
        in_color: bool,
        encode: bool,
        factor: u32,
        bounds: photocraft_geom::Rect,
    ) -> Option<egui::TextureId> {
        if t.warp.is_some() {
            return None;
        }
        self.texture_with_mode(t, view, in_color, encode, factor, Some(bounds))
    }

    fn texture_with_mode(
        &mut self,
        t: &TransformSession,
        view: &ChannelView,
        in_color: bool,
        encode: bool,
        factor: u32,
        move_bounds: Option<photocraft_geom::Rect>,
    ) -> Option<egui::TextureId> {
        let component = t.target.as_ref()?.get("color")?.as_u64()?;
        if component >= 3 {
            return None;
        }
        if self.last.as_ref().is_some_and(|(old, v, c, e, bounds)| old == t && v == view && *c == in_color && *e == encode && *bounds == move_bounds) {
            return Some(self.id);
        }
        let k = f64::from(factor);
        let rect = t.rect.map(|v| v / k);
        let quad = t.quad.map(|p| p.map(|v| v / k));
        let h = photocraft_algo::transform::Homography::rect_to_quad(rect, quad)?;
        let denominators = [[rect[0], rect[1]], [rect[2], rect[1]], [rect[2], rect[3]], [rect[0], rect[3]]].map(|[x, y]| h.0[6] * x + h.0[7] * y + h.0[8]);
        if denominators.iter().any(|v| !v.is_finite() || v.abs() < 1e-12 || v.signum() != denominators[0].signum()) {
            return None;
        }
        let inv = h.inverse()?;
        let frame = photocraft_geom::Rect::new(rect[0].floor() as i32, rect[1].floor() as i32, rect[2].ceil() as i32, rect[3].ceil() as i32);
        let source_bounds = if let Some(bounds) = move_bounds {
            let canvas = photocraft_geom::Rect::from_xywh(0, 0, self.size[0], self.size[1]);
            if self.selected || bounds.is_empty() || !canvas.contains_rect(&bounds) {
                return None;
            }
            bounds
        } else if self.selected {
            if t.warp.is_some() && !frame.contains_rect(&self.selection?) {
                return None;
            }
            *self.lifted.get(component as usize)?
        } else if t.warp.is_none() {
            *self.content.get(component as usize)?
        } else {
            if frame != photocraft_geom::Rect::from_xywh(0, 0, self.size[0], self.size[1]) {
                // A smaller frame clips the vacated region; a larger one lifts default
                // samples outside the upload. Both need the CPU's framed coverage.
                return None;
            }
            frame
        };
        let source_rect = [source_bounds.x0, source_bounds.y0, source_bounds.x1, source_bounds.y1].map(f64::from);
        let mut interp = match t.interpolation.as_str() {
            "nearest" => 0.0,
            "bilinear" => 1.0,
            _ => 2.0,
        };
        // Affine/projective Bicubic now prefilters even mild reductions on the CPU. Warp has
        // its own per-triangle footprint filter below; Bilinear samples the source directly.
        if t.warp.is_none() && interp == 2.0 && h.local_scale((source_rect[0] + source_rect[2]) * 0.5, (source_rect[1] + source_rect[3]) * 0.5) < 0.999 {
            return None;
        }
        let m = h.0;
        let integer_translation = t.warp.is_none()
            && m.iter().all(|v| v.is_finite())
            && [m[0] - 1.0, m[1], m[3], m[4] - 1.0, m[6], m[7], m[8] - 1.0, m[2] - m[2].round(), m[5] - m[5].round()].iter().all(|v| v.abs() < 1e-9);
        // f32 inverse coordinates cannot reproduce the CPU's f64 nearest-neighbour ties under
        // fractional transforms. Preserve those pixel choices via the CPU oracle.
        if interp == 0.0 && !integer_translation {
            return None;
        }
        if integer_translation {
            interp = 0.0;
        }
        let mut data = Vec::with_capacity(32);
        for (i, row) in inv.0.as_chunks::<3>().0.iter().enumerate() {
            data.extend(row.iter().map(|v| *v as f32));
            data.push(if i == 0 { photocraft_geom::TILE_SIZE as f32 } else { 0.0 });
        }
        data.extend([self.size[0] as f32, self.size[1] as f32, component as f32, *self.fill.get(component as usize)?]);
        data.extend(source_rect.map(|v| v as f32));
        let visible = (0..3).fold(0u32, |v, c| v | (u32::from(view.color_visible(c)) << c));
        data.extend([visible as f32, f32::from(in_color), self.opacity, f32::from(encode)]);
        let identity = t.warp.as_ref().map_or_else(|| h.is_identity(), photocraft_geom::warp::Warp::is_identity);
        data.extend([interp, f32::from(self.selected), self.steps, if move_bounds.is_some() { 2.0 } else { f32::from(identity) }]);
        // Perspective edge coverage uses the CPU sampler's destination-tile midpoint.
        let output = if let Some(bounds) = move_bounds {
            bounds
        } else if t.warp.is_none() && interp != 0.0 {
            photocraft_algo::transform::warp_output_bounds(source_bounds, &h).unwrap_or(photocraft_geom::Rect::EMPTY)
        } else {
            photocraft_geom::Rect::EMPTY
        };
        data.extend([output.x0, output.y0, output.x1, output.y1].map(|v| v as f32));
        if data.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let geometry = match &t.warp {
            Some(w) => Some(warp_geometry(w, source_rect, k)?),
            None => None,
        };
        self.rs.queue.write_buffer(&self.uniform, 0, &bytes(&data));
        let mut encoder = self.rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("channel preview") });
        if let Some((vertices, indices)) = geometry {
            let vb = bytes(&vertices);
            let ib: Vec<u8> = indices.iter().flat_map(|v| v.to_le_bytes()).collect();
            if vb.len() > 8 * 1024 * 1024 || ib.len() > 8 * 1024 * 1024 {
                return None;
            }
            for (buffer, size, usage) in [(&mut self.vertices, vb.len(), wgpu::BufferUsages::VERTEX), (&mut self.indices, ib.len(), wgpu::BufferUsages::INDEX)]
            {
                if size as u64 > buffer.size() {
                    *buffer = self.rs.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("channel warp geometry"),
                        size: (size as u64).next_power_of_two(),
                        usage: usage | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                }
            }
            self.rs.queue.write_buffer(&self.vertices, 0, &vb);
            self.rs.queue.write_buffer(&self.indices, 0, &ib);
            {
                let mut pass = render_pass(&mut encoder, &self.moved_view);
                pass.set_pipeline(&self.pipes.warp);
                pass.set_bind_group(0, &self.inputs, &[]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..u32::try_from(indices.len()).ok()?, 0, 0..1);
            }
            let mut pass = render_pass(&mut encoder, &self.output_view);
            pass.set_pipeline(&self.pipes.finish);
            pass.set_bind_group(0, &self.inputs, &[]);
            pass.set_bind_group(1, &self.moved, &[]);
            pass.draw(0..3, 0..1);
        } else {
            let mut pass = render_pass(&mut encoder, &self.output_view);
            pass.set_pipeline(&self.pipes.affine);
            pass.set_bind_group(0, &self.inputs, &[]);
            pass.draw(0..3, 0..1);
        }
        self.rs.queue.submit([encoder.finish()]);
        self.last = Some((t.clone(), view.clone(), in_color, encode, move_bounds));
        Some(self.id)
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.rs.renderer.write().free_texture(&self.id);
    }
}

#[cfg(test)]
impl Preview {
    pub(super) fn readback(&self) -> Vec<egui::Color32> {
        let row = (self.size[0] * 4).div_ceil(256) * 256;
        let buffer = self.rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("channel preview readback"),
            size: u64::from(row) * u64::from(self.size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.rs.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self._output, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(self.size[1]) },
            },
            wgpu::Extent3d { width: self.size[0], height: self.size[1], depth_or_array_layers: 1 },
        );
        self.rs.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(10)) }).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let pixels = data
            .chunks(row as usize)
            .flat_map(|r| r[..self.size[0] as usize * 4].as_chunks::<4>().0.iter().map(|p| egui::Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3])))
            .collect();
        drop(data);
        buffer.unmap();
        pixels
    }
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn write_band(rs: &RenderState, texture: &wgpu::Texture, width: u32, y: u32, rows: u32, channels: u32, values: &[f32]) {
    rs.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y, z: 0 }, aspect: wgpu::TextureAspect::All },
        &bytes(values),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * channels * 4), rows_per_image: Some(rows) },
        wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
    );
}

/// Detect curved outline coverage that changes on the GPU's subpixel raster grid. This
/// visits at most 4096 pixel centres per segment; oversized/degenerate edges use the CPU.
fn rounded_outline_changes_coverage(a: [f64; 2], b: [f64; 2]) -> bool {
    let axis = usize::from((b[1] - a[1]).abs() > (b[0] - a[0]).abs());
    let other = 1 - axis;
    let lo = (a[axis].min(b[axis]) - 0.5).ceil();
    let hi = (a[axis].max(b[axis]) - 0.5).floor();
    let count = hi - lo + 1.0;
    if count <= 0.0 {
        return false;
    }
    if !count.is_finite() || count > 4096.0 || lo.abs().max(hi.abs()) > f64::from(1 << 24) {
        return true;
    }
    let span = b[axis] - a[axis];
    if span.abs() < 1e-12 {
        return true;
    }
    (0..count as usize).any(|i| {
        let at = lo + i as f64 + 0.5;
        let v = a[other] + (at - a[axis]) * (b[other] - a[other]) / span - 0.5;
        // Vulkan guarantees at least four fractional raster bits, not eight. Check
        // 4..16-bit grids and bound finer rounding rather than assuming one adapter.
        // https://docs.vulkan.org/spec/latest/chapters/limits.html#limits-minmax
        let fine_error = 2.0 * (1.0 + ((b[other] - a[other]) / span).abs()) / 65536.0;
        !v.is_finite()
            || (v - v.round()).abs() <= fine_error
            || (4..=16).any(|bits| {
                let scale = f64::from(1 << bits);
                let rounded = |p: [f64; 2]| p.map(|v| (v * scale).round() / scale);
                let (ra, rb) = (rounded(a), rounded(b));
                let rspan = rb[axis] - ra[axis];
                if rspan.abs() < 1e-12 {
                    return true;
                }
                let rv = ra[other] + (at - ra[axis]) * (rb[other] - ra[other]) / rspan - 0.5;
                !rv.is_finite() || v.floor() != rv.floor() || (rv - rv.round()).abs() < 1e-9
            })
    })
}

fn warp_geometry(w: &photocraft_geom::warp::Warp, rect: [f64; 4], factor: f64) -> Option<(Vec<f32>, Vec<u32>)> {
    // Bound cloning, tessellation and index allocation before visiting input-driven knots.
    if !factor.is_finite() || factor <= 0.0 || w.mesh.as_ref().is_some_and(|m| m.us.len() > 65 || m.vs.len() > 65 || m.points.len() > 193 * 193) {
        return None;
    }
    let mesh = w.to_mesh(1, 1);
    if !mesh.is_valid() {
        return None;
    }
    // The scalar CPU oracle lifts an integer source region before tessellation, including
    // the enclosing pixels of fractional proxy frames.
    let bounds = photocraft_geom::Rect::new(rect[0] as i32, rect[1] as i32, rect[2] as i32, rect[3] as i32);
    let affine = mesh.us.len() == 2
        && mesh.vs.len() == 2
        && rect[0] * factor >= w.bounds[0]
        && rect[1] * factor >= w.bounds[1]
        && rect[2] * factor <= w.bounds[2]
        && rect[3] * factor <= w.bounds[3]
        && mesh.points.first().zip(mesh.points.get(3)).zip(mesh.points.get(12)).is_some_and(|((o, x), y)| {
            mesh.points.iter().enumerate().all(|(i, p)| {
                p.iter().zip(o).zip(x).zip(y).all(|(((p, o), x), y)| (p - (o + (x - o) * (i % 4) as f64 / 3.0 + (y - o) * (i / 4) as f64 / 3.0)).abs() < 1e-9)
            })
        });
    let map = |x: f64, y: f64| {
        let (u, v) = w.map(x * factor, y * factor);
        (u / factor, v / factor)
    };
    // An affine Bezier patch has identical sampling through any tessellation. Keep translations,
    // scaling and shear cheap; curved maps share the CPU grid in source and destination pixels.
    let (nx, ny) = if affine { (1, 1) } else { photocraft_algo::warp::mesh_grid(bounds, &map) };
    let us: Vec<f64> = (0..=nx).map(|i| i as f64 / nx as f64).collect();
    let vs: Vec<f64> = (0..=ny).map(|i| i as f64 / ny as f64).collect();
    let mut vertices = Vec::with_capacity(us.len().checked_mul(vs.len())?.checked_mul(4)?);
    let mut mapped = Vec::with_capacity(us.len().checked_mul(vs.len())?);
    for v in &vs {
        for u in &us {
            let src = [rect[0] + u * (rect[2] - rect[0]), rect[1] + v * (rect[3] - rect[1])];
            let dst = map(src[0], src[1]);
            let dst = [dst.0, dst.1];
            if dst.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite()) {
                return None;
            }
            mapped.push(dst);
            vertices.extend([dst[0] as f32, dst[1] as f32, src[0] as f32, src[1] as f32]);
        }
    }
    // Strong reductions/folds need the CPU oracle's footprint filter. Also retain the
    // engine's row-major ordering for the GPU triangles, rather than the overlay's z-order.
    let mut indices = Vec::new();
    for j in 0..vs.len().saturating_sub(1) {
        for i in 0..us.len().saturating_sub(1) {
            let a = u32::try_from(j.checked_mul(us.len())?.checked_add(i)?).ok()?;
            let stride = u32::try_from(us.len()).ok()?;
            let point = |v: u32| -> Option<[f64; 2]> {
                // Decide CPU filtering from unrounded geometry. f32 vertex rounding can
                // otherwise move a near-threshold footprint below the two-sample boundary.
                mapped.get(usize::try_from(v).ok()?).copied()
            };
            let right_index = a.checked_add(1)?;
            let bottom_index = a.checked_add(stride)?;
            let diagonal_index = bottom_index.checked_add(1)?;
            let (origin, right, bottom, diagonal) = (point(a)?, point(right_index)?, point(bottom_index)?, point(diagonal_index)?);
            // CPU triangles include their boundary. GPU top-left rasterization can omit an
            // outer axis-aligned edge through pixel centres, losing a whole colour column.
            // Preserve the CPU coverage for these ties rather than nudging source geometry.
            let through_centres =
                |a: [f64; 2], b: [f64; 2]| (0..2).any(|axis| (a[axis] - b[axis]).abs() < 1e-9 && (a[axis] - 0.5 - (a[axis] - 0.5).round()).abs() < 1e-6);
            let needs_cpu = |a, b| through_centres(a, b) || (!affine && rounded_outline_changes_coverage(a, b));
            if (i == 0 && needs_cpu(origin, bottom))
                || (i + 1 == nx && needs_cpu(right, diagonal))
                || (j == 0 && needs_cpu(origin, right))
                || (j + 1 == ny && needs_cpu(bottom, diagonal))
            {
                return None;
            }
            let dx = (us.get(i + 1)? - us.get(i)?) * (rect[2] - rect[0]);
            let dy = (vs.get(j + 1)? - vs.get(j)?) * (rect[3] - rect[1]);
            if dx <= 0.0 || dy <= 0.0 {
                return None;
            }
            // Check both triangles: a pulled fourth corner can fold just one of them.
            for (x0, x1, y0, y1) in [(origin, right, right, diagonal), (bottom, diagonal, origin, bottom)] {
                let x = [(x1[0] - x0[0]) / dx, (x1[1] - x0[1]) / dx];
                let y = [(y1[0] - y0[0]) / dy, (y1[1] - y0[1]) / dy];
                let determinant = x[0] * y[1] - x[1] * y[0];
                // warp_triangles starts bilinear footprint averaging when either inverse
                // axis length rounds to two samples. A single GPU sample must not alias it.
                let footprint_x = y[1].hypot(x[1]) / determinant;
                let footprint_y = y[0].hypot(x[0]) / determinant;
                if !determinant.is_finite() || determinant <= 0.0 || footprint_x >= 1.5 - 1e-6 || footprint_y >= 1.5 - 1e-6 {
                    return None;
                }
            }
            indices.extend([a, right_index, diagonal_index, a, diagonal_index, bottom_index]);
        }
    }
    Some((vertices, indices))
}

fn render_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &wgpu::TextureView) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("channel preview"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Option<Self> {
        use std::future::Future;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let result = Self::build(device);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        for pop in [internal.pop(), validation.pop()] {
            if let std::task::Poll::Ready(Some(e)) = std::pin::pin!(pop).poll(&mut cx) {
                log::warn!("channel preview uses CPU: {e}");
                return None;
            }
        }
        Some(result)
    }

    fn build(device: &wgpu::Device) -> Self {
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let inputs = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("channel preview inputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(UNIFORM_BYTES),
                    },
                    count: None,
                },
                tex(1),
                tex(2),
            ],
        });
        let moved = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("channel preview moved"), entries: &[tex(0)] });
        let simple = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("channel preview"),
            bind_group_layouts: &[Some(&inputs)],
            immediate_size: 0,
        });
        let finish_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("channel preview finish"),
            bind_group_layouts: &[Some(&inputs), Some(&moved)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("channel preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("channel_transform_gpu.wgsl").into()),
        });
        let pipe = |layout: &wgpu::PipelineLayout, vertex, fragment, format, mesh| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("channel preview"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    buffers: if mesh {
                        &[Some(wgpu::VertexBufferLayout {
                            array_stride: 16,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 }],
                        })]
                    } else {
                        &[]
                    },
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let affine = pipe(&simple, "vs_full", "fs_affine", OUTPUT, false);
        let warp = pipe(&simple, "vs_mesh", "fs_warp", FLOAT, true);
        let finish = pipe(&finish_layout, "vs_full", "fs_finish", OUTPUT, false);
        Self { inputs, moved, affine, warp, finish }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::{Layer, LayerMask, Size};
    use photocraft_geom::warp::{BezierMesh, Warp};

    #[test]
    fn unsupported_stacks_keep_the_cpu_oracle() {
        let doc = Document::with_background("eligibility", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let id = doc.layers[0].id;
        assert!(source(&doc, Some(id)).is_some());
        let mut opaque_stack = doc.clone();
        let mut top = Layer::raster("opaque top", doc.pixel_format());
        top.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.4, 0.6, 1.0]);
        let top_id = top.id;
        opaque_stack.layers.push(top);
        assert!(source(&opaque_stack, Some(top_id)).is_some(), "an opaque top layer can hide its Background");
        assert!(source(&opaque_stack, Some(id)).is_none(), "a covered lower layer cannot use the one-layer shader");
        opaque_stack.layers.last_mut().unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-1, 0, 0, 1), &[0.1, 0.2, 0.3, 1.0]);
        assert!(source(&opaque_stack, Some(top_id)).is_none(), "a moved off-canvas source needs the CPU preview");
        for case in 0..10 {
            let mut d = doc.clone();
            match case {
                0 => d.mode = ColorMode::Cmyk,
                1 => d.layers[0].mask = Some(LayerMask::reveal_all()),
                2 => d.layers[0].clipped = true,
                3 => d.layers[0].blend = BlendMode::Multiply,
                4 => d.layers[0].excluded_channels = 1,
                5 => d.layers[0].opacity = f32::NAN,
                6 => d.layers[0].visible = false,
                7 => d.layers.push(doc.layers[0].clone()),
                8 => d.layers[0].opacity = -1.0,
                9 => d.layers[0].fill_opacity = 2.0,
                _ => {}
            }
            assert!(source(&d, Some(id)).is_none(), "case={case}");
        }
        assert!(source(&doc, None).is_none());
    }

    #[test]
    fn invalid_or_strongly_reduced_warps_retain_the_cpu_filter() {
        let rect = [0.0, 0.0, 32.0, 24.0];
        let m = BezierMesh::identity(rect, 1, 1);
        assert!(warp_geometry(&Warp::custom(m.clone(), rect), rect, 1.0).is_some());
        for scale in [-1.0, 0.0, 0.2, 0.55, 0.6, 0.65, 2.0 / 3.0, f64::NAN, f64::INFINITY] {
            let w = Warp::custom(m.map_points(|p| [p[0] * scale, p[1]]), rect);
            assert!(warp_geometry(&w, rect, 1.0).is_none(), "scale={scale}");
        }
        let w = Warp::custom(m.map_points(|p| [p[0] * 0.75, p[1] * 0.75]), rect);
        assert!(warp_geometry(&w, rect, 1.0).is_some());
        let w = Warp::custom(m.map_points(|p| [100_000.0 + p[0] * 0.66666, p[1]]), rect);
        assert!(warp_geometry(&w, rect, 1.0).is_none(), "rounding vertices must not bypass the CPU footprint filter");
        let mut folded = m.clone();
        *folded.points.last_mut().unwrap() = [0.0, 0.0];
        assert!(warp_geometry(&Warp::custom(folded, rect), rect, 1.0).is_none());
        let w = Warp::custom(BezierMesh { us: vec![], vs: vec![], points: vec![] }, rect);
        assert!(warp_geometry(&w, rect, 1.0).is_none());
        // The CPU covers pixel (27, 0), but rounding this curved top edge to the GPU
        // subpixel grid moves it just below that pixel's centre.
        assert!(rounded_outline_changes_coverage([24.88888888888889, 0.4240329218106997], [28.444444444444443, 0.5267489711934157]));
        assert!(
            rounded_outline_changes_coverage([21.57758620689655, 43.97146254458978], [24.68103448275862, 45.68370986920333]),
            "coarser raster grids also need the CPU coverage"
        );
        assert!(!rounded_outline_changes_coverage([0.0, 0.0], [4.0, 0.0]));
        assert!(rounded_outline_changes_coverage([0.0, 0.0], [10000.0, 1.0]), "outline scans stay bounded");
    }
}
