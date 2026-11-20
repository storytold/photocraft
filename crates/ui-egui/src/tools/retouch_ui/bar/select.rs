//! The Retouch options bar's selection family: Quick Selection and Object Selection.

use craft::ui::contract::Show as _;
use craft::ui::display::Divider;
use craft::ui::text::RangeAlert;

use super::heal::check;
use crate::PhotocraftApp;
use crate::option_cells::hint;
use crate::panels::options_bar::selection::select_subject;

/// Quick Selection's options.
pub(super) fn quick_selection(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.quick_selection, true, None);
    check(ui, tl!("Enhance Edge"), &mut o.enhance_edge, true, None);
    hint(ui, crate::i18n::fmt(tl!("{key} to subtract"), &[("key", &crate::shortcuts::pretty("Alt"))]));
    Divider::vertical().show(ui);
    select_subject(app, ui, tl!("No subject found — try Object Selection or the Lasso"));
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
    select_subject(app, ui, tl!("No subject found — try a rectangle around the object"));
    None
}
