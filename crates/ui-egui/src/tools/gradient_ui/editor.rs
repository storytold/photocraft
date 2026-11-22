//! The Gradient Editor panel (`gradientEditor`). With a gradient fill layer selected it shows that layer's gradient
//! (the Properties rows); without one it edits the gradient the Gradient tool paints with.

use craft::behavior::EditEvent;
// The stop bar is craft-color's.
use craft::color_ui::ui::gradient::{GradientEditor, StopRef, StopRows, StopRules};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::Label;
use serde_json::json;

use super::{properties_panel, selected_gradient, stops};
use crate::{PhotocraftApp, color_picker_ui, panel_docking};

/// The panel body: the selected gradient fill layer's rows, else the tool gradient's stop bar with Save As Preset and
/// the Gradients panel.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(layer) = selected_gradient(app) {
        properties_panel::show(app, ui, &layer);
        return;
    }
    Text::new(tl!("The gradient the Gradient tool paints with").to_string()).shade(Shade::Muted).show(ui);
    tool_gradient(app, ui);
    Row::new().show(ui, |ui, _| {
        let save = Button::new(tl!("Save As Preset")).tooltip(Label::from(tl!("Adds the current gradient to the Gradients panel").to_string())).show(ui);
        if save.clicked() {
            let _ = app.run("gradient.presets.new", json!({}));
        }
        if Button::new(tl!("Gradients")).show(ui).clicked() {
            let _ = panel_docking::show_panel(app, "gradients", None);
        }
    });
}

/// The stop bar of the Gradient tool's current gradient. An edit is one `gradient.presets.stop {action: "set"}` on
/// commit (no document step); a drag in progress shows through `LiveGradient::preset` and is dropped on cancel. A second
/// pass in one frame gets no pointer input, so its editor reports no event and applies nothing again.
fn tool_gradient(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let mut shown = app.gradient.preset.clone().unwrap_or_else(|| stops::of_preset(&app.session.presets.gradient, fg, bg));
    let sampler = stops::sampler(fg, bg);

    let sampled = (0..shown.color.len()).find_map(|i| crate::screen_picker::take(app, ("gradient-editor", i)));
    let mut editor = GradientEditor::new(tl!("Gradient stops").to_string(), Bind::bound(&mut shown))
        .rules(StopRules::new(StopRows::Split))
        .midpoints(false)
        .opacity_midpoints(false)
        .sampler(&sampler);
    if let Some(color) = sampled {
        editor = editor.sampled(color);
    }
    let output = editor.show(ui);
    for event in output.edit.events.iter() {
        match event {
            EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => {
                app.gradient.preset = None;
                let _ = app.run(photocraft_engine::presets::gradients::STOP, stops::preset_params(after));
            }
            EditEvent::Update { value, .. } => app.gradient.preset = Some(value.clone()),
            EditEvent::Cancel { .. } => app.gradient.preset = None,
            _ => {}
        }
    }
    if let Some(requested) = output.color_requested {
        app.gradient.preset = None;
        let rgb = shown.color.get(requested.index).map_or([0.0; 3], |s| [s.color.r, s.color.g, s.color.b]);
        color_picker_ui::open_for_command(
            app,
            "Color Picker (Stop Color)",
            rgb,
            photocraft_engine::presets::gradients::STOP,
            json!({"action": "color", "index": requested.index}),
        );
    }
    if let Some(gesture) = output.screen_pick {
        let stop = match output.selected {
            Some(StopRef::Color(i)) => i,
            _ => 0,
        };
        crate::screen_picker::hold(app, ui.ctx(), ("gradient-editor", stop), gesture);
    }
}
