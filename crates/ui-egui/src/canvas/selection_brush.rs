//! Selection Brush overlay. Rasterise only new samples and upload their dirty rectangle.
//! A document revision or brush-setting change rebuilds the base; overlay tint changes do not.
use super::view_xform::ViewXform;
use crate::{PhotocraftApp, state::Tool};
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use photocraft_algo::selection::brush::{Options, StrokeMask};
use photocraft_geom::Rect;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, PartialEq)]
struct Key {
    revision: u64,
    size: [u32; 2],
    factor: u32,
    options: Options,
    subtract: bool,
    start: Option<[u64; 2]>,
}
struct Cache {
    key: Key,
    texture: TextureHandle,
    stroke: Option<StrokeMask>,
    fed: usize,
    dirty: Rect,
    failed: bool,
    needs_base: bool,
}

/// Sample live and committed coverage at identical document coordinates.
fn image(app: &PhotocraftApp, idx: usize, cache: &Cache, area: Rect) -> ColorImage {
    let Some(st) = app.session.documents().get(idx) else { return ColorImage::new([0, 0], Vec::new()) };
    let mut pixels = Vec::with_capacity(area.size().area() as usize);
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let dx = (i64::from(x) * i64::from(cache.key.factor)).min(i64::from(i32::MAX)) as i32;
            let dy = (i64::from(y) * i64::from(cache.key.factor)).min(i64::from(i32::MAX)) as i32;
            let old = st.doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(dx, dy, 0));
            let new = cache.stroke.as_ref().map_or(0.0, |s| s.mask().sample_channel(dx, dy, 0));
            let value = if cache.key.subtract { (old - new).max(0.0) } else { old.max(new) };
            pixels.push(Color32::from_white_alpha((value * 255.0).round() as u8));
        }
    }
    ColorImage::new([area.width() as usize, area.height() as usize], pixels)
}

/// Maintain the document's display texture and upload damaged coverage rectangles.
pub(super) fn paint(app: &PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, idx: usize) {
    if app.ui.tool != Tool::SelectionBrush {
        return;
    }
    let Some(st) = app.session.documents().get(idx) else { return };
    let settings = &app.ui.tool_options.selection_brush;
    let fallback = crate::state::SelectionBrushOptions::default();
    let settings = if settings.valid() { settings } else { &fallback };
    let drag = app.drag.as_ref().filter(|d| d.tool == Tool::SelectionBrush && d.selection_brush_valid(app) && app.session.active_index() == Some(idx));
    let max_side = ctx.input(|i| i.max_texture_side).clamp(1, 1024) as u32;
    let factor = st.doc.size.width.max(st.doc.size.height).div_ceil(max_side).max(1);
    let (w, h) = (st.doc.size.width.div_ceil(factor).max(1), st.doc.size.height.div_ceil(factor).max(1));
    let bounds = Rect::new(0, 0, w as i32, h as i32);
    let brush = &app.session.tools.brush;
    let key = Key {
        revision: st.revision,
        size: [st.doc.size.width, st.doc.size.height],
        factor,
        // Rasterize in document coordinates, exactly like the command. Scaling the geometry
        // first also scales antialiasing and pixel centres, changing the outline on release.
        options: Options { size: brush.size, hardness: brush.hardness, opacity: brush.opacity, fill_closed: false },
        subtract: settings.subtracts(drag.is_some_and(|d| d.modifiers.alt)),
        start: drag.map(|d| [d.start[0].to_bits(), d.start[1].to_bits()]),
    };
    let id = egui::Id::unique(("selection-brush-overlay", st.doc.id.0));
    let cached = ctx.data(|d| d.get_temp::<Arc<Mutex<Cache>>>(id));
    let shared = cached.unwrap_or_else(|| {
        let texture = ctx.load_texture(
            "Selection Brush coverage",
            ColorImage::new([w as usize, h as usize], vec![Color32::TRANSPARENT; w as usize * h as usize]),
            TextureOptions::LINEAR,
        );
        let cache = Arc::new(Mutex::new(Cache { key: key.clone(), texture, stroke: None, fed: 0, dirty: Rect::EMPTY, failed: false, needs_base: true }));
        ctx.data_mut(|d| d.insert_temp(id, cache.clone()));
        cache
    });
    let mut cache = shared.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.texture.size() != [w as usize, h as usize] {
        cache.texture = ctx.load_texture(
            "Selection Brush coverage",
            ColorImage::new([w as usize, h as usize], vec![Color32::TRANSPARENT; w as usize * h as usize]),
            TextureOptions::LINEAR,
        );
        cache.needs_base = true;
    }
    let mut dirty = if cache.needs_base { bounds } else { Rect::EMPTY };
    cache.needs_base = false;
    if cache.key != key {
        cache.key = key;
        cache.stroke = None;
        cache.fed = 0;
        cache.failed = false;
        cache.dirty = Rect::EMPTY;
        dirty = bounds;
    }
    if let Some(drag) = drag {
        if cache.fed > drag.points.len() {
            cache.stroke = None;
            cache.fed = 0;
            dirty = cache.dirty;
            cache.failed = false;
        }
        if cache.stroke.is_none() && !cache.failed {
            match StrokeMask::new(cache.key.options, st.doc.bounds()) {
                Ok(stroke) => cache.stroke = Some(stroke),
                Err(_) => cache.failed = true,
            }
        }
        if cache.fed < drag.points.len() && !cache.failed {
            let points: Vec<[f64; 2]> = drag.points.get(cache.fed..).unwrap_or_default().iter().map(|p| [p[0], p[1]]).collect();
            if let Some(stroke) = cache.stroke.as_mut() {
                match stroke.append(&points) {
                    Ok(rect) => {
                        let k = factor as i32;
                        let rect = Rect::new(rect.x0 / k, rect.y0 / k, rect.x1.saturating_add(k - 1) / k, rect.y1.saturating_add(k - 1) / k).intersect(&bounds);
                        dirty = dirty.union(&rect);
                        cache.dirty = cache.dirty.union(&rect);
                    }
                    Err(_) => cache.failed = true,
                }
            }
            cache.fed = drag.points.len();
        }
    } else if cache.stroke.is_some() || cache.fed > 0 {
        dirty = dirty.union(&cache.dirty);
        cache.stroke = None;
        cache.fed = 0;
        cache.failed = false;
        cache.dirty = Rect::EMPTY;
    }
    if !dirty.is_empty() {
        let pixels = image(app, idx, &cache, dirty);
        cache.texture.set_partial([dirty.x0 as usize, dirty.y0 as usize], pixels, TextureOptions::LINEAR);
    }
    super::paint::paint_mapped_image_at(
        painter,
        xf,
        cache.texture.id(),
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        [0.0, 0.0],
        [st.doc.size.width as f32, st.doc.size.height as f32],
        crate::theme::Tokens::get(ctx).selection_brush_overlay(settings.overlay_color, settings.overlay_opacity),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;
    /// Resize, rotate and undo/redo must replace the texture rather than upload outside its old extent.
    #[test]
    fn selection_brush_texture_tracks_document_dimensions() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":800,"height":600})).unwrap();
        app.ui.tool = Tool::SelectionBrush;
        let doc = app.session.active().unwrap().doc.id;
        let ctx = egui::Context::default();
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 800.0)),
            zoom: 1.0,
            center: [400.0, 300.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let mut previous = None;
        for command in [
            None,
            Some(("image.rotation.arbitrary", json!({"angle":90}))),
            Some(("image.canvasSize", json!({"width":400,"height":300}))),
            Some(("edit.undo", json!({}))),
            Some(("edit.redo", json!({}))),
            Some(("image.canvasSize", json!({"width":1500,"height":900}))),
        ] {
            if let Some((id, p)) = command {
                app.run(id, p).unwrap();
            }
            assert_eq!(app.session.active().unwrap().doc.id, doc);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| paint(&app, ui.ctx(), ui.painter(), &xf, 0));
            let cache = ctx.data(|d| d.get_temp::<Arc<Mutex<Cache>>>(egui::Id::unique(("selection-brush-overlay", doc.0)))).unwrap();
            let cache = cache.lock().unwrap();
            let size = app.session.active().unwrap().doc.size;
            let factor = size.width.max(size.height).div_ceil(1024).max(1);
            let expected = [size.width.div_ceil(factor) as usize, size.height.div_ceil(factor) as usize];
            assert_eq!(cache.texture.size(), expected);
            if let Some(old) = previous {
                assert_ne!(old, cache.texture.id());
            }
            assert!(output.textures_delta.set.values().flatten().any(|delta| delta.pos.is_none() && delta.image.size() == expected));
            previous = Some(cache.texture.id());
            output.textures_delta.clear();
        }
    }

    #[test]
    /// Compare live and committed coverage, preserving only the intentional closed interior change.
    fn selection_brush_preview_matches_commit_at_document_and_reduced_scales() {
        for width in [800, 3877] {
            for closed in [false, true] {
                for subtract in [false, true] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.run("file.new", json!({"width":width,"height":if width == 800 {800} else {2161}})).unwrap();
                    if subtract {
                        app.run("select.all", json!({})).unwrap();
                    }
                    app.ui.tool = Tool::SelectionBrush;
                    crate::paint_mouse::sync_tool_brush(&mut app);
                    app.run("tools.setBrush", json!({"brush":{"size":41,"hardness":0.4,"opacity":0.65}})).unwrap();
                    app.ui.tool_options.selection_brush.subtract = subtract;
                    let ctx = egui::Context::default();
                    let xf = ViewXform {
                        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 600.0)),
                        zoom: 0.25,
                        center: [400.0, 1080.0],
                        flip: false,
                        rotation: 0.0,
                        aspect: 1.0,
                    };
                    let capture = |app: &PhotocraftApp| {
                        ctx.run_ui(egui::RawInput::default(), |ui| paint(app, ui.ctx(), ui.painter(), &xf, 0)).textures_delta.clear();
                        let st = app.session.active().unwrap();
                        let id = egui::Id::unique(("selection-brush-overlay", st.doc.id.0));
                        let shared = ctx.data(|d| d.get_temp::<Arc<Mutex<Cache>>>(id)).unwrap();
                        let cache = shared.lock().unwrap();
                        let k = cache.key.factor;
                        let bounds = Rect::new(0, 0, st.doc.size.width.div_ceil(k) as i32, st.doc.size.height.div_ceil(k) as i32);
                        (image(app, 0, &cache, bounds), k)
                    };
                    let mut points = vec![[100.25, 100.75], [600.25, 100.75], [600.25, 700.75], [100.25, 700.75]];
                    if closed {
                        points.push(points[0]);
                    }
                    for (i, p) in points.iter().enumerate() {
                        let event =
                            if i == 0 { ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 } } else { ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 } };
                        tool_event(&mut app, event, egui::Modifiers::NONE);
                        capture(&app);
                    }
                    let (before, k) = capture(&app);
                    let p = points.last().unwrap();
                    tool_event(&mut app, ToolEvent::Up { x: p[0], y: p[1] }, egui::Modifiers::NONE);
                    let (after, _) = capture(&app);
                    assert!(!app.ui.status_error, "{}", app.ui.status);
                    for (i, (a, b)) in before.pixels.iter().zip(&after.pixels).enumerate() {
                        let x = (i % before.size[0]) as f64 * f64::from(k);
                        let y = (i / before.size[0]) as f64 * f64::from(k);
                        if !closed || !(100.25..600.25).contains(&x) || !(100.75..700.75).contains(&y) {
                            assert_eq!(a, b, "outline changed: width={width}, closed={closed}, subtract={subtract}, at {x},{y}");
                        }
                    }
                    if closed {
                        assert_ne!(before.pixels, after.pixels, "closed interior must still fill on release");
                    }
                }
            }
        }
    }
    #[test]
    /// Ensure new segments use partial uploads and idle or tint-only frames upload nothing.
    fn selection_brush_drag_uploads_only_the_dirty_rectangle_and_idle_frames_upload_nothing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":3840,"height":2160})).unwrap();
        app.ui.tool = Tool::SelectionBrush;
        app.run("tools.setBrush", json!({"brush":{"size":60,"hardness":0.7,"opacity":0.5}})).unwrap();
        let ctx = egui::Context::default();
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 600.0)),
            zoom: 0.25,
            center: [1920.0, 1080.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let frame = |app: &PhotocraftApp| ctx.run_ui(egui::RawInput::default(), |ui| paint(app, ui.ctx(), ui.painter(), &xf, 0));
        frame(&app).textures_delta.clear();
        tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
        frame(&app).textures_delta.clear();
        tool_event(&mut app, ToolEvent::Move { x: 220.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let mut changed = frame(&app);
        let patches: Vec<_> = changed.textures_delta.set.values().flat_map(|deltas| deltas.iter()).filter(|delta| delta.pos.is_some()).collect();
        assert!(!patches.is_empty());
        assert!(patches.iter().all(|delta| delta.image.size().iter().product::<usize>() < 1024 * 540 / 10));
        changed.textures_delta.clear();
        assert!(frame(&app).textures_delta.set.is_empty());
        app.ui.tool_options.selection_brush.overlay_color = [1.0, 0.0, 1.0];
        app.ui.tool_options.selection_brush.overlay_opacity = 60.0;
        assert!(frame(&app).textures_delta.set.is_empty());
    }
}
