//! Filter › Other › Frequency Separation (`layer.frequencySeparation`, #1714): splits the active
//! pixel layer into a "Low Frequency" layer (a Gaussian-blurred copy: colour and tone) and a
//! "High Frequency" layer (the detail the blur took out) in a "Frequency Separation" group right
//! above it, so tone and texture can be retouched apart. One history step; the source layer is
//! left as it is, under the group.
//!
//! High = (Source − Low) / 2 + ½ in every colour channel, on Linear Light (Low + 2·High − 1),
//! gives the source back. That is Photoshop's recipe (Apply Image › Subtract, Scale 2, Offset 128
//! in 8-bit; Add with Invert, Scale 2, Offset 0 in 16-bit: the same values) and it holds in 32-bit
//! too, since our Linear Light is the same formula at every depth. It clips at 0 and 1, so 32-bit
//! values above 1 come back as 1. 8-bit stores High to the nearest level, so the result is within
//! one level of the source, as in Photoshop.
//!
//! Photoshop has no such menu item (retouchers record it as an action), so the id is our own. It
//! is not a `filter.*` command: it adds layers instead of filtering the target, so it must not
//! follow a targeted mask or channel, nor be offered to Fade or Last Filter. The new layers carry
//! the source's pixels only (no mask, style or blending options), and the selection is ignored.

use photocraft_color::{BlendMode, ColorMode};
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use rayon::prelude::*;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The command id.
pub const ID: &str = "layer.frequencySeparation";

const LABEL: &str = "Frequency Separation";
const GAUSSIAN: &str = "filter.blur.gaussianBlur";
/// Gaussian Blur's own range and a portrait-sized default.
const RADIUS: (f64, f64, f64) = (0.1, 1000.0, 4.0);

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn radius(p: &Value) -> Result<f64> {
    let (min, max, default) = RADIUS;
    match p.get("radius") {
        None => Ok(default),
        Some(v) => {
            v.as_f64().filter(|r| r.is_finite() && (min..=max).contains(r)).ok_or_else(|| bad(format!("`radius` must be a number in {min}..{max}, not {v}")))
        }
    }
}

/// The pixels of `l`, when it is a pixel layer with any. Only the tile count is checked here, as
/// the menu asks often; [`run`] looks at the pixels themselves.
fn pixels_of(l: &Layer) -> std::result::Result<&Surface, String> {
    match &l.content {
        LayerContent::Raster(surf) if surf.tile_count() > 0 => Ok(surf),
        LayerContent::Raster(_) => Err(format!("the layer \"{}\" has no pixels", l.name)),
        other => Err(format!("Frequency Separation needs a pixel layer (\"{}\" is {} {} layer)", l.name, other.article(), other.kind_name())),
    }
}

/// Menu state: an RGB or Grayscale document whose active layer is a pixel layer with pixels, its
/// pixels targeted (not a channel, Quick Mask or the layer's mask).
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Err("Frequency Separation works on layer pixels, not on a channel or Quick Mask".into());
    }
    let d = s.active().ok_or("no document open")?;
    if d.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask || crate::mask_view_cmds::current(d).is_some() {
        return Err("Frequency Separation works on layer pixels, not on a layer mask".into());
    }
    match d.doc.mode {
        ColorMode::Rgb | ColorMode::Grayscale => {}
        m => return Err(format!("Frequency Separation needs an RGB or Grayscale document (this one is {m:?})")),
    }
    pixels_of(crate::active_layer_of(s)?).map(|_| ())
}

/// High = (Source − Low) / 2 + ½ in the colour channels, the source's transparency kept. Where a
/// layer with transparency is transparent High stays so; a layer without (the Background) is
/// computed over the whole canvas.
fn high_pass(src: &Surface, low: &Surface, canvas: Rect) -> Surface {
    let fmt = src.format();
    let (n, colors) = (fmt.channels(), fmt.mode.color_channels());
    let coords: Vec<_> = if fmt.alpha { src.tiles().map(|(c, _)| *c).collect() } else { canvas.union(&src.tile_bounds()).tiles().collect() };
    let mut high = src.clone();
    // A bounded batch at a time: the f32 copies of a whole 36 MP layer would take gigabytes.
    for batch in coords.chunks(rayon::current_num_threads().max(1) * 4) {
        let done: Vec<_> = batch
            .par_iter()
            .map(|c| {
                let r = c.rect();
                let mut px = src.read_region(r);
                for (h, l) in px.chunks_exact_mut(n).zip(low.read_region(r).chunks_exact(n)) {
                    // Nothing to rebuild where the source is transparent: leaving it as it is keeps
                    // High's bounds the source's (mid-grey there would fill out whole tiles).
                    if fmt.alpha && h.get(colors).is_some_and(|a| *a <= 0.0) {
                        continue;
                    }
                    for (h, l) in h.iter_mut().zip(l).take(colors) {
                        *h = (*h - l) * 0.5 + 0.5;
                    }
                }
                (r, px)
            })
            .collect();
        for (r, px) in done {
            high.write_region(r, &px);
        }
    }
    high.prune();
    high
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let radius = radius(p)?;
    enabled(s).map_err(EngineError::Other)?;
    let before = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let (group, low, high) = s.edit(LABEL, |doc, active| {
        let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let canvas = doc.bounds();
        let source = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let src = pixels_of(source).map_err(EngineError::Other)?;
        if src.content_bounds().is_empty() {
            return Err(EngineError::Other(format!("the layer \"{}\" has no pixels", source.name)));
        }
        let low = crate::filters::apply_filter_to_surface(GAUSSIAN, &json!({ "radius": radius }), src, canvas, None, canvas)
            .ok_or_else(|| EngineError::Other("Gaussian Blur is unavailable".into()))?;
        // From the Low that is stored (quantised at 8 and 16 bits), so the two add up exactly.
        let high = high_pass(src, &low, canvas);
        let low = Layer::new("Low Frequency", LayerContent::Raster(low));
        let mut high = Layer::new("High Frequency", LayerContent::Raster(high));
        high.blend = BlendMode::LinearLight;
        let ids = (low.id, high.id);
        let group = doc.insert_above(Some(id), Layer::group(LABEL, vec![low, high]));
        crate::layer_multi_cmds::check_group_depth(doc, LABEL)?;
        *active = Some(ids.1);
        Ok((group, ids.0, ids.1))
    })?;
    crate::layer_multi_cmds::note_insert(s, &before);
    Ok(json!({ "group": group.0, "low": low.0, "high": high.0, "radius": radius }))
}

/// The command spec.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Frequency Separation…",
        menu: &["Filter", "Other"],
        shortcut: None,
        params: r##"{"radius":0.1..1000=4} → {group,low,high,radius} (Low Frequency = Gaussian Blur of the active pixel layer, High Frequency = (layer − Low) / 2 + ½ on Linear Light, grouped above it)"##,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineError, Session};
    use photocraft_color::BlendMode;
    use photocraft_doc::{Document, LayerContent, LayerId};
    use photocraft_geom::Rect;
    use serde_json::json;

    const W: i32 = 40;
    const H: i32 = 28;

    /// Fine detail on a slow gradient, different in every channel, so a blur changes it a lot.
    fn texture(x: i32, y: i32) -> [f32; 4] {
        let hash = |k: i32| {
            let v = ((x * 73 + y * 151 + k * 211) as f32 * 12.9898).sin() * 43758.547;
            v - v.floor()
        };
        let ramp = (x + y) as f32 / (W + H) as f32;
        [0.1 + 0.5 * ramp + 0.35 * hash(1), 0.8 - 0.6 * ramp + 0.15 * hash(2), 0.5 + 0.45 * (hash(3) - 0.5), 1.0]
    }

    /// A document with a white Background and an opaque textured pixel layer on top (active).
    fn session(depth: u32, mode: &str) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "mode": mode})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Skin"})).unwrap();
        paint(&mut s);
        s
    }

    fn paint(s: &mut Session) {
        s.edit("texture", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            let px: Vec<f32> = (0..H).flat_map(|y| (0..W).flat_map(move |x| photocraft_raster::from_rgba(&fmt, texture(x, y)))).collect();
            surf.write_region(Rect::new(0, 0, W, H), &px);
            Ok(())
        })
        .unwrap();
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    fn active(s: &Session) -> LayerId {
        s.active().unwrap().active_layer.unwrap()
    }

    fn composite(d: &Document) -> Vec<[f32; 4]> {
        photocraft_compose::flatten(d).px
    }

    fn max_diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]).abs())).fold(0.0, f32::max)
    }

    /// The document with layer `id` hidden.
    fn without(d: &Document, id: LayerId) -> Document {
        let mut d = d.clone();
        d.layer_mut(id).unwrap().visible = false;
        d
    }

    /// The new group: (group id, [Low, High] ids, bottom to top).
    fn group_of(d: &Document, source: LayerId) -> (LayerId, [LayerId; 2]) {
        let path = d.path_of(source).unwrap();
        let (&k, parent) = path.split_last().unwrap();
        let mut above = parent.to_vec();
        above.push(k + 1);
        let g = d.layer_at(&above).expect("a layer right above the source");
        let ch = g.children().expect("a group right above the source");
        assert_eq!(ch.len(), 2, "two layers in the group");
        (g.id, [ch[0].id, ch[1].id])
    }

    fn pixels(d: &Document, id: LayerId) -> Vec<f32> {
        d.layer(id).unwrap().surface().unwrap().read_region(Rect::new(0, 0, W, H))
    }

    /// The defining identity: High on Linear Light over Low gives the source back, through the
    /// compositor, to within one step of the depth (8-bit stores High to the nearest level: Photoshop's
    /// recipe is ±1 level too). Low and High each differ from it, so the identity isn't a no-op.
    #[test]
    fn low_and_high_composite_back_to_the_source_at_every_depth() {
        for (depth, mode, tol) in [(8, "rgb", 1.0 / 255.0), (16, "rgb", 1.0 / 65535.0), (32, "rgb", 1e-5), (8, "gray", 1.0 / 255.0), (32, "gray", 1e-5)] {
            let tol = tol + 1e-6;
            let mut s = session(depth, mode);
            let source = active(&s);
            let before = composite(doc(&s));
            s.execute(ID, json!({"radius": 3})).unwrap_or_else(|e| panic!("{depth} {mode}: {e}"));
            let d = doc(&s);
            let (_, [low, high]) = group_of(d, source);
            // The group alone rebuilds the source: hide the source so it can't show through.
            let rebuilt = composite(&without(d, source));
            let err = max_diff(&before, &rebuilt);
            assert!(err <= tol, "{depth} {mode}: Low + High is off by {err} (tolerance {tol})");
            assert!(max_diff(&composite(d), &rebuilt) <= 1e-6, "{depth} {mode}: the group covers the source");
            // Low alone is visibly smoother than the source; High carries what it lost.
            let low_only = composite(&without(&without(d, source), high));
            assert!(max_diff(&before, &low_only) > 0.1, "{depth} {mode}: Low is blurred");
            let h = d.layer(high).unwrap().surface().unwrap();
            assert!((0..W).any(|x| (h.rgba(x, 5)[0] - 0.5).abs() > 0.05), "{depth} {mode}: High holds the detail");
            assert_ne!(pixels(d, low), pixels(d, source), "{depth} {mode}");
        }
    }

    /// Breaking the recipe breaks the identity: a High layer left in Normal mode covers the image
    /// with grey (guards the test above against passing for the wrong reason).
    #[test]
    fn high_must_be_on_linear_light() {
        let mut s = session(16, "rgb");
        let source = active(&s);
        let before = composite(doc(&s));
        s.execute(ID, json!({})).unwrap();
        let (_, [_, high]) = group_of(doc(&s), source);
        let mut d = doc(&s).clone();
        d.layer_mut(high).unwrap().blend = BlendMode::Normal;
        assert!(max_diff(&before, &composite(&d)) > 0.1);
    }

    #[test]
    fn builds_a_named_group_above_the_source_and_targets_high() {
        let mut s = session(8, "rgb");
        let source = active(&s);
        let source_px = pixels(doc(&s), source);
        let r = s.execute(ID, json!({})).unwrap();
        let d = doc(&s);
        let (group, [low, high]) = group_of(d, source);
        let g = d.layer(group).unwrap();
        assert_eq!(g.name, "Frequency Separation");
        assert_eq!(g.blend, BlendMode::PassThrough);
        let (l, h) = (d.layer(low).unwrap(), d.layer(high).unwrap());
        assert_eq!((l.name.as_str(), l.blend), ("Low Frequency", BlendMode::Normal));
        assert_eq!((h.name.as_str(), h.blend), ("High Frequency", BlendMode::LinearLight));
        assert!(matches!(l.content, LayerContent::Raster(_)) && matches!(h.content, LayerContent::Raster(_)));
        assert!(l.visible && h.visible && g.visible);
        // High draws where the source does, not grey over the transparent rest of its tiles.
        let bounds = |id: LayerId| d.layer(id).unwrap().surface().unwrap().content_bounds();
        assert_eq!(bounds(source), Rect::new(0, 0, W, H));
        assert_eq!(bounds(high), bounds(source));
        // The source is left exactly as it was, visible, below the group.
        let src = d.layer(source).unwrap();
        assert_eq!((src.name.as_str(), src.visible), ("Skin", true));
        assert_eq!(pixels(d, source), source_px);
        assert_eq!(active(&s), high, "retouching starts on the High layer");
        assert_eq!(r, json!({"group": group.0, "low": low.0, "high": high.0, "radius": 4.0}));
    }

    /// Low is exactly what Filter › Blur › Gaussian Blur makes of the source at that radius.
    #[test]
    fn low_is_the_gaussian_blur_of_the_source_at_the_radius() {
        for radius in [2.0, 6.5] {
            let mut s = session(16, "rgb");
            let source = active(&s);
            s.execute(ID, json!({"radius": radius})).unwrap();
            let (_, [low, _]) = group_of(doc(&s), source);
            let mut t = session(16, "rgb");
            t.execute("filter.blur.gaussianBlur", json!({"radius": radius})).unwrap();
            assert_eq!(pixels(doc(&s), low), pixels(doc(&t), active(&t)), "radius {radius}");
        }
    }

    #[test]
    fn one_undo_removes_everything_and_redo_brings_it_back() {
        let mut s = session(8, "rgb");
        let source = active(&s);
        let before = doc(&s).clone();
        let steps = s.active().unwrap().history.past_len();
        s.execute(ID, json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Frequency Separation"));
        let after = doc(&s).clone();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*doc(&s), before);
        assert_eq!(active(&s), source);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*doc(&s), after);
    }

    /// A Background source stays the Background; the copies are ordinary, unlocked layers.
    #[test]
    fn works_on_the_background() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H})).unwrap();
        paint(&mut s);
        let source = active(&s);
        assert!(doc(&s).is_background(source));
        let before = composite(doc(&s));
        s.execute(ID, json!({"radius": 2})).unwrap();
        let d = doc(&s);
        assert!(d.is_background(source));
        let (_, [low, high]) = group_of(d, source);
        for id in [low, high] {
            assert_eq!(d.layer(id).unwrap().locks, Default::default());
        }
        assert!(max_diff(&before, &composite(d)) <= 1.0 / 255.0 + 1e-6);
    }

    fn refused(s: &mut Session, why: &str) {
        assert!(!s.is_enabled(ID), "{why}: enabled");
        let before = s.active().map(|d| d.doc.clone());
        let e = s.execute(ID, json!({})).expect_err(why);
        assert!(!e.to_string().is_empty());
        assert_eq!(s.active().map(|d| d.doc.clone()), before, "{why}: the document changed");
    }

    #[test]
    fn needs_a_pixel_layer_with_pixels() {
        refused(&mut Session::new(), "no document");

        let mut s = session(8, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        refused(&mut s, "an empty layer");
        let e = s.execute(ID, json!({})).unwrap_err().to_string();
        assert!(e.contains("no pixels"), "{e}");

        let mut s = session(8, "rgb");
        s.execute("type.create", json!({"x": 4, "y": 20, "text": "Hi", "size": 12})).unwrap();
        refused(&mut s, "a type layer");
        let e = s.execute(ID, json!({})).unwrap_err().to_string();
        assert!(e.contains("pixel layer"), "{e}");

        let mut s = session(8, "rgb");
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        refused(&mut s, "an adjustment layer");

        let mut s = session(8, "rgb");
        s.execute("layer.new.group", json!({})).unwrap();
        refused(&mut s, "a group");

        let mut s = session(8, "rgb");
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        refused(&mut s, "a smart object");
    }

    #[test]
    fn refuses_a_channel_or_mask_target() {
        let mut s = session(8, "rgb");
        s.execute("select.all", json!({})).unwrap();
        s.execute("select.saveSelection", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "Alpha 1"})).unwrap();
        assert!(crate::channel_cmds::edits_channel(&s));
        refused(&mut s, "an alpha channel");

        let mut s = session(8, "rgb");
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "mask"})).unwrap();
        assert_eq!(s.active().unwrap().channel_view.target, crate::channel_cmds::ChannelTarget::LayerMask);
        refused(&mut s, "a targeted layer mask");
        // The same layer, its pixels targeted again: allowed (the refusal was the target's).
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        assert!(s.is_enabled(ID));
        s.execute(ID, json!({})).unwrap();

        // Showing the mask (⌥-click its thumbnail) targets it too.
        let mut s = session(8, "rgb");
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        assert!(s.is_enabled(ID));
        s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
        refused(&mut s, "a mask shown in mask view");
    }

    /// Linear Light over Low gives the source back in RGB and Grayscale only: CMYK and Lab blend
    /// in other values (measured: off by 0.7 and 0.14), so they are refused rather than split.
    #[test]
    fn refuses_cmyk_and_lab() {
        for mode in ["cmyk", "lab"] {
            let mut s = session(8, mode);
            refused(&mut s, mode);
            let e = s.execute(ID, json!({})).unwrap_err().to_string();
            assert!(e.contains("RGB or Grayscale"), "{mode}: {e}");
        }
    }

    /// A layer whose tiles hold only transparency (not yet pruned) passes the menu's quick check
    /// but has nothing to split: the command says so and changes nothing.
    #[test]
    fn a_layer_of_transparent_tiles_is_refused_when_run() {
        let mut s = session(8, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("clear", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.write_region(Rect::new(0, 0, 8, 8), &vec![0.0; 8 * 8 * surf.channels()]);
            Ok(())
        })
        .unwrap();
        assert!(s.is_enabled(ID));
        let before = doc(&s).clone();
        let e = s.execute(ID, json!({})).unwrap_err().to_string();
        assert!(e.contains("no pixels"), "{e}");
        assert_eq!(*doc(&s), before);
    }

    #[test]
    fn bad_params_are_rejected_without_changes() {
        let mut s = session(8, "rgb");
        let before = doc(&s).clone();
        for p in [
            json!(7),
            json!("4"),
            json!({"radius": 0}),
            json!({"radius": 0.05}),
            json!({"radius": -3}),
            json!({"radius": 1000.5}),
            json!({"radius": 1e308}),
            json!({"radius": "4"}),
            json!({"radius": null}),
            json!({"radius": [4]}),
        ] {
            assert!(matches!(s.execute(ID, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
        }
        assert_eq!(*doc(&s), before);
    }
}
