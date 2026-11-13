//! Select a subject and replace it with its surroundings in one cancellable edit.

use photocraft_algo::segment::{self, Region};
use photocraft_algo::selection;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_ml::{MlError, Rgb};
use photocraft_paint::retouch::Region as Pixels;
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::jobs::JobCtx;
use crate::smartselect_cmds::find_subject_region;
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.removeSubject";
const LABEL: &str = "Remove Subject";
// Bound dense buffers before sampling, including sparse or hostile documents.
const MAX_CANVAS: usize = 64 << 20;
const MAX_WINDOW: usize = 16 << 20;
const SEED: u64 = 0x0052_e30e;

#[cfg(test)]
thread_local! {
    static MODEL_FILL_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn check(doc: &Document, layer: &Layer) -> std::result::Result<(), String> {
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(format!("the layer is {} {} layer, not a pixel layer", layer.content.article(), layer.content.kind_name()));
    }
    let locks = doc.effective_locks(layer.id);
    if locks.pixels || locks.all {
        return Err(format!("Could not complete your request because the layer \"{}\" is locked", layer.name));
    }
    Ok(())
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    check_target(s, &Value::Null)?;
    let d = s.active().ok_or("no document open")?;
    check(&d.doc, crate::active_layer_of(s)?)
}

/// Removal needs colour context and writes a complete pixel fill, so channel targets are
/// refused before a job can start rather than silently writing the layer underneath them.
fn check_target(s: &Session, p: &Value) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.channel_view.target != crate::channel_cmds::ChannelTarget::Composite
        || d.doc.quick_mask.is_some()
        || s.tools.mask_targeted
        || crate::mask_view_cmds::current(d).is_some()
        || crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels
    {
        return Err("Remove Subject requires the composite pixel target; select the layer's pixels and all colour channels, and exit Quick Mask".into());
    }
    Ok(())
}

fn flag(p: &Value, key: &str) -> Result<bool> {
    match p.get(key) {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("`{key}` must be a boolean"))),
    }
}

fn choice(p: &Value, key: &str, choices: &[&'static str]) -> Result<&'static str> {
    let default = choices.first().copied().ok_or_else(|| bad("missing option choices"))?;
    let value = match p.get(key) {
        None => return Ok(default),
        Some(v) => v.as_str().ok_or_else(|| bad(format!("`{key}` must be {}", choices.join("|"))))?,
    };
    choices.iter().copied().find(|c| *c == value).ok_or_else(|| bad(format!("`{key}` must be {}", choices.join("|"))))
}

fn dimensions(area: Rect, max: usize) -> Result<(usize, usize, usize)> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let n = w.checked_mul(h).ok_or_else(|| bad("image size overflow"))?;
    if area.is_empty() || w > photocraft_ml::MAX_SIDE || h > photocraft_ml::MAX_SIDE || n > max {
        return Err(bad(format!("area too large or empty ({w}×{h}; at most {max} pixels and {} pixels per side)", photocraft_ml::MAX_SIDE)));
    }
    Ok((w, h, n))
}

fn ml_error(e: MlError) -> EngineError {
    match e {
        MlError::Cancelled => EngineError::Cancelled,
        other => EngineError::Other(other.to_string()),
    }
}

struct Hole {
    window: Rect,
    mask: Vec<bool>,
    coverage: Vec<f32>,
    bounds: Rect,
}

fn removal_hole(doc: &Document, subject: &Region, expand: usize, ctx: &JobCtx) -> Result<Hole> {
    let area = subject.bbox.inflate(expand as i32).intersect(&doc.bounds());
    dimensions(area, MAX_WINDOW)?;
    let margin = (u64::from(area.width().max(area.height())) * 3 / 4).clamp(32, 512) as i32;
    let window = area.inflate(margin).intersect(&doc.bounds());
    let (w, h, _) = dimensions(window, MAX_WINDOW)?;
    ctx.check()?;
    let mask = (window.y0..window.y1).flat_map(|y| (window.x0..window.x1).map(move |x| subject.at(x, y) >= 0.5)).collect::<Vec<_>>();
    let mut mask = photocraft_algo::remove::dilate(w, h, &mask, expand);
    ctx.check()?;
    let mut coverage = match doc.selection.as_ref() {
        Some(sel) => selection::mask_from_surface(Some(sel), window),
        None => vec![1.0; mask.len()],
    };
    for (m, c) in mask.iter_mut().zip(&mut coverage) {
        *c = if *m && c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
        *m = *c > 0.0;
    }
    let region = segment::trim_region(Region { bbox: window, mask: mask.iter().map(|m| if *m { 255 } else { 0 }).collect() })
        .ok_or_else(|| bad("the subject does not intersect the selection"))?;
    if mask.iter().all(|v| *v) {
        return Err(bad("nothing is left around the subject to fill it from"));
    }
    Ok(Hole { window, mask, coverage, bounds: region.bbox })
}

/// The native-channel fill; injected inference keeps model tests independent of installed weights.
fn fill_window(
    source: &Pixels,
    fmt: PixelFormat,
    hole: &[bool],
    fill: &str,
    ctx: &JobCtx,
    infer: Option<impl FnOnce(&Rgb, &[f32], &dyn Fn() -> bool) -> std::result::Result<Rgb, MlError>>,
) -> Result<(Vec<f32>, &'static str)> {
    let (w, h, n) = dimensions(source.rect, MAX_WINDOW)?;
    let ch = fmt.channels();
    if source.ch != ch || source.data.len() != n.saturating_mul(ch) || hole.len() != n {
        return Err(bad("fill buffers do not match the image size"));
    }
    ctx.check()?;
    let hdr =
        fmt.sample == SampleType::F32 && source.data.chunks_exact(ch).any(|px| px.iter().take(fmt.mode.color_channels()).any(|v| !(0.0..=1.0).contains(v)));
    if fill == "lama" && hdr {
        return Err(bad("LaMa is SDR-only; this F32 fill window contains values outside 0..1; use fill:patch to preserve HDR values"));
    }
    if fill == "lama" && infer.is_none() {
        return Err(bad("the LaMa model is unavailable; install or enable it in AI Models, or use fill:patch"));
    }
    if let Some(infer) = infer.filter(|_| fill != "patch" && !hdr) {
        #[cfg(test)]
        MODEL_FILL_PREPARATIONS.with(|n| n.set(n.get() + 1));
        ctx.progress(0.5, "Filling subject (LaMa)");
        let mut rgb = Vec::with_capacity(n.saturating_mul(3));
        for px in source.data.chunks_exact(ch) {
            let [r, g, b, a] = to_rgba(&fmt, px);
            let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
            rgb.extend([r, g, b].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) * a + 0.5 * (1.0 - a) } else { 0.5 }));
        }
        let rgb = Rgb::new(w, h, rgb).map_err(ml_error)?;
        let mask: Vec<f32> = hole.iter().map(|v| if *v { 1.0 } else { 0.0 }).collect();
        match infer(&rgb, &mask, &|| ctx.cancelled()) {
            Ok(out) => {
                ctx.check()?;
                if out.w != w || out.h != h || out.px.len() != n.saturating_mul(3) || out.px.iter().any(|v| !v.is_finite()) {
                    return Err(EngineError::Other("inpaint model returned invalid pixels".into()));
                }
                let mut data = vec![0.0; n.saturating_mul(ch)];
                for (dst, &[r, g, b]) in data.chunks_exact_mut(ch).zip(out.px.as_chunks::<3>().0) {
                    from_rgba_into(&fmt, [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), 1.0], dst);
                }
                return Ok((data, "lama"));
            }
            Err(MlError::ModelMissing { .. } | MlError::Unsupported) if fill == "auto" => {}
            Err(e) => return Err(ml_error(e)),
        }
    }
    ctx.check()?;
    let data = ctx
        .stage(0.5, 0.95, "Filling subject (patch)", |ctl| photocraft_algo::remove::remove_with(w, h, ch, &source.data, hole, SEED, ctl))
        .map_err(|_| EngineError::Cancelled)?;
    Ok((data, "patch"))
}

fn write_fill(doc: &mut Document, id: LayerId, hole: &Hole, filled: &[f32], new_layer: bool, ctx: &JobCtx) -> Result<LayerId> {
    let target = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let surf = target.surface().ok_or_else(|| bad("the target has no pixels"))?;
    let fmt = surf.format();
    let out_fmt = PixelFormat { alpha: fmt.alpha || new_layer, ..fmt };
    let (n, out_n) = (fmt.channels(), out_fmt.channels());
    let mut data = if new_layer { vec![0.0; hole.mask.len().saturating_mul(out_n)] } else { surf.read_region(hole.window) };
    let lock_alpha = !new_layer && doc.effective_locks(id).transparency;
    let colours = fmt.mode.color_channels();
    for (i, ((dst, src), &coverage)) in data.chunks_exact_mut(out_n).zip(filled.chunks_exact(n)).zip(&hole.coverage).enumerate() {
        if i % 4096 == 0 {
            ctx.check()?;
        }
        if coverage <= 0.0 {
            continue;
        }
        for (d, s) in dst.iter_mut().zip(src).take(colours) {
            let value = if fmt.sample == SampleType::F32 { *s } else { s.clamp(0.0, 1.0) };
            *d = if new_layer { value } else { *d + (value - *d) * coverage };
        }
        if out_fmt.alpha
            && !lock_alpha
            && let Some(alpha) = dst.get_mut(colours)
        {
            *alpha = if new_layer { coverage } else { *alpha + (1.0 - *alpha) * coverage };
        }
    }
    ctx.check()?;
    if new_layer {
        let mut layer = Layer::raster(doc.next_layer_name(LABEL), out_fmt);
        let surf = crate::pixels_mut(&mut layer)?;
        surf.write_region(hole.window, &data);
        surf.prune();
        Ok(doc.insert_above(Some(id), layer))
    } else {
        crate::pixels_mut(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?)?.write_region(hole.window, &data);
        Ok(id)
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.is_object() {
        return Err(bad("params must be an object"));
    }
    check_target(s, p).map_err(bad)?;
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    check(doc, doc.layer(id).ok_or(EngineError::NoLayer(id))?).map_err(EngineError::Other)?;
    dimensions(doc.bounds(), MAX_CANVAS)?;
    let (all_layers, keep_mask, new_layer) = (flag(p, "sampleAllLayers")?, flag(p, "keepMask")?, flag(p, "newLayer")?);
    let engine = choice(p, "engine", &["auto", "model", "classical"])?;
    let fill = choice(p, "fill", &["auto", "lama", "patch"])?;
    // As in Select Subject: unit tests never run an installed model, matching CI.
    let fill = if cfg!(test) && fill == "auto" { "patch" } else { fill };
    let expand = match p.get("expand") {
        None => 8,
        Some(v) => v.as_f64().filter(|v| v.is_finite() && (0.0..=200.0).contains(v)).ok_or_else(|| bad("`expand` must be 0..200 pixels"))?.round() as usize,
    };
    crate::jobs::edit_job(
        s,
        LABEL,
        move |doc, active, ctx| {
            let cmyk = photocraft_compose::cmyk_space(doc);
            photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                let found = ctx.stage(0.0, 0.4, "Finding subject", |ctl| find_subject_region(doc, Some(id), all_layers, doc.bounds(), engine, ctl))?;
                let (subject, subject_engine) = found.ok_or_else(|| EngineError::Other("no subject found".into()))?;
                ctx.check()?;
                ctx.progress(0.4, "Growing subject mask");
                let hole = removal_hole(doc, &subject, expand, ctx)?;
                let surf = doc.layer(id).and_then(Layer::surface).ok_or_else(|| bad("the target has no pixels"))?;
                let fmt = surf.format();
                let source = if all_layers {
                    crate::retouch_cmds::composite_region(doc, Some(id), crate::retouch_cmds::SampleLayers::All, hole.window, fmt)
                } else {
                    Pixels::read(surf, hole.window)
                };
                let infer =
                    (!cfg!(target_arch = "wasm32") && photocraft_ml::model_path(photocraft_ml::Model::Lama).is_some()).then_some(photocraft_ml::inpaint_with);
                let (filled, filled_with) = fill_window(&source, fmt, &hole.mask, fill, ctx, infer)?;
                ctx.check()?;
                ctx.progress(0.95, "Applying fill");
                let output = write_fill(doc, id, &hole, &filled, new_layer, ctx)?;
                if new_layer {
                    *active = Some(output);
                }
                if keep_mask {
                    doc.selection = Some(selection::mask_to_surface(&hole.coverage, hole.window));
                }
                ctx.check()?;
                let b = hole.bounds;
                Ok(
                    json!({"layer": output.0, "bounds": [b.x0, b.y0, b.width(), b.height()], "subjectFound": true, "filledWith": filled_with, "subjectEngine": subject_engine}),
                )
            })
        },
        |v| v,
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: LABEL,
        menu: &["Edit"],
        shortcut: None,
        params: r##"{"layer":id?,"sampleAllLayers":bool=false,"expand":px=8 (0..200),"keepMask":bool=false,"fill":"auto|lama|patch"="auto","newLayer":bool=false,"engine":"auto|model|classical"="auto"} → {layer,bounds,subjectFound,filledWith,subjectEngine} (missing/unsupported models fall back; keepMask retains the expanded, selection-limited mask)"##,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use crate::{EngineError, Session};
    use photocraft_algo::segment::Rng;
    use photocraft_doc::{Layer, LayerId};
    use photocraft_geom::Rect;
    use serde_json::{Value, json};

    const CMD: &str = "edit.removeSubject";
    const CANVAS: Rect = Rect::new(0, 0, 160, 128);
    type Inpaint = fn(&photocraft_ml::Rgb, &[f32], &dyn Fn() -> bool) -> std::result::Result<photocraft_ml::Rgb, photocraft_ml::MlError>;

    fn fixture(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 160, "height": 128, "depth": depth})).unwrap();
        let mut rng = Rng::new(2);
        s.edit("fixture", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            let mut data = Vec::new();
            for y in 0..128 {
                for x in 0..160 {
                    let n = 0.015 * rng.normal();
                    let inside = (x - 80i32).pow(2) + (y - 64i32).pow(2) <= 27 * 27;
                    let rgb = if inside { [0.9 + n, 0.15 + n, 0.1 + n] } else { [0.25 + n, 0.45 + n, 0.7 + n] };
                    data.extend_from_slice(&rgb);
                    if surf.channels() == 4 {
                        data.push(1.0);
                    }
                }
            }
            surf.write_region(CANVAS, &data);
            Ok(())
        })
        .unwrap();
        s
    }

    fn id(s: &Session) -> LayerId {
        s.active().unwrap().active_layer.unwrap()
    }
    fn layer(s: &Session, id: LayerId) -> &Layer {
        s.active().unwrap().doc.layer(id).unwrap()
    }
    fn pixels(s: &Session, id: LayerId) -> Vec<f32> {
        layer(s, id).surface().unwrap().read_region(CANVAS)
    }
    fn patch() -> Value {
        json!({"engine": "classical", "fill": "patch", "expand": 8})
    }

    #[test]
    fn review_remove_subject_rejects_non_composite_targets_without_edits() {
        for setup in ["mask", "alpha", "quickMask", "red"] {
            let mut s = fixture(8);
            match setup {
                "mask" => {
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
                }
                "alpha" => {
                    s.execute("channel.new", json!({})).unwrap();
                }
                "quickMask" => {
                    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
                }
                _ => {
                    s.execute("channel.target", json!({"channel": "red"})).unwrap();
                }
            }
            let target = id(&s);
            let before = pixels(&s, target);
            let revision = s.active().unwrap().revision;
            let steps = s.active().unwrap().history.past_len();
            assert!(!s.is_enabled(CMD), "{setup}");
            // Also call run directly: the guard must not depend on dispatcher admission.
            let err = super::run(&mut s, &patch()).unwrap_err().to_string();
            assert!(err.contains("composite"), "{setup}: {err}");
            assert!(s.start(CMD, patch()).is_err(), "{setup}");
            assert_eq!(pixels(&s, target), before, "{setup}");
            assert_eq!(s.active().unwrap().revision, revision);
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
    }

    #[test]
    fn review_remove_subject_rejects_explicit_mask_and_channel_targets() {
        for explicit in [json!("mask"), json!("quickMask"), json!({"channel": 0})] {
            let mut s = fixture(8);
            let target = id(&s);
            let before = pixels(&s, target);
            let revision = s.active().unwrap().revision;
            let mut params = patch();
            params["target"] = explicit;
            let err = super::run(&mut s, &params).unwrap_err().to_string();
            assert!(err.contains("composite"), "{err}");
            assert_eq!(pixels(&s, target), before);
            assert_eq!(s.active().unwrap().revision, revision);
        }
    }

    #[test]
    fn review_missing_lama_skips_buffer_preparation() {
        let source = photocraft_paint::retouch::Region { rect: Rect::new(0, 0, 8, 8), ch: 4, data: [0.2, 0.4, 0.6, 1.0].repeat(64) };
        let mut hole = vec![false; 64];
        hole[27] = true;
        let ctx = crate::jobs::JobCtx::new();
        super::MODEL_FILL_PREPARATIONS.with(|n| n.set(0));
        let (_, used) = super::fill_window(&source, photocraft_color::PixelFormat::RGBA8, &hole, "auto", &ctx, None::<Inpaint>).unwrap();
        assert_eq!(used, "patch");
        assert_eq!(super::MODEL_FILL_PREPARATIONS.with(|n| n.get()), 0);
        let err = super::fill_window(&source, photocraft_color::PixelFormat::RGBA8, &hole, "lama", &ctx, None::<Inpaint>).unwrap_err().to_string();
        assert!(err.contains("LaMa model is unavailable"), "{err}");
        assert_eq!(super::MODEL_FILL_PREPARATIONS.with(|n| n.get()), 0);
    }

    #[test]
    fn review_hdr_auto_uses_patch_and_explicit_lama_is_sdr_only() {
        let fmt = photocraft_color::PixelFormat { sample: photocraft_color::SampleType::F32, ..photocraft_color::PixelFormat::RGBA8 };
        let source = photocraft_paint::retouch::Region { rect: Rect::new(0, 0, 8, 8), ch: 4, data: [2.5, -0.25, 0.6, 1.0].repeat(64) };
        let mut hole = vec![false; 64];
        hole[27] = true;
        let ctx = crate::jobs::JobCtx::new();
        let (out, used) = super::fill_window(
            &source,
            fmt,
            &hole,
            "auto",
            &ctx,
            Some(|_: &photocraft_ml::Rgb, _: &[f32], _: &dyn Fn() -> bool| panic!("HDR must bypass LaMa")),
        )
        .unwrap();
        assert_eq!(used, "patch");
        assert!(out[27 * 4] > 1.0 && out[27 * 4 + 1] < 0.0);
        let err = super::fill_window(
            &source,
            fmt,
            &hole,
            "lama",
            &ctx,
            Some(|_: &photocraft_ml::Rgb, _: &[f32], _: &dyn Fn() -> bool| panic!("HDR must bypass LaMa")),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("SDR-only"), "{err}");
    }

    #[test]
    fn review_hdr_writeback_preserves_negative_and_highlight_values() {
        let mut s = fixture(32);
        let target = id(&s);
        let ctx = crate::jobs::JobCtx::new();
        let window = Rect::new(80, 64, 81, 65);
        let hole = super::Hole { window, mask: vec![true], coverage: vec![1.0], bounds: window };
        for new_layer in [false, true] {
            s.edit("HDR fill", |doc, _| {
                let fmt = doc.layer(target).unwrap().surface().unwrap().format();
                let mut filled = vec![2.5, -0.25, 0.6];
                if fmt.alpha {
                    filled.push(1.0);
                }
                let output = super::write_fill(doc, target, &hole, &filled, new_layer, &ctx)?;
                let px = doc.layer(output).unwrap().surface().unwrap().read_region(window);
                assert_eq!(&px[..3], &[2.5, -0.25, 0.6], "newLayer={new_layer}");
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn remove_subject_fills_at_every_depth_and_undo_redo_is_one_step() {
        for depth in [8, 16, 32] {
            let mut s = fixture(depth);
            assert!(s.is_enabled(CMD));
            let target = id(&s);
            let before = pixels(&s, target);
            let corner = layer(&s, target).surface().unwrap().read_region(Rect::new(0, 0, 8, 8));
            let steps = s.active().unwrap().history.past_len();
            let r = s.execute(CMD, patch()).unwrap();
            assert_eq!(r["subjectFound"], true);
            assert_eq!(r["subjectEngine"], "classical");
            assert_eq!(r["filledWith"], "patch");
            assert_eq!(r["layer"], target.0);
            assert!(r["bounds"].is_array());
            let surf = layer(&s, target).surface().unwrap();
            let centre = surf.read_region(Rect::new(78, 62, 83, 67));
            for px in centre.chunks_exact(surf.channels()) {
                for (v, bg) in px.iter().zip([0.25, 0.45, 0.7]) {
                    assert!((*v - bg).abs() < 0.12, "depth {depth}: {px:?}");
                }
            }
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
            assert!(s.active().unwrap().doc.selection.is_none());
            let after = pixels(&s, target);
            assert_ne!(after, before);
            assert_eq!(layer(&s, target).surface().unwrap().read_region(Rect::new(0, 0, 8, 8)), corner);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(pixels(&s, target), before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(pixels(&s, target), after);
        }
    }

    #[test]
    fn remove_subject_keeps_mask_only_when_requested() {
        let mut s = fixture(8);
        let mut p = patch();
        p["keepMask"] = json!(true);
        s.execute(CMD, p).unwrap();
        let mask = s.active().unwrap().doc.selection.as_ref().unwrap();
        assert!(mask.read_region(Rect::new(80, 64, 81, 65))[0] > 0.9);
        assert_eq!(mask.read_region(Rect::new(0, 0, 1, 1))[0], 0.0);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
    }

    #[test]
    fn remove_subject_new_layer_preserves_target_and_is_transparent_outside_hole() {
        let mut s = fixture(16);
        let target = id(&s);
        let before = pixels(&s, target);
        let mut p = patch();
        p["newLayer"] = json!(true);
        let r = s.execute(CMD, p).unwrap();
        let output = id(&s);
        assert_ne!(output, target);
        assert_eq!(r["layer"], output.0);
        assert_eq!(pixels(&s, target), before);
        let surf = layer(&s, output).surface().unwrap();
        assert!(surf.format().alpha);
        let centre = surf.read_region(Rect::new(80, 64, 81, 65));
        assert_eq!(centre.last(), Some(&1.0));
        assert!(centre[0] < 0.4);
        assert_eq!(surf.read_region(Rect::new(0, 0, 1, 1)).last(), Some(&0.0));
        assert_eq!(s.active().unwrap().doc.layers.len(), 2);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert_eq!(pixels(&s, target), before);
    }

    #[test]
    fn remove_subject_selection_limits_changes_and_is_preserved() {
        let mut s = fixture(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 80, "height": 128})).unwrap();
        let target = id(&s);
        let before = pixels(&s, target);
        let selection = s.active().unwrap().doc.selection.clone().unwrap().read_region(CANVAS);
        s.execute(CMD, patch()).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(CANVAS), selection);
        let n = layer(&s, target).surface().unwrap().channels();
        let after = pixels(&s, target);
        for y in 0..128 {
            assert_eq!(&after[(y * 160 + 80) * n..(y + 1) * 160 * n], &before[(y * 160 + 80) * n..(y + 1) * 160 * n]);
        }
        assert_ne!(before, after);
    }

    #[test]
    fn remove_subject_soft_selection_blends_and_new_layer_keeps_coverage() {
        let mut full = fixture(32);
        let target = id(&full);
        let original = layer(&full, target).surface().unwrap().read_region(Rect::new(80, 64, 81, 65));
        full.execute(CMD, patch()).unwrap();
        let filled = layer(&full, target).surface().unwrap().read_region(Rect::new(80, 64, 81, 65));
        for new_layer in [false, true] {
            let mut s = fixture(32);
            s.edit("soft selection", |doc, _| {
                let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
                selection.write_region(CANVAS, &vec![0.5; 160 * 128]);
                doc.selection = Some(selection);
                Ok(())
            })
            .unwrap();
            let weight = s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(80, 64, 0);
            let mut p = patch();
            p["newLayer"] = json!(new_layer);
            s.execute(CMD, p).unwrap();
            let centre = layer(&s, id(&s)).surface().unwrap().read_region(Rect::new(80, 64, 81, 65));
            for c in 0..3 {
                let expected = if new_layer { filled[c] } else { original[c] + weight * (filled[c] - original[c]) };
                assert!((centre[c] - expected).abs() < 0.002, "{new_layer}: {centre:?} versus {expected}");
            }
            if new_layer {
                assert!((centre.last().unwrap() - weight).abs() < 0.002);
            }
        }
    }

    #[test]
    fn remove_subject_all_layers_samples_composite_into_empty_target() {
        let mut s = fixture(8);
        let bottom = id(&s);
        let before = pixels(&s, bottom);
        s.execute("layer.new.layer", json!({})).unwrap();
        assert!(s.execute(CMD, patch()).is_err());
        let mut p = patch();
        p["sampleAllLayers"] = json!(true);
        s.execute(CMD, p).unwrap();
        assert_eq!(pixels(&s, bottom), before);
        let centre = layer(&s, id(&s)).surface().unwrap().read_region(Rect::new(80, 64, 81, 65));
        assert!(centre[0] < 0.4);
        assert_eq!(centre.last(), Some(&1.0));
    }

    #[test]
    fn remove_subject_bad_params_are_errors_without_edits() {
        let mut s = fixture(8);
        for p in [
            json!(null),
            json!([]),
            json!({"expand": -1}),
            json!({"expand": 201}),
            json!({"expand": "8"}),
            json!({"fill": "wrong"}),
            json!({"fill": 4}),
            json!({"engine": "wrong"}),
            json!({"engine": false}),
            json!({"keepMask": 1}),
            json!({"sampleAllLayers": "true"}),
            json!({"newLayer": []}),
            json!({"layer": u64::MAX}),
            json!({"layer": {}}),
        ] {
            let revision = s.active().unwrap().revision;
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
            assert_eq!(s.active().unwrap().revision, revision, "{p}");
        }
    }

    #[test]
    fn remove_subject_selection_outside_subject_is_an_error_without_an_edit() {
        let mut s = fixture(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        let target = id(&s);
        let before = pixels(&s, target);
        let revision = s.active().unwrap().revision;
        assert!(s.execute(CMD, patch()).is_err());
        assert_eq!(pixels(&s, target), before);
        assert_eq!(s.active().unwrap().revision, revision);
    }

    #[test]
    fn remove_subject_expand_is_clipped_to_canvas_and_preserves_zero() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc = &s.active().unwrap().doc;
        let subject = photocraft_algo::segment::Region { bbox: Rect::new(0, 0, 3, 3), mask: vec![255; 9] };
        let ctx = crate::jobs::JobCtx::new();
        let original = super::removal_hole(doc, &subject, 0, &ctx).unwrap();
        let grown = super::removal_hole(doc, &subject, 2, &ctx).unwrap();
        assert_eq!(original.bounds, subject.bbox);
        assert_eq!(original.mask.iter().filter(|v| **v).count(), 9);
        assert_eq!(grown.bounds, Rect::new(0, 0, 5, 5));
        assert_eq!(grown.mask.iter().filter(|v| **v).count(), 25);
        assert!(super::dimensions(Rect::new(0, 0, 100_000, 100_000), super::MAX_CANVAS).is_err());
    }

    #[test]
    fn remove_subject_disabled_without_document_pixels_or_when_locked() {
        let mut s = Session::new();
        assert!(!s.is_enabled(CMD));
        assert!(s.execute(CMD, patch()).is_err());
        let mut s = fixture(8);
        s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
        assert!(!s.is_enabled(CMD));
        assert!(s.execute(CMD, patch()).is_err());
        let mut s = fixture(8);
        s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
        assert!(!s.is_enabled(CMD));
        assert!(s.execute(CMD, patch()).is_err());
    }

    #[test]
    fn remove_subject_cancel_and_no_subject_leave_document_unchanged() {
        let mut s = fixture(8);
        let target = id(&s);
        let before = pixels(&s, target);
        let revision = s.active().unwrap().revision;
        let ctx = crate::jobs::JobCtx::new();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute(CMD, patch()), Err(EngineError::Cancelled)));
        assert_eq!(pixels(&s, target), before);
        assert_eq!(s.active().unwrap().revision, revision);
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
        let revision = s.active().unwrap().revision;
        assert!(s.execute(CMD, patch()).is_err());
        assert_eq!(s.active().unwrap().revision, revision);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn remove_subject_background_job_commits_one_edit() {
        let mut s = fixture(8);
        let target = id(&s);
        let before = pixels(&s, target);
        let steps = s.active().unwrap().history.past_len();
        let started = s.start(CMD, patch()).unwrap();
        let crate::jobs::Started::Job(job) = started else { panic!("expected a background job") };
        assert_eq!(pixels(&s, target), before);
        let r = s.wait_job(job).unwrap();
        assert_eq!(r["subjectFound"], true);
        assert_ne!(pixels(&s, target), before);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixels(&s, target), before);
    }

    #[test]
    fn remove_subject_model_fill_stub_and_missing_model_fallback() {
        use photocraft_color::PixelFormat;
        use photocraft_ml::{MlError, Rgb};
        use photocraft_paint::retouch::Region;
        let source = Region { rect: Rect::new(0, 0, 8, 8), ch: 4, data: [0.2, 0.4, 0.6, 0.5].repeat(64) };
        let mut hole = vec![false; 64];
        hole[27] = true;
        let ctx = crate::jobs::JobCtx::new();
        let (out, used) = super::fill_window(
            &source,
            PixelFormat::RGBA8,
            &hole,
            "auto",
            &ctx,
            Some(|img: &Rgb, mask: &[f32], cancel: &dyn Fn() -> bool| {
                assert_eq!(mask[27], 1.0);
                assert!(!cancel());
                Ok(Rgb { w: img.w, h: img.h, px: [0.7, 0.8, 0.9].repeat(64) })
            }),
        )
        .unwrap();
        assert_eq!(used, "lama");
        assert_eq!(&out[27 * 4..28 * 4], &[0.7, 0.8, 0.9, 1.0]);
        let (_, used) =
            super::fill_window(&source, PixelFormat::RGBA8, &hole, "auto", &ctx, Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Err(MlError::Unsupported)))
                .unwrap();
        assert_eq!(used, "patch");
        // `lama` insists on the model, like `engine: "model"`.
        assert!(
            super::fill_window(&source, PixelFormat::RGBA8, &hole, "lama", &ctx, Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Err(MlError::Unsupported)))
                .is_err()
        );
        let (_, used) = super::fill_window(
            &source,
            PixelFormat::RGBA8,
            &hole,
            "patch",
            &ctx,
            Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| panic!("patch must bypass inference")),
        )
        .unwrap();
        assert_eq!(used, "patch");
    }

    #[test]
    fn remove_subject_model_errors_and_bad_outputs_are_not_silently_filled() {
        use photocraft_color::PixelFormat;
        use photocraft_ml::{MlError, Rgb};
        use photocraft_paint::retouch::Region;
        let source = Region { rect: Rect::new(0, 0, 8, 8), ch: 3, data: [0.2, 0.4, 0.6].repeat(64) };
        let mut hole = vec![false; 64];
        hole[27] = true;
        let ctx = crate::jobs::JobCtx::new();
        let fmt = PixelFormat { alpha: false, ..PixelFormat::RGBA8 };
        assert!(matches!(
            super::fill_window(&source, fmt, &hole, "auto", &ctx, Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Err(MlError::Cancelled))),
            Err(EngineError::Cancelled)
        ));
        assert!(
            super::fill_window(
                &source,
                fmt,
                &hole,
                "auto",
                &ctx,
                Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Err(MlError::Backend("broken model".into())))
            )
            .is_err()
        );
        assert!(
            super::fill_window(&source, fmt, &hole, "auto", &ctx, Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Ok(Rgb { w: 8, h: 8, px: vec![] }))).is_err()
        );
        assert!(
            super::fill_window(
                &source,
                fmt,
                &hole,
                "auto",
                &ctx,
                Some(|_: &Rgb, _: &[f32], _: &dyn Fn() -> bool| Ok(Rgb { w: 8, h: 8, px: vec![f32::NAN; 64 * 3] }))
            )
            .is_err()
        );
    }
}
