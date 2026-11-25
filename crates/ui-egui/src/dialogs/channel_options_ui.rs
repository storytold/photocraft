//! Channels panel › Channel Options… (double-click an alpha or spot channel's thumbnail, or the
//! Quick Mask row): name, Color Indicates Masked Areas / Selected Areas / Spot Color, the overlay
//! or ink colour, and opacity (solidity for spot channels). OK runs the engine's
//! `channel.options`, which agents can also call directly with params (#3761).
//!
//! On the layer mask (double-click its Channels row, or Mask Options… on its Layers thumbnail)
//! it is Photoshop's Layer Mask Display Options: just the rubylith's colour and opacity.
//!
//! The Channels panel's New Channel… is the same dialog for a channel still to be made (name,
//! Masked / Selected Areas, colour, opacity); OK runs `channel.new` (#11451).
//!
//! New Spot Channel… asks for Name and Ink Characteristics (Color, Solidity); OK runs
//! `channel.newSpot` (from the selection, as Photoshop does) and remembers the ink.
//!
//! The channel a dialog edits (an alpha/spot channel index, `"quickMask"` or `"mask"`) is captured
//! at open, not a row.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use photocraft_doc::ColorIndicates;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

pub const COMMAND: &str = "channel.options";
pub const NEW_CHANNEL: &str = "channel.new";
pub const NEW_SPOT: &str = "channel.newSpot";

/// Which of the five dialogs a key and captured channel name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Channel,
    QuickMask,
    LayerMask,
    New,
    Spot,
}

fn kind_of(key: &str, channel: &Value) -> Kind {
    match key {
        NEW_CHANNEL => Kind::New,
        NEW_SPOT => Kind::Spot,
        _ => match channel.as_str() {
            Some("mask") => Kind::LayerMask,
            Some("quickMask") => Kind::QuickMask,
            _ => Kind::Channel,
        },
    }
}

/// Whether `key` is one of these dialogs'.
pub fn owns(key: &str) -> bool {
    matches!(key, COMMAND | NEW_CHANNEL | NEW_SPOT)
}

fn hex(rgb: [f32; 3]) -> String {
    crate::color_picker_ui::hex(rgb)
}

fn open_fields(name: Option<String>, indicates: Option<&str>, color: [f32; 3], percent: f64, percent_key: &str) -> DialogFields {
    let mut f = DialogFields::new();
    if let Some(n) = name {
        f.set("name", FieldValue::Text(n));
    }
    if let Some(i) = indicates {
        f.set("indicates", FieldValue::Text(i.to_string()));
    }
    f.set("color", FieldValue::Color(color));
    f.set(percent_key, FieldValue::Number(percent));
    f
}

/// The open values for `channel` (an alpha/spot channel index, `"quickMask"`, or `"mask"` for
/// Layer Mask Display Options), or `None` when there's no document or no such channel.
pub fn fields(app: &PhotocraftApp, channel: &Value) -> Option<DialogFields> {
    let doc = &app.session.active()?.doc;
    let kind = |i: ColorIndicates| if i == ColorIndicates::SelectedAreas { "selected" } else { "masked" };
    if channel.as_str() == Some("mask") {
        let o = &app.session.prefs().layer_mask_overlay;
        return Some(open_fields(None, None, o.rgb(), f64::from((o.alpha() * 100.0).round()), "opacity"));
    }
    let (name, indicates, color, percent) = if channel.as_str() == Some("quickMask") {
        let o = app.session.quick_mask_options;
        (None, kind(o.indicates), o.color, o.opacity)
    } else {
        let ch = doc.channels.get(usize::try_from(channel.as_u64()?).ok()?)?;
        match ch.spot {
            Some((ink, solidity)) => (Some(ch.name.clone()), "spot", ink, solidity),
            None => (Some(ch.name.clone()), kind(ch.indicates), ch.color, ch.opacity),
        }
    };
    Some(open_fields(name, Some(indicates), color.to_rgb(), f64::from((percent.clamp(0.0, 1.0) * 100.0).round()), "opacity"))
}

/// The New Channel values: Channel Options' for a channel still to be made (the next "Alpha N"
/// name, Masked Areas, red at 50%), or `None` without a document. OK runs `channel.new`.
pub fn new_channel_fields(app: &PhotocraftApp) -> Option<DialogFields> {
    let doc = &app.session.active()?.doc;
    let name = photocraft_engine::channel_cmds::next_alpha_name(doc, "Alpha");
    Some(open_fields(Some(name), Some("masked"), [1.0, 0.0, 0.0], 50.0, "opacity"))
}

/// The New Spot Channel values: the next "Spot Color N" name, red and 0% solidity (the framework
/// opens the last remembered ink over them), or `None` without a document.
pub fn new_spot_fields(app: &PhotocraftApp) -> Option<DialogFields> {
    let doc = &app.session.active()?.doc;
    let name = photocraft_engine::channel_cmds::next_alpha_name(doc, "Spot Color");
    Some(open_fields(Some(name), None, [1.0, 0.0, 0.0], 0.0, "solidity"))
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Channel => tl!("Channel Options"),
        Kind::QuickMask => tl!("Quick Mask Options"),
        Kind::LayerMask => tl!("Layer Mask Display Options"),
        Kind::New => tl!("New Channel"),
        Kind::Spot => tl!("New Spot Channel"),
    }
}

/// The rows of a dialog of `kind`; `spot` is whether Color Indicates is Spot Color, which turns
/// Opacity into Solidity.
fn rows(kind: Kind, spot: bool) -> Vec<Param> {
    let mut rows = Vec::new();
    if matches!(kind, Kind::Channel | Kind::New | Kind::Spot) {
        rows.push(Param::new("name", tl!("Name:"), ParamKind::text("")));
    }
    if !matches!(kind, Kind::LayerMask | Kind::Spot) {
        let mut options = vec![("masked".to_string(), tl!("Masked Areas").into()), ("selected".to_string(), tl!("Selected Areas").into())];
        // Quick Mask can't be a spot channel; New Spot Channel… makes spot channels.
        if kind == Kind::Channel {
            options.push(("spot".to_string(), tl!("Spot Color").into()));
        }
        rows.push(Param::new("indicates", tl!("Color Indicates:"), ParamKind::radio(options, Some("masked".to_string()))));
    }
    let ink = kind == Kind::Spot;
    let mut color = Param::new("color", tl!("Color:"), ParamKind::color(Some([1.0, 0.0, 0.0])));
    // New Spot keeps its ink; the others' default follows the document.
    if !ink {
        color = color.not_remembered();
    }
    rows.push(color);
    let solid = ink || spot;
    let (key, label) = if ink {
        ("solidity", tl!("Solidity:"))
    } else if spot {
        ("opacity", tl!("Solidity:"))
    } else {
        ("opacity", tl!("Opacity:"))
    };
    rows.push(Param::new(key, label, ParamKind::number(Some(0.0), Some(100.0), if solid { 0.0 } else { 50.0 }, 1.0, 0)));
    rows
}

fn build(key: &'static str, channel: Value) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |f: &mut DialogFields| {
        let kind = kind_of(key, &channel);
        ParamDialog::new(key, title(kind), rows(kind, f.text("indicates") == Some("spot")))
    }
}

fn open_with(app: &mut PhotocraftApp, key: &'static str, fields: DialogFields, channel: Value) -> DialogId {
    let captured = json!({ "channel": channel });

    params::open_param(app, key, fields, captured, build(key, channel))
}

/// Opens Channel Options on `channel`, or `None` if it doesn't exist.
pub fn open(app: &mut PhotocraftApp, channel: &Value) -> Option<DialogId> {
    let f = fields(app, channel)?;
    Some(open_with(app, COMMAND, f, channel.clone()))
}

/// Opens New Channel, or `None` without a document.
pub fn open_new_channel(app: &mut PhotocraftApp) -> Option<DialogId> {
    let f = new_channel_fields(app)?;
    Some(open_with(app, NEW_CHANNEL, f, Value::Null))
}

/// Opens New Spot Channel, or `None` without a document.
pub fn open_new_spot(app: &mut PhotocraftApp) -> Option<DialogId> {
    let f = new_spot_fields(app)?;
    Some(open_with(app, NEW_SPOT, f, Value::Null))
}

/// The `channel.options` / `channel.new` params for the dialog's values: spot channels take
/// `solidity`, the others `opacity`. `channel` is the captured channel, `null` for a new one.
pub fn params(f: &DialogFields, channel: &Value) -> Value {
    let spot = f.text("indicates") == Some("spot");
    let mut p = Map::new();
    if !channel.is_null() {
        p.insert("channel".into(), channel.clone());
    }
    for key in ["name", "indicates"] {
        if let Some(v) = f.text(key) {
            p.insert(key.into(), json!(v));
        }
    }
    if let Some(c) = f.color("color") {
        p.insert("color".into(), json!(hex(c)));
    }
    if let Some(v) = f.number("opacity") {
        p.insert(if spot { "solidity" } else { "opacity" }.into(), json!(v));
    }
    Value::Object(p)
}

/// The `channel.newSpot` params for the dialog's values.
pub fn new_spot_params(f: &DialogFields) -> Value {
    let mut p = Map::new();
    if let Some(v) = f.text("name") {
        p.insert("name".into(), json!(v));
    }
    if let Some(c) = f.color("color") {
        p.insert("color".into(), json!(hex(c)));
    }
    if let Some(v) = f.number("solidity") {
        p.insert("solidity".into(), json!(v));
    }
    Value::Object(p)
}

/// OK: runs the dialog's command; New Spot Channel remembers its ink once the channel is made.
pub fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let channel = params::captured(app, &result.key).get("channel").cloned().unwrap_or(Value::Null);
    let outcome = if result.key == NEW_SPOT {
        let made = app.run(NEW_SPOT, new_spot_params(&result.fields));
        if made.is_ok() {
            result.remember(ctx);
        }
        made
    } else {
        app.run(&result.key, params(&result.fields, &channel))
    };
    if let Err(e) = outcome {
        crate::dialogs::alerts::report_error(app, e);
    }
}

#[cfg(test)]
mod tests;
