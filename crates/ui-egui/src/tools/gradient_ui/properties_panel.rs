//! The Gradient section of Properties and of the Gradient Editor panel for a gradient fill layer: the framework's
//! `GradientFillEditor` over the layer's fill.

use craft::behavior::EditEvent;
use craft::draw::ui::gradient::{GradientFill, GradientFillEditor, GradientRef};
use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::patterns::presets::PresetState;
use craft::ui::runtime::env::with_env;
use craft::ui::sections::Section;
use photocraft_doc::Layer;
use serde_json::json;

use super::{cmds, edit_stop_color, edited_fill, fill_of, stops};
use crate::PhotocraftApp;

/// The Gradient section for `layer`. An edit is one `gradient.fill.set` on commit, and only for the fields it changed. A
/// drag in progress previews through `LiveGradient::panel` and is dropped on cancel. The editor reads no input in a second
/// pass of a frame, so the events kept from the pass that did are applied once.
pub(super) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(fill) = fill_of(layer) else { return };
    let committed = stops::fill_value(fill);
    let preview = app.gradient.panel.clone().filter(|(id, _)| *id == layer.id);
    let mut value = match &preview {
        Some((_, params)) => {
            let shown = app.session.active().and_then(|st| edited_fill(app, layer, st.doc.bounds(), cmds::SET, params));
            shown.map_or_else(|| committed.clone(), |f| stops::fill_value(&f))
        }
        None => committed.clone(),
    };

    // The editor applies a sampled colour to the stop it has selected (`GradientFillEditorOutput::selected`), so the
    // pick is owned by the layer's fill alone.
    let owner = ("gradient-fill", layer.id.0);
    let sampled = crate::screen_picker::take(app, owner);
    let document = app.session.active().map(|s| s.doc.id);
    let mut events: Vec<EditEvent<GradientFill>> = Vec::new();
    let mut requested = None;
    let mut pick = None;
    with_env(
        ui,
        |set| set.id_salt = Some(egui::Id::unique(("gradient-properties", document, layer.id.0))),
        |ui| {
            // The body can run twice in a pass; only the run that read the input has events, and they are kept.
            let _ = Section::new(tl!("Gradient").to_string()).show(ui, |ui, _| {
                let mut state = PresetState::<GradientRef>::default();
                let out = GradientFillEditor::new(tl!("Gradient").to_string(), Bind::bound(&mut value), &[], Bind::bound(&mut state)).sampled(sampled).show(ui);
                if out.events.iter().next().is_some() {
                    events = out.events.iter().cloned().collect();
                }
                if out.color_requested.is_some() {
                    requested = out.color_requested;
                }
                if out.screen_pick.is_some() {
                    pick = out.screen_pick;
                }
            });
        },
    );
    for event in &events {
        match event {
            EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => {
                app.gradient.panel = None;
                let mut params = stops::value_params(&committed, after);
                if params.as_object().is_some_and(|fields| !fields.is_empty()) {
                    params["layer"] = json!(layer.id.0);
                    let _ = app.run(cmds::SET, params);
                }
            }
            EditEvent::Update { value, .. } => app.gradient.panel = Some((layer.id, stops::value_params(&committed, value))),
            EditEvent::Cancel { .. } => app.gradient.panel = None,
            _ => {}
        }
    }
    if let Some(requested) = requested {
        app.gradient.panel = None;
        edit_stop_color(app, layer.id, requested.index);
    }
    if let Some(gesture) = pick {
        crate::screen_picker::hold(app, ui.ctx(), owner, gesture);
    }
}
