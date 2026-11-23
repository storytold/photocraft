//! Raw Options: how File › Open As reads a file as Photoshop Raw (#14051), as the reference
//! application's dialog asks: Describe "<file>": (<n> bytes), the width and height in pixels, the
//! channel count, depth, byte order and header size.
//!
//! A `file.openAs` with `"as": "raw"` and none of the Raw Options fields is parked behind this
//! form on the dialog host when the file isn't a camera raw (those keep opening as they did). OK
//! runs `file.openAs` with the fields, so an action being recorded keeps them and plays back
//! without the dialog; Cancel and Escape open nothing. Automation never sees the dialog: it passes
//! the fields itself.
//!
//! Not measured, so not offered: what Swap and Guess do and Retain When Saving (shown greyed
//! out), and Last Channel is Transparency, which the reader has no channels for yet.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_io::photoshop_raw::{ByteOrder, MAX_CHANNELS, MAX_SIDE, RawLayout};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The dialog's key.
pub const KIND: &str = "rawOptions";

/// Whether the hosted dialog with `key` is Raw Options.
pub(crate) fn owns(key: &str) -> bool {
    key == KIND
}

/// The title line: Describe "notes.txt": (16 bytes).
pub fn describe(name: &str, len: u64) -> String {
    crate::i18n::fmt(tl!("Describe \"{name}\": ({bytes} bytes)"), &[("name", name), ("bytes", &len.to_string())])
}

/// The form's values for `layout`.
pub(crate) fn fields_of(layout: &RawLayout) -> DialogFields {
    DialogFields::from_pairs([
        ("width", FieldValue::Int(i64::from(layout.width))),
        ("height", FieldValue::Int(i64::from(layout.height))),
        ("channels", FieldValue::Int(i64::from(layout.channels))),
        ("interleaved", FieldValue::Bool(layout.interleaved)),
        ("transparency", FieldValue::Bool(false)),
        ("depth", FieldValue::Text(layout.depth.to_string())),
        ("byteOrder", FieldValue::Text(if layout.byte_order == ByteOrder::IbmPc { "ibm" } else { "mac" }.into())),
        ("header", FieldValue::Int(i64::try_from(layout.header).unwrap_or(i64::MAX))),
        ("retain", FieldValue::Bool(false)),
    ])
}

/// The `file.openAs` params OK runs: the ones that asked, plus every Raw Options field.
pub fn answer(asked: &Value, fields: &DialogFields) -> Value {
    let int = |key: &str, default: i64| fields.int(key).unwrap_or(default).max(0);
    let mut p = asked.clone();
    if let Some(o) = p.as_object_mut() {
        o.insert("width".into(), int("width", 0).into());
        o.insert("height".into(), int("height", 0).into());
        o.insert("channels".into(), int("channels", 1).into());
        o.insert("depth".into(), fields.text("depth").and_then(|d| d.parse::<u32>().ok()).unwrap_or(8).into());
        o.insert("byteOrder".into(), json!(if fields.text("byteOrder") == Some("ibm") { "ibm" } else { "mac" }));
        o.insert("interleaved".into(), fields.get("interleaved").is_none_or(|v| *v == FieldValue::Bool(true)).into());
        o.insert("header".into(), int("header", 0).into());
    }
    p
}

/// Whether `params` ask Open As for the raw type without saying how to read it: no Raw Options
/// field at all (see `photocraft_engine::file_cmds::RAW_LAYOUT_KEYS`).
pub fn asks(params: &Value) -> bool {
    let as_raw =
        params.get("as").or_else(|| params.get("format")).and_then(Value::as_str).is_some_and(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case("raw"));
    as_raw && !photocraft_engine::file_cmds::RAW_LAYOUT_KEYS.iter().any(|key| params.get(*key).is_some_and(|v| !v.is_null()))
}

/// Whether Raw Options is open.
fn is_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.open_dialogs().iter().any(|d| d.key.as_deref() == Some(KIND))
}

/// Parks a `file.openAs` that [`asks`] behind Raw Options, unless automation sent it or the file
/// is a camera raw. A file that can't be read (missing, not a regular file, over the open size
/// limit) is left to the engine, which reports it.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "file.openAs" || app.automation_input || !asks(params) {
        return None;
    }
    let path = params.get("path").and_then(Value::as_str)?;
    // Read as the engine's Open As reads it: bounded, regular files only.
    let bytes = photocraft_format::read_file(std::path::Path::new(path)).ok()?;
    if photocraft_io::raw::is_raw(&bytes) {
        return None;
    }
    if is_open(app) {
        return Some(Err(tl!("Raw Options is already open").into()));
    }
    let name = crate::file_open::display_name(path);
    open(app, params.clone(), &name, bytes.len() as u64);
    Some(Ok(json!({"rawOptions": true})))
}

/// Opens Raw Options for the file `name` of `len` bytes that `asked` (the `file.openAs` params)
/// names, starting at [`RawLayout::defaults_for`].
pub(crate) fn open(app: &mut PhotocraftApp, asked: Value, name: &str, len: u64) -> DialogId {
    let layout = RawLayout::defaults_for(len);
    let describe = describe(name, len);
    params::open_param_unsaved(app, KIND, fields_of(&layout), asked, move |fields: &mut DialogFields| {
        ParamDialog::command(KIND, tl!("Raw Options"), rows(&describe, &layout)).values(Bind::bound(fields))
    })
}

/// The form's rows. Interleaved needs more than one channel and Byte Order more than 8 bits, as in
/// the reference application; Last Channel is Transparency and Retain When Saving stay greyed.
fn rows(describe: &str, layout: &RawLayout) -> Vec<Param> {
    let side = i64::from(MAX_SIDE);
    let several: Vec<FieldValue> = (2..=i64::from(MAX_CHANNELS)).map(FieldValue::Int).collect();
    let never = Vec::<FieldValue>::new;
    let options = |pairs: &[(&str, &str)]| pairs.iter().map(|(id, label)| ((*id).to_owned(), Label::from((*label).to_owned()))).collect::<Vec<_>>();
    vec![
        Param::read_only_text("describe", Label::Empty, describe.to_owned()).full_width(),
        Param::section(tl!("Dimensions")),
        Param::new("width", tl!("Width (pixels)"), ParamKind::int(Some(0), Some(side), i64::from(layout.width))),
        Param::new("height", tl!("Height (pixels)"), ParamKind::int(Some(0), Some(side), i64::from(layout.height))),
        Param::section(tl!("Channels")),
        Param::new("channels", tl!("Count"), ParamKind::int(Some(1), Some(i64::from(MAX_CHANNELS)), i64::from(layout.channels))),
        Param::new("interleaved", tl!("Interleaved"), ParamKind::bool(layout.interleaved)).enabled_when_in("channels", several),
        Param::new("transparency", tl!("Last Channel is Transparency"), ParamKind::bool(false)).enabled_when_in("channels", never()),
        Param::section(tl!("Depth")),
        Param::new("depth", tl!("Depth"), ParamKind::radio(options(&[("8", tl!("8 Bits")), ("16", tl!("16 Bits")), ("32", tl!("32 Bits"))]), Some("8".into())))
            .options_in_row(),
        Param::section(tl!("Byte Order")),
        Param::new("byteOrder", tl!("Byte Order"), ParamKind::radio(options(&[("mac", tl!("Mac")), ("ibm", tl!("IBM PC"))]), Some("mac".into())))
            .options_in_row()
            .enabled_when_in("depth", ["16", "32"]),
        Param::section(tl!("Header")),
        Param::new("header", tl!("Size (bytes)"), ParamKind::int(Some(0), None, 0)),
        Param::new("retain", tl!("Retain When Saving"), ParamKind::bool(false)).enabled_when_in("channels", never()),
    ]
}

/// OK: opens the file with the fields as set, reporting a refusal (a size the file can't fill,
/// a layout not read yet) on the status line. The dialog closes either way.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let params = answer(&params::captured(app, &result.key), &result.fields);
    let outcome = app.run("file.openAs", params);
    crate::dialogs::alerts::report(app, outcome);
}

#[cfg(test)]
#[path = "raw_options_ui_tests.rs"]
mod tests;
