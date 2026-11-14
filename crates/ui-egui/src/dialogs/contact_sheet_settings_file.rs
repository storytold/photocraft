//! The settings file of Contact Sheet II's Load… and Save… (#13698), in the reference
//! application's layout:
//!
//! ```xml
//! <ContactSheetIISettings>
//!   <source imageSource="Folder" path="…" includeSubfolders="true" groupImages="false"/>
//!   <document units="inches" width="8" height="10" resolution="300" resUnits="pixels/inch" …/>
//!   <thumbnail place="across first" cols="5" rows="6" … horz="0.014 in" vert="0.014 in"/>
//!   <caption enabled="true"><font name="Inter-Regular" size="12"/></caption>
//!   <date>…</date><version>…</version><locale>…</locale>
//! </ContactSheetIISettings>
//! ```
//!
//! Values are the dialog's entry labels, numbers have a dot whatever the locale, the font is its
//! PostScript name and the resolution is in the shown unit. What the dialog has no control for
//! yet is written as today's run has it and is not taken when read: the Files list and the Use
//! dropdown (#13686; `imageSource` is always Folder), the colour profile (#13688; a fixed name
//! per mode) and the spacing (#13689; the field's default, 1 px, which today's run does not use).

use serde_json::{Map, Value, json};

use super::{CM, DEPTHS, MODES, PLACES, RES_MAX, RES_MIN, RES_UNIT, RES_UNITS, UNITS, Unit, defaults, flag, number, text, unit_of};

const ROOT: &str = "ContactSheetIISettings";

/// The file name Save… suggests.
pub(super) const FILE_NAME: &str = "Contact Sheet II.xml";

/// Spacing is stored in inches at 72 px/in, whatever the sheet's resolution.
const SPACING_PPI: f64 = 72.0;

/// What Save… writes after the values.
pub(super) struct Stamp {
    pub(super) date: String,
    pub(super) version: String,
    pub(super) locale: String,
}

impl Stamp {
    /// The time (UTC: the crate has no local clock), this build's version and the UI language
    /// code with `_` (`en`, `zh_hans`: the language setting has no region).
    pub(super) fn now() -> Self {
        let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let date = i64::try_from(secs).ok().and_then(|s| chrono::DateTime::from_timestamp(s, 0)).unwrap_or_default();
        Self {
            date: date.format("%Y-%m-%dT%H:%M:%S").to_string(),
            version: env!("CARGO_PKG_VERSION").into(),
            locale: crate::i18n::current().code().replace('-', "_"),
        }
    }
}

/// The settings file for the dialog's fields `f`, over the dialog's defaults.
pub(super) fn write(f: &Map<String, Value>, stamp: &Stamp) -> String {
    let mut all = defaults().as_object().cloned().unwrap_or_default();
    all.extend(f.iter().map(|(k, v)| (k.clone(), v.clone())));
    let f = &all;
    let res_unit = text(f, RES_UNIT, "in");
    let per = if res_unit == "cm" { CM } else { 1.0 };
    let mode = text(f, "mode", "");
    let depth = f.get("depth").and_then(Value::as_f64).map_or(0, |d| d as u64);
    let (units, width, height) = size_in_entry_units(f);
    let spacing = format!("{:.3} in", 1.0 / SPACING_PPI);
    let source = [
        ("imageSource", "Folder".into()),
        ("path", text(f, "input", "")),
        ("includeSubfolders", flag(f, "includeSubfolders", true).to_string()),
        ("groupImages", flag(f, "groupByFolder", false).to_string()),
    ];
    let document = [
        ("units", units.into()),
        ("width", num(width)),
        ("height", num(height)),
        ("resolution", num(number(f, "resolution", 0.0) / per)),
        ("resUnits", label(&RES_UNITS, res_unit.as_str(), "in").into()),
        ("mode", label(&MODES, mode.as_str(), "rgb").into()),
        ("bitDepth", label(&DEPTHS, depth, 8).into()),
        ("colorProfile", profile(&mode).into()),
        ("flattenLayers", flag(f, "flatten", true).to_string()),
    ];
    let thumbnail = [
        ("place", label(&PLACES, flag(f, "placeAcrossFirst", true), true).into()),
        ("cols", num(number(f, "columns", 0.0))),
        ("rows", num(number(f, "rows", 0.0))),
        ("bestFit", flag(f, "rotateForBestFit", false).to_string()),
        ("useAutospacing", flag(f, "autoSpacing", true).to_string()),
        ("horz", spacing.clone()),
        ("vert", spacing),
    ];
    let font = [("name", postscript_name(&text(f, "font", ""))), ("size", num(number(f, "fontSize", 0.0)))];
    let caption = flag(f, "caption", true);
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<{ROOT}>\n  <source{}/>\n  <document{}/>\n  <thumbnail{}/>\n  <caption enabled=\"{caption}\">\n    <font{}/>\n  </caption>\n  <date>{}</date>\n  <version>{}</version>\n  <locale>{}</locale>\n</{ROOT}>\n",
        attrs(&source),
        attrs(&document),
        attrs(&thumbnail),
        attrs(&font),
        escape(&stamp.date),
        escape(&stamp.version),
        escape(&stamp.locale),
    )
}

/// Units, Width and Height as an entry of the Units dropdown: units it does not list (`mm` set
/// over the control channel) are written as inches, converted at the dialog's resolution.
fn size_in_entry_units(f: &Map<String, Value>) -> (&'static str, f64, f64) {
    let (units, width, height) = (text(f, "units", ""), number(f, "width", 0.0), number(f, "height", 0.0));
    if let Some((id, _)) = UNITS.iter().find(|(id, _)| *id == units) {
        return (id, width, height);
    }
    let (from, ppi) = (unit_of(&units), number(f, "resolution", 0.0));
    let inches = |v: f64| Unit::Inches.from_px(from.to_px(v, ppi, 0.0, 72.0), ppi, 0.0, 72.0);
    ("inches", inches(width), inches(height))
}

/// The dialog's fields `current` with the values of the settings file `xml` over them. A value
/// missing from the file, or one that is not an entry label, a number or `true`/`false`, leaves
/// the current one; the caller checks the ranges (`settings`). `Err` when `xml` is not XML or
/// not a Contact Sheet II settings file.
pub(super) fn read(xml: &str, current: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name(ROOT) {
        return Err(tl!("not a Contact Sheet II settings file").into());
    }
    let element = |name: &str| root.children().find(|n| n.has_tag_name(name));
    let attr = |name: &str, key: &str| element(name).and_then(|n| n.attribute(key));
    let font = element("caption").and_then(|c| c.children().find(|n| n.has_tag_name("font")));
    let mut f = current.clone();
    // The Files list is not stored, and the dialog has only the folder (#13686).
    if attr("source", "imageSource") == Some("Folder") {
        put(&mut f, "input", attr("source", "path").map(|p| json!(p)));
    }
    // A width means nothing without its units: the three only together.
    let size =
        (attr("document", "units").and_then(|l| id_of(&UNITS, l)), attr("document", "width").and_then(parse), attr("document", "height").and_then(parse));
    if let (Some(units), Some(width), Some(height)) = size {
        f.extend([("units".into(), json!(units)), ("width".into(), json!(width)), ("height".into(), json!(height))]);
    }
    put(&mut f, RES_UNIT, attr("document", "resUnits").and_then(|l| id_of(&RES_UNITS, l)).map(|id| json!(id)));
    let per = if text(&f, RES_UNIT, "in") == "cm" { CM } else { 1.0 };
    put(&mut f, "resolution", attr("document", "resolution").and_then(parse).and_then(|r| finite(r * per)));
    put(&mut f, "mode", attr("document", "mode").and_then(|l| id_of(&MODES, l)).map(|id| json!(id)));
    put(&mut f, "depth", attr("document", "bitDepth").and_then(|l| id_of(&DEPTHS, l)).map(|d| json!(d)));
    put(&mut f, "placeAcrossFirst", attr("thumbnail", "place").and_then(|l| id_of(&PLACES, l)).map(|a| json!(a)));
    for (key, name, at) in [
        ("includeSubfolders", "source", "includeSubfolders"),
        ("groupByFolder", "source", "groupImages"),
        ("flatten", "document", "flattenLayers"),
        ("rotateForBestFit", "thumbnail", "bestFit"),
        ("autoSpacing", "thumbnail", "useAutospacing"),
        ("caption", "caption", "enabled"),
    ] {
        put(&mut f, key, attr(name, at).and_then(|v| v.parse::<bool>().ok()).map(|b| json!(b)));
    }
    for (key, at) in [("columns", "cols"), ("rows", "rows")] {
        put(&mut f, key, attr("thumbnail", at).and_then(parse).and_then(finite));
    }
    put(&mut f, "font", font.and_then(|n| n.attribute("name")).and_then(family_of).map(|family| json!(family)));
    put(&mut f, "fontSize", font.and_then(|n| n.attribute("size")).and_then(parse).and_then(finite));
    // The spacing in the sheet's units, the engine's `horizontal` and `vertical`. The dialog has
    // no control for them yet (#13689), so `settings` leaves them out. Only at a resolution the
    // dialog takes: a broken one would give a broken spacing.
    let units = unit_of(&text(&f, "units", ""));
    if let Some(ppi) = f.get("resolution").and_then(Value::as_f64).filter(|r| (RES_MIN..=RES_MAX).contains(r)) {
        for (key, at) in [("horizontal", "horz"), ("vertical", "vert")] {
            put(&mut f, key, attr("thumbnail", at).and_then(spacing_px).and_then(|px| finite(units.from_px(px, ppi, 0.0, SPACING_PPI))));
        }
    }
    Ok(f)
}

fn put(f: &mut Map<String, Value>, key: &str, v: Option<Value>) {
    if let Some(v) = v {
        f.insert(key.into(), v);
    }
}

/// `n` as a JSON number; `None` for infinity and NaN, which JSON would turn into `null`.
fn finite(n: f64) -> Option<Value> {
    n.is_finite().then(|| json!(n))
}

/// A number with a dot (`1,5` is not one).
fn parse(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A spacing string in whole pixels: `0.111 in` (at 72 px/in) is 8, `20 px` is 20.
pub(super) fn spacing_px(s: &str) -> Option<f64> {
    let (n, unit) = s.trim().split_once(' ')?;
    let n = parse(n)?;
    match unit.trim() {
        "in" => Some((n * SPACING_PPI).round()),
        "px" => Some(n.round()),
        _ => None,
    }
}

/// `v` as the reference writes numbers: at most 15 significant digits, a dot, no trailing zeros
/// (`96.6666666666667`, `10`).
fn num(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let decimals = 14i32.saturating_sub(v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// The entry label of `v`, else that of `d`.
fn label<T: PartialEq + Copy>(options: &[(T, &'static str)], v: T, d: T) -> &'static str {
    let find = |x: T| options.iter().find(|(id, _)| *id == x).map(|(_, l)| *l);
    find(v).or_else(|| find(d)).unwrap_or_default()
}

/// The value of the entry labelled `l`.
fn id_of<T: Copy>(options: &[(T, &'static str)], l: &str) -> Option<T> {
    options.iter().find(|(_, label)| *label == l).map(|(id, _)| *id)
}

/// The profile a sheet in `mode` gets: the mode's default (the dialog has no Color Profile yet).
fn profile(mode: &str) -> &'static str {
    match mode {
        "gray" => "Working Gray",
        "cmyk" => "Working CMYK",
        "lab" => "Lab",
        _ => "sRGB IEC61966-2.1",
    }
}

/// The PostScript name of `family`'s upright face nearest regular weight; the family itself when
/// none is installed.
fn postscript_name(family: &str) -> String {
    let mut text = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let faces = text.fonts.faces(family);
    let regular = faces.iter().filter(|f| f.postscript_name.is_some()).min_by_key(|f| (f.italic, (f.weight - 400.0).abs() as u32));
    regular.and_then(|f| f.postscript_name.clone()).unwrap_or_else(|| family.to_string())
}

/// The family a stored font name stands for: an installed family of that name, or the family of
/// the face with that PostScript name. `None` when neither is installed.
fn family_of(name: &str) -> Option<String> {
    let mut text = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    if text.fonts.has_family(name) {
        return Some(name.to_string());
    }
    // Only the families the name's guessed family starts with: `resolve_postscript` would read
    // every installed family on an unknown name, on the UI thread.
    let guess = photocraft_text::fonts::guess_from_postscript(name).family.replace(' ', "").to_lowercase();
    let mut candidates = text.fonts.families().into_iter().filter(|f| !guess.is_empty() && f.replace(' ', "").to_lowercase().starts_with(&guess));
    candidates.find(|family| text.fonts.faces(family).iter().any(|face| face.postscript_name.as_deref() == Some(name)))
}

fn attrs(pairs: &[(&str, String)]) -> String {
    pairs.iter().map(|(k, v)| format!(" {k}=\"{}\"", escape(v))).collect()
}

fn escape(s: &str) -> String {
    // Line breaks and tabs as references: a parser turns literal ones in an attribute into spaces.
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

#[cfg(test)]
#[path = "contact_sheet_settings_file_tests.rs"]
mod tests;
