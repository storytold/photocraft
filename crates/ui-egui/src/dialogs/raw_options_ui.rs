//! Raw Options: how File › Open As reads a file as Photoshop Raw (#14051), as the reference
//! application's dialog asks: Describe "<file>": (<n> bytes), the width and height in pixels, the
//! channel count, depth, byte order and header size.
//!
//! A `file.openAs` with `"as": "raw"` and none of the Raw Options fields is parked here when the
//! file isn't a camera raw (those keep opening as they did). OK runs `file.openAs` with the
//! fields, so an action being recorded keeps them and plays back without the dialog; Cancel opens
//! nothing. Automation never sees the dialog: it passes the fields itself.
//!
//! Not measured, so not offered: what Swap and Guess do and Retain When Saving (shown greyed
//! out), and Last Channel is Transparency, which the reader has no channels for yet.

use photocraft_io::photoshop_raw::{ByteOrder, MAX_CHANNELS, MAX_SIDE, RawLayout};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// An Open As waiting on the Raw Options answer.
pub struct Prompt {
    /// The `file.openAs` params that asked: the path, the type and anything else given.
    pub params: Value,
    /// The file's name, as the title line shows it.
    pub name: String,
    /// The file's size in bytes.
    pub len: u64,
    /// The fields as the user has set them so far.
    pub layout: RawLayout,
}

impl Prompt {
    /// A prompt for the file `name` of `len` bytes, starting at [`RawLayout::defaults_for`].
    pub fn new(params: Value, name: String, len: u64) -> Self {
        Prompt { params, name, len, layout: RawLayout::defaults_for(len) }
    }

    /// The title line: Describe "notes.txt": (16 bytes).
    pub fn describe(&self) -> String {
        crate::i18n::fmt(tl!("Describe \"{name}\": ({bytes} bytes)"), &[("name", &self.name), ("bytes", &self.len.to_string())])
    }

    /// The `file.openAs` params OK runs: the ones that asked, plus every Raw Options field.
    pub fn answer(&self) -> Value {
        let l = &self.layout;
        let mut p = self.params.clone();
        if let Some(o) = p.as_object_mut() {
            o.insert("width".into(), l.width.into());
            o.insert("height".into(), l.height.into());
            o.insert("channels".into(), l.channels.into());
            o.insert("depth".into(), l.depth.into());
            o.insert("byteOrder".into(), json!(if l.byte_order == ByteOrder::IbmPc { "ibm" } else { "mac" }));
            o.insert("interleaved".into(), l.interleaved.into());
            o.insert("header".into(), l.header.into());
        }
        p
    }
}

/// Whether `params` ask Open As for the raw type without saying how to read it: no Raw Options
/// field at all (see `photocraft_engine::file_cmds::RAW_LAYOUT_KEYS`).
pub fn asks(params: &Value) -> bool {
    let as_raw =
        params.get("as").or_else(|| params.get("format")).and_then(Value::as_str).is_some_and(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case("raw"));
    as_raw && !photocraft_engine::file_cmds::RAW_LAYOUT_KEYS.iter().any(|key| params.get(*key).is_some_and(|v| !v.is_null()))
}

/// Parks a `file.openAs` that [`asks`] behind Raw Options, unless automation sent it or the file
/// is a camera raw. A file that can't be read is left to the engine, which reports it.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "file.openAs" || app.automation_input || !asks(params) {
        return None;
    }
    let path = params.get("path").and_then(Value::as_str)?;
    let bytes = std::fs::read(path).ok()?;
    if photocraft_io::raw::is_raw(&bytes) {
        return None;
    }
    if app.raw_options.is_some() {
        return Some(Err(tl!("Raw Options is already open").into()));
    }
    let name = crate::file_open::display_name(path);
    app.raw_options = Some(Prompt::new(params.clone(), name, bytes.len() as u64));
    Some(Ok(json!({"rawOptions": true})))
}

/// One labelled number field of the dialog, with its unit after it.
fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, label: &str, value: &mut T, range: std::ops::RangeInclusive<T>, unit: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 18.0], egui::Label::new(label));
        ui.add(egui::DragValue::new(value).range(range).speed(1.0));
        ui.label(unit);
    });
}

/// The dialog's fields for `l`. Interleaved needs more than one channel and Byte Order more than
/// 8 bits, as in the reference application.
fn fields(ui: &mut egui::Ui, l: &mut RawLayout) {
    crate::widgets::section_label(ui, tl!("Dimensions"));
    number(ui, tl!("Width:"), &mut l.width, 0..=MAX_SIDE, tl!("pixels"));
    number(ui, tl!("Height:"), &mut l.height, 0..=MAX_SIDE, tl!("pixels"));
    ui.add_space(6.0);
    crate::widgets::section_label(ui, tl!("Channels"));
    number(ui, tl!("Count:"), &mut l.channels, 1..=MAX_CHANNELS, "");
    let several = l.channels > 1;
    ui.add_enabled(several, egui::Checkbox::new(&mut l.interleaved, tl!("Interleaved")));
    ui.add_enabled(false, egui::Checkbox::new(&mut false, tl!("Last Channel is Transparency")));
    ui.add_space(6.0);
    crate::widgets::section_label(ui, tl!("Depth"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut l.depth, 8, tl!("8 Bits"));
        ui.radio_value(&mut l.depth, 16, tl!("16 Bits"));
        ui.radio_value(&mut l.depth, 32, tl!("32 Bits"));
    });
    ui.add_space(6.0);
    crate::widgets::section_label(ui, tl!("Byte Order"));
    ui.add_enabled_ui(l.depth > 8, |ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut l.byte_order, ByteOrder::Mac, tl!("Mac"));
            ui.radio_value(&mut l.byte_order, ByteOrder::IbmPc, tl!("IBM PC"));
        });
    });
    ui.add_space(6.0);
    crate::widgets::section_label(ui, tl!("Header"));
    number(ui, tl!("Size:"), &mut l.header, 0..=u64::MAX, tl!("bytes"));
    ui.add_enabled(false, egui::Checkbox::new(&mut false, tl!("Retain When Saving")));
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.raw_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let describe = p.describe();
    let modal = egui::Modal::new(egui::Id::new("raw-options")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.label(egui::RichText::new(tl!("Raw Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(describe);
        ui.add_space(6.0);
        fields(ui, &mut p.layout);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
            ui.add_enabled(false, egui::Button::new(tl!("Guess")));
            ui.add_enabled(false, egui::Button::new(tl!("Swap")));
        });
    });
    cancel |= modal.should_close();
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        answer(app);
    } else if cancel {
        app.raw_options = None;
    }
}

/// OK: opens the file with the fields as set, reporting a refusal (a size the file can't fill,
/// a layout not read yet) as an error. The dialog closes either way.
pub fn answer(app: &mut PhotocraftApp) {
    let Some(p) = app.raw_options.take() else { return };
    if let Err(e) = app.run("file.openAs", p.answer()) {
        crate::notices::error(app, e);
    }
}

#[cfg(test)]
#[path = "raw_options_ui_tests.rs"]
mod tests;
