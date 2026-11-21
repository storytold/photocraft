//! The Color panel on the framework: `ColorPanel` over the tool colours, its edits run as
//! `tools.setColors` (one history step per gesture) and recolour what is selected (`apply_color`). While the
//! Gradient Editor has a colour stop selected, the panel shows and edits that stop in place of the foreground (#10783).

use craft::behavior::EditEvent;
use craft::color_ui::ui::{ColorPanel, ColorPanelCommand, ColorPanelMenu, PanelTarget, PickedColor};
use craft::theme::Srgba;
use craft::ui::contract::Bind;
use serde_json::json;

use super::PhotocraftApp;

/// The Color tab: the panel in the view its panel menu picked (#6000).
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let colors = [srgba(edited_foreground(app)), srgba(app.session.tools.background)];
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let out = ColorPanel::new(Bind::bound(&mut app.ui.color_panel.panel), colors).dynamic_tracks(dynamic).show(ui);
    for (target, event) in out.edits {
        set_color(app, target, &event);
    }
    if let Some(target) = out.pick_color.and_then(|request| PanelTarget::from_key(&request.key)) {
        crate::color_picker_ui::open(app, target_name(target));
    }
    if let Some(target) = out.add_to_swatches {
        let color = colors[usize::from(target == PanelTarget::Background)];
        if let Err(e) = app.run("swatches.add", json!({"color": color.to_hex()})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The colour the panel shows and edits as the foreground: the Gradient Editor's selected colour stop while that panel
/// shows one (#10783), else the foreground.
pub(crate) fn edited_foreground(app: &PhotocraftApp) -> [f32; 4] {
    crate::gradient_ui::editor_stop_color(app).map_or(app.session.tools.foreground, |c| [c[0], c[1], c[2], 1.0])
}

/// One edit of a colour: `tools.setColors` under the gesture's coalesce key, so a drag or a
/// typed value is one history step; a new foreground recolours what is selected. A colour equal
/// to the current one runs nothing, so it leaves no empty history step. A foreground edit while the
/// Gradient Editor has a colour stop selected recolours that stop instead, one step per gesture too,
/// and leaves the foreground (#10783).
fn set_color(app: &mut PhotocraftApp, target: PanelTarget, event: &EditEvent<PickedColor>) {
    let Some(picked) = event.landed() else { return };
    let c = picked.color;
    let key = crate::history_keys::coalesce("color-panel", event);
    if target != PanelTarget::Background && crate::gradient_ui::panel_color_picked(app, [c.r, c.g, c.b], key.as_deref()) {
        return;
    }
    let now = match target {
        PanelTarget::Background => app.session.tools.background,
        _ => app.session.tools.foreground,
    };
    if [c.r, c.g, c.b, c.a] == now {
        return;
    }
    let mut params = json!({});
    if let Some(key) = &key {
        params["coalesce"] = json!(key);
    }
    params[target_name(target)] = json!([c.r, c.g, c.b, c.a]);
    if let Err(e) = app.run("tools.setColors", params) {
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    if target == PanelTarget::Foreground {
        crate::apply_color::foreground_changed(app, key);
    }
}

fn target_name(target: PanelTarget) -> &'static str {
    match target {
        PanelTarget::Background => "background",
        _ => "foreground",
    }
}

/// A tool colour (sRGB-encoded RGBA) as the framework's colour.
pub(crate) fn srgba(c: [f32; 4]) -> Srgba {
    Srgba::new(c[0], c[1], c[2], c[3])
}

/// Runs a Color panel menu command on the panel state; Copy Color as HTML and Copy Color's Hex
/// Code return the clipboard text of the edited colour.
pub(crate) fn run_menu_command(app: &mut PhotocraftApp, command: ColorPanelCommand) -> Option<String> {
    let edited = srgba(if app.ui.color_panel.panel.background { app.session.tools.background } else { edited_foreground(app) });
    ColorPanelMenu::run_command(&mut app.ui.color_panel.panel, command, edited)
}
