//! The Adjustment Brush Tool (#12519): painting on the image makes an adjustment layer whose mask
//! is what was painted, or paints into the mask of the active adjustment layer of the chosen kind.
//! The kinds it offers, which of them a document's colour mode and bit depth grey out, and the
//! starting values a new layer gets, as measured in the reference application.

use photocraft_doc::{Adjustment, ColorMode, Layer, LayerContent, LayerMask, SampleType};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::commands::adjustment_kind;
use crate::commands::{mask_at_depth, selection_mask};
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.adjustmentBrush";

/// The history state of a stroke that makes the layer.
pub const NEW_LAYER_LABEL: &str = "New Adjustment Layer";
/// The history state of a stroke into an existing adjustment layer's mask.
pub const STROKE_LABEL: &str = "Adjustment Brush";

/// The alert a stroke on an Indexed Color document shows; nothing changes.
pub const INDEXED_ALERT: &str = "Could not complete your request because the tool does not work with Indexed Color images. Please convert the image to RGB or another supported mode before using.";

/// One entry of the options bar's Adjustment dropdown: the kind id (the `layer.newAdjustmentLayer`
/// suffix for the kinds that exist), its label, and whether a separator follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: &'static str,
    pub label: &'static str,
    pub separator_after: bool,
}

const fn entry(kind: &'static str, label: &'static str, separator_after: bool) -> Entry {
    Entry { kind, label, separator_after }
}

/// The dropdown in the reference application's order, separators after Grain, Exposure and
/// Selective Color.
pub const ENTRIES: [Entry; 18] = [
    entry("light", "Light", false),
    entry("colorVibrance", "Color and vibrance", false),
    entry("clarityDehaze", "Clarity and dehaze", false),
    entry("grain", "Grain", true),
    entry("levels", "Levels", false),
    entry("curves", "Curves", false),
    entry("exposure", "Exposure", true),
    entry("hueSaturation", "Hue/Saturation", false),
    entry("colorBalance", "Color Balance", false),
    entry("blackWhite", "Black & White", false),
    entry("photoFilter", "Photo Filter", false),
    entry("channelMixer", "Channel Mixer", false),
    entry("colorLookup", "Color Lookup", false),
    entry("selectiveColor", "Selective Color", true),
    entry("invert", "Invert", false),
    entry("posterize", "Posterize", false),
    entry("threshold", "Threshold", false),
    entry("gradientMap", "Gradient Map", false),
];

/// Entries whose adjustment doesn't exist yet: always greyed.
const MISSING: [&str; 4] = ["light", "colorVibrance", "clarityDehaze", "grain"];

/// The entry the tool starts on: the reference application's default (Light) while it is
/// available, else the first enabled entry.
pub const DEFAULT_KIND: &str = "levels";

/// The entry with id `kind`.
pub fn entry_of(kind: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.kind == kind)
}

/// Is `kind` offered (not greyed) in a document of `mode` and `depth`? Indexed Color refuses
/// the stroke as a whole ([`INDEXED_ALERT`]), so its entries stay as RGB's. Bitmap, Duotone and
/// Multichannel were not measured and grey nothing extra.
pub fn enabled_in(kind: &str, mode: ColorMode, depth: SampleType) -> bool {
    if entry_of(kind).is_none() || MISSING.contains(&kind) {
        return false;
    }
    let greyed: &[&str] = match mode {
        ColorMode::Grayscale => &["hueSaturation", "colorBalance", "blackWhite", "photoFilter", "channelMixer", "colorLookup", "selectiveColor"],
        ColorMode::Cmyk => &["exposure", "blackWhite"],
        ColorMode::Lab => &["exposure", "blackWhite", "channelMixer", "selectiveColor"],
        _ => &[],
    };
    let float: &[&str] = if depth == SampleType::F32 { &["invert", "posterize", "threshold", "gradientMap"] } else { &[] };
    !greyed.contains(&kind) && !float.contains(&kind)
}

/// The kind the dropdown shows when `kind` is greyed in this document: `kind` itself when it is
/// offered, else the first offered entry.
pub fn offered_kind(kind: &str, mode: ColorMode, depth: SampleType) -> &str {
    if enabled_in(kind, mode, depth) {
        return kind;
    }
    ENTRIES.iter().map(|e| e.kind).find(|k| enabled_in(k, mode, depth)).unwrap_or(kind)
}

/// The params of the starting values a layer of `kind` gets (the layer already changes the
/// image; these are not Layer › New Adjustment Layer's identity values). Color Lookup starts on a
/// built-in film look: the reference application ships its own film-stock table, which is not
/// ours to copy.
pub fn starting_params(kind: &str) -> Option<Value> {
    Some(match kind {
        "levels" => json!({"inBlack": 20, "gamma": 1.0, "inWhite": 220, "outBlack": 0, "outWhite": 255}),
        "curves" => json!({"points": [[0, 0], [127, 191], [255, 255]]}),
        "exposure" => json!({"exposure": 1.0, "offset": 0.0, "gamma": 1.0}),
        "hueSaturation" => json!({"hue": 0, "saturation": 30, "lightness": 0, "colorize": false}),
        "colorBalance" => json!({"shadows": [0, 0, 0], "midtones": [20, -15, 15], "highlights": [0, 0, 0], "preserveLuminosity": true}),
        "blackWhite" => json!({"reds": 40, "yellows": 60, "greens": 40, "cyans": 60, "blues": 20, "magentas": 80, "tint": false}),
        // Warming Filter (85), the reference application's Lab (67.06, 32, 120) shown in sRGB.
        "photoFilter" => json!({"filter": "warming85", "density": 25, "preserveLuminosity": true}),
        "channelMixer" => json!({"red": [80, 10, 10, 5], "green": [0, 100, 0, 0], "blue": [0, 0, 100, 0], "monochrome": false}),
        "colorLookup" => json!({"lut": "fadedFilm"}),
        "selectiveColor" => json!({"method": "relative", "reds": [-20, 100, -60, 40]}),
        "invert" => json!({}),
        "posterize" => json!({"levels": 4}),
        "threshold" => json!({"level": 128}),
        "gradientMap" => json!({"stops": [[0, "#000000"], [1, "#ffffff"]], "reverse": false, "dither": false}),
        _ => return None,
    })
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The adjustment a new layer of `kind` starts with in a document of `mode`.
pub fn starting_adjustment(kind: &str, mode: ColorMode) -> Result<Adjustment> {
    let p = starting_params(kind).ok_or_else(|| bad(format!("`{kind}` is not available yet")))?;
    crate::adjust_params::from_params(kind, &p, None, mode)
}

/// The mask colours of the stroke: white adds the adjustment, black (Subtract) takes it away.
fn paint_colors(subtract: bool) -> ([f32; 4], [f32; 4]) {
    let (white, black) = ([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]);
    if subtract { (black, white) } else { (white, black) }
}

/// The kind of the active layer when it is an adjustment layer (the dropdown follows it).
pub fn active_kind(s: &Session) -> Option<&'static str> {
    let st = s.active()?;
    let layer = st.doc.layer(st.active_layer?)?;
    match &layer.content {
        LayerContent::Adjustment(adj) => Some(adjustment_kind(adj)).filter(|k| entry_of(k).is_some()),
        _ => None,
    }
}

fn kind_param(p: &Value) -> Result<&str> {
    match p.get("kind") {
        None | Some(Value::Null) => Ok(DEFAULT_KIND),
        Some(Value::String(k)) => entry_of(k).map(|e| e.kind).ok_or_else(|| bad(format!("unknown adjustment `{k}`"))),
        Some(other) => Err(bad(format!("`kind` must be an adjustment id, not {other}"))),
    }
}

fn subtract_param(p: &Value) -> Result<bool> {
    match p.get("subtract") {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad("`subtract` must be true or false")),
    }
}

/// `paint.adjustmentBrush`.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.is_object() {
        return Err(bad("params must be an object"));
    }
    let (mode, depth) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        (st.doc.mode, st.doc.depth)
    };
    if mode == ColorMode::Indexed {
        return Err(EngineError::Other(INDEXED_ALERT.into()));
    }
    let kind = kind_param(p)?;
    if MISSING.contains(&kind) {
        return Err(bad(format!("`{kind}` is not available yet")));
    }
    if !enabled_in(kind, mode, depth) {
        return Err(bad(format!("`{kind}` is not available in this document's color mode or bit depth")));
    }
    let subtract = subtract_param(p)?;
    let (fg, bg) = paint_colors(subtract);
    // The stroke params without the tool's own keys, painting the mask in its colour.
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.remove("kind");
        o.remove("subtract");
        o.remove("erase");
        o.remove("mode");
        o.remove("fill");
        o.insert("color".into(), json!([fg[0], fg[1], fg[2]]));
        o.insert("target".into(), json!("mask"));
    }
    let pts = super::brush_cmds::parse_points(&q, CMD)?;
    let mut brush = super::brush_cmds::resolve_brush(s, &q, CMD)?;
    brush.color = fg;
    brush.color_cmyk = None;
    brush.erase = false;
    brush.mode = photocraft_color::BlendMode::Normal;
    brush.behind = false;
    let zoom = q.get("zoom").and_then(Value::as_f64).filter(|z| z.is_finite() && *z > 0.0).unwrap_or(1.0) as f32;

    // An adjustment layer of this kind with a mask is active: paint into its mask.
    let paints_active =
        active_kind(s) == Some(kind) && s.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).is_some_and(|l| l.mask.is_some());
    let result = if paints_active {
        let mut r = super::brush_cmds::stroke_into(s, CMD, &q, STROKE_LABEL, brush, pts)?;
        r["created"] = json!(false);
        r
    } else {
        let adj = starting_adjustment(kind, mode)?;
        let label = entry_of(kind).map_or(kind, |e| e.label);
        let (id, damage) = s.edit(NEW_LAYER_LABEL, |doc, active| {
            let mut l = Layer::new(doc.next_layer_name(label), LayerContent::Adjustment(adj));
            // The selection becomes the mask and goes away; the stroke adds to it unclipped.
            let (mut mask, revealed) = match selection_mask(doc)? {
                Some(mask) => {
                    let revealed = doc.selection.take().map_or(photocraft_geom::Rect::EMPTY, |sel| sel.content_bounds());
                    (mask, revealed)
                }
                None => (mask_at_depth(LayerMask::hide_all(), doc.depth)?, photocraft_geom::Rect::EMPTY),
            };
            let clip = doc.bounds().union(&mask.surface.tile_bounds());
            let painted = photocraft_paint::render_stroke_clipped(&mut mask.surface, &brush, &pts, None, false, zoom, Some(clip));
            l.mask = Some(mask);
            let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
            *active = Some(id);
            // The image changes where the mask reveals the adjustment: the selection and the stroke.
            Ok((id, painted.union(&revealed).intersect(&doc.bounds())))
        })?;
        s.hint_damage(damage);
        json!({"layer": id.0, "created": true, "damage": [damage.x0, damage.y0, damage.width(), damage.height()]})
    };
    // The new layer's mask is the target, and the colours are the stroke's.
    crate::commands::channel_cmds::target_layer_mask(s);
    s.tools.foreground = fg;
    s.tools.background = bg;
    Ok(result)
}

/// Enabled with a document open.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.active().is_none() {
        return Err("no document".into());
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Adjustment Brush",
        menu: &[],
        shortcut: None,
        params: r#"{"kind":"levels|curves|exposure|hueSaturation|colorBalance|blackWhite|photoFilter|channelMixer|colorLookup|selectiveColor|invert|posterize|threshold|gradientMap"="levels","subtract":bool=false (paint black: remove the adjustment),"points":[[x,y,pressure?],…],"size":px=tool size,"hardness":0..1,"opacity":0..1,"flow":0..1,"smoothing":0..1,"seed":u64?,"zoom":number=1} → {"layer"?,"created":bool,"damage"?} (with an adjustment layer of this kind active, the stroke paints its mask; else it makes the layer above the active one, its mask black or the selection, and paints the stroke white)"#,
        enabled,
        run,
        journal: true,
    }]
}
