//! The contextual task bar for an open Free Transform box (#11104).
//!
//! The active document must own the transform preview. The bar is hidden when the shared
//! task bar is disabled, a dialog is open, or the transform is committing. Crop, Warp,
//! Transform Selection and Content-Aware Scale keep their own controls.
//!
//! The bar follows the box's screen bounds on the active canvas. It uses the same placement
//! and pin, reset and hide menu as Crop. The Window menu can show the shared bar again.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{Tool, TransformSession};
use crate::widgets::{self, task_bar};

enum Action {
    Preset(&'static str),
    Cancel,
    Done,
}

#[derive(Clone, Copy)]
struct ShownRect {
    session: u64,
    pass: u64,
    rect: [f32; 4],
}

fn shown_id() -> egui::Id {
    egui::Id::unique("free-transform-task-bar-rect")
}

/// Crop keeps its own bar. Warp, Transform Selection and Content-Aware Scale use their own controls.
fn open_box(app: &PhotocraftApp) -> Option<&TransformSession> {
    let active = app.session.active()?;
    let preview = app.transform_preview.as_ref()?;
    if !app.ui.panels.context_task_bar
        || app.ui.tool == Tool::Crop
        || !app.ui.dialogs.is_empty()
        || app.ui.shell.dialog.is_some()
        || preview.doc.id != active.doc.id
        || crate::transform_tool::committing(app).is_some()
    {
        return None;
    }
    app.ui.transform.as_ref().filter(|t| t.warp.is_none() && !t.selection && t.content_aware.is_none())
}

/// Whether Free Transform can show its bar.
/// This checks current eligibility without changing placement or the transform.
pub fn eligible(app: &PhotocraftApp) -> bool {
    open_box(app).is_some()
}

/// The current or last painted rectangle of this transform session. Crop keeps its own rectangle.
pub(crate) fn shown_rect(app: &PhotocraftApp, ctx: &egui::Context) -> Option<[f32; 4]> {
    let session = open_box(app)?.session;
    let shown = ctx.data(|data| data.get_temp::<ShownRect>(shown_id()))?;
    // Extra document windows draw within the root pass. Inspection also runs between passes,
    // when egui has already incremented the counter for the last drawn rectangle.
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    (shown.session == session && pass.checked_sub(shown.pass).is_some_and(|age| age <= 1)).then_some(shown.rect)
}

/// Draw the shared bar, persist its placement and apply the selected action before Hide.
/// Report failed preset commands in the status bar. After the action, publish the rectangle
/// for inspection while Free Transform remains eligible, or clear it if the action closed or hid it.
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    let Some(transform) = open_box(app) else { return };
    let session = transform.session;
    let corners = transform.quad.map(|[x, y]| xf.to_screen(x as f32, y as f32));
    let anchor = egui::Rect::from_points(&corners);
    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = task_bar::show(&ctx, &mut place, anchor, xf.rect, contents);
    app.ui.panels.task_bar = place;
    let Some(shown) = shown else { return };
    match shown.inner {
        Some(Action::Preset(id)) => {
            if let Err(error) = crate::menus::invoke(app, &ctx, id, json!({})) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
        Some(Action::Cancel) => crate::transform_tool::cancel(app),
        Some(Action::Done) => crate::transform_tool::commit(app),
        None => {}
    }
    if shown.hide {
        app.ui.panels.context_task_bar = false;
    }
    // An action in an extra window can invalidate the rect written by the main canvas.
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    if open_box(app).is_some_and(|transform| transform.session == session) {
        let r = shown.rect;
        ctx.data_mut(|data| data.insert_temp(shown_id(), ShownRect { session, pass, rect: [r.min.x, r.min.y, r.max.x, r.max.y] }));
    } else {
        ctx.data_mut(|data| data.remove::<ShownRect>(shown_id()));
    }
}

/// Draw presets in the measured order, with the shared menu before Cancel and Done.
fn contents(ui: &mut egui::Ui, more: &mut task_bar::MoreMenu) -> Option<Action> {
    let mut action = None;
    for (icon, label, id) in [
        ("rotate-ccw", tl!("Rotate 90° Counter Clockwise"), "edit.transform.rotate90Ccw"),
        ("rotate-cw", tl!("Rotate 90° Clockwise"), "edit.transform.rotate90Cw"),
    ] {
        if crate::icons::button_with_icon_size_tip(ui, icon, task_bar::BUTTON, 14.0, false, label, false).clicked() {
            action = Some(Action::Preset(id));
        }
    }
    for (label, id) in [(tl!("Flip Horizontal"), "edit.transform.flipHorizontal"), (tl!("Flip Vertical"), "edit.transform.flipVertical")] {
        if ui.button(label).clicked() {
            action = Some(Action::Preset(id));
        }
    }
    more.show(ui);
    if widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
        action = Some(Action::Cancel);
    }
    if widgets::primary_button(ui, tl!("Done"), 0.0).clicked() {
        action = Some(Action::Done);
    }
    action
}
