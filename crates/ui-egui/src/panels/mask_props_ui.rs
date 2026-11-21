//! Non-destructive layer-mask controls (pixel and vector), shown in the Properties panel.

use craft::keymap::CommandRef;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show, ShowContainer};
use craft::ui::form::SliderField;
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::Section;
use craft::ui::value_drag::SliderScale;
use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Image › Adjustments › Invert, which inverts a targeted layer mask in place (#780).
const INVERT: &str = "image.adjustments.invert";

/// The Layers panel targets the active layer's pixel mask (not its vector mask) and Properties
/// shows it: a just-created adjustment or fill layer shows its own settings instead (#3369).
pub fn targeted(app: &PhotocraftApp, layer: &Layer) -> bool {
    app.ui.mask_target && !app.ui.vector_mask_target && layer.mask.is_some() && app.ui.properties_layer != Some(layer.id.0)
}

/// The Layers panel targets the active layer's vector mask.
pub fn vector_targeted(app: &PhotocraftApp, layer: &Layer) -> bool {
    app.ui.vector_mask_target && layer.vector_mask.is_some()
}

/// One mask section: its title, the command that edits it and whether it has the Refine row.
struct MaskSection {
    /// Keys the section's ids and its coalescing.
    key: &'static str,
    title: &'static str,
    command: &'static str,
    refine: bool,
}

/// The pixel-mask section: Density, Feather and the Refine row. Pixels stay intact; the two
/// properties ride on the mask and are applied by the compositor.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(mask) = &layer.mask else { return };
    let section = MaskSection { key: "pixel-mask", title: tl!("Layer Mask"), command: "layer.layerMask.edit", refine: true };
    mask_section(app, ui, layer, &section, mask.density, mask.feather);
}

/// The vector-mask section: Density and Feather, likewise non-destructive.
pub fn vector_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(vm) = &layer.vector_mask else { return };
    let section = MaskSection { key: "vector-mask", title: tl!("Vector Mask"), command: "layer.vectorMask.edit", refine: false };
    mask_section(app, ui, layer, &section, vm.density, vm.feather);
}

/// A shape layer's path is its vector mask (Photoshop's Properties › Masks): its Density and
/// Feather soften the fill through `layer.vectorMask.edit`, the shape staying editable.
pub fn shape_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerContent::Shape(sh) = &layer.content else { return };
    if layer.vector_mask.is_some() {
        return;
    }
    let section = MaskSection { key: "shape-mask", title: tl!("Vector Mask"), command: "layer.vectorMask.edit", refine: false };
    mask_section(app, ui, layer, &section, sh.density, sh.feather);
}

/// A section of Density (%) and Feather (px) fields writing one undoable edit per gesture: the value lives on
/// the mask, not in its pixels. The gesture's id keys the coalescing, so two drags are two undo steps.
fn mask_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer, section: &MaskSection, density: f32, feather: f32) {
    let Some(document) = app.session.active().map(|s| s.doc.id.0) else { return };
    let layer_id = layer.id.0;
    let mut edits: Vec<Value> = Vec::new();
    ui.push_id((document, layer.id, section.key), |ui| {
        Section::new(section.title).show(ui, |ui, _| {
            // Feather's track is curved like Photoshop's, so a few px can be dragged to (#6545).
            let rows = [
                ("density", tl!("Density"), f64::from(density) * 100.0, 100.0, 100.0, UnitKind::Percent, SliderScale::Linear),
                ("feather", tl!("Feather"), f64::from(feather), 1000.0, 0.0, UnitKind::Pixels, SliderScale::Power(3.0)),
            ];
            for (field, label, value, max, reset, unit, scale) in rows {
                let out = SliderField::new(label, Bind::given(value).reset(reset), 0.0..=max).scale(scale).unit_kind(unit).show(ui);
                for event in &out.events {
                    // A cancel lands on the value from before the drag, written into the same undo step.
                    let Some(value) = event.landed() else { continue };
                    let mut params = json!({"document": document, "layer": layer_id});
                    if let Some(gesture) = event.id() {
                        params["coalesce"] = json!(format!("{}:{document}:{layer_id}:{field}:{}", section.key, gesture.0));
                    }
                    params[field] = json!(value);
                    edits.push(params);
                }
            }
            if section.refine {
                refine_row(ui);
            }
        });
    });
    for params in edits {
        if let Err(error) = app.run(section.command, params) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

/// Photoshop's Refine row below Density and Feather: Select and Mask… and Color Range… open their
/// dialogs (the mask is Select and Mask's input when nothing is selected), and Invert flips the
/// mask. Invert names its target itself (`"target":"mask"`) rather than leaning on the shell's
/// target routing, so it edits the mask and never the layer's pixels (#1137).
fn refine_row(ui: &mut egui::Ui) {
    Bar::new(tl!("Refine"), BarKind::Options).show(ui, |ui, _| {
        Button::command(CommandRef::new("select.selectAndMask")).show(ui);
        Button::command(CommandRef::new("select.colorRange")).show(ui);
        Button::command(CommandRef::with_params(INVERT, json!({"target": "mask"}))).show(ui);
    });
}
