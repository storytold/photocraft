//! Layer › New › Layer… (⇧⌘N): Photoshop's New Layer dialog. Name, Use Previous Layer to Create
//! Clipping Mask, Color, Mode, Opacity and Fill with neutral color; OK runs `layer.new.layer` with
//! them. The Layers panel's New Layer button makes a layer without asking, and ⌥-click opens the
//! dialog, as in Photoshop.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use photocraft_doc::{BlendMode, LabelColor};
use photocraft_engine::new_layer::neutral_color;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The command the dialog stands for; it is also the dialog's key.
pub const COMMAND: &str = "layer.new.layer";

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND
}

/// The label colour row of New Layer and New Group.
pub(crate) fn color_param() -> Param {
    let options = LabelColor::ALL.into_iter().map(|c| (c.id().to_string(), Label::from(crate::layer_menu_ui::color_name(c)))).collect();
    Param::new("color", tl!("Color:"), ParamKind::choice(options, Some(LabelColor::None.id().to_string())))
}

/// The opacity row of New Layer and New Group, in percent.
pub(crate) fn opacity_param() -> Param {
    Param::new("opacity", tl!("Opacity:"), ParamKind::number(Some(0.0), Some(100.0), 100.0, 1.0, 0)).unit(UnitKind::Percent)
}

/// Open the dialog for the active document, with the name the layer would get.
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let name = app.session.active().ok_or("no document open")?.doc.next_layer_name("Layer");
    Ok(params::open_param(app, COMMAND, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let neutral_modes: Vec<String> =
            BlendMode::LAYER_MODES.into_iter().filter(|m| neutral_color(*m).is_some()).map(|mode| mode.key().to_string()).collect();
        let rows = vec![
            Param::new("name", tl!("Name:"), ParamKind::text(name.clone())),
            Param::new("clipToPrevious", tl!("Use Previous Layer to Create Clipping Mask"), ParamKind::bool(false)),
            color_param(),
            // Opacity shares the Mode row, as in the reference application.
            crate::dialogs::blend_options::blend_param("mode", tl!("Mode:"), false, BlendMode::Normal).beside_next(),
            opacity_param(),
            Param::new("fillNeutral", neutral_label(mode(fields)), ParamKind::bool(false)).enabled_when_in("mode", neutral_modes),
        ];
        ParamDialog::command(COMMAND, tl!("New Layer"), rows).values(Bind::bound(fields))
    }))
}

/// OK: makes the layer the values describe.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let result = app.run(COMMAND, params(&result.fields));
    // PLAN: assumed `dialogs::alerts::report(app, Result<Value, String>)` (dialogs-w21)
    super::alerts::report(app, result);
}

fn mode(f: &DialogFields) -> BlendMode {
    f.text("mode").and_then(photocraft_engine::commands::parse_blend).filter(|m| *m != BlendMode::PassThrough).unwrap_or(BlendMode::Normal)
}

fn flag(f: &DialogFields, k: &str) -> bool {
    f.bool(k).unwrap_or(false)
}

/// The `layer.new.layer` params the dialog stands for (opacity is a percentage in the dialog).
pub fn params(f: &DialogFields) -> Value {
    let mode = mode(f);
    let opacity = f.number("opacity").filter(|o| o.is_finite()).unwrap_or(100.0).clamp(0.0, 100.0) / 100.0;
    let mut p = json!({
        "clipToPrevious": flag(f, "clipToPrevious"),
        "color": f.text("color").and_then(LabelColor::from_id).unwrap_or_default().id(),
        "mode": mode.label(),
        "opacity": opacity,
        // Only modes with a neutral color can fill with one (the checkbox is off for the others).
        "fillNeutral": flag(f, "fillNeutral") && neutral_color(mode).is_some(),
    });
    // An empty name gets the next "Layer N", as the Layers panel button does.
    if let Some(name) = f.text("name").filter(|n| !n.trim().is_empty()) {
        p["name"] = json!(name);
    }
    p
}

/// The neutral-fill checkbox's label for `mode`, or the note that it has none.
pub fn neutral_label(mode: BlendMode) -> String {
    let name = tl!(mode.label());
    match neutral_color(mode) {
        Some((_, "white")) => crate::i18n::fmt(tl!("Fill with {mode}-neutral color (white)"), &[("mode", name)]),
        Some((_, "black")) => crate::i18n::fmt(tl!("Fill with {mode}-neutral color (black)"), &[("mode", name)]),
        Some(_) => crate::i18n::fmt(tl!("Fill with {mode}-neutral color (50% gray)"), &[("mode", name)]),
        None => crate::i18n::fmt(tl!("(No neutral color exists for {mode} mode.)"), &[("mode", name)]),
    }
}

/// Helpers the tests of the hosted dialogs share.
#[cfg(test)]
pub(crate) mod testing {
    use craft::ui::dialog::DialogId;
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
    use serde_json::Value;

    use crate::PhotocraftApp;

    /// The dialog on top of the stack.
    pub(crate) fn top(app: &PhotocraftApp) -> DialogId {
        app.framework.dialogs.stack().top().expect("a dialog is open")
    }

    /// The values of the open hosted dialog `id`.
    pub(crate) fn fields(app: &PhotocraftApp, id: DialogId) -> &DialogFields {
        app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("a hosted dialog").fields()
    }

    /// Sets `field` as the control surface would.
    pub(crate) fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) -> Result<(), String> {
        let ctx = app.ctx().clone();
        app.framework.dialogs.set_json_field(&ctx, id, field, value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::testing::{set, top};
    use super::*;

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app
    }

    fn top_layer(app: &PhotocraftApp) -> &photocraft_doc::Layer {
        app.session.active().unwrap().doc.layers.last().unwrap()
    }

    /// The menu (and its shortcut) opens the dialog; OK makes the layer it describes in one step.
    #[test]
    fn menu_opens_the_dialog_and_ok_makes_the_layer() {
        let mut app = app();
        let ctx = app.ctx().clone();
        let layers = app.session.active().unwrap().doc.layers.len();
        crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "nothing made until OK");
        let id = top(&app);
        assert!(set(&mut app, id, "fillNeutral", json!(true)).is_err(), "Normal has no neutral color");
        set(&mut app, id, "name", json!("Dodge & Burn")).unwrap();
        set(&mut app, id, "mode", json!(BlendMode::Overlay.key().to_string())).unwrap();
        set(&mut app, id, "opacity", json!(60.0)).unwrap();
        set(&mut app, id, "color", json!("violet")).unwrap();
        set(&mut app, id, "fillNeutral", json!(true)).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        let l = top_layer(&app);
        assert_eq!((l.name.as_str(), l.blend, l.label), ("Dodge & Burn", BlendMode::Overlay, LabelColor::Violet));
        assert!((l.opacity - 0.6).abs() < 1e-6);
        assert!(l.surface().unwrap().tile_count() > 0, "filled with 50% gray");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    }

    /// #15539: Opacity sits on the Mode row, right of the Mode dropdown, and keeps its % unit.
    #[test]
    fn opacity_shares_the_mode_row() {
        use egui::accesskit::Role;
        use egui_kittest::Harness;
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut app = app();
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(64)
            .with_context(ctx)
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        let mode = h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Mode")).expect("Mode").rect();
        let opacity = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Opacity")).expect("Opacity").rect();
        assert!((mode.center().y - opacity.center().y).abs() < 1.0, "Mode {mode:?} and Opacity {opacity:?} share a row");
        assert!(mode.right() <= opacity.left(), "Opacity is right of Mode");
        assert_eq!(opacity_param().unit, Some(UnitKind::Percent));
    }

    #[test]
    fn params_follow_photoshop_rules() {
        let mut f = DialogFields::new();
        f.set("name", "  ");
        f.set("mode", "hue");
        f.set("opacity", 250.0);
        f.set("fillNeutral", true);
        f.set("color", "plaid");
        f.set("clipToPrevious", true);
        let p = params(&f);
        assert_eq!(p, json!({"clipToPrevious": true, "color": "none", "mode": "Hue", "opacity": 1.0, "fillNeutral": false}));
        assert_eq!(neutral_label(BlendMode::Multiply), "Fill with Multiply-neutral color (white)");
        assert_eq!(neutral_label(BlendMode::Screen), "Fill with Screen-neutral color (black)");
        assert_eq!(neutral_label(BlendMode::SoftLight), "Fill with Soft Light-neutral color (50% gray)");
        assert_eq!(neutral_label(BlendMode::Normal), "(No neutral color exists for Normal mode.)");
    }
}
