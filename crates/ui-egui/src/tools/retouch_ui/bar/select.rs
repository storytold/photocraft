//! The Retouch options bar's selection family: Quick Selection and Object Selection.

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::menu::{Menu, MenuItem, entry_rows};
use craft::ui::pickers::Select;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::access::PopupKind;
use craft::ui::text::RangeAlert;
use egui::Color32;
use serde_json::json;

use super::heal::check;
use crate::PhotocraftApp;
use crate::option_cells::{hint, number};
use crate::state::{ObjectFinderMode, OverlayShow};

/// The Select Subject button both selection tools carry; `empty` is the message when nothing is found.
fn select_subject(ui: &mut egui::Ui, app: &mut PhotocraftApp, empty: &str) {
    if clicked(&Button::new(tl!("Select Subject")).show(ui)) {
        let r = app.run("select.subject", json!({}));
        crate::retouch_ui::report_smart_result(app, r, empty);
    }
}

/// Quick Selection's options.
pub(super) fn quick_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.quick_selection, true, None);
    check(ui, tl!("Enhance Edge"), &mut o.enhance_edge, true, None);
    hint(ui, crate::i18n::fmt(tl!("{key} to subtract"), &[("key", &crate::shortcuts::pretty("Alt"))]));
    Divider::vertical().show(ui);
    select_subject(ui, app, tl!("No subject found — try Object Selection or the Lasso"));
    None
}

/// A row of the Object Selection gear menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GearOption {
    Finder,
    Subtract,
    Auto,
    Manual,
}

/// The gear's four entries (#12857), in the reference's order.
fn gear_entries(o: &crate::state::ToolOptions) -> [Entry<GearOption>; 4] {
    let auto = o.object_finder_mode == ObjectFinderMode::Auto;
    [
        Entry::new(GearOption::Finder, tl!("Object Finder").to_string()).checked(o.object_finder),
        Entry::new(GearOption::Subtract, tl!("Object Subtract").to_string()).checked(o.object_subtract),
        Entry::new(GearOption::Auto, tl!("Auto Refresh").to_string()).checked(auto),
        Entry::new(GearOption::Manual, tl!("Manual Refresh").to_string()).checked(!auto),
    ]
}

/// The swatch of an Overlay Options colour: the name's colour in a square, both in the list's row and
/// on the closed dropdown (#12860).
fn overlay_swatch(id: &str, cx: &ContentPainterCx, rect: egui::Rect) {
    let rgb = crate::object_finder::OVERLAY_COLORS.iter().find(|(known, ..)| *known == id).map_or([0, 0, 0], |(.., rgb)| *rgb);
    cx.painter.add(egui::Shape::rect_filled(rect, egui::CornerRadius::same(1), Color32::from_rgb(rgb[0], rgb[1], rgb[2])));
}

/// The section the gear popup ends with (#12860): Color with its thirteen names and their swatches,
/// the Outline and Opacity fields at 0-50 px and 0-100 %, and the three Show Overlay radios. The
/// range alert of the two fields comes back in `alert` for the bar to open.
fn overlay_options(app: &mut PhotocraftApp, ui: &mut egui::Ui, alert: &mut Option<RangeAlert>) {
    Divider::horizontal().show(ui);
    MenuItem::heading(tl!("Overlay Options")).show(ui);
    let o = &mut app.ui.tool_options;
    let colors: Vec<Entry<String>> =
        crate::object_finder::OVERLAY_COLORS.iter().map(|(id, name, _)| Entry::new((*id).to_owned(), (*name).to_owned())).collect();
    let picked = Field::new(tl!("Color"))
        .show(ui, |ui, _| {
            Select::new(tl!("Color"), Bind::given(o.object_overlay_color.clone()), &colors)
                .picture(|id: &String, cx: &ContentPainterCx, rect: egui::Rect| overlay_swatch(id, cx, rect))
                .show(ui)
                .value()
                .cloned()
        })
        .inner
        .flatten();
    if let Some(picked) = picked {
        o.object_overlay_color = picked;
    }
    // Both fields take whole numbers, show a slider over their range, and hand back the alert of a
    // typed value past an end, which the bar's own range-alert rule words.
    let outline = number(tl!("Outline"), &mut o.object_overlay_outline, 0.0..=50.0).whole().unit(tl!("px")).popup_slider().show(ui).out_of_range;
    let opacity = number(tl!("Opacity"), &mut o.object_overlay_opacity, 0.0..=100.0)
        .whole()
        .unit("%")
        .popup_slider()
        .tooltip(tl!("Set selection preview overlay opacity."))
        .show(ui)
        .out_of_range;
    *alert = outline.or(opacity);
    let modes = [
        (tl!("Auto Show Overlay").to_string(), OverlayShow::Auto),
        (tl!("Always Show Overlay").to_string(), OverlayShow::Always),
        (tl!("Never Show Overlay").to_string(), OverlayShow::Never),
    ];
    let picked = RadioGroup::new(tl!("Show Overlay"), Bind::given(o.object_overlay_show))
        .show(ui, |ui, _| {
            for (label, mode) in &modes {
                Radio::new(*mode).label(label.clone()).show(ui);
            }
        })
        .output
        .value()
        .cloned();
    if let Some(picked) = picked {
        o.object_overlay_show = picked;
    }
}

/// The bar's Show all objects toggle (#12860): off by default, square icon, and N flips what it
/// shows while it is held without changing the stored setting. What the mode shows on the canvas is
/// not measured, so the preview itself stays as it is today.
fn show_all_objects(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let held = ui.input(|i| i.key_down(egui::Key::N));
    let o = &mut app.ui.tool_options;
    let on = o.object_show_all_objects != held;
    let out = ToggleButton::new(tl!("Show all objects").to_string(), Bind::given(on))
        .icon(IconId::new("square"))
        .icon_only()
        .tooltip(tl!("Show all objects (hold N to toggle preview mode)"))
        .show(ui);
    if let Some(picked) = out.value().copied() {
        o.object_show_all_objects = picked;
    }
}

/// The Refresh button, the Show all objects toggle and the gear menu after the operation segments
/// (#12857, #12860), the popup ending with the Overlay Options section.
fn finder_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> Option<RangeAlert> {
    let refresh = IconButton::new(IconId::new("rotate_cw")).label(tl!("Refresh").to_string()).tooltip(tl!("Click to refresh object finder.")).show(ui);
    show_all_objects(app, ui);
    let label = tl!("Set additional options").to_string();
    let button = IconButton::new(IconId::new("settings")).label(label.clone()).tooltip(tl!("Set additional options")).popup(PopupKind::Menu).show(ui);
    let entries = gear_entries(&app.ui.tool_options);
    let mut picked = None;
    let mut alert = None;
    // The section is the menu's companion, not one of its rows: a menu closes on the first row that
    // runs, which would drop the popup the moment a field or a radio is touched. The companion is the
    // framework's modeless layer, shown in the menu's end region - below the four rows - and it closes
    // with the menu.
    let mut section = |ui: &mut egui::Ui| overlay_options(app, ui, &mut alert);
    Menu::new(label).opened_by(&button, Trigger::Press).companion(&mut section).show(ui, |ui, _| {
        picked = entry_rows(ui, &entries).and_then(|at| entries.get(at)).and_then(Entry::value).cloned();
    });
    match picked {
        Some(GearOption::Finder) => app.ui.tool_options.object_finder = !app.ui.tool_options.object_finder,
        Some(GearOption::Subtract) => app.ui.tool_options.object_subtract = !app.ui.tool_options.object_subtract,
        Some(GearOption::Auto) => app.ui.tool_options.object_finder_mode = ObjectFinderMode::Auto,
        Some(GearOption::Manual) => app.ui.tool_options.object_finder_mode = ObjectFinderMode::Manual,
        None => {}
    }
    if clicked(&refresh) {
        crate::object_finder::refresh(app);
    }
    alert
}

/// Object Selection's options: the selection operation (#12853), Refresh, the Show all objects
/// toggle and the gear menu with its Overlay Options section, Sample All Layers and Hard Edge.
pub(super) fn object_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    crate::panels::options_bar::selection::operation_segments(app, ui, true);
    Divider::vertical().show(ui);
    let alert = finder_buttons(app, ui);
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.object_selection, true, None);
    check(ui, tl!("Hard Edge"), &mut o.object_hard_edge, true, None);
    Divider::vertical().show(ui);
    select_subject(ui, app, tl!("No subject found — try a rectangle around the object"));
    alert
}
