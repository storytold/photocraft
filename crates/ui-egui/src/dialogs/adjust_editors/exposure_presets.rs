//! Exposure's presets in Image › Adjustments and the adjustment layer's Properties panel.

use std::sync::Arc;

use craft::behavior::{EditEvent, EditEvents, Mixed};
use craft::imaging::ui::adjust::{AdjustContext, AdjustField, AdjustSliders, AdjustWords};
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use egui::{Id, Ui};

/// Exposure compensation in stops, with neutral offset and gamma.
const STOPS: [f64; 5] = [0.0, -2.0, -1.0, 1.0, 2.0];
const CUSTOM: usize = STOPS.len();

const CUSTOM_KEY: &str = "exposure-preset-custom";

fn dialog_key() -> Id {
    Id::unique(("adjustment-dialog", CUSTOM_KEY))
}

/// Returns the menu to the named preset that matches its values after Properties resets them.
pub(crate) fn clear_custom(ui: &Ui) {
    ui.data_mut(|data| data.remove::<Vec<f64>>(ui.make_persistent_id(CUSTOM_KEY)));
}

pub(crate) fn clear_dialog_custom(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Vec<f64>>(dialog_key()));
}

pub(super) fn show(ui: &mut Ui, values: &mut Vec<f64>, context: AdjustContext) -> EditEvents<Vec<f64>> {
    let key = if context.is_dialog() { dialog_key() } else { ui.make_persistent_id(CUSTOM_KEY) };
    let manually_edited = ui.data(|data| data.get_temp::<Vec<f64>>(key).is_some_and(|edited| edited == *values));
    let active = if manually_edited { CUSTOM } else { STOPS.iter().position(|&stops| *values == [stops, 0.0, 1.0]).unwrap_or(CUSTOM) };
    let entries = [
        Entry::new(0, tl!("Default")),
        Entry::new(1, tl!("−2 EV")),
        Entry::new(2, tl!("−1 EV")),
        Entry::new(3, tl!("+1 EV")),
        Entry::new(4, tl!("+2 EV")),
        Entry::new(CUSTOM, tl!("Custom")),
    ];
    let mut edit = EditEvents::new();
    let _ = Field::new(tl!("Preset")).show(ui, |ui, _| {
        let menu = Select::new(tl!("Preset"), Bind::given(active), &entries).show(ui);
        if let Some(&index) = menu.events.committed() {
            if let Some(&stops) = STOPS.get(index) {
                ui.data_mut(|data| data.remove::<Vec<f64>>(key));
                let next = vec![stops, 0.0, 1.0];
                if *values != next {
                    let before = std::mem::replace(values, next.clone());
                    edit.push(EditEvent::set(Mixed::Same(before), next, Arc::from(tl!("Preset")), None));
                }
            } else {
                ui.data_mut(|data| data.insert_temp(key, values.clone()));
            }
        }
    });

    let fields = AdjustField::exposure(&Env::of(ui).words::<AdjustWords>());
    let sliders = AdjustSliders::new("Exposure", &fields, Bind::bound(values)).context(context).show(ui);
    if let Some(after) = sliders.events.committed() {
        // Typing a preset's exact value is still a manual edit. Keep that distinction in the
        // editor's session without putting UI state in the document's adjustment parameters.
        ui.data_mut(|data| data.insert_temp(key, after.clone()));
    }
    edit.extend(sliders.events);
    edit
}
