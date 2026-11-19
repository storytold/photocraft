//! Artboards to Files' Export Options (#13636): a checkbox below File Type, unticked by default,
//! that shows an Options section with the chosen file type's settings. Each type keeps its own
//! settings (its own rows, keyed `<type>.<param>`); the command gets the plain params of the shown
//! type only while the checkbox is ticked, so an export without it writes what it wrote before.
//! The checkbox and every type's settings are rows of the form, so they are remembered after a
//! successful run only, as every remembered row is.

use super::{Label, Param, ParamKind, Value, opts};
use craft::ui::patterns::form_dialogs::DialogFields;
use craft::ui::value_drag::SliderScale;
use serde_json::Map;

pub(super) const COMMAND: &str = "file.export.artboardsToFiles";
/// The checkbox. The command never gets it.
const CHECKBOX: &str = "exportOptions";
/// The settings key of the type whose rows show now, `""` while none does: shell state the rows are shown by, kept
/// by [`derive`] before each pass, OK and automation write (one `visible_when` condition per row).
const SHOWN_TYPE: &str = "__exportOptionsType";
/// Whether the Options section shows (the checkbox is ticked and the type has options).
const SHOWN: &str = "__exportOptionsShown";
/// The settings keys of the types with options.
const KINDS: [&str; 6] = ["psd", "bmp", "tga", "jpg", "tiff", "png"];

/// The settings key of a File Type value (`jpg` and `jpeg` share one, as `tif` and `tiff` do).
fn kind(format: &str) -> Option<&'static str> {
    Some(match format.to_ascii_lowercase().as_str() {
        "psd" | "psb" => "psd",
        "bmp" => "bmp",
        "jpg" | "jpeg" => "jpg",
        "tif" | "tiff" => "tiff",
        "tga" => "tga",
        "png" | "png24" => "png",
        _ => return None,
    })
}

/// Sets the shell keys the option rows are shown by from the checkbox and the File Type.
pub(super) fn derive(fields: &mut DialogFields) {
    let shown = if fields.bool(CHECKBOX) == Some(true) { fields.text("format").and_then(kind) } else { None };
    fields.set(SHOWN_TYPE, shown.unwrap_or(""));
    fields.set(SHOWN, shown.is_some());
}

/// The rows below File Type: the checkbox, then each type's settings with the reference application's defaults, shown
/// only for the chosen type while the checkbox is ticked.
pub(super) fn rows() -> Vec<Param> {
    let of = |kind: &str, param: &str, label: &'static str, row: ParamKind| Param::new(format!("{kind}.{param}"), label, row).visible_when(SHOWN_TYPE, kind);
    let icc = |kind: &str| of(kind, "iccProfile", tl!("Include ICC Profile"), ParamKind::bool(true));
    let quality = |kind: &str| of(kind, "quality", tl!("Quality"), ParamKind::range(0.0, 12.0, 8.0, SliderScale::Linear)).decimals(1);
    let depth = |kind: &str, param: &str| {
        of(kind, param, tl!("Depth"), ParamKind::radio(opts(&[("16", tl!("16bit")), ("24", tl!("24bit")), ("32", tl!("32bit"))]), Some("24".to_owned())))
    };
    let compression = opts(&[("none", tl!("None")), ("lzw", tl!("LZW")), ("zip", tl!("ZIP")), ("jpeg", tl!("JPEG"))]);
    vec![
        Param::new(CHECKBOX, tl!("Export Options"), ParamKind::bool(false)),
        Param::section(Label::from(tl!("Options"))).visible_when(SHOWN, true),
        of("psd", "maximizeCompatibility", tl!("Maximize Compatibility"), ParamKind::bool(true)),
        of("psd", "preserveArtboard", tl!("Preserve Artboard"), ParamKind::bool(true)),
        icc("psd"),
        depth("bmp", "bmpBits"),
        icc("bmp"),
        // Targa, for when the File Type list offers it (#13635).
        depth("tga", "tgaBits"),
        icc("tga"),
        quality("jpg"),
        icc("jpg"),
        of("tiff", "tiffCompression", tl!("Image Compression"), ParamKind::choice(compression, Some("none".to_owned()))),
        // The TIFF quality applies to JPEG compression only.
        quality("tiff").enabled_when("tiff.tiffCompression", "jpeg"),
        icc("tiff"),
        of("png", "transparency", tl!("Transparency"), ParamKind::bool(true)),
        of("png", "pngInterlaced", tl!("Interlaced"), ParamKind::bool(false)),
        icc("png"),
    ]
}

/// The command params of the form's values `map`: the chosen type's settings under their plain names while the
/// checkbox is ticked, else none; every type's own row keys and the checkbox go. TIFF sends its quality only with JPEG
/// compression, and the depths are numbers. Values without the checkbox come from the editor of a recorded step,
/// whose rows are the command's own plain params: they go to the command as they are.
pub(crate) fn nest(map: &mut Map<String, Value>) {
    let Some(on) = map.remove(CHECKBOX) else { return };
    let on = on.as_bool() == Some(true);
    let chosen = map.get("format").and_then(Value::as_str).and_then(kind).filter(|_| on);
    let own: Vec<String> = map.keys().filter(|k| k.split_once('.').is_some_and(|(kind, _)| KINDS.contains(&kind))).cloned().collect();
    let mut settings = Map::new();
    for key in own {
        let value = map.remove(&key);
        if let (Some((kind, param)), Some(value)) = (key.split_once('.'), value)
            && Some(kind) == chosen
        {
            settings.insert(param.to_owned(), value);
        }
    }
    // The quality comes from the options only.
    map.remove("quality");
    let jpeg_tiff = settings.get("tiffCompression").and_then(Value::as_str) == Some("jpeg");
    for (param, value) in settings {
        let value = match param.as_str() {
            "bmpBits" | "tgaBits" => value.as_str().and_then(|bits| bits.parse::<u64>().ok()).map_or(value, Value::from),
            "quality" if chosen == Some("tiff") && !jpeg_tiff => continue,
            _ => value,
        };
        map.insert(param, value);
    }
}
