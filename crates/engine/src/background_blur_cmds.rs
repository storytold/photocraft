//! Blur the background of one pixel layer while preserving the detected subject. Subject
//! coverage is temporary: the user's selection, layer masks and alpha are left intact.

use photocraft_algo::{
    FilterParams,
    segment::{Region, subject},
};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

const CMD: &str = "layer.blurBackground";

fn check(doc: &Document, layer: &Layer) -> std::result::Result<(), String> {
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err("Blur Background needs a pixel layer".into());
    }
    let locks = doc.effective_locks(layer.id);
    if locks.pixels || locks.all {
        return Err(format!("Could not complete your request because the layer \"{}\" is locked", layer.name));
    }
    Ok(())
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    check(&d.doc, crate::active_layer_of(s)?)
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: CMD.into(), msg: msg.into() };
    if !p.is_object() {
        return Err(bad("parameters must be an object"));
    }
    let radius = match p.get("radius") {
        None => 8.0,
        Some(v) => v.as_f64().filter(|r| r.is_finite() && (0.1..=1000.0).contains(r)).ok_or_else(|| bad("radius must be a number from 0.1 to 1000"))? as f32,
    };
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    check(doc, doc.layer(id).ok_or(EngineError::NoLayer(id))?).map_err(EngineError::Other)?;
    crate::jobs::edit_job(
        s,
        "Blur Background",
        move |doc, _, ctx| {
            let found = ctx
                .stage(0.0, 0.45, "Finding subject", |ctl| {
                    ctl.check()?;
                    crate::smartselect_cmds::with_doc_sampler(doc, Some(id), false, |sampler, d| subject::select_subject_with(sampler, d.bounds(), ctl))
                })
                .map_err(|_| EngineError::Cancelled)?;
            let subject = found.ok_or_else(|| EngineError::Other("no subject found; the layer was not changed".into()))?;
            let source = doc.layer(id).and_then(Layer::surface).ok_or(EngineError::NoLayer(id))?;
            let blurred = blur_background(source, &subject, doc.selection.as_ref(), doc.bounds(), radius, ctx)?;
            ctx.check()?;
            *crate::pixels_mut(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?)? = blurred;
            Ok(())
        },
        move |()| json!({"layer": id.0, "radius": radius}),
    )
}

fn blur_background(source: &Surface, subject: &Region, selection: Option<&Surface>, bounds: Rect, radius: f32, ctx: &JobCtx) -> Result<Surface> {
    let format = source.format();
    let colors = format.mode.color_channels();
    // Gaussian Blur premultiplies, blurs, then divides by alpha. Excluding subject coverage
    // from this scratch alpha prevents fully selected foreground colours bleeding outward.
    // Keep fractional coverage in float even for 8-bit documents.
    let scratch_format = PixelFormat::new(format.mode, SampleType::F32, true);
    let mut scratch = Surface::new(scratch_format);
    ctx.progress(0.45, "Preparing background");
    for tile in bounds.tiles() {
        ctx.check()?;
        let area = tile.rect().intersect(&bounds);
        let pixels = crate::allocation::read_region(source, area, "Blur Background")?;
        let count = pixels.len() / source.channels();
        let mut data = crate::allocation::capacity(count * scratch_format.channels(), "Blur Background")?;
        for (i, pixel) in pixels.chunks_exact(source.channels()).enumerate() {
            let (x, y) = (area.x0 + (i % area.width() as usize) as i32, area.y0 + (i / area.width() as usize) as i32);
            data.extend(pixel.iter().take(colors));
            let alpha = if format.alpha { pixel.last().copied().unwrap_or(0.0).clamp(0.0, 1.0) } else { 1.0 };
            data.push(alpha * (1.0 - subject.at(x, y)));
        }
        scratch.try_write_region(area, &data)?;
    }
    let filtered = ctx
        .stage(0.5, 0.9, "Blurring background", |ctl| {
            photocraft_algo::apply_in_with(&scratch, &FilterParams::GaussianBlur { radius }, bounds, bounds, None, bounds, ctl)
        })
        .ok_or(EngineError::Cancelled)?;
    // Sample freely across the background, then restrict only the result to the user's
    // selection. The original layer remains the source for alpha and untouched pixels.
    let mut output = source.clone();
    ctx.progress(0.9, "Applying background blur");
    for tile in bounds.tiles() {
        ctx.check()?;
        let area = tile.rect().intersect(&bounds);
        let mut pixels = crate::allocation::read_region(source, area, "Blur Background")?;
        let blurred = crate::allocation::read_region(&filtered, area, "Blur Background")?;
        let mut changed = false;
        for (i, (pixel, blurred)) in pixels.chunks_exact_mut(source.channels()).zip(blurred.chunks_exact(scratch_format.channels())).enumerate() {
            let (x, y) = (area.x0 + (i % area.width() as usize) as i32, area.y0 + (i / area.width() as usize) as i32);
            let weight = (1.0 - subject.at(x, y)) * selection.map_or(1.0, |mask| mask.sample_channel(x, y, 0).clamp(0.0, 1.0));
            if weight <= 0.0 || blurred.last().copied().unwrap_or(0.0) <= 1e-7 || (format.alpha && pixel.last().copied().unwrap_or(0.0) <= 0.0) {
                continue;
            }
            for (original, blurred) in pixel.iter_mut().zip(blurred).take(colors) {
                let value = *original * (1.0 - weight) + blurred * weight;
                changed |= value != *original;
                *original = value;
            }
        }
        if changed {
            output.try_write_region(area, &pixels)?;
        }
    }
    ctx.check()?;
    Ok(output)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Blur Background",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id|name?,"radius":0.1..1000=8} → {layer,radius}. Detect a subject on the pixel layer and blur its background within the current selection; preserve selection, masks and alpha. One undo step. No detected subject leaves the document unchanged."##,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::segment::Rng;
    use photocraft_doc::{LayerId, LayerMask};
    use std::sync::Arc;

    fn disc(width: u32, height: u32, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": width, "height": height, "depth": depth})).unwrap();
        s.edit("test photo", |doc, _| {
            let area = doc.bounds();
            let surface = doc.layers[0].surface_mut().unwrap();
            let mut rng = Rng::new(2);
            for tile in area.tiles() {
                let r = tile.rect().intersect(&area);
                let mut pixels = Vec::new();
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        let (dx, dy) = (x as f32 + 0.5 - width as f32 * 0.5, y as f32 + 0.5 - height as f32 * 0.5);
                        let noise = 0.05 * rng.normal();
                        let inside = dx * dx + dy * dy <= (height as f32 * 0.28).powi(2);
                        let color = if inside {
                            let texture = ((x + y) as f32 * 0.35).sin() * 0.5 + 0.5;
                            [0.85 + 0.05 * texture + noise, 0.2 + 0.3 * texture + noise, 0.15 + noise]
                        } else {
                            [0.25 + noise, 0.45 + noise, 0.7 + noise]
                        };
                        pixels.extend(color.map(|v| v.clamp(0.0, 1.0)));
                        if surface.format().alpha {
                            pixels.push(1.0);
                        }
                    }
                }
                surface.write_region(r, &pixels);
            }
            Ok(())
        })
        .unwrap();
        s
    }

    fn id(s: &Session) -> LayerId {
        s.active().unwrap().active_layer.unwrap()
    }

    fn surface(s: &Session, layer: LayerId) -> &Surface {
        s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap()
    }

    fn background_energy(surface: &Surface) -> f32 {
        (10..40).flat_map(|y| (10..40).map(move |x| (x, y))).map(|(x, y)| (surface.sample_channel(x, y, 0) - 0.25).powi(2)).sum::<f32>() / 900.0
    }

    #[test]
    fn blur_background_preserves_subject_and_undoes_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = disc(200, 160, depth);
            let layer = id(&s);
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            assert!(s.is_enabled(CMD));
            // Exercise the shipping action, including its command params and single-step undo.
            s.execute("actions.loadDefaults", json!({})).unwrap();
            let result = s.execute("actions.play", json!({"action": "Blur Background"})).unwrap();
            assert_eq!(result["ran"], 1, "{depth}: {result}");
            assert!(result.get("failed").is_none(), "{depth}: {result}");
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            let old = before.layer(layer).unwrap().surface().unwrap();
            let new = surface(&s, layer);
            assert_eq!(
                new.read_region(Rect::new(85, 65, 115, 95)),
                old.read_region(Rect::new(85, 65, 115, 95)),
                "the subject interior remains sharp at {depth}"
            );
            assert!(background_energy(new) < background_energy(old) * 0.1, "background noise is reduced at {depth}");
            let after = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, after);
        }
    }

    #[test]
    fn blur_background_preserves_selection_alpha_masks_and_other_layers() {
        let mut s = disc(200, 160, 16);
        let layer = id(&s);
        s.execute("layer.new.layer", json!({"name": "Unchanged overlay"})).unwrap();
        s.edit("existing state", |doc, _| {
            let layer = doc.layer_mut(layer).unwrap();
            let surf = layer.surface_mut().unwrap();
            for y in 0..160 {
                for x in 0..200 {
                    let mut pixel = surf.pixel(x, y);
                    *pixel.last_mut().unwrap() = if x < 50 { 0.5 } else { 1.0 };
                    surf.write_pixel(x, y, &pixel);
                }
            }
            // Off-canvas pixels must survive the canvas-bounded edit too.
            surf.fill_rect(Rect::new(-8, -8, 0, 0), &[0.1, 0.2, 0.3, 0.4]);
            layer.mask = Some(LayerMask { density: 0.7, feather: 2.0, ..LayerMask::reveal_all() });
            layer.opacity = 0.6;
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.fill_rect(Rect::new(0, 0, 60, 160), &[0.5]);
            doc.selection = Some(selection);
            Ok(())
        })
        .unwrap();
        // A mask target must not turn this layer operation into a mask filter.
        s.active_mut().unwrap().active_layer = Some(layer);
        s.active_mut().unwrap().channel_view.target = crate::channel_cmds::ChannelTarget::LayerMask;
        let before = s.active().unwrap().doc.clone();
        s.execute(CMD, json!({"radius": 4})).unwrap();
        let mut expected = (*before).clone();
        expected.layer_mut(layer).unwrap().content = s.active().unwrap().doc.layer(layer).unwrap().content.clone();
        assert_eq!(*s.active().unwrap().doc, expected, "only target layer pixels may change");
        let (old, new) = (before.layer(layer).unwrap().surface().unwrap(), surface(&s, layer));
        assert_ne!(new.read_region(Rect::new(10, 10, 40, 40)), old.read_region(Rect::new(10, 10, 40, 40)));
        assert_eq!(new.read_region(Rect::new(60, 0, 200, 160)), old.read_region(Rect::new(60, 0, 200, 160)), "outside the user's selection");
        assert_eq!(new.read_region(Rect::new(-8, -8, 0, 0)), old.read_region(Rect::new(-8, -8, 0, 0)));
        for y in 0..160 {
            for x in 0..200 {
                assert_eq!(new.sample_channel(x, y, 3), old.sample_channel(x, y, 3));
            }
        }
        assert_eq!(s.active().unwrap().channel_view.target, crate::channel_cmds::ChannelTarget::LayerMask);
    }

    #[test]
    fn background_sampling_excludes_subject_colors_for_both_blur_paths() {
        let bounds = Rect::new(0, 0, 80, 60);
        let subject = Region { bbox: Rect::new(30, 10, 50, 50), mask: vec![255; 20 * 40] };
        use photocraft_color::ColorMode;
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut source = Surface::new(PixelFormat::new(mode, depth, true));
                let colors = mode.color_channels();
                let mut background = vec![0.2; colors];
                let mut foreground = vec![0.9; colors];
                if mode == ColorMode::Rgb {
                    background = vec![0.0, 0.3, 0.85];
                    foreground = vec![1.0, 0.0, 0.0];
                }
                background.push(0.5);
                foreground.push(1.0);
                source.fill_rect(bounds, &background);
                source.fill_rect(subject.bbox, &foreground);
                for radius in [3.0, 8.0] {
                    let output = blur_background(&source, &subject, None, bounds, radius, &JobCtx::new()).unwrap();
                    assert_eq!(output.format(), source.format());
                    let edge = output.pixel(29, 30);
                    for (c, value) in edge.iter().take(colors).enumerate() {
                        assert!(
                            (value - source.sample_channel(29, 30, c)).abs() < 0.005,
                            "foreground leaked into {mode:?} channel {c}, {depth:?}, radius {radius}: {edge:?}"
                        );
                    }
                    if mode == ColorMode::Rgb {
                        assert!(edge[0] < 1e-6, "subject red leaked through at {depth:?}, radius {radius}: {edge:?}");
                    }
                    assert_eq!(output.read_region(subject.bbox), source.read_region(subject.bbox));
                    assert_eq!(edge[colors], source.sample_channel(29, 30, colors));
                }
            }
        }
        let mut tiny_alpha = Surface::new(PixelFormat::RGBA32F);
        tiny_alpha.fill_rect(bounds, &[0.2, 0.3, 0.4, 1e-9]);
        let output = blur_background(&tiny_alpha, &subject, None, bounds, 8.0, &JobCtx::new()).unwrap();
        assert_eq!(output, tiny_alpha, "no usable background contribution must not turn into black");
    }

    #[test]
    fn blur_background_without_subject_or_when_cancelled_changes_nothing() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 16, "height": 16})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        assert!(s.execute(CMD, json!({})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(steps, s.active().unwrap().history.past_len());

        let mut s = disc(200, 160, 8);
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        let ctx = JobCtx::new();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute(CMD, json!({})), Err(EngineError::Cancelled)));
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(steps, s.active().unwrap().history.past_len());
    }

    #[test]
    fn blur_background_rejects_invalid_targets_params_and_effective_locks() {
        let mut empty = Session::new();
        assert!(!empty.is_enabled(CMD));
        assert!(empty.execute(CMD, json!({})).is_err());
        let mut s = disc(200, 160, 8);
        let before = s.active().unwrap().doc.clone();
        for params in [
            Value::Null,
            json!([]),
            json!(false),
            json!({"radius": "8"}),
            json!({"radius": null}),
            json!({"radius": 0}),
            json!({"radius": -1}),
            json!({"radius": 1001}),
            json!({"radius": 1e100}),
            json!({"layer": false}),
            json!({"layer": -1}),
            json!({"layer": 1.5}),
            json!({"layer": "missing"}),
            json!({"layer": 999_999}),
        ] {
            assert!(s.execute(CMD, params.clone()).is_err(), "{params}");
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        }
        s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
        assert!(!s.is_enabled(CMD));
        assert!(s.execute(CMD, json!({})).is_err());
        s.execute("layer.setProps", json!({"locks": {"pixels": false}})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        // The group is not a raster target; a locked parent also protects the member's pixels.
        assert!(!s.is_enabled(CMD));
        assert!(s.execute(CMD, json!({})).is_err());
        let group = id(&s);
        s.execute("layer.setProps", json!({"locks": {"all": true}})).unwrap();
        let child = s.active().unwrap().doc.layer(group).unwrap().children().unwrap().first().unwrap().id;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(CMD, json!({"layer": child.0})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }

    #[test]
    #[ignore = "release-only 24 MP background blur timing; run explicitly"]
    fn blur_background_24mp_timing() {
        let mut s = disc(6000, 4000, 8);
        let started = std::time::Instant::now();
        let result = s.execute(CMD, json!({})).unwrap();
        println!("Blur Background 24 MP: {:?}; {result}", started.elapsed());
    }
}
