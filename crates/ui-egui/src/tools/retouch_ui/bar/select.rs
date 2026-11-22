//! The Retouch options bar's selection family: Quick Selection and Object Selection.

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Entry, Props as _, Show as _, Trigger, clicked};
use craft::ui::display::Divider;
use craft::ui::menu::Menu;
use craft::ui::runtime::access::PopupKind;
use craft::ui::text::RangeAlert;
use serde_json::json;

use super::heal::check;
use crate::PhotocraftApp;
use crate::option_cells::hint;
use crate::state::ObjectFinderMode;

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

/// The Refresh button and the gear menu after the operation segments (#12857): Object Finder,
/// Object Subtract (stored only) and Object Finder Mode (Auto or Manual Refresh).
fn finder_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let refresh = IconButton::new(IconId::new("rotate_cw")).label(tl!("Refresh").to_string()).tooltip(tl!("Click to refresh object finder.")).show(ui);
    let label = tl!("Set additional options").to_string();
    let button = IconButton::new(IconId::new("settings")).label(label.clone()).tooltip(tl!("Set additional options")).popup(PopupKind::Menu).show(ui);
    let o = &app.ui.tool_options;
    let auto = o.object_finder_mode == ObjectFinderMode::Auto;
    let entries = [
        Entry::new(GearOption::Finder, tl!("Object Finder").to_string()).checked(o.object_finder),
        Entry::new(GearOption::Subtract, tl!("Object Subtract").to_string()).checked(o.object_subtract),
        Entry::new(GearOption::Auto, tl!("Auto Refresh").to_string()).checked(auto),
        Entry::new(GearOption::Manual, tl!("Manual Refresh").to_string()).checked(!auto),
    ];
    let picked = Menu::new(label).opened_by(&button, Trigger::Press).pick(ui, &entries).picked;
    let o = &mut app.ui.tool_options;
    match picked {
        Some(GearOption::Finder) => o.object_finder = !o.object_finder,
        Some(GearOption::Subtract) => o.object_subtract = !o.object_subtract,
        Some(GearOption::Auto) => o.object_finder_mode = ObjectFinderMode::Auto,
        Some(GearOption::Manual) => o.object_finder_mode = ObjectFinderMode::Manual,
        None => {}
    }
    if clicked(&refresh) {
        crate::object_finder::refresh(app);
    }
}

/// Object Selection's options: the selection operation (#12853), Refresh and the gear menu,
/// Sample All Layers and Hard Edge.
pub(super) fn object_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    crate::panels::options_bar::selection::operation_segments(app, ui, true);
    Divider::vertical().show(ui);
    finder_buttons(app, ui);
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.object_selection, true, None);
    check(ui, tl!("Hard Edge"), &mut o.object_hard_edge, true, None);
    Divider::vertical().show(ui);
    select_subject(ui, app, tl!("No subject found — try a rectangle around the object"));
    None
}
