//! Preset grayscale mixes for the Black & White adjustment.

use std::sync::Arc;

use craft::behavior::{EditEvent, EditEvents, Mixed};
use craft::imaging::ui::adjust::{AdjustContext, AdjustWords, BlackWhite, BlackWhiteEditor};
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::{AdjustAsk, AdjustOut, AdjustValue};

type Mix = [f64; 6];

/// The presets change the grayscale mix and keep the layer's tint settings.
fn entries() -> [Entry<Option<Mix>>; 16] {
    [
        Entry::new(Some(BlackWhite::DEFAULT_WEIGHTS), tl!("Default")),
        Entry::separator(),
        Entry::new(Some([0.0, 0.0, 0.0, 110.0, 110.0, 110.0]), tl!("Blue Filter")),
        Entry::new(Some([30.0, 50.0, 30.0, 50.0, 10.0, 70.0]), tl!("Darker")),
        Entry::new(Some([50.0, 120.0, 90.0, 50.0, 0.0, 0.0]), tl!("Green Filter")),
        Entry::new(Some([-50.0, -50.0, -50.0, 150.0, 150.0, 150.0]), tl!("High Contrast Blue Filter")),
        Entry::new(Some([120.0, 120.0, -10.0, -50.0, -50.0, 120.0]), tl!("High Contrast Red Filter")),
        Entry::new(Some([-40.0, 235.0, 144.0, -68.0, -3.0, -107.0]), tl!("Infrared")),
        Entry::new(Some([50.0, 70.0, 50.0, 70.0, 30.0, 90.0]), tl!("Lighter")),
        Entry::new(Some([0.0; 6]), tl!("Maximum Black")),
        Entry::new(Some([100.0; 6]), tl!("Maximum White")),
        Entry::new(Some([128.0, 128.0, 100.0, 100.0, 128.0, 100.0]), tl!("Neutral Density")),
        Entry::new(Some([120.0, 110.0, -10.0, -50.0, 0.0, 120.0]), tl!("Red Filter")),
        Entry::new(Some([120.0, 110.0, 40.0, -30.0, 0.0, 70.0]), tl!("Yellow Filter")),
        Entry::separator(),
        Entry::new(None, tl!("Custom")),
    ]
}

/// A manual edit can land on exactly the weights of a named recipe. Remember that edit locally
/// so its value still shows as Custom. The Properties inspector scopes this memory to its target.
#[derive(Clone, Copy, PartialEq)]
struct ManualValue {
    weights: Mix,
    tint: Option<[u8; 3]>,
}

impl From<BlackWhite> for ManualValue {
    fn from(value: BlackWhite) -> Self {
        let [r, g, b, _] = value.tint_color.to_rgba().0;
        Self { weights: value.weights, tint: value.tint.then_some([r, g, b]) }
    }
}

fn manual_id(ui: &Ui) -> egui::Id {
    ui.make_persistent_id("black-white-manual-value")
}

pub(crate) fn clear_custom(ui: &Ui) {
    ui.data_mut(|d| d.remove::<ManualValue>(manual_id(ui)));
    ui.ctx().request_repaint();
}

/// The Properties host applies accepted Color Picker colors and Auto outside the editor's pass.
pub(super) fn mark_custom(ui: &Ui, value: &AdjustValue) {
    if let AdjustValue::BlackWhite { value } = value {
        ui.data_mut(|d| d.insert_temp(manual_id(ui), ManualValue::from(*value)));
    }
}

pub(super) fn show(ui: &mut Ui, value: &mut BlackWhite, context: AdjustContext) -> AdjustOut {
    let id = manual_id(ui);
    let mut manual = ui.data(|d| d.get_temp::<ManualValue>(id));
    let entries = entries();
    let shown = (manual != Some((*value).into()) && entries.iter().any(|entry| entry.value() == Some(&Some(value.weights)))).then_some(value.weights);
    let label = tl!("Preset");
    let picked = Field::new(label).show(ui, |ui, _| Select::new(label, Bind::given(shown), &entries).show(ui).events.committed().copied()).inner.flatten();
    let mut edit = EditEvents::new();
    if let Some(picked) = picked {
        manual = picked.is_none().then(|| (*value).into());
        if let Some(weights) = picked
            && weights != value.weights
        {
            let before = *value;
            value.weights = weights;
            edit.push(EditEvent::set(Mixed::Same(before), *value, Arc::from(label), None));
        }
    }

    let out = BlackWhiteEditor::new("Black & White", Bind::bound(&mut *value)).context(context).show(ui);
    if let Some(after) = out.edit.events.committed() {
        let words = Env::of(ui).words::<AdjustWords>();
        let reset = matches!(out.edit.events.ended(), Some(EditEvent::Set { label, .. }) if label.as_ref() == words.default_button.as_str());
        manual = (!reset).then(|| (*after).into());
    }
    ui.data_mut(|d| match manual {
        Some(value) => {
            d.insert_temp(id, value);
        }
        None => d.remove::<ManualValue>(id),
    });

    let ask = if out.auto { Some(AdjustAsk::Auto) } else { out.pick_color.map(AdjustAsk::PickColor) };
    edit.extend(out.edit.events);
    AdjustOut { ask, edit: edit.map(|value| AdjustValue::BlackWhite { value }) }
}
