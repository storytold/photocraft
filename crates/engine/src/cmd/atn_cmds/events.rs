const DIFFUSE_MODE: &[(&str, &str)] = &[("Nrml", "normal"), ("DrkO", "darkenOnly"), ("LghO", "lightenOnly"), ("Anst", "anisotropic")];
use super::adjustments::{adjustment_by_class, adjustment_steps, levels_from, levels_to, shadows_highlights_from, shadows_highlights_to};
use super::descriptors::{desc, get_bool, get_ref, jbool, layer_ref, layer_target, path_text, path_value, ref_class, target_ref};
use super::fields::{F, Kind, Mapped, fields_from, fields_to, none, step};
use super::steps_artboard::{artboard_edit_from, artboard_edit_to, artboard_from, artboard_to};
use super::steps_doc::{
    align_from, align_to, canvas_size_from, canvas_size_to, close_from, close_to, conditional_from, conditional_to, convert_mode_from, convert_mode_to,
    crop_from, crop_to, export_from, export_to, image_size_from, image_size_to, merge_visible_from, merge_visible_to, open_from, open_to, paste_from, paste_to,
    place_from, place_to, profile_from, profile_to, purge_from, purge_to, reset_from, reset_to, save_from, save_to, stop_from, stop_to,
};
use super::steps_image::{
    GALLERY_IDS, REFINE_EDGE, RIPPLE_SIZE, SMART_BRUSH, apply_image_from, apply_image_to, camera_raw_from, camera_raw_to, color_range_from, color_range_to,
    gallery_from, gallery_to, has_refine_strokes, radial_blur_from, radial_blur_to, refine_edge_to, transform_from, transform_to,
};
use super::steps_layer::{
    STROKE, apply_comp_from, apply_comp_to, clear_from, clear_to, delete_from, delete_to, duplicate_from, duplicate_to, fill_from, fill_to, flip_from, flip_to,
    move_from, move_to, play_from, play_to, rotate_from, rotate_to, select_from, select_to, stroke_to, visibility_from, visibility_to,
};
use super::steps_make::{locking_from, locking_to, make_from, make_to, set_from, set_to};
use super::tables::{NOISE, OFFSET_FILL, SMART_BLUR_MODE, SMART_BLUR_QUALITY, TRIM};
use super::{AtnEvent, D, Descriptor, Map, STOP_MESSAGE, Value, json};

/// An event: code (empty when Photoshop only has a string name), string name, the panel
/// label, the command ids it exports (the first is the one simple events import to), and the
/// two converters.
pub(super) struct Event {
    pub(super) code: &'static str,
    pub(super) name: &'static str,
    pub(super) label: &'static str,
    pub(super) ids: &'static [&'static str],
    pub(super) from: fn(&Descriptor) -> Mapped,
    to: fn(&str, &Value) -> Option<Descriptor>,
}

macro_rules! ev {
    ($code:literal, $name:literal, $label:literal, $id:literal, $fields:expr) => {
        ev!($code, $name, $label, $id, $fields, unless | _ | false)
    };
    // `unless`: params the descriptor can't carry, so such a step isn't exported (it would replay
    // differently) rather than losing them.
    ($code:literal, $name:literal, $label:literal, $id:literal, $fields:expr, unless $skip:expr) => {
        Event {
            code: $code,
            name: $name,
            label: $label,
            ids: &[$id],
            from: |d| match fields_from(d, $fields) {
                Some(m) => step($id, Value::Object(m)),
                None => Mapped::Unsupported { harmless: false },
            },
            to: |_, p| if pixel_target(p) && !($skip)(p) && holds_its_settings($id, p) { fields_to(p, $fields, desc("null")) } else { None },
        }
    };
}

/// Minimum / Maximum with Preserve: Roundness. The descriptor here maps the radius only, and
/// Photoshop's encoding of the shape isn't verified, so such a step is not exported: it would
/// replay as the default Squareness (#7233).
fn preserves_roundness(p: &Value) -> bool {
    p.get("preserve").and_then(Value::as_str) == Some("roundness")
}

/// Whether any of the boolean `flags` is on in `p` (present, not null and not `false`).
fn flag_set(p: &Value, flags: &[&str]) -> bool {
    flags.iter().any(|f| p.get(*f).is_some_and(|v| !v.is_null() && *v != Value::Bool(false)))
}

/// Select > Modify with "Apply effect at canvas bounds": the descriptor has no field for it, so such a
/// step is not exported (it would replay without the option).
fn applies_at_canvas_bounds(p: &Value) -> bool {
    flag_set(p, &["applyAtCanvasBounds"])
}

/// Settings a mapped event has no field for, by command: `(param, the value the event plays as)`.
/// Photoshop's event has nowhere to put these, so a step that sets one cannot be written: it
/// would import as a different edit, silently. Such a step is refused, the way [`pixel_target`]
/// refuses a filter aimed at a mask -- and no key Photoshop does not define is invented for it,
/// which the clean-room rule forbids and which would make the file wrong in another reader.
///
/// A step that leaves the setting at the value the event plays as exports as before.
/// How a setting the event cannot carry behaves once the step is replayed without it. Which of
/// the two a setting takes follows the command's own reading of it, not the parameter spec.
enum Lost {
    /// Replay uses this value, so only a step already at it survives. Numbers and strings are
    /// both compared as written, since the commands below read these settings as written.
    ReplaysAs(&'static str),
    /// The command reads only this one spelling as anything other than what replay does, so every
    /// other value survives -- a differently cased one included, because it too plays as replay's.
    DiffersAt(&'static str),
}

const UNHOLDABLE: &[(&str, &[(&str, Lost)])] = &[
    // Crst carries ClSz only; replay seeds the pattern from 0 (#7668).
    ("filter.pixelate.crystallize", &[("seed", Lost::ReplaysAs("0"))]),
    // RdlB carries Amnt, BlrM and BlrQ; replay spins or zooms about the middle of the layer.
    ("filter.blur.radialBlur", &[("centerX", Lost::ReplaysAs("0.5")), ("centerY", Lost::ReplaysAs("0.5"))]),
    // Mnm /Mxm carry Rds only; replay keeps squareness, and so does the command for anything but
    // the exact string "roundness" -- see `preserve` in filters.rs.
    ("filter.other.minimum", &[("preserve", Lost::DiffersAt("roundness"))]),
    ("filter.other.maximum", &[("preserve", Lost::DiffersAt("roundness"))]),
];

/// Whether every setting of `p` that its event cannot hold would play the same without it
/// (see [`UNHOLDABLE`]).
fn holds_its_settings(id: &str, p: &Value) -> bool {
    let Some((_, settings)) = UNHOLDABLE.iter().find(|(cmd, _)| *cmd == id) else { return true };
    settings.iter().all(|(param, lost)| match (p.get(param), lost) {
        (None | Some(Value::Null), _) => true,
        (Some(v), Lost::ReplaysAs(played)) => v.as_str() == Some(played) || v.as_f64().is_some_and(|n| played.parse::<f64>().is_ok_and(|d| n == d)),
        (Some(v), Lost::DiffersAt(spelling)) => v.as_str() != Some(spelling),
    })
}

/// Whether `p` edits the layer's pixels: no `"target"` or `"target":"pixels"`. Photoshop's
/// descriptor for a plain event has no field for a mask, Quick Mask or channel target, so such
/// a step can't be exported faithfully.
pub(super) fn pixel_target(p: &Value) -> bool {
    p.get("target").is_none_or(|t| t.as_str() == Some("pixels"))
}

fn levels_event_from(d: &Descriptor) -> Mapped {
    if get_bool(d, "Auto") == Some(true) {
        return none("image.autoTone");
    }
    if get_bool(d, "AuCo") == Some(true) {
        return none("image.autoContrast");
    }
    match levels_from(d) {
        Some(p) => step("image.adjustments.levels", p),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn levels_event_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "image.autoTone" => desc("null").with("Auto", D::Boolean(true)),
        "image.autoContrast" => desc("null").with("AuCo", D::Boolean(true)),
        _ => levels_to(p)?,
    })
}

/// A direct adjustment event (Image › Adjustments › …). The command id is `image.adjustments.<name>`
/// unless it is given.
macro_rules! adjustment_event {
    ($code:literal, $class:literal, $name:literal, $label:literal) => {
        adjustment_event!($code, $class, $name, $label, concat!("image.adjustments.", $name))
    };
    ($code:literal, $class:literal, $name:literal, $label:literal, $id:expr) => {
        Event {
            code: $code,
            name: $name,
            label: $label,
            ids: &[$id],
            from: |d| match adjustment_by_class($class) {
                Some(a) => adjustment_steps(a, "image.adjustments.", Some(d)),
                None => Mapped::Unsupported { harmless: false },
            },
            to: |_, p| adjustment_by_class($class).and_then(|a| (a.to)(p)),
        }
    };
}

/// Create / Release Clipping Mask (`groupEvent` / `ungroup`) on the target or a named layer.
fn clipping_from(d: &Descriptor, id: &str) -> Mapped {
    match get_ref(d, "null") {
        None => none(id),
        Some(r) if ref_class(r).as_deref() == Some("Lyr") => match layer_target(r) {
            Some(p) => step(id, Value::Object(p)),
            None => Mapped::Unsupported { harmless: false },
        },
        Some(_) => Mapped::Unsupported { harmless: false },
    }
}

fn clipping_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", layer_ref(p)?))
}

// Adapter-only transport bit: the reference permits either no target key or the
// current-layer target. Save must retain that distinction; native behavior is identical.
const NO_TARGET: &str = "_atnNoTarget";

/// Accept only bare or current-layer targets, retaining their key-presence distinction for save.
fn target_params(d: &Descriptor) -> Option<Map<String, Value>> {
    let mut p = Map::new();
    if d.items.is_empty() {
        p.insert(NO_TARGET.into(), Value::Bool(true));
    } else if d.items != desc("null").with("null", target_ref("Lyr ")).items {
        return None;
    }
    Some(p)
}

/// Restore the bare or explicit current-layer descriptor selected by the private transport bit.
fn target_descriptor(p: &Value) -> Option<Descriptor> {
    match p.get(NO_TARGET) {
        None => Some(desc("null").with("null", target_ref("Lyr "))),
        Some(Value::Bool(true)) => Some(desc("null")),
        Some(_) => None,
    }
}

// Native-parameter allowlist adapted from #10628; refuse unrepresented options on export.
/// Refuse native options that the selected Photoshop form cannot represent.
fn params(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_some_and(|p| p.keys().all(|key| allowed.contains(&key.as_str())))
}

/// Import a target-only event, refusing other targets and extra descriptor settings.
fn target_only_from(d: &Descriptor, id: &str) -> Mapped {
    target_params(d).map_or(Mapped::Unsupported { harmless: false }, |p| step(id, Value::Object(p)))
}

/// Export a current-target native step only when no unrepresented options are present.
fn target_only_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &[NO_TARGET]) {
        return None;
    }
    target_descriptor(p)
}

const SHAPE_BLUR: &[F] = &[F("Rds ", "radius", Kind::Px)];

/// Map the radius-only Shape Blur form; keep custom assets or extra keys unsupported.
fn shape_blur_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(p) = fields_from(d, SHAPE_BLUR) else { return unsupported };
    let p = Value::Object(p);
    if shape_blur_to("filter.blur.shapeBlur", &p).is_some_and(|expected| d.items == expected.items) { step("filter.blur.shapeBlur", p) } else { unsupported }
}

/// Write an optional finite 5–1000 pixel radius, refusing explicit shapes and other options.
fn shape_blur_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &["radius"]) {
        return None;
    }
    if let Some(radius) = p.get("radius") {
        let radius = radius.as_f64()?;
        if !radius.is_finite() || !(5.0..=1000.0).contains(&radius) {
            return None;
        }
    }
    fields_to(p, SHAPE_BLUR, desc("null"))
}

// Recorded Link and Unlink need explicit state: replaying the UI toggle can
// perform the opposite operation. Target matching follows #10628's adapter.
/// Import Link or Unlink as an explicit operation rather than the state-dependent UI toggle.
fn link_from(d: &Descriptor, linked: bool) -> Mapped {
    let Some(mut p) = target_params(d) else { return Mapped::Unsupported { harmless: false } };
    p.insert("linked".into(), Value::Bool(linked));
    step("layer.linkLayers", Value::Object(p))
}

/// Export only the matching explicit link state and supported current-layer target form.
fn link_to(p: &Value, linked: bool) -> Option<Descriptor> {
    if !params(p, &["linked", NO_TARGET]) || p.get("linked") != Some(&Value::Bool(linked)) {
        return None;
    }
    target_descriptor(p)
}

/// Map canonical Smart Object file descriptors, refusing unrepresented paths, targets or keys.
fn smart_path_from(d: &Descriptor, id: &str) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let key = if id == "layer.smartObjects.convertToLinked" { "Usng" } else { "null" };
    let Some(path) = d.get(key).and_then(path_text) else { return unsupported };
    let p = json!({"path": path});
    let Some(mut expected) = smart_path_to(id, &p) else { return unsupported };
    if let Some(hint) = d.get("LyrI") {
        // The reference explicitly omits recording-session IDs on save (#10603,
        // docs/atn.md:200). They must never become PhotoCraft's playback target.
        if !matches!(id, "layer.smartObjects.relinkToFile" | "layer.smartObjects.replaceContents") || !matches!(hint, D::Integer(n) if *n > 0) {
            return unsupported;
        }
        expected = expected.with("LyrI", hint.clone());
    }
    // Normalize the top-level header as Open/Place do, but refuse unknown keys,
    // targets and path encodings rather than partly replaying them.
    if d.items == expected.items { step(id, p) } else { unsupported }
}

/// Export path-only native parameters using the existing Open/Place path encoding.
fn smart_path_to(id: &str, p: &Value) -> Option<Descriptor> {
    let p = p.as_object()?;
    let path = p.get("path")?.as_str()?;
    if p.len() != 1 || path.is_empty() || path.contains('\0') {
        return None;
    }
    Some(if id == "layer.smartObjects.convertToLinked" {
        desc("null").with("null", target_ref("Lyr ")).with("Usng", path_value(path))
    } else {
        desc("null").with("null", path_value(path))
    })
}

/// A clipping mask event: `clipping_from` / `clipping_to` with one command id.
macro_rules! clipping_event {
    ($code:literal, $name:literal, $label:literal, $id:literal) => {
        Event { code: $code, name: $name, label: $label, ids: &[$id], from: |d| clipping_from(d, $id), to: clipping_to }
    };
}

macro_rules! target_event {
    ($name:literal, $label:literal, $id:literal) => {
        Event { code: "", name: $name, label: $label, ids: &[$id], from: |d| target_only_from(d, $id), to: target_only_to }
    };
}

/// Every Photoshop event PhotoCraft maps. The order matters only for the panel label.
pub(super) const EVENTS: &[Event] = &[
    // Filters.
    ev!("GsnB", "gaussianBlur", "Gaussian Blur", "filter.blur.gaussianBlur", &[F("Rds ", "radius", Kind::Px)]),
    ev!(
        "UnsM",
        "unsharpMask",
        "Unsharp Mask",
        "filter.sharpen.unsharpMask",
        &[F("Amnt", "amount", Kind::Pct), F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]
    ),
    ev!(
        "AdNs",
        "addNoise",
        "Add Noise",
        "filter.noise.addNoise",
        &[
            F("Dstr", "distribution", Kind::Enum("Dstr", NOISE)),
            F("Nose", "amount", Kind::Pct),
            F("Mnch", "monochromatic", Kind::Bool),
            F("FlRs", "seed", Kind::Long)
        ]
    ),
    ev!("Mdn ", "median", "Median", "filter.noise.median", &[F("Rds ", "radius", Kind::Px)]),
    ev!("DstS", "dustAndScratches", "Dust & Scratches", "filter.noise.dustAndScratches", &[F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]),
    ev!("Dspc", "despeckle", "Despeckle", "filter.noise.despeckle", &[]),
    // Photoshop records Emboss's angle, height and amount as plain integers (#7946).
    ev!(
        "Embs",
        "emboss",
        "Emboss",
        "filter.stylize.emboss",
        &[F("Angl", "angle", Kind::Long), F("Hght", "height", Kind::Long), F("Amnt", "amount", Kind::Long)]
    ),
    ev!("MtnB", "motionBlur", "Motion Blur", "filter.blur.motionBlur", &[F("Angl", "angle", Kind::Ang), F("Dstn", "distance", Kind::Px)]),
    ev!("", "boxblur", "Box Blur", "filter.blur.boxBlur", &[F("Rds ", "radius", Kind::Px)]),
    ev!("", "surfaceBlur", "Surface Blur", "filter.blur.surfaceBlur", &[F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]),
    Event { code: "", name: "shapeBlur", label: "Shape Blur", ids: &["filter.blur.shapeBlur"], from: shape_blur_from, to: shape_blur_to },
    ev!(
        "SmrB",
        "smartBlur",
        "Smart Blur",
        "filter.blur.smartBlur",
        &[
            F("Rds ", "radius", Kind::Dbl),
            F("Thsh", "threshold", Kind::Dbl),
            F("SmBQ", "quality", Kind::Enum("SmBQ", SMART_BLUR_QUALITY)),
            F("SmBM", "mode", Kind::Enum("SmBM", SMART_BLUR_MODE))
        ]
    ),
    Event { code: "RdlB", name: "radialBlur", label: "Radial Blur", ids: &["filter.blur.radialBlur"], from: radial_blur_from, to: radial_blur_to },
    ev!("Blr ", "blur", "Blur", "filter.blur.blur", &[]),
    ev!("BlrM", "blurMore", "Blur More", "filter.blur.blurMore", &[]),
    ev!("Shrp", "sharpen", "Sharpen", "filter.sharpen.sharpen", &[]),
    ev!("ShrM", "sharpenMore", "Sharpen More", "filter.sharpen.sharpenMore", &[]),
    ev!("ShrE", "sharpenEdges", "Sharpen Edges", "filter.sharpen.sharpenEdges", &[]),
    ev!("HghP", "highPass", "High Pass", "filter.other.highPass", &[F("Rds ", "radius", Kind::Px)]),
    ev!("Mnm ", "minimum", "Minimum", "filter.other.minimum", &[F("Rds ", "radius", Kind::Px)], unless preserves_roundness),
    ev!("Mxm ", "maximum", "Maximum", "filter.other.maximum", &[F("Rds ", "radius", Kind::Px)], unless preserves_roundness),
    ev!(
        "Ofst",
        "offset",
        "Offset",
        "filter.other.offset",
        &[F("Hrzn", "horizontal", Kind::Long), F("Vrtc", "vertical", Kind::Long), F("Fl  ", "undefinedAreas", Kind::Enum("FlMd", OFFSET_FILL))]
    ),
    ev!("Rple", "ripple", "Ripple", "filter.distort.ripple", &[F("Amnt", "amount", Kind::ExactLong), F("RplS", "size", Kind::Enum("RplS", RIPPLE_SIZE))]),
    ev!("Crst", "crystallize", "Crystallize", "filter.pixelate.crystallize", &[F("ClSz", "cellSize", Kind::ExactLong)]),
    ev!("Dfs ", "diffuse", "Diffuse", "filter.stylize.diffuse", &[F("Md  ", "mode", Kind::Enum("DfsM", DIFFUSE_MODE)), F("FlRs", "seed", Kind::Long)]),
    Event { code: "GEfc", name: "filterGallery", label: "Filter Gallery", ids: GALLERY_IDS, from: gallery_from, to: gallery_to },
    // What Photoshop needs in a written step (process version, the full settings set) is
    // unverified, so only an imported step saves back, as its recorded descriptor while unedited.
    Event { code: "", name: "Adobe Camera Raw Filter", label: "Camera Raw Filter", ids: &[crate::lens_cmds::RAW], from: camera_raw_from, to: camera_raw_to },
    // Adjustments (Image › Adjustments).
    Event {
        code: "",
        name: "adaptCorrect",
        label: "Shadows/Highlights",
        ids: &["image.adjustments.shadowsHighlights"],
        from: shadows_highlights_from,
        to: shadows_highlights_to,
    },
    Event {
        code: "Lvls",
        name: "levels",
        label: "Levels",
        ids: &["image.adjustments.levels", "image.autoTone", "image.autoContrast"],
        from: levels_event_from,
        to: levels_event_to,
    },
    adjustment_event!("Crvs", "Crvs", "curves", "Curves"),
    adjustment_event!("HStr", "HStr", "hueSaturation", "Hue/Saturation"),
    adjustment_event!("BrgC", "BrgC", "brightnessEvent", "Brightness/Contrast", "image.adjustments.brightnessContrast"),
    adjustment_event!("Thrs", "Thrs", "threshold", "Threshold"),
    adjustment_event!("Pstr", "Pstr", "posterize", "Posterize"),
    adjustment_event!("Invr", "Invr", "invert", "Invert"),
    adjustment_event!("ClrB", "ClrB", "colorBalance", "Color Balance"),
    adjustment_event!("Vbrn", "vibrance", "vibrance", "Vibrance"),
    adjustment_event!("Exps", "Exps", "exposure", "Exposure"),
    // Photoshop runs Photo Filter by name only: `PhtF` is the class, not an event (#8301).
    adjustment_event!("", "PhtF", "photoFilter", "Photo Filter"),
    adjustment_event!("", "BanW", "blackAndWhite", "Black & White", "image.adjustments.blackWhite"),
    adjustment_event!("ChnM", "ChnM", "channelMixer", "Channel Mixer"),
    adjustment_event!("GdMp", "GdMp", "gradientMapEvent", "Gradient Map", "image.adjustments.gradientMap"),
    adjustment_event!("SlcC", "SlcC", "selectiveColor", "Selective Color"),
    ev!("Dstt", "desaturate", "Desaturate", "image.adjustments.desaturate", &[]),
    ev!("Eqlz", "equalize", "Equalize", "image.adjustments.equalize", &[]),
    ev!("", "autoColor", "Auto Color", "image.autoColor", &[]),
    // Image.
    Event { code: "ImgS", name: "imageSize", label: "Image Size", ids: &["image.imageSize"], from: image_size_from, to: image_size_to },
    Event { code: "CnvS", name: "canvasSize", label: "Canvas Size", ids: &["image.canvasSize"], from: canvas_size_from, to: canvas_size_to },
    Event {
        code: "Rtte",
        name: "rotateEventEnum",
        label: "Rotate",
        ids: &[
            "image.imageRotation.90cw",
            "image.imageRotation.90ccw",
            "image.imageRotation.180",
            "image.rotation.arbitrary",
            "edit.transform.rotate90Cw",
            "edit.transform.rotate90Ccw",
            "edit.transform.rotate180",
        ],
        from: rotate_from,
        to: rotate_to,
    },
    Event {
        code: "Flip",
        name: "flip",
        label: "Flip",
        ids: &[
            "image.imageRotation.flipCanvasHorizontal",
            "image.imageRotation.flipCanvasVertical",
            "edit.transform.flipHorizontal",
            "edit.transform.flipVertical",
        ],
        from: flip_from,
        to: flip_to,
    },
    Event { code: "Crop", name: "crop", label: "Crop", ids: &["image.crop"], from: crop_from, to: crop_to },
    ev!(
        "",
        "trim",
        "Trim",
        "image.trim",
        &[
            F("trimBasedOn", "basedOn", Kind::Enum("trimBasedOn", TRIM)),
            F("Top ", "top", Kind::Bool),
            F("Btom", "bottom", Kind::Bool),
            F("Left", "left", Kind::Bool),
            F("Rght", "right", Kind::Bool)
        ]
    ),
    ev!("RvlA", "revealAll", "Reveal All", "image.revealAll", &[]),
    Event {
        code: "CnvM",
        name: "convertMode",
        label: "Convert Mode",
        ids: &[
            "image.mode.rgb",
            "image.mode.grayscale",
            "image.mode.cmyk",
            "image.mode.lab",
            "image.mode.multichannel",
            "image.mode.bits8",
            "image.mode.bits16",
            "image.mode.bits32",
            "image.mode.bitmap",
        ],
        from: convert_mode_from,
        to: convert_mode_to,
    },
    Event { code: "Dplc", name: "duplicate", label: "Duplicate", ids: &["layer.duplicate", "image.duplicate"], from: duplicate_from, to: duplicate_to },
    // Layers.
    target_event!("reverse", "Reverse", "layer.arrange.reverse"),
    target_event!("selectLinkedLayers", "Select Linked Layers", "layer.selectLinkedLayers"),
    target_event!("disableLayerStyle", "Clear Layer Style", "layer.layerStyle.clear"),
    target_event!("rasterizeAll", "Rasterize All Layers", "layer.rasterize.allLayers"),
    target_event!("addBlankVideoLayer", "New Blank Video Layer", "layer.videoLayers.newBlankVideoLayer"),
    target_event!("isolateLayers", "Isolate Layers", "select.isolateLayers"),
    target_event!("textLoadDefaultTypeStyles", "Load Default Type Styles", "type.loadDefaultTypeStyles"),
    target_event!("textSaveDefaultTypeStyles", "Save Default Type Styles", "type.saveDefaultTypeStyles"),
    target_event!("recordMeasurements", "Record Measurements", "image.analysis.recordMeasurements"),
    target_event!("CropPhotosAuto0001", "Crop and Straighten Photos", "file.automate.cropAndStraightenPhotos"),
    Event {
        code: "",
        name: "artboardFromLayerGroupEvent",
        label: "Artboard from Group",
        ids: &["layer.new.artboardFromGroup"],
        from: artboard_from,
        to: artboard_to,
    },
    Event { code: "", name: "editArtboardEvent", label: "Edit Artboard", ids: &["layer.artboard.set"], from: artboard_edit_from, to: artboard_edit_to },
    Event { code: "", name: "linkSelectedLayers", label: "Link Layers", ids: &["layer.linkLayers"], from: |d| link_from(d, true), to: |_, p| link_to(p, true) },
    Event {
        code: "",
        name: "unlinkSelectedLayers",
        label: "Unlink Layers",
        ids: &["layer.linkLayers"],
        from: |d| link_from(d, false),
        to: |_, p| link_to(p, false),
    },
    Event {
        code: "Mk  ",
        name: "make",
        label: "Make",
        ids: &[
            "type.convertToShape",
            "layer.new.layer",
            "layer.new.group",
            "layer.groupLayers",
            "layer.duplicate",
            "layer.new.layerViaCopy",
            "layer.new.layerViaCut",
            "layer.newFillLayer.solidColor",
            "layer.newFillLayer.pattern",
            "layer.newAdjustmentLayer.levels",
            "layer.newAdjustmentLayer.curves",
            "layer.newAdjustmentLayer.hueSaturation",
            "layer.newAdjustmentLayer.brightnessContrast",
            "layer.newAdjustmentLayer.threshold",
            "layer.newAdjustmentLayer.posterize",
            "layer.newAdjustmentLayer.invert",
            "layer.newAdjustmentLayer.colorBalance",
            "layer.newAdjustmentLayer.vibrance",
            "layer.newAdjustmentLayer.exposure",
            "layer.newAdjustmentLayer.photoFilter",
            "layer.newAdjustmentLayer.blackWhite",
            "layer.newAdjustmentLayer.channelMixer",
            "layer.newAdjustmentLayer.selectiveColor",
            "layer.newAdjustmentLayer.gradientMap",
            "layer.layerMask.revealAll",
            "layer.layerMask.hideAll",
            "layer.layerMask.revealSelection",
            "layer.layerMask.hideSelection",
            "file.new",
            "file.newFromClipboard",
            "history.newSnapshot",
        ],
        from: make_from,
        to: make_to,
    },
    Event {
        code: "setd",
        name: "set",
        label: "Set",
        ids: &[
            "select.all",
            "select.deselect",
            "select.reselect",
            "select.loadSelection",
            "select.rect",
            "select.lasso",
            "select.editInQuickMaskMode",
            "tools.setColors",
            "layer.setProps",
            "layer.renameLayer",
            "layer.setLabelColor",
            "layer.setAdjustment",
            "layer.fill.pattern.set",
            "layer.new.layerFromBackground",
            "layer.layerStyle.blendingOptions",
            "layer.smartFilter.blendingOptions",
        ],
        from: set_from,
        to: set_to,
    },
    Event {
        code: "slct",
        name: "select",
        label: "Select",
        ids: &[
            "layer.select",
            "layer.selectAbove",
            "layer.selectBelow",
            "layer.addAboveToSelection",
            "layer.addBelowToSelection",
            "layer.selectTop",
            "layer.selectBottom",
            "channel.target",
            "edit.undo",
            "edit.redo",
            "view.fitOnScreen",
            "view.actualPixels",
            "view.zoomIn",
            "view.zoomOut",
            "document.activate",
        ],
        from: select_from,
        to: select_to,
    },
    Event {
        code: "Dlt ",
        name: "delete",
        label: "Delete",
        ids: &["layer.delete", "edit.clear", "layer.layerMask.delete", "layer.smartFilter.deleteFilterMask"],
        from: delete_from,
        to: delete_to,
    },
    Event { code: "", name: "applyLocking", label: "Apply Locking", ids: &["layer.lockLayers"], from: locking_from, to: locking_to },
    // Timeline › Make Frames from Layers: no frame animation in PhotoCraft, no document change.
    Event {
        code: "",
        name: "animationFramesFromLayers",
        label: "Make Frames from Layers",
        ids: &[],
        from: |_| Mapped::Unsupported { harmless: true },
        to: |_, _| None,
    },
    Event { code: "", name: "clearEvent", label: "Clear", ids: &["select.editInQuickMaskMode", "history.clear"], from: clear_from, to: clear_to },
    Event {
        code: "Trnf",
        name: "transform",
        label: "Transform",
        ids: &["edit.transform", "select.transformSelection"],
        from: transform_from,
        to: transform_to,
    },
    Event { code: "AppI", name: "applyImageEvent", label: "Apply Image", ids: &["image.applyImage"], from: apply_image_from, to: apply_image_to },
    ev!("", "newPlacedLayer", "Convert to Smart Object", "layer.smartObjects.convertToSmartObject", &[]),
    ev!("", "placedLayerEditContents", "Edit Contents", "layer.smartObjects.editContents", &[]),
    ev!("", "updatePlacedLayer", "Update Smart Objects", "layer.smartObjects.updateModifiedContent", &[]),
    target_event!("placedLayerMakeCopy", "New Smart Object via Copy", "layer.smartObjects.newSmartObjectViaCopy"),
    target_event!("placedLayerConvertToLayers", "Convert to Layers", "layer.smartObjects.convertToLayers"),
    target_event!("placedLayerConvertToEmbedded", "Convert to Embedded", "layer.smartObjects.convertToEmbedded"),
    target_event!("placedLayerUpdateAllModified", "Update All Modified Content", "layer.smartObjects.updateAllModifiedContent"),
    Event {
        code: "",
        name: "placedLayerExportContents",
        label: "Export Contents",
        ids: &["layer.smartObjects.exportContents"],
        from: |d| smart_path_from(d, "layer.smartObjects.exportContents"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerConvertToLinked",
        label: "Convert to Linked",
        ids: &["layer.smartObjects.convertToLinked"],
        from: |d| smart_path_from(d, "layer.smartObjects.convertToLinked"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerRelinkToFile",
        label: "Relink to File",
        ids: &["layer.smartObjects.relinkToFile"],
        from: |d| smart_path_from(d, "layer.smartObjects.relinkToFile"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerReplaceContents",
        label: "Replace Contents",
        ids: &["layer.smartObjects.replaceContents"],
        from: |d| smart_path_from(d, "layer.smartObjects.replaceContents"),
        to: smart_path_to,
    },
    Event { code: "Hd  ", name: "hide", label: "Hide", ids: &["layer.hideLayers"], from: |d| visibility_from(d, "layer.hideLayers"), to: visibility_to },
    Event { code: "Shw ", name: "show", label: "Show", ids: &["layer.showLayers"], from: |d| visibility_from(d, "layer.showLayers"), to: visibility_to },
    Event {
        code: "move",
        name: "move",
        label: "Move",
        ids: &["layer.translate", "layer.arrange.bringToFront", "layer.arrange.sendToBack", "layer.arrange.bringForward", "layer.arrange.sendBackward"],
        from: move_from,
        to: move_to,
    },
    ev!("FltI", "flattenImage", "Flatten Image", "layer.flattenImage", &[]),
    Event {
        code: "MrgV",
        name: "mergeVisible",
        label: "Merge Visible",
        ids: &["layer.mergeVisible", "layer.stampVisible"],
        from: merge_visible_from,
        to: merge_visible_to,
    },
    ev!("Mrg2", "mergeLayersNew", "Merge Layers", "layer.mergeLayers", &[]),
    clipping_event!("Ungr", "ungroup", "Release Clipping Mask", "layer.releaseClippingMask"),
    ev!("CpTL", "copyToLayer", "Layer Via Copy", "layer.new.layerViaCopy", &[]),
    ev!("CtTL", "cutToLayer", "Layer Via Cut", "layer.new.layerViaCut", &[]),
    ev!("", "rasterizeLayer", "Rasterize", "layer.rasterize.layer", &[]),
    ev!("", "selectAllLayers", "Select All Layers", "select.allLayers", &[]),
    ev!("", "selectNoLayers", "Deselect Layers", "select.deselectLayers", &[]),
    clipping_event!("GrpL", "groupEvent", "Create Clipping Mask", "layer.createClippingMask"),
    ev!("", "ungroupLayersEvent", "Ungroup Layers", "layer.ungroupLayers", &[]),
    ev!("", "copyEffects", "Copy Layer Style", "layer.layerStyle.copyLayerStyle", &[]),
    ev!("", "pasteEffects", "Paste Layer Style", "layer.layerStyle.pasteLayerStyle", &[]),
    Event {
        code: "",
        name: "collapseAllGroupsEvent",
        label: "Collapse All Groups",
        ids: &["layer.setExpanded"],
        from: |_| step("layer.setExpanded", json!({"all": true, "expanded": false})),
        to: |_, p| (jbool(p, "all") == Some(true) && jbool(p, "expanded") == Some(false)).then(|| desc("null")),
    },
    ev!("Dfrg", "defringe", "Defringe", "layer.matting.defringe", &[F("Wdth", "width", Kind::Px)]),
    Event {
        code: "Algn",
        name: "align",
        label: "Align",
        ids: &[
            "layer.align.leftEdges",
            "layer.align.horizontalCenters",
            "layer.align.rightEdges",
            "layer.align.topEdges",
            "layer.align.verticalCenters",
            "layer.align.bottomEdges",
        ],
        from: align_from,
        to: align_to,
    },
    // Selection.
    ev!("Invs", "inverse", "Inverse", "select.inverse", &[]),
    ev!("Fthr", "feather", "Feather", "select.modify.feather", &[F("Rds ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Expn", "expand", "Expand", "select.modify.expand", &[F("By  ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Cntc", "contract", "Contract", "select.modify.contract", &[F("By  ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Smth", "smoothness", "Smooth", "select.modify.smooth", &[F("Rds ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Brdr", "border", "Border", "select.modify.border", &[F("Wdth", "radius", Kind::Px)]),
    ev!("Grow", "grow", "Grow", "select.grow", &[F("Tlrn", "tolerance", Kind::Long)]),
    ev!("Smlr", "similar", "Similar", "select.similar", &[F("Tlrn", "tolerance", Kind::Long)]),
    // `refineSelectionEdge` has no Sample All Layers, so a step with it exports as
    // `smartBrushWorkspace` instead of dropping it.
    Event {
        code: "",
        name: "refineSelectionEdge",
        label: "Select and Mask",
        ids: &["select.refineEdge"],
        from: |d| match fields_from(d, REFINE_EDGE) {
            Some(m) => step("select.refineEdge", Value::Object(m)),
            None => Mapped::Unsupported { harmless: false },
        },
        to: refine_edge_to,
    },
    ev!("", "smartBrushWorkspace", "Select and Mask", "select.refineEdge", SMART_BRUSH, unless has_refine_strokes),
    ev!("", "autoCutout", "Select Subject", "select.subject", &[F("sampleAllLayers", "sampleAllLayers", Kind::Bool)]),
    Event { code: "ClrR", name: "colorRange", label: "Color Range", ids: &["select.colorRange"], from: color_range_from, to: color_range_to },
    // Edit.
    Event { code: "Fl  ", name: "fill", label: "Fill", ids: &["edit.fill", "paint.bucket"], from: fill_from, to: fill_to },
    Event {
        code: "Strk",
        name: "stroke",
        label: "Stroke",
        ids: &["edit.stroke"],
        from: |d| match fields_from(d, STROKE) {
            Some(m) => step("edit.stroke", Value::Object(m)),
            None => Mapped::Unsupported { harmless: false },
        },
        to: stroke_to,
    },
    ev!("copy", "copyEvent", "Copy", "edit.copy", &[]),
    ev!("CpyM", "copyMerged", "Copy Merged", "edit.copyMerged", &[]),
    ev!("cut ", "cut", "Cut", "edit.cut", &[]),
    Event { code: "past", name: "paste", label: "Paste", ids: &["edit.paste", "edit.pasteSpecial.pasteInPlace"], from: paste_from, to: paste_to },
    Event {
        code: "Prge",
        name: "purge",
        label: "Purge",
        ids: &["edit.purge.all", "edit.purge.clipboard", "edit.purge.histories", "edit.purge.undo"],
        from: purge_from,
        to: purge_to,
    },
    Event { code: "Rset", name: "reset", label: "Reset", ids: &["tools.defaultColors"], from: reset_from, to: reset_to },
    ev!("Exch", "exchange", "Exchange", "tools.swapColors", &[]),
    Event {
        code: "",
        name: "assignProfile",
        label: "Assign Profile",
        ids: &["edit.assignProfile"],
        from: |d| profile_from(d, "edit.assignProfile"),
        to: profile_to,
    },
    Event {
        code: "",
        name: "convertToProfile",
        label: "Convert to Profile",
        ids: &["edit.convertToProfile"],
        from: |d| profile_from(d, "edit.convertToProfile"),
        to: profile_to,
    },
    // File and actions.
    Event { code: "Cls ", name: "close", label: "Close", ids: &["file.close"], from: close_from, to: close_to },
    Event { code: "Expr", name: "export", label: "Export", ids: &["file.export.saveForWebLegacy"], from: export_from, to: export_to },
    Event { code: "Plc ", name: "placeEvent", label: "Place", ids: &["file.placeEmbedded", "file.placeLinked"], from: place_from, to: place_to },
    Event { code: "Opn ", name: "open", label: "Open", ids: &["file.openAs"], from: open_from, to: open_to },
    Event { code: "", name: "conditional", label: "If", ids: &["actions.conditional"], from: conditional_from, to: conditional_to },
    Event { code: "save", name: "save", label: "Save", ids: &["file.save", "file.saveAs", "file.saveACopy"], from: save_from, to: save_to },
    ev!("Rvrt", "revert", "Revert", "file.revert", &[]),
    Event { code: "Ply ", name: "play", label: "Play", ids: &["actions.play"], from: play_from, to: play_to },
    Event { code: "", name: "applyComp", label: "Apply Layer Comp", ids: &["layerComp.apply"], from: apply_comp_from, to: apply_comp_to },
    Event { code: "Stop", name: "stop", label: "Stop", ids: &[STOP_MESSAGE], from: stop_from, to: stop_to },
];

/// Four-char codes PhotoCraft once exported for events Photoshop only runs by name (or by
/// another code), with the name they stand for. Measured in Photoshop: `executeAction` with
/// these codes fails ("the command is not currently available"), and `Fltn` is the
/// `flatness` property, not the flatten event. Import keeps accepting them so actions saved
/// before still load; export never writes them.
const LEGACY_CODES: &[(&str, &str)] =
    &[("BoxB", "boxblur"), ("SrfB", "surfaceBlur"), ("Fltn", "flattenImage"), ("BlnW", "blackAndWhite"), ("PhtF", "photoFilter")];

pub(super) fn event_for(item: &AtnEvent) -> Option<&'static Event> {
    let id = item.id();
    let find = |id: &str| EVENTS.iter().find(|e| (!e.code.is_empty() && e.code.trim_end() == id.trim_end()) || e.name == id);
    find(&id).or_else(|| match item {
        AtnEvent::Code(_) => LEGACY_CODES.iter().find(|(code, _)| *code == id).and_then(|(_, name)| find(name)),
        AtnEvent::Name(_) => None,
    })
}

/// The event `id` exports as: the first event listing it whose converter accepts the params
/// (Quick Mask on is a `set`, off a `clearEvent`).
pub(super) fn event_and_descriptor(id: &str, params: &Value) -> Option<(&'static Event, Descriptor)> {
    EVENTS.iter().filter(|e| e.ids.contains(&id)).find_map(|e| (e.to)(id, params).map(|d| (e, d)))
}
