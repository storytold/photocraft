//! The design timeline (engine `design.*`) in two views: a strip docked along the bottom of the
//! canvas (Window › Design Timeline) and a list tab in the History card. Each step shows its icon;
//! hovering shows its result and name, right-click suppresses, deletes, rolls the marker to it or
//! edits its parameters. In the strip the marker is a handle to drag; steps past it are dimmed.
//! Failed steps are red, steps placed elsewhere or baked steps gone stale are yellow, suppressed
//! steps are struck through. A bar above a step takes the colour label of the layer it acted on.
//! The views only read state: every action runs a `design.*` command, so the control channel and
//! MCP drive it the same way.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use photocraft_doc::Document;
use photocraft_doc::design::{DesignStep, DesignTimeline, StepState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;

pub const ID: &str = "window.panel.designTimeline";

/// The key Edit Parameters leaves in the step's dialog: `{step, restore}` (the marker to return to).
pub const EDIT: &str = "__designStep";

const STRIP_H: f32 = 44.0;
const CELL: f32 = 30.0;
/// Longest side of a step's preview texture; shown at up to half that in points.
const THUMB: u32 = 512;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DesignTimelineUi {
    pub open: bool,
}

/// The Window menu check mark for the panel.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == ID).then_some(app.ui.design_timeline.open)
}

/// Whether `id` is the panel's menu command.
pub fn handles(id: &str) -> bool {
    id == ID
}

/// Window › Design Timeline: show or hide the panel.
pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    (id == ID).then(|| {
        app.ui.design_timeline.open = !app.ui.design_timeline.open;
        Ok(json!({ "open": app.ui.design_timeline.open }))
    })
}

/// Hover text for a step's state, and whether it is a failure (red) or a warning (yellow).
fn describe(state: &StepState) -> (Option<String>, Option<bool>) {
    match state {
        StepState::Ok => (None, None),
        StepState::Error { message } => (Some(message.clone()), Some(true)),
        StepState::MissingReference { .. } => (Some(tl!("A layer this step acted on was removed.").into()), Some(true)),
        StepState::Retargeted { .. } => (Some(tl!("This step would now change a layer it didn't change before, so it was skipped.").into()), Some(true)),
        StepState::PlacedElsewhere => (Some(tl!("The layer this step placed its new layers beside was removed; they went elsewhere.").into()), Some(false)),
        StepState::Stale => (Some(tl!("An earlier step changed, so this step's stored result may be out of date.").into()), Some(false)),
    }
}

enum Act {
    Run(&'static str, Value),
    Edit(usize),
    Close,
}

/// Whether `command` has a dialog Edit Parameters can open.
fn has_dialog(command: &str) -> bool {
    crate::filter_dialog::has_dialog(command) || crate::adjust_dialog::has_dialog(command)
}

/// The colour label of the layer a step created or changed, in the current document.
fn swatch(doc: &Document, st: &DesignStep, t: &Tokens) -> Option<Color32> {
    let id = st.created.first().or(st.changed.first())?;
    t.layer_label_colors(doc.layer(*id)?.label).map(|(c, _)| c)
}

/// A step icon's colour: red for a failure, yellow for a warning, faint past the marker.
fn tint(st: &DesignStep, dimmed: bool, t: &Tokens) -> Color32 {
    match describe(&st.state).1 {
        Some(true) => t.danger,
        Some(false) => t.warning,
        None if dimmed => t.text_faint,
        None => t.icon,
    }
}

/// Step previews by (step id, checkpoint): `None` while one renders.
type Thumbs = Arc<Mutex<HashMap<(u64, usize), Option<egui::TextureHandle>>>>;

/// The preview of a step's result, rendered from its checkpoint off the UI thread on first
/// request. Steps past the checkpoint memory budget have none.
fn thumb(ctx: &egui::Context, tl: &DesignTimeline, st: &DesignStep) -> Option<egui::TextureHandle> {
    let doc = st.checkpoint.0.clone()?;
    let key = (st.step, Arc::as_ptr(&doc) as usize);
    let thumbs = ctx.data_mut(|d| d.get_temp_mut_or_default::<Thumbs>(egui::Id::new("design_timeline_thumbs")).clone());
    let mut map = thumbs.lock().ok()?;
    if let Some(t) = map.get(&key) {
        return t.clone();
    }
    // Replays give steps new checkpoints: drop the previews of the old ones.
    map.retain(|k, _| tl.steps.iter().any(|s| s.step == k.0 && s.checkpoint.0.as_ref().is_some_and(|d| Arc::as_ptr(d) as usize == k.1)));
    map.insert(key, None);
    drop(map);
    let ctx = ctx.clone();
    let work = move || {
        let img = photocraft_compose::thumbnail(&doc, THUMB);
        let img = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
        let tex = ctx.load_texture(format!("design-step-{}", key.0), img, egui::TextureOptions::LINEAR);
        if let Ok(mut map) = thumbs.lock() {
            map.insert(key, Some(tex));
        }
        ctx.request_repaint();
    };
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::thread::Builder::new().name("design-step-thumb".into()).spawn(work);
    #[cfg(target_arch = "wasm32")]
    work();
    None
}

/// The hover call-out: the step's result, name and state.
fn callout(ui: &mut egui::Ui, tl: &DesignTimeline, st: &DesignStep) {
    let t = Tokens::get(ui.ctx());
    if let Some(tex) = thumb(ui.ctx(), tl, st) {
        let half = tex.size_vec2() * 0.5;
        ui.add(egui::Image::new(&tex).fit_to_exact_size(half));
    }
    ui.label(RichText::new(&st.label).strong().color(t.text));
    let (why, failure) = describe(&st.state);
    if let Some(why) = why {
        ui.label(RichText::new(why).color(if failure == Some(true) { t.danger } else { t.warning }));
    }
}

/// Suppress, Delete, Roll to Here and Edit Parameters for step `i`.
fn step_menu(ui: &mut egui::Ui, i: usize, st: &DesignStep, act: &mut Option<Act>) {
    let (cmd, label) = if st.suppressed { ("design.unsuppress", tl!("Unsuppress")) } else { ("design.suppress", tl!("Suppress")) };
    if ui.button(label).clicked() {
        *act = Some(Act::Run(cmd, json!({ "step": st.step })));
    }
    if ui.button(tl!("Delete")).clicked() {
        *act = Some(Act::Run("design.delete", json!({ "step": st.step })));
    }
    if ui.button(tl!("Roll to Here")).clicked() {
        *act = Some(Act::Run("design.rollTo", json!({ "at": i + 1 })));
    }
    let edit = ui.add_enabled(has_dialog(&st.command), egui::Button::new(tl!("Edit Parameters…")));
    if edit.on_disabled_hover_text(tl!("This step has no dialog to edit.")).clicked() {
        *act = Some(Act::Edit(i));
    }
}

/// Move the marker to `at` of `n` steps (the end as null).
fn roll(at: usize, n: usize) -> Act {
    Act::Run("design.rollTo", json!({ "at": if at >= n { Value::Null } else { json!(at) } }))
}

/// Window › Design Timeline: the strip along the bottom of the canvas.
pub fn strip(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !app.ui.design_timeline.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let doc = app.session.active().map(|d| d.doc.clone());
    let mut act = None;
    egui::Panel::bottom("design_timeline_strip").exact_size(STRIP_H).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 0))).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 22.0, false, tl!("Close")).clicked() {
                        act = Some(Act::Close);
                    }
                    let tl = doc.as_ref().and_then(|d| d.design_timeline.clone());
                    if tl.is_some() && icons::button(ui, "trash", 22.0, false, tl!("Stop Design Timeline")).clicked() {
                        act = Some(Act::Run("design.stop", json!({})));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| match (&doc, &tl) {
                        (None, _) => {
                            ui.weak(tl!("Open a document to use the design timeline."));
                        }
                        (Some(_), None) => {
                            if ui.button(tl!("Start Design Timeline")).clicked() {
                                act = Some(Act::Run("design.start", json!({})));
                            }
                            ui.weak(tl!("Records each edit from now on, so you can suppress, delete or edit any of them later."));
                        }
                        (Some(doc), Some(tl)) => strip_body(ui, doc, tl, &t, &mut act),
                    });
                });
            });
        },
    );
    finish(app, act);
}

/// The strip's roll buttons and step icons.
fn strip_body(ui: &mut egui::Ui, doc: &Document, tl: &Arc<DesignTimeline>, t: &Tokens, act: &mut Option<Act>) {
    let n = tl.steps.len();
    let at = tl.in_effect();
    for (icon, tip, to) in [
        ("chevrons-left", tl!("Roll to Start"), 0),
        ("chevron-left", tl!("Step Backward"), at.saturating_sub(1)),
        ("chevron-right", tl!("Step Forward"), (at + 1).min(n)),
        ("chevrons-right", tl!("Roll to End"), n),
    ] {
        if ui.add_enabled_ui(to != at, |ui| icons::button(ui, icon, 24.0, false, tip)).inner.clicked() {
            *act = Some(roll(to, n));
        }
    }
    ui.separator();
    if n == 0 {
        ui.weak(tl!("No steps yet: edits to this document appear here."));
        return;
    }
    egui::ScrollArea::horizontal().show(ui, |ui| {
        let (row, _) = ui.allocate_exact_size(vec2(n as f32 * CELL + 12.0, STRIP_H - 6.0), Sense::hover());
        let left = row.left() + 6.0;
        let x = |k: usize| left + k as f32 * CELL;
        // The marker handle: drag it between steps; the dimming follows it until it's dropped.
        let handle = Rect::from_center_size(pos2(x(at), row.center().y), vec2(12.0, row.height()));
        let h = ui.interact(handle, ui.id().with("marker"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let target = h.interact_pointer_pos().map(|p| (((p.x - left) / CELL).round().max(0.0) as usize).min(n));
        let shown = if h.dragged() { target.unwrap_or(at) } else { at };
        if h.drag_stopped()
            && let Some(to) = target.filter(|to| *to != at)
        {
            *act = Some(roll(to, n));
        }
        for (i, st) in tl.steps.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x(i) + 2.0, row.top() + 4.0), vec2(CELL - 4.0, CELL));
            let r = ui.interact(cell, ui.id().with(("step", st.step)), Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &st.label));
            if r.hovered() {
                ui.painter().rect_filled(cell, t.radius_sm, t.hover);
            }
            if let Some(c) = swatch(doc, st, t) {
                ui.painter().rect_filled(Rect::from_min_size(cell.left_top() - vec2(0.0, 3.0), vec2(cell.width(), 3.0)), 0.0, c);
            }
            let tint = tint(st, i >= shown, t);
            icons::paint(ui, cell, icons::step_icon(&st.label), 16.0, tint);
            if st.suppressed {
                ui.painter().line_segment([cell.left_bottom() + vec2(4.0, -4.0), cell.right_top() + vec2(-4.0, 4.0)], Stroke::new(1.5, tint));
            }
            if r.clicked() {
                *act = Some(roll(i + 1, n));
            }
            r.on_hover_ui(|ui| callout(ui, tl, st)).context_menu(|ui| step_menu(ui, i, st, act));
        }
        let mx = if h.dragged() { x(shown) } else { x(at) };
        let accent = if h.hovered() || h.dragged() { t.accent } else { t.text };
        ui.painter().line_segment([pos2(mx, row.top() + 6.0), pos2(mx, row.bottom())], Stroke::new(2.0, accent));
        let top = row.top() + 2.0;
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(mx - 5.0, top), pos2(mx + 5.0, top), pos2(mx, top + 6.0)], accent, Stroke::NONE));
    });
}

/// The Design Timeline tab of the History card: one row per step, the marker as a rule.
pub fn list(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        ui.weak(tl!("Open a document to use the design timeline."));
        return;
    };
    let mut act = None;
    match &doc.design_timeline {
        None => {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                if crate::widgets::primary_button(ui, tl!("Start Design Timeline"), 220.0).clicked() {
                    act = Some(Act::Run("design.start", json!({})));
                }
                ui.add_space(4.0);
                ui.weak(tl!("Records each edit from now on, so you can suppress, delete or edit any of them later."));
            });
        }
        Some(tl) if tl.steps.is_empty() => {
            ui.weak(tl!("No steps yet: edits to this document appear here."));
        }
        Some(tl) => {
            let at = tl.in_effect();
            for (i, st) in tl.steps.iter().enumerate() {
                let (row, r) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i + 1 == at, &st.label));
                if i + 1 == at {
                    ui.painter().rect_filled(row, 0.0, if t.pro { t.row_selected } else { t.accent_soft });
                } else if r.hovered() {
                    ui.painter().rect_filled(row, 0.0, t.hover.gamma_multiply(0.5));
                }
                if let Some(c) = swatch(&doc, st, &t) {
                    ui.painter().rect_filled(Rect::from_min_size(row.left_top() + vec2(4.0, 4.0), vec2(3.0, row.height() - 8.0)), 0.0, c);
                }
                let tint = tint(st, i >= at, &t);
                icons::paint(ui, Rect::from_center_size(pos2(row.left() + 20.0, row.center().y), vec2(16.0, 16.0)), icons::step_icon(&st.label), 14.0, tint);
                let mut text = RichText::new(&st.label).color(if tint == t.icon { t.text } else { tint });
                if st.suppressed {
                    text = text.strikethrough();
                }
                let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), row.width() - 40.0, egui::TextStyle::Body);
                ui.painter().galley(pos2(row.left() + 34.0, row.center().y - galley.size().y / 2.0), galley, t.text);
                if i + 1 == at && at < tl.steps.len() {
                    ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(2.0, t.accent));
                }
                if r.clicked() {
                    act = Some(roll(i + 1, tl.steps.len()));
                }
                r.on_hover_ui(|ui| callout(ui, tl, st)).context_menu(|ui| step_menu(ui, i, st, &mut act));
            }
        }
    }
    finish(app, act);
}

/// Carry out what the panel's user asked for this frame.
fn finish(app: &mut PhotocraftApp, act: Option<Act>) {
    let r = match act {
        None => return,
        Some(Act::Close) => {
            app.ui.design_timeline.open = false;
            return;
        }
        Some(Act::Run(cmd, p)) => app.run(cmd, p).map(drop),
        Some(Act::Edit(i)) => edit(app, i),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

/// Edit Parameters: roll back to just before step `i` (so the dialog previews on what the step
/// saw), open its dialog with the recorded values, and remember where the marker was.
fn edit(app: &mut PhotocraftApp, i: usize) -> Result<(), String> {
    let tl = app.session.active().and_then(|d| d.doc.design_timeline.clone()).ok_or("the design timeline is off")?;
    let st = tl.steps.get(i).ok_or("no such step")?;
    let size = app.session.active().map(|d| [d.doc.size.width, d.doc.size.height]);
    // Not a background job: the dialog must open on the rolled-back document. Usually a
    // checkpoint restore; after a reload (no checkpoints) it replays up to step `i` here.
    app.session.execute("design.rollTo", json!({ "at": i })).map_err(|e| e.to_string())?;
    let dialog = crate::dialogs::open_command_dialog(app, &st.command, &st.label);
    if let Some(d) = app.ui.dialog_mut(dialog) {
        if let Some(o) = st.params.as_object() {
            d.fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        d.fields.insert(EDIT.into(), json!({ "step": st.step, "restore": tl.marker }));
    }
    app.filter_preview_hold = size.map(|size| (dialog, size));
    Ok(())
}

/// A confirmed Edit Parameters dialog: replay the step with the new values and put the marker back,
/// in one replay. `None` for any other dialog.
pub fn confirm_edit(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Option<Result<Value, String>> {
    let e = fields.get(EDIT)?;
    let params = crate::filter_dialog::params_of(fields);
    let r = app.run("design.editStep", json!({ "step": e["step"], "params": params, "restore": e["restore"] }));
    // While the replay runs as a job, the edited preview stays on screen (as for a filter's OK).
    let running = app.session.active().is_some_and(|st| app.session.job_on(st.doc.id).is_some());
    match app.filter_preview.as_mut() {
        Some(p) if r.is_ok() && running => p.committing = true,
        _ => app.filter_preview = None,
    }
    Some(r)
}

/// A cancelled Edit Parameters dialog: the marker goes back where it was.
pub fn cancel_edit(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    if let Some(e) = fields.get(EDIT)
        && let Err(e) = app.run("design.rollTo", json!({ "at": e["restore"] }))
    {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// An app with a 64×48 document and the panel open.
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.design_timeline.open = true;
        app
    }

    /// A harness drawing the strip, with the app's fonts in place.
    fn harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        // The app's own font families apply from the next frame: show the strip once they're in.
        app.ui.design_timeline.open = false;
        let mut h = Harness::builder().with_size(vec2(1000.0, 300.0)).build_ui_state(|ui, app| strip(app, ui), app);
        crate::theme::install_fonts(&h.ctx);
        h.run_steps(1);
        h.state_mut().ui.design_timeline.open = true;
        h.run_steps(2);
        h
    }

    /// The active document's timeline.
    fn timeline(app: &PhotocraftApp) -> Option<Arc<DesignTimeline>> {
        app.session.active().and_then(|d| d.doc.design_timeline.clone())
    }

    #[test]
    fn start_record_roll_back_and_forward() {
        let mut h = harness(app());
        h.get_by_label("Start Design Timeline").click();
        h.run_steps(2);
        assert!(timeline(h.state()).is_some());
        h.state_mut().run("layer.new.layer", json!({"name": "A"})).unwrap();
        h.state_mut().run("layer.new.layer", json!({"name": "B"})).unwrap();
        h.run_steps(2);
        let labels: Vec<String> = timeline(h.state()).unwrap().steps.iter().map(|s| s.label.clone()).collect();
        assert_eq!(labels.len(), 2);
        // Clicking the first step rolls the marker to just after it.
        h.get_all_by_label(&labels[0]).next().unwrap().click();
        h.run_steps(2);
        assert_eq!(timeline(h.state()).unwrap().in_effect(), 1);
        h.get_by_label("Roll to End").click();
        h.run_steps(2);
        assert_eq!(timeline(h.state()).unwrap().in_effect(), 2);
    }

    #[test]
    fn a_failed_step_shows_why() {
        let mut a = app();
        a.run("design.start", json!({})).unwrap();
        a.run("layer.new.layer", json!({"name": "A"})).unwrap();
        a.run("layer.setProps", json!({"opacity": 0.5})).unwrap();
        let first = timeline(&a).unwrap().steps[0].step;
        a.run("design.suppress", json!({"step": first})).unwrap();
        let tl = timeline(&a).unwrap();
        assert!(tl.steps[1].state.is_failure());
        let (why, failure) = describe(&tl.steps[1].state);
        assert_eq!(failure, Some(true));
        assert!(why.is_some());
    }

    #[test]
    fn a_step_preview_renders_from_its_checkpoint() {
        let mut a = app();
        a.run("design.start", json!({})).unwrap();
        a.run("layer.new.layer", json!({"name": "A"})).unwrap();
        let tl = timeline(&a).unwrap();
        let ctx = egui::Context::default();
        assert!(thumb(&ctx, &tl, &tl.steps[0]).is_none(), "the first request starts the render");
        for _ in 0..500 {
            if let Some(tex) = thumb(&ctx, &tl, &tl.steps[0]) {
                assert_eq!(tex.size(), [64, 48], "small documents preview at their own size");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the preview never rendered");
    }

    #[test]
    fn edit_parameters_replays_the_step_and_restores_the_marker() {
        let mut a = app();
        a.run("design.start", json!({})).unwrap();
        a.run("layer.new.layer", json!({"name": "A"})).unwrap();
        a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
        a.run("layer.new.layer", json!({"name": "B"})).unwrap();
        assert!(has_dialog("filter.noise.addNoise"));
        edit(&mut a, 1).unwrap();
        assert_eq!(timeline(&a).unwrap().in_effect(), 1, "the dialog previews on what the step saw");
        let d = a.ui.dialogs.last().unwrap().id;
        a.ui.dialog_mut(d).unwrap().fields.insert("amount".into(), json!(40));
        crate::dialogs::confirm(&mut a, d).unwrap();
        let tl = timeline(&a).unwrap();
        assert_eq!(tl.steps[1].params["amount"], json!(40));
        assert_eq!(tl.in_effect(), 3, "the marker goes back to the end");
        // Cancel puts the marker back too.
        edit(&mut a, 1).unwrap();
        let d = a.ui.dialogs.last().unwrap().id;
        crate::dialogs::cancel(&mut a, d).unwrap();
        assert_eq!(timeline(&a).unwrap().in_effect(), 3);
    }

    #[test]
    fn edit_parameters_holds_the_canvas_until_its_preview_lands() {
        let mut a = app();
        a.run("design.start", json!({})).unwrap();
        a.run("layer.new.layer", json!({"name": "A"})).unwrap();
        a.run("edit.fillForeground", json!({})).unwrap();
        a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
        // As in the desktop app: previews compute on a worker, never on the UI thread.
        a.background_jobs = true;
        edit(&mut a, 2).unwrap();
        let doc = a.session.active().unwrap().doc.clone();
        // Showing the rolled-back document, without the step, while the worker runs is the flicker.
        let shown = crate::canvas::ensure_filter_preview(&mut a, 0, &egui::Context::default());
        assert_eq!(shown, Some((1, doc.id.0, [doc.size.width, doc.size.height])), "the live texture, as it was");
        assert!(a.filter_preview.is_none(), "nothing computed inline");
    }

    #[test]
    fn a_background_edit_keeps_its_preview_up_until_the_replay_lands() {
        let mut a = app();
        a.run("design.start", json!({})).unwrap();
        a.run("layer.new.layer", json!({"name": "A"})).unwrap();
        a.run("edit.fillForeground", json!({})).unwrap();
        a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
        a.run("layer.new.layer", json!({"name": "B"})).unwrap();
        edit(&mut a, 2).unwrap();
        let d = a.ui.dialogs.last().unwrap().id;
        a.ui.dialog_mut(d).unwrap().fields.insert("amount".into(), json!(40));
        assert!(crate::canvas::ensure_filter_preview(&mut a, 0, &egui::Context::default()).is_some());
        a.background_jobs = true;
        crate::dialogs::confirm(&mut a, d).unwrap();
        let doc = a.session.active().unwrap().doc.id;
        assert!(a.session.job_on(doc).is_some(), "the replay runs as a job");
        assert!(a.filter_preview.as_ref().is_some_and(|p| p.committing), "the edited preview stays up meanwhile");
        let job = a.session.job_on(doc).unwrap().id;
        a.session.wait_job(job).unwrap();
        let tl = timeline(&a).unwrap();
        assert_eq!((tl.steps[2].params["amount"].clone(), tl.in_effect()), (json!(40), 4));
    }
}
