//! The frame's contextual task bar: Import or Replace Image and Fit Content to Frame. It shares the crop bar's
//! place, pin and Hide setting.

use super::{Target, report, select, selected, visible};
use crate::PhotocraftApp;
use crate::state::{TaskBarContext, Tool};
use craft::behavior::kurbo::Point;
use craft::ui::buttons::Button;
use craft::ui::canvas_controls::TaskBar;
use craft::ui::contract::{Bind, Show as _};
use serde_json::json;

/// Shows the bar under the selected frame inside `at` (the canvas rect), while the Frame tool is active and no
/// frame drag, transform or dialog is running.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp, at: egui::Rect) {
    if app.ui.tool != Tool::Frame || crate::canvas::tools::in_gesture(app) || app.ui.transform.is_some() || app.dialog_open() || !app.ui.panels.context_task_bar
    {
        return;
    }
    let Some(l) = selected(app) else { return };
    if !visible(app, l.id) {
        return;
    }
    let (id, empty) = (l.id, l.children().is_none_or(|c| c.is_empty()));
    let Some(f) = &l.frame else { return };
    let b = f.bounds();
    let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    let anchor = [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];

    if let Some(doc) = app.session.active().map(|d| d.doc.id.0) {
        app.ui.enter_task_bar_context(TaskBarContext::Frame { doc, layer: id.0 });
    }
    let mut import = false;
    let mut fit = false;
    let shown = TaskBar::new(tl!("Frame"), &anchor, Bind::bound(&mut app.ui.panels.task_bar)).show(ui, at, |ui, _| {
        import |= Button::new(if empty { tl!("Import Image…") } else { tl!("Replace Image…") }).show(ui).clicks > 0;
        if !empty {
            fit |= Button::new(tl!("Fit Content to Frame")).show(ui).clicks > 0;
        }
    });
    let hide = shown.output.hide;
    if hide {
        app.ui.panels.context_task_bar = false;
    }
    if import {
        select(app, id, Target::Frame);
        let ctx = ui.ctx().clone();
        let r = crate::menus::invoke(app, &ctx, "file.placeEmbedded", json!({}));
        report(app, r);
    }
    if fit {
        let r = app.run("layer.frame.fit", json!({"layer":id.0,"mode":"contentToFrame"}));
        report(app, r);
    }
}
