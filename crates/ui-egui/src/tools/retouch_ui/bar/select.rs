//! The Retouch options bar's selection family: Quick Selection and Object Selection.

use craft::ui::buttons::Button;
use craft::ui::contract::{Show as _, clicked};
use craft::ui::display::Divider;
use craft::ui::text::RangeAlert;
use serde_json::json;

use super::heal::check;
use crate::PhotocraftApp;
use crate::option_cells::hint;

/// The Select Subject button both selection tools carry; `empty` is the message when nothing is found.
fn select_subject(ui: &mut egui::Ui, app: &mut PhotocraftApp, empty: &str) {
    if clicked(&Button::new(tl!("Select Subject")).show(ui)) {
        let r = app.run("select.subject", json!({}));
        crate::retouch_ui::report_smart_result(app, r, empty);
    }
}

/// Quick Selection's options.
pub(super) fn quick_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    // The Quick Selection's New / Add / Subtract buttons (#11837), its own mode as in Photoshop;
    // the framework's segmented control binds the document's mode, so it is swapped in for the
    // frame and taken back out.
    if app.ui.tool_options.quick_selection_mode > 2 {
        app.ui.tool_options.quick_selection_mode = 1;
    }
    let saved = app.ui.selection_mode;
    app.ui.selection_mode = app.ui.tool_options.quick_selection_mode;
    crate::panels::options_bar::selection::operation_segments(app, ui, true);
    app.ui.tool_options.quick_selection_mode = app.ui.selection_mode.min(2);
    app.ui.selection_mode = saved;
    Divider::vertical().show(ui);
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.quick_selection, true, None);
    check(ui, tl!("Enhance Edge"), &mut o.enhance_edge, true, None);
    hint(ui, crate::i18n::fmt(tl!("{key} to subtract"), &[("key", &crate::shortcuts::pretty("Alt"))]));
    Divider::vertical().show(ui);
    select_subject(ui, app, tl!("No subject found — try Object Selection or the Lasso"));
    None
}

/// Object Selection's options: the selection operation (#12853), Object Finder, Sample All Layers
/// and Hard Edge.
pub(super) fn object_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    crate::panels::options_bar::selection::operation_segments(app, ui, true);
    Divider::vertical().show(ui);
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Object Finder"), &mut o.object_finder, true, None);
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.object_selection, true, None);
    check(ui, tl!("Hard Edge"), &mut o.object_hard_edge, true, None);
    hint(ui, tl!("Drag a rectangle around the object"));
    Divider::vertical().show(ui);
    select_subject(ui, app, tl!("No subject found — try a rectangle around the object"));
    None
}
