//! PhotoCraft's built-in Channel Mixer presets, shared by Properties and the adjustment dialog.

use craft::behavior::{EditEvent, EditEvents, Mixed};
use craft::imaging::ui::adjust::{AdjustContext, ChannelMixer, ChannelMixerEditor};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::pickers::Select;

#[derive(Clone, Copy, PartialEq)]
enum Preset {
    Default,
    Monochrome([f64; 4]),
    Custom,
}

// Original mixes with a neutral-gray total of 100% and zero offset. Color mixes emphasize
// their named primary or combination of primaries. Infrared emphasizes green while subtracting blue and red.
fn mixes() -> [(&'static str, [f64; 4]); 6] {
    [
        (tl!("Infrared Mix"), [-20.0, 155.0, -35.0, 0.0]),
        (tl!("Blue Mix"), [8.0, 12.0, 80.0, 0.0]),
        (tl!("Green Mix"), [15.0, 75.0, 10.0, 0.0]),
        (tl!("Orange Mix"), [68.0, 27.0, 5.0, 0.0]),
        (tl!("Red Mix"), [80.0, 16.0, 4.0, 0.0]),
        (tl!("Yellow Mix"), [40.0, 55.0, 5.0, 0.0]),
    ]
}

const MANUAL_PRESET: &str = "channel-mixer-manual-preset";

fn dialog_id() -> egui::Id {
    egui::Id::new(("adjustment-dialog", MANUAL_PRESET))
}

pub(crate) fn clear_dialog_status(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Preset>(dialog_id()));
}

pub(crate) fn clear_properties_status(ui: &egui::Ui) {
    ui.data_mut(|data| data.remove::<Preset>(ui.make_persistent_id(MANUAL_PRESET)));
}

fn matching(value: &ChannelMixer, mixes: &[(&str, [f64; 4])]) -> Preset {
    if let Some(gray) = value.gray
        && mixes.iter().any(|(_, row)| *row == gray)
    {
        Preset::Monochrome(gray)
    } else if *value == ChannelMixer::default() {
        Preset::Default
    } else {
        Preset::Custom
    }
}

pub(super) fn show(ui: &mut egui::Ui, value: &mut ChannelMixer, context: AdjustContext) -> EditEvents<ChannelMixer> {
    let id = if context.is_dialog() { dialog_id() } else { ui.make_persistent_id(MANUAL_PRESET) };
    let mixes = mixes();
    let mut preset = matching(value, &mixes);
    // A manual edit can end on the same coefficients as a named preset.
    if ui.data(|data| data.get_temp(id)) == Some(preset) {
        preset = Preset::Custom;
    }
    let mut entries = vec![Entry::new(Preset::Default, tl!("Default")), Entry::separator()];
    entries.extend(mixes.map(|(label, row)| Entry::new(Preset::Monochrome(row), label)));
    entries.extend([Entry::separator(), Entry::new(Preset::Custom, tl!("Custom"))]);

    let mut selected = None;
    Field::new(tl!("Preset")).id_salt("channel-mixer-preset").show(ui, |ui, _| {
        let out = Select::new(tl!("Preset"), Bind::given(preset), &entries).show(ui);
        selected = out.events.committed().copied();
    });
    let before = *value;
    let mut edits = EditEvents::new();
    if let Some(picked) = selected {
        match picked {
            Preset::Default => *value = ChannelMixer::default(),
            Preset::Monochrome(row) => value.gray = Some(row),
            Preset::Custom => {}
        }
        ui.data_mut(|data| {
            if picked == Preset::Custom {
                data.insert_temp(id, matching(value, &mixes));
            } else {
                data.remove::<Preset>(id);
            }
        });
        if *value != before {
            edits.push(EditEvent::set(Mixed::Same(before), *value, tl!("Preset").into(), None));
        }
    }

    let out = ChannelMixerEditor::new("Channel Mixer", Bind::bound(value)).context(context).show(ui);
    if let Some(EditEvent::Commit { before, after, .. } | EditEvent::Set { before, after, .. }) = out.events.ended()
        && *before != Mixed::Same(*after)
    {
        ui.data_mut(|data| data.insert_temp(id, matching(after, &mixes)));
    }
    edits.extend(out.events);
    edits
}
