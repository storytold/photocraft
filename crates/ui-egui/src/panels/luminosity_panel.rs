//! The Luminosity Masks panel (a tab of the Layers card, after Paths): one click makes a Lights,
//! Darks or Midtones mask of degree 1 to 6, or a colour mask, from the composite or the active
//! layer, and puts it on the active layer's mask, in a new alpha channel or in the selection.
//!
//! The panel computes nothing: every click runs the engine's `mask.luminosity` command, so agents
//! and scripts get the same masks through the control channel, the CLI and MCP. What the panel
//! remembers (output, source, colour narrowing) lives in egui's per-session memory, not in the
//! document, so it is not saved or recorded.

use egui::{Color32, RichText};
use photocraft_algo::luminosity::MAX_DEGREE;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Where a mask goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    LayerMask,
    Channel,
    Selection,
}

impl Output {
    const ALL: [Output; 3] = [Output::LayerMask, Output::Channel, Output::Selection];

    /// The `output` value `mask.luminosity` takes.
    fn id(self) -> &'static str {
        match self {
            Output::LayerMask => "layerMask",
            Output::Channel => "channel",
            Output::Selection => "selection",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Output::LayerMask => "Layer mask",
            Output::Channel => "Channel",
            Output::Selection => "Selection",
        }
    }
}

/// What the panel remembers between frames.
#[derive(Clone, Debug)]
struct PanelState {
    output: Output,
    /// Sample the composite (true) or only the active layer.
    composite: bool,
    /// The tone a colour mask is narrowed by (`lights`, `darks`, `midtones`), if any.
    colour_tone: Option<&'static str>,
    colour_degree: u32,
    /// The last refusal from the engine, shown until the next click.
    error: Option<String>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self { output: Output::LayerMask, composite: true, colour_tone: None, colour_degree: 1, error: None }
    }
}

const TONES: [(&str, &str); 3] = [("Lights", "lights"), ("Darks", "darks"), ("Midtones", "midtones")];

/// Colour families: label, command kind, button fill.
const COLOURS: [(&str, &str, Color32); 6] = [
    ("Reds", "reds", Color32::from_rgb(190, 60, 60)),
    ("Yellows", "yellows", Color32::from_rgb(180, 150, 30)),
    ("Greens", "greens", Color32::from_rgb(60, 150, 70)),
    ("Cyans", "cyans", Color32::from_rgb(40, 150, 160)),
    ("Blues", "blues", Color32::from_rgb(70, 100, 190)),
    ("Magentas", "magentas", Color32::from_rgb(170, 70, 160)),
];

fn state_id() -> egui::Id {
    egui::Id::unique("luminosity-panel-state")
}

/// The `mask.luminosity` params for one click. A tonal kind carries its `degree`; a colour kind
/// carries the panel's narrowing tone and degree instead.
fn mask_params(kind: &str, degree: u32, st: &PanelState) -> Value {
    let mut p = json!({ "kind": kind, "output": st.output.id(), "sampleAllLayers": st.composite });
    let coloured = COLOURS.iter().any(|(_, k, _)| *k == kind);
    if coloured {
        if let Some(tone) = st.colour_tone {
            p["tone"] = json!(tone);
            p["toneDegree"] = json!(st.colour_degree);
        }
    } else {
        p["degree"] = json!(degree);
    }
    p
}

/// Runs one mask; the engine's refusal (no layer to mask, a locked layer…) is kept for the panel.
fn create(app: &mut PhotocraftApp, kind: &str, degree: u32, st: &mut PanelState) {
    st.error = app.run("mask.luminosity", mask_params(kind, degree, st)).err();
}

pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("No document")).color(t.text_faint));
        return;
    }
    let ctx = ui.ctx().clone();
    let mut st = ctx.data_mut(|d| d.get_temp::<PanelState>(state_id())).unwrap_or_default();
    let mut click: Option<(&'static str, u32)> = None;

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tl!("Output")).color(t.text_faint));
        for o in Output::ALL {
            ui.selectable_value(&mut st.output, o, tl!(o.label()));
        }
    });
    let mut layer_only = !st.composite;
    ui.checkbox(&mut layer_only, tl!("Active layer only"));
    st.composite = !layer_only;
    ui.separator();

    egui::Grid::new("luminosity-tones").spacing([4.0, 4.0]).show(ui, |ui| {
        for (label, kind) in TONES {
            ui.label(tl!(label));
            for degree in 1..=MAX_DEGREE {
                let tip = format!("{} {degree}", tl!(label));
                if ui.button(degree.to_string()).on_hover_text(tip).clicked() {
                    click = Some((kind, degree));
                }
            }
            ui.end_row();
        }
    });
    ui.separator();

    ui.label(RichText::new(tl!("Colors")).color(t.text_faint));
    ui.horizontal_wrapped(|ui| {
        for (label, kind, fill) in COLOURS {
            let button = egui::Button::new(RichText::new(tl!(label)).color(Color32::WHITE)).fill(fill);
            if ui.add(button).clicked() {
                click = Some((kind, 1));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tl!("Narrow by")).color(t.text_faint));
        ui.selectable_value(&mut st.colour_tone, None, tl!("None"));
        for (label, kind) in TONES {
            ui.selectable_value(&mut st.colour_tone, Some(kind), tl!(label));
        }
    });
    if st.colour_tone.is_some() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Degree")).color(t.text_faint));
            ui.add(egui::Slider::new(&mut st.colour_degree, 1..=MAX_DEGREE));
        });
    }

    if let Some((kind, degree)) = click {
        create(app, kind, degree, &mut st);
    }
    if let Some(error) = &st.error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(t.text_faint));
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), st));
}

#[cfg(test)]
mod tests;
