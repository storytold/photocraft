//! The Color panel on the framework: `ColorPanel` over the tool colours, its edits run as
//! `tools.setColors` (one history step per gesture) and recolour what is selected (`apply_color`).

use craft::behavior::EditEvent;
use craft::color_ui::ui::{ColorPanel, ColorPanelAction, ColorPanelCommand, ColorPanelMenu, ColorPanelState, PanelTarget, PickedColor};
use craft::theme::Srgba;
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox};
use serde_json::json;

use super::PhotocraftApp;

/// The Color tab: the panel in the view its panel menu picked (#6000).
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let colors = [srgba(app.session.tools.foreground), srgba(app.session.tools.background)];
    let mut sent = Vec::new();
    let props =
        ColorProps { colors, dynamic: app.session.prefs().interface.dynamic_color_sliders, state: app.ui.color_panel.panel, out: Outbox::new(&mut sent) };
    let _ = MemoView::new("color-panel", props, color_view).show(ui);
    for event in sent {
        match event {
            ColorEvent::State(state) => app.ui.color_panel.panel = state,
            ColorEvent::Edit(target, event) => set_color(app, target, &event),
            ColorEvent::PickColor(target) => {
                crate::color_picker_ui::open(app, target_name(target));
            }
            ColorEvent::AddToSwatches(target) => {
                let color = colors[usize::from(target == PanelTarget::Background)];
                if let Err(e) = app.run("swatches.add", json!({"color": color.to_hex()})) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
        }
    }
}

/// Every input of the Color tab's view: the tool colours, the slider option and the panel's settings.
#[derive(MemoProps)]
struct ColorProps<'a> {
    colors: [Srgba; 2],
    dynamic: bool,
    state: ColorPanelState,
    out: Outbox<'a, ColorEvent>,
}

/// What a run of the Color tab's view reports.
enum ColorEvent {
    /// The panel's settings changed.
    State(ColorPanelState),
    /// A colour edit.
    Edit(PanelTarget, EditEvent<PickedColor>),
    /// The Color Picker was asked for.
    PickColor(PanelTarget),
    /// Add to Swatches was chosen.
    AddToSwatches(PanelTarget),
}

/// The Color tab's view.
fn color_view(props: &mut ColorProps<'_>, ui: &mut egui::Ui) {
    let before = props.state;
    let out = ColorPanel::new(Bind::bound(&mut props.state), props.colors).dynamic_tracks(props.dynamic).show(ui);
    if props.state != before {
        props.out.send(ColorEvent::State(props.state));
    }
    for (target, event) in out.edits {
        props.out.send(ColorEvent::Edit(target, event));
    }
    if let Some(target) = out.pick_color.and_then(|request| PanelTarget::from_key(&request.key)) {
        props.out.send(ColorEvent::PickColor(target));
    }
    if let Some(target) = out.add_to_swatches {
        props.out.send(ColorEvent::AddToSwatches(target));
    }
}

/// One edit of a colour: `tools.setColors` under the gesture's coalesce key, so a drag or a
/// typed value is one history step; a new foreground recolours what is selected. A colour equal
/// to the current one runs nothing, so it leaves no empty history step.
fn set_color(app: &mut PhotocraftApp, target: PanelTarget, event: &EditEvent<PickedColor>) {
    let Some(picked) = event.landed() else { return };
    let c = picked.color;
    let now = match target {
        PanelTarget::Background => app.session.tools.background,
        _ => app.session.tools.foreground,
    };
    if [c.r, c.g, c.b, c.a] == now {
        return;
    }
    let key = crate::history_keys::coalesce("color-panel", event);
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
    let tools = &app.session.tools;
    let edited = srgba(if app.ui.color_panel.panel.background { tools.background } else { tools.foreground });
    // Photoshop's layout has no Invert, Complement or Create New Swatch rows, and a mode is set on the state itself:
    // only the copy commands leave the host something to do.
    match ColorPanelMenu::run_command(&mut app.ui.color_panel.panel, command, edited)? {
        ColorPanelAction::Copy(text) => Some(text),
        _ => None,
    }
}
