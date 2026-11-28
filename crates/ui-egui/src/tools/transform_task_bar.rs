//! Free Transform's contextual task bar, using the framework's shared `TaskBar`.
//! Presets edit the open box; Cancel and Done use the existing session actions.
//! Crop, Warp, Transform Selection, Content-Aware Scale and Frame keep their controls.

use craft::behavior::IconId;
use craft::draw::ui::transform::TransformPhase;
use craft::theme::Variant;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas_controls::TaskBar;
use craft::ui::contract::{Bind, Props as _, Show as _};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{FreeTransform, TaskBarContext, Tool};

enum Action {
    Preset(&'static str),
    Cancel,
    Done,
}

#[derive(Clone, Copy)]
struct ShownRect {
    session: u64,
    rect: [f32; 4],
}

fn shown_id() -> egui::Id {
    egui::Id::unique("free-transform-task-bar-rect")
}

fn open_box(app: &PhotocraftApp) -> Option<&FreeTransform> {
    let active = app.session.active()?;
    let preview = app.transform_preview.as_ref()?;
    if !app.ui.panels.context_task_bar
        || app.ui.tool == Tool::Crop
        || app.dialog_open()
        || preview.doc.id != active.doc.id
        || crate::transform_tool::committing(app).is_some()
    {
        return None;
    }
    app.ui.transform.as_ref().filter(|t| {
        t.session.phase() == TransformPhase::Box
            && !t.selection
            && t.content_aware.is_none()
            // Main's Frame tool has its own task bar and must keep it.
            && !crate::frame_tool::frame_transform(t)
    })
}

pub fn eligible(app: &PhotocraftApp) -> bool {
    open_box(app).is_some()
}

pub(crate) fn shown_rect(app: &PhotocraftApp, ctx: &egui::Context) -> Option<[f32; 4]> {
    let session = open_box(app)?.id;
    let shown = ctx.data(|data| data.get_temp::<ShownRect>(shown_id()))?;
    (shown.session == session).then_some(shown.rect)
}

/// Called on the document's primary canvas, where the framework shows its transform.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, area: egui::Rect) {
    // The canvas run that shows the bar records where; a replay of that run shows it there still.
    let Some(transform) = open_box(app) else {
        ui.ctx().data_mut(|data| data.remove::<ShownRect>(shown_id()));
        return;
    };
    let (session, corners) = (transform.id, transform.session.quad());
    if let Some(doc) = app.session.active().map(|d| d.doc.id.0) {
        app.ui.enter_task_bar_context(TaskBarContext::Transform { doc, session });
    }
    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = TaskBar::new(tl!("Contextual Task Bar"), &corners, Bind::bound(&mut place)).show(ui, area, |ui, cx| {
        let mut action = None;
        for (icon, label, id) in [
            ("rotate_ccw", tl!("Rotate 90° Counter Clockwise"), "edit.transform.rotate90Ccw"),
            ("rotate_cw", tl!("Rotate 90° Clockwise"), "edit.transform.rotate90Cw"),
        ] {
            if IconButton::new(IconId::new(icon)).label(label).show(ui).clicked() {
                action = Some(Action::Preset(id));
            }
        }
        for (label, id) in [(tl!("Flip Horizontal"), "edit.transform.flipHorizontal"), (tl!("Flip Vertical"), "edit.transform.flipVertical")] {
            if Button::new(label).show(ui).clicked() {
                action = Some(Action::Preset(id));
            }
        }
        cx.more_menu(ui);
        if Button::new(tl!("Cancel")).show(ui).clicked() {
            action = Some(Action::Cancel);
        }
        if Button::new(tl!("Done")).variant(Variant::PRIMARY).show(ui).clicked() {
            action = Some(Action::Done);
        }
        action
    });
    app.ui.panels.task_bar = place;
    match shown.inner.flatten() {
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
    if shown.output.hide {
        app.ui.panels.context_task_bar = false;
    }
    if open_box(app).is_some_and(|t| t.id == session) {
        let r = shown.output.rect;
        ctx.data_mut(|data| data.insert_temp(shown_id(), ShownRect { session, rect: [r.min.x, r.min.y, r.max.x, r.max.y] }));
    } else {
        ctx.data_mut(|data| data.remove::<ShownRect>(shown_id()));
    }
}
