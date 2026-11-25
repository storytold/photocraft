//! The Retouch options bar's healing family: Remove, Patch, Content-Aware Move, Spot Healing, Clone Stamp and
//! Healing Brush. Also holds the cells the toning and selection families share.

use craft::behavior::Value;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::text::RangeAlert;
use photocraft_engine::presets::PresetsExt;
use serde_json::Value as Json;

use crate::PhotocraftApp;
use crate::option_cells::{hint, number};
use crate::state::Tool;

/// The tooltip of the Structure field of Content-Aware Move.
const STRUCTURE_TIP: &str = "Adjust how strictly the source structure is preserved";
/// The tooltip of the Color field of Content-Aware Move.
const COLOR_TIP: &str = "Adjust how much the source color can be modified";

/// A check box over `value`, with an optional tooltip.
pub(super) fn check(ui: &mut egui::Ui, label: &str, value: &mut bool, enabled: bool, tip: Option<&str>) {
    let mut cell = Checkbox::new(Bind::given(*value)).label(label.to_owned()).enabled(enabled);
    if let Some(tip) = tip {
        cell = cell.tooltip(tip.to_owned());
    }
    if let Some(on) = cell.show(ui).value().copied() {
        *value = on;
    }
}

/// A select over `entries` captioned by a field label, with an optional tooltip.
pub(super) fn choose<T: Value + Clone + 'static>(ui: &mut egui::Ui, caption: &str, value: &mut T, entries: &[Entry<T>], enabled: bool, tip: Option<&str>) {
    let picked = Field::new(caption.to_owned())
        .show(ui, |ui, _| {
            let mut cell = Select::new(caption.to_owned(), Bind::given(value.clone()), entries).enabled(enabled);
            if let Some(tip) = tip {
                cell = cell.tooltip(tip.to_owned());
            }
            cell.show(ui).value().cloned()
        })
        .inner
        .flatten();
    if let Some(picked) = picked {
        *value = picked;
    }
}

/// The entries of a string option from `(engine value, label)` pairs.
pub(super) fn string_entries(items: &[(&str, &str)]) -> Vec<Entry<String>> {
    items.iter().map(|(value, label)| Entry::new((*value).to_owned(), (*label).to_owned())).collect()
}

/// A radio group over a string option.
fn radios(ui: &mut egui::Ui, label: &str, value: &mut String, items: &[(&str, &str)]) {
    let picked = RadioGroup::new(label.to_owned(), Bind::given(value.clone()))
        .show(ui, |ui, _| {
            for (v, l) in items {
                Radio::new((*v).to_owned()).label((*l).to_owned()).show(ui);
            }
        })
        .output
        .value()
        .cloned();
    if let Some(picked) = picked {
        *value = picked;
    }
}

/// The first alert of two cells, both of which have been shown.
fn first(a: Option<RangeAlert>, b: Option<RangeAlert>) -> Option<RangeAlert> {
    a.or(b)
}

/// Remove's options. The apply and cancel pair belongs to the bar's pending strip.
pub(super) fn remove(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let composite = crate::canvas::paint_target(app).as_str() == Some("pixels");
    let o = &mut app.ui.tool_options;
    // The reference's options bar, in its order and words (#12546).
    check(ui, tl!("Sample all layers"), &mut o.sample_all.remove, true, Some(tl!("Sample content from all visible layers")));
    check(ui, tl!("Remove after each stroke"), &mut o.remove_after_each_stroke, true, Some(tl!("Remove area after a single stroke")));
    check(ui, tl!("Create new layer"), &mut o.remove_new_layer, composite, Some(tl!("Create new layer with results")));
    None
}

/// Patch's options: the lasso's operation, then Normal (Source / Destination) or Content-Aware.
pub(super) fn patch(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    crate::panels::options_bar::selection::operation_segments(app, ui, true);
    Divider::vertical().show(ui);
    let o = &mut app.ui.tool_options;
    let kinds = [Entry::new(false, tl!("Normal")), Entry::new(true, tl!("Content-Aware"))];
    choose(ui, tl!("Patch"), &mut o.patch_content_aware, &kinds, true, None);
    let mut alert = None;
    if o.patch_content_aware {
        let structure = number(tl!("Structure"), &mut o.patch_structure, 1.0..=7.0).whole().show(ui).out_of_range;
        let color = number(tl!("Color"), &mut o.patch_color, 0.0..=10.0).whole().show(ui).out_of_range;
        check(ui, tl!("Sample All Layers"), &mut o.sample_all.patch, true, None);
        alert = first(structure, color);
    } else {
        radios(ui, tl!("Patch"), &mut o.patch_mode, &[("source", tl!("Source")), ("destination", tl!("Destination"))]);
    }
    Divider::vertical().show(ui);
    hint(ui, tl!("Lasso around an area, then drag the selection"));
    alert
}

/// Content-Aware Move's options; every cell is disabled while the Transform On Drop box is up.
pub(super) fn content_aware_move(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let live = crate::cam_transform::active(app);
    crate::panels::options_bar::selection::operation_segments(app, ui, !live);
    Divider::vertical().show(ui);
    let o = &mut app.ui.tool_options;
    let modes = string_entries(&[("move", tl!("Move")), ("extend", tl!("Extend"))]);
    choose(ui, tl!("Mode"), &mut o.cam_mode, &modes, !live, Some(tl!("Choose a remix mode")));
    let structure =
        number(tl!("Structure"), &mut o.cam_structure, 1.0..=7.0).whole().popup_slider().tooltip(tl!(STRUCTURE_TIP)).enabled(!live).show(ui).out_of_range;
    let color = number(tl!("Color"), &mut o.cam_color, 0.0..=10.0).whole().popup_slider().tooltip(tl!(COLOR_TIP)).enabled(!live).show(ui).out_of_range;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.content_aware_move, !live, Some(tl!("Enable to remix all layers")));
    check(ui, tl!("Transform On Drop"), &mut o.cam_transform_on_drop, !live, Some(tl!("Allows rotation and scaling of selection")));
    if !live {
        Divider::vertical().show(ui);
        hint(ui, tl!("Lasso around an area, then drag the selection"));
    }
    first(structure, color)
}

/// Spot Healing's options.
pub(super) fn spot_healing(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let o = &mut app.ui.tool_options;
    radios(
        ui,
        tl!("Type"),
        &mut o.spot_type,
        &[("contentAware", tl!("Content-Aware")), ("createTexture", tl!("Create Texture")), ("proximityMatch", tl!("Proximity Match"))],
    );
    Divider::vertical().show(ui);
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.spot_healing, true, None);
    None
}

/// The options of Clone Stamp and Healing Brush. In an Indexed Color document the Clone Stamp's
/// Sample greys at Current Layer and keeps the stored choice for other documents (#12613).
pub(super) fn clone(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let source_unset = app.session.presets().clone.active().source.is_none();
    let clone_stamp = app.ui.tool == Tool::CloneStamp;
    let indexed = crate::retouch_ui::clone_stamp_indexed(app);
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Aligned"), &mut o.clone_aligned, true, clone_stamp.then(|| tl!("Keep the same source offset between strokes")));
    let samples = string_entries(&[("current", tl!("Current Layer")), ("currentAndBelow", tl!("Current & Below")), ("all", tl!("All Layers"))]);
    let mut greyed = "current".to_owned();
    let sample = if indexed { &mut greyed } else { &mut o.clone_sample };
    choose(ui, tl!("Sample"), sample, &samples, !indexed, clone_stamp.then(|| tl!("Choose which layers to sample")));
    if source_unset {
        Divider::vertical().show(ui);
        hint(ui, crate::i18n::fmt(tl!("{key}-click to set the source"), &[("key", &crate::shortcuts::pretty("Alt"))]));
    }
    None
}

#[allow(dead_code)]
fn _json_is_not_used(_: Json) {}
