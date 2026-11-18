//! The options bar's members of the Move, Eyedropper, Zoom, Rotate View and Hand tools.

use craft::ui::canvas::{NavigateBar, NavigationOptions};
use craft::ui::contract::{Bind, Entry, Show as _};
use craft::ui::display::Divider;
use craft::ui::pickers::Select;

use super::check;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The members of the active tool when it is one of the navigation tools.
pub(super) fn members(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    match app.ui.tool {
        Tool::Move => move_members(app, ui),
        Tool::Eyedropper => crate::eyedropper_ui::options(app, ui),
        Tool::Zoom | Tool::RotateView | Tool::Hand => view_members(app, ui),
        _ => {}
    }
}

/// Auto-Select, its target, Show Transform Controls, then the align strip.
fn move_members(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let o = &mut app.ui.tool_options;
    check(ui, tl!("Auto-Select:"), &mut o.move_auto_select);
    let targets = [Entry::new("group".to_owned(), tl!("Group").to_owned()), Entry::new("layer".to_owned(), tl!("Layer").to_owned())];
    Select::new(tl!("Auto-Select:").to_owned(), Bind::bound(&mut o.move_target), &targets).show(ui);
    Divider::vertical().show(ui);
    check(ui, tl!("Show Transform Controls"), &mut o.move_show_transform);
    Divider::vertical().show(ui);
    crate::align_bar::show(app, ui);
}

/// The navigation choices the bar edits: the preferences and the tools' own options, as the framework's
/// [`NavigationOptions`].
fn navigation_options(app: &PhotocraftApp) -> NavigationOptions {
    let general = &app.session.prefs().general;
    let mut options = NavigationOptions::default();
    options.zoom_resizes_windows = general.zoom_resizes_windows;
    options.animated_zoom = general.animated_zoom;
    options.scrubby_zoom = app.ui.tool_options.zoom_scrubby;
    options.scroll_all_windows = app.ui.tool_options.hand_scroll_all;
    options
}

/// Zoom, Rotate View and Hand on the framework's `NavigateBar`. The bar edits a copy of the active document's view,
/// which goes back to the document when a field changed it. Its 100 %, Fit Screen and Fill Screen buttons are view
/// commands the canvas runs.
fn view_members(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let tool = app.ui.tool;
    let mut options = navigation_options(app);
    let before = options;
    if tool == Tool::Hand {
        NavigateBar::hand(&mut options).show(ui);
    } else if let Some(index) = app.session.active_index()
        && let Some(mut view) = crate::canvas::document_display::active_view(app)
    {
        if tool == Tool::Zoom {
            let target = crate::canvas::view_target(app, index);
            // Zoom Out is the bar's `out`; the tool keeps the direction as `zoom_in`.
            let mut out = !app.ui.tool_options.zoom_in;
            let shown = NavigateBar::zoom(Bind::bound(&mut view), target, &mut options, Bind::bound(&mut out)).show(ui);
            app.ui.tool_options.zoom_in = !out;
            crate::chrome_ui::zoom_alert(app, shown.invalid_zoom);
            write_view(app, index, &view, !shown.view.is_empty());
        } else {
            let shown = NavigateBar::rotate_view(Bind::bound(&mut view)).show(ui);
            write_view(app, index, &view, !shown.view.is_empty());
        }
    }
    if options != before {
        app.ui.tool_options.zoom_scrubby = options.scrubby_zoom;
        app.ui.tool_options.hand_scroll_all = options.scroll_all_windows;
        // Resize Windows to Fit is Preferences › Zoom Resizes Windows itself, as in Photoshop: one setting, shown in
        // both places and applied by `canvas/document_windows.rs`.
        if options.zoom_resizes_windows != before.zoom_resizes_windows {
            app.session.prefs.edit(|p| p.general.zoom_resizes_windows = options.zoom_resizes_windows);
        }
    }
}

/// Gives document `index` the bar's `view` when a field edited it this pass.
fn write_view(app: &mut PhotocraftApp, index: usize, view: &craft::behavior::CanvasView, edited: bool) {
    if !edited {
        return;
    }
    let ppp = app.canvas_ppp();
    if let Some(v) = app.ui.views.get_mut(index) {
        v.set_from(view, ppp);
    }
}
