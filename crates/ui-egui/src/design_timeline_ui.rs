//! The design timeline (engine `design.*`) in two panels: a strip below the canvas (Window ›
//! Design Timeline) and a list tab in the History group. Each step shows its icon; hovering shows
//! its result and name, right-click suppresses, deletes, rolls the marker to it or edits its
//! parameters. In the strip the marker is a handle to drag; steps past it are dimmed. Failed steps
//! are red, steps placed elsewhere or baked steps gone stale are yellow, suppressed steps are
//! struck through, and each step's accessible name says so. A bar above a step takes the colour
//! label of the layer it acted on. The panels only read state: every action runs a `design.*`
//! command, so the control channel and MCP drive it the same way.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use photocraft_doc::Document;
use photocraft_doc::design::{DesignStep, DesignTimeline, StepState};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;

/// Longest side of a step's preview texture; shown at up to half that in points.
const THUMB: u32 = 512;

fn label(text: &str) -> Label {
    Label::from(text.to_string())
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

/// A step's accessible name: its label, then whether it is suppressed and what went wrong.
fn spoken(st: &DesignStep) -> String {
    let mut name = st.label.clone();
    if st.suppressed {
        name.push_str(&format!(" ({})", tl!("suppressed")));
    }
    if let Some(why) = describe(&st.state).0 {
        name.push_str(&format!(": {why}"));
    }
    name
}

enum Act {
    Run(&'static str, Value),
    Edit(usize),
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

/// A step's icon in `color`.
fn glyph(st: &DesignStep, color: Color32) -> IconSpec<'static> {
    IconSpec::svg(IconId::new(icons::step_icon(&st.label))).data_color(DataColor::new(color))
}

/// Step previews by (step id, checkpoint address), with the checkpoint they show: `None` while
/// one renders.
type Thumbs = Arc<Mutex<HashMap<(u64, usize), (std::sync::Weak<Document>, Option<egui::TextureHandle>)>>>;

/// The preview of a step's result, rendered from its checkpoint off the UI thread on first
/// request. Steps past the checkpoint memory budget have none.
fn thumb(ctx: &egui::Context, tl: &DesignTimeline, st: &DesignStep) -> Option<egui::TextureHandle> {
    let doc = st.checkpoint.0.clone()?;
    let key = (st.step, Arc::as_ptr(&doc) as usize);
    let thumbs = ctx.data_mut(|d| d.get_temp_mut_or_default::<Thumbs>(egui::Id::unique("design_timeline_thumbs")).clone());
    let mut map = thumbs.lock().ok()?;
    // The address alone could be a freed checkpoint's, reused.
    if let Some((shown, t)) = map.get(&key)
        && shown.upgrade().is_some_and(|d| Arc::ptr_eq(&d, &doc))
    {
        return t.clone();
    }
    // Replays give steps new checkpoints: drop the previews of the old ones.
    map.retain(|k, _| tl.steps.iter().any(|s| s.step == k.0 && s.checkpoint.0.as_ref().is_some_and(|d| Arc::as_ptr(d) as usize == k.1)));
    let shown = Arc::downgrade(&doc);
    map.insert(key, (shown.clone(), None));
    drop(map);
    let ctx = ctx.clone();
    let work = move || {
        let img = photocraft_compose::thumbnail(&doc, THUMB);
        let img = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
        let tex = ctx.load_texture(format!("design-step-{}", key.0), img, egui::TextureOptions::LINEAR);
        if let Ok(mut map) = thumbs.lock() {
            map.insert(key, (shown, Some(tex)));
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
    let (cmd, text) = if st.suppressed { ("design.unsuppress", tl!("Unsuppress")) } else { ("design.suppress", tl!("Suppress")) };
    if ui.button(text).clicked() {
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

/// What the panels show without steps to list: no document, or the timeline off (with its Start
/// button). `None` when there is a timeline.
fn empty_state(ui: &mut egui::Ui, doc: Option<&Document>, act: &mut Option<Act>) -> Option<()> {
    match doc {
        None => {
            ui.weak(tl!("Open a document to use the design timeline."));
            Some(())
        }
        Some(doc) if doc.design_timeline.is_none() => {
            if Button::new(label(tl!("Start Design Timeline"))).show(ui).clicked() {
                *act = Some(Act::Run("design.start", json!({})));
            }
            ui.weak(tl!("Records each edit from now on, so you can suppress, delete or edit any of them later."));
            Some(())
        }
        Some(_) => None,
    }
}

/// Window › Design Timeline: the strip below the canvas.
pub fn strip(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let doc = app.session.active().map(|d| d.doc.clone());
    let mut act = None;
    ui.horizontal_centered(|ui| {
        if empty_state(ui, doc.as_deref(), &mut act).is_some() {
            return;
        }
        let Some((doc, tl)) = doc.as_ref().and_then(|d| Some((d, d.design_timeline.clone()?))) else { return };
        if IconButton::new(IconId::new("trash")).label(label(tl!("Stop Design Timeline"))).tooltip(label(tl!("Stop Design Timeline"))).show(ui).clicked() {
            act = Some(Act::Run("design.stop", json!({})));
        }
        strip_body(ui, doc, &tl, &mut act);
    });
    finish(app, act);
}

/// The strip's roll buttons and step icons.
fn strip_body(ui: &mut egui::Ui, doc: &Document, tl: &Arc<DesignTimeline>, act: &mut Option<Act>) {
    let t = Tokens::get(ui.ctx());
    let n = tl.steps.len();
    let at = tl.in_effect();
    for (icon, tip, to) in [
        ("chevrons_left", tl!("Roll to Start"), 0),
        ("chevron_left", tl!("Step Backward"), at.saturating_sub(1)),
        ("chevron_right", tl!("Step Forward"), (at + 1).min(n)),
        ("chevrons_right", tl!("Roll to End"), n),
    ] {
        if IconButton::new(IconId::new(icon)).label(label(tip)).tooltip(label(tip)).enabled(to != at).show(ui).clicked() {
            *act = Some(roll(to, n));
        }
    }
    ui.separator();
    if n == 0 {
        ui.weak(tl!("No steps yet: edits to this document appear here."));
        return;
    }
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            // Where each step's button landed: the marker sits on the boundaries between them.
            let mut cells = Vec::with_capacity(n);
            for (i, st) in tl.steps.iter().enumerate() {
                let r = IconButton::new(glyph(st, tint(st, i >= at, &t))).label(label(&spoken(st))).show(ui).response;
                if let Some(c) = swatch(doc, st, &t) {
                    ui.painter().rect_filled(Rect::from_min_size(r.rect.left_top() - vec2(0.0, 3.0), vec2(r.rect.width(), 3.0)), 0.0, c);
                }
                if st.suppressed {
                    let tint = tint(st, i >= at, &t);
                    ui.painter().line_segment([r.rect.left_bottom() + vec2(4.0, -4.0), r.rect.right_top() + vec2(-4.0, 4.0)], Stroke::new(1.5, tint));
                }
                if r.clicked() {
                    *act = Some(roll(i + 1, n));
                }
                cells.push(r.rect);
                r.on_hover_ui(|ui| callout(ui, tl, st)).context_menu(|ui| step_menu(ui, i, st, act));
            }
            marker(ui, &cells, at, n, &t, act);
        });
    });
}

/// The marker between the steps in effect and the rest: a handle to drag to another boundary.
fn marker(ui: &mut egui::Ui, cells: &[Rect], at: usize, n: usize, t: &Tokens, act: &mut Option<Act>) {
    let Some(first) = cells.first() else { return };
    let boundary = |k: usize| if k == 0 { first.left() } else { cells[k - 1].right() };
    let (top, bottom) = (first.top() - 6.0, first.bottom());
    let handle = Rect::from_x_y_ranges(boundary(at) - 6.0..=boundary(at) + 6.0, top..=bottom);
    let h = ui.interact(handle, ui.scope_id().with("design_marker"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let target = h.interact_pointer_pos().map(|p| (0..=n).min_by(|a, b| (boundary(*a) - p.x).abs().total_cmp(&(boundary(*b) - p.x).abs())).unwrap_or(at));
    let shown = if h.dragged() { target.unwrap_or(at) } else { at };
    if h.drag_stopped()
        && let Some(to) = target.filter(|to| *to != at)
    {
        *act = Some(roll(to, n));
    }
    let x = boundary(shown);
    let color = if h.hovered() || h.dragged() { t.accent } else { t.text };
    ui.painter().line_segment([pos2(x, top + 6.0), pos2(x, bottom)], Stroke::new(2.0, color));
    ui.painter().add(egui::Shape::convex_polygon(vec![pos2(x - 5.0, top), pos2(x + 5.0, top), pos2(x, top + 6.0)], color, Stroke::NONE));
}

/// The Design Timeline tab of the History group: one row per step, the marker as a rule.
pub fn list(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let doc = app.session.active().map(|d| d.doc.clone());
    let mut act = None;
    if empty_state(ui, doc.as_deref(), &mut act).is_none()
        && let Some((doc, tl)) = doc.as_ref().and_then(|d| Some((d, d.design_timeline.clone()?)))
    {
        if tl.steps.is_empty() {
            ui.weak(tl!("No steps yet: edits to this document appear here."));
        }
        let at = tl.in_effect();
        for (i, st) in tl.steps.iter().enumerate() {
            let row = ui.horizontal(|ui| {
                ui.set_min_size(vec2(ui.available_width(), 24.0));
                if let Some(c) = swatch(doc, st, &t) {
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
                    ui.painter().rect_filled(bar, 0.0, c);
                }
                let tint = tint(st, i >= at, &t);
                let r = IconButton::new(glyph(st, tint)).label(label(&spoken(st))).show(ui).response;
                let mut text = RichText::new(&st.label).color(if tint == t.icon { t.text } else { tint });
                if st.suppressed {
                    text = text.strikethrough();
                }
                ui.add(egui::Label::new(text).truncate().selectable(false));
                r
            });
            let rect = row.response.rect;
            let r = row.inner | ui.interact(rect, ui.scope_id().with(("design_step", st.step)), Sense::click());
            if i + 1 == at {
                ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.accent_border), egui::StrokeKind::Inside);
                if at < tl.steps.len() {
                    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(2.0, t.accent));
                }
            }
            if r.clicked() {
                act = Some(roll(i + 1, tl.steps.len()));
            }
            r.on_hover_ui(|ui| callout(ui, &tl, st)).context_menu(|ui| step_menu(ui, i, st, &mut act));
        }
    }
    finish(app, act);
}

/// Carry out what the panel's user asked for this frame.
fn finish(app: &mut PhotocraftApp, act: Option<Act>) {
    let r = match act {
        None => return,
        Some(Act::Run(cmd, p)) => app.run(cmd, p).map(drop),
        Some(Act::Edit(i)) => edit(app, i),
    };
    if let Err(e) = r {
        crate::notices::error(app, e);
    }
}

/// Edit Parameters: roll back to just before step `i` (so the dialog previews on what the step
/// saw), open its dialog with the recorded values, and remember where the marker was.
fn edit(app: &mut PhotocraftApp, i: usize) -> Result<(), String> {
    let tl = app.session.active().and_then(|d| d.doc.design_timeline.clone()).ok_or("the design timeline is off")?;
    let st = tl.steps.get(i).ok_or("no such step")?;
    // Not a background job: the dialog must open on the rolled-back document. Usually a
    // checkpoint restore; after a reload (no checkpoints) it replays up to step `i` here.
    app.session.execute("design.rollTo", json!({ "at": i })).map_err(|e| e.to_string())?;
    // The layers the step acted on, so the dialog previews where OK replays it.
    if let Some(d) = app.session.active_mut() {
        let active = st.active.filter(|a| d.doc.layer(*a).is_some());
        d.set_active_layer(active);
        *d.selection_mut() = st.selected.iter().copied().filter(|id| d.doc.layer(*id).is_some()).collect();
        d.layer_anchor = active;
    }
    let doc = app.session.active().map(|d| d.doc.id.0);
    let opened = if crate::adjust_dialog::has_dialog(&st.command) {
        crate::adjust_dialog::open_with_params_checked(app, &st.command, &st.params)
            .map_err(|why| why.unwrap_or_else(|| tl!("This step has no dialog to edit.").into()))
    } else {
        crate::filter_dialog::open_smart_edit(app, &st.command, &st.params.as_object().cloned().unwrap_or_default(), Value::Null)
    };
    match opened {
        Ok(dialog) => {
            app.design_edit = Some((dialog, json!({ "step": st.step, "restore": tl.marker, "doc": doc })));
            Ok(())
        }
        Err(e) => {
            // No dialog to edit in: the marker goes back.
            let _ = app.session.execute("design.rollTo", json!({ "at": tl.marker }));
            Err(e)
        }
    }
}

/// Whether `dialog` is an Edit Parameters dialog for a document other than the active one: its
/// preview must not show on whichever document a background open made active.
pub(crate) fn edits_other(app: &PhotocraftApp, dialog: DialogId) -> bool {
    let active = app.session.active().map(|st| st.doc.id.0);
    app.design_edit.as_ref().is_some_and(|(id, e)| *id == dialog && e["doc"].as_u64() != active)
}

/// OK on an Edit Parameters dialog: replay the step with the new values and put the marker back,
/// in one replay. `None` for any other dialog.
pub(crate) fn confirm_edit(app: &mut PhotocraftApp, dialog: DialogId, params: Value) -> Option<Result<Value, String>> {
    let (_, e) = app.design_edit.take_if(|(id, _)| *id == dialog)?;
    let r = on_edited_document(app, &e, |app| {
        let r = app.run("design.editStep", json!({ "step": e["step"], "params": params, "restore": e["restore"] }));
        (r, app.session.active().is_some_and(|st| app.session.job_on(st.doc.id).is_some()))
    });
    let (r, running) = r.unwrap_or_else(|e| (Err(e), false));
    if r.is_err() {
        // Still an edit: OK again retries it, and a dialog that closes now puts the marker back (`poll`).
        app.design_edit = Some((dialog, e));
    } else if running && let Some(job) = r.as_ref().ok().and_then(|v| v["job"].as_u64()) {
        // A cancelled or failed replay puts the marker back (`on_job_event`), as Cancel would.
        app.jobs.edit_steps.push((photocraft_engine::jobs::JobId(job), e));
    }
    Some(r)
}

/// An Edit Parameters dialog that closed without OK (Cancel, or closed with its document): the
/// marker goes back where it was. Run once a frame.
pub(crate) fn poll(app: &mut PhotocraftApp) {
    let Some((_, e)) = app.design_edit.take_if(|(id, _)| app.framework.dialogs.get_dyn(*id).is_none()) else { return };
    if let Err(err) = on_edited_document(app, &e, |app| app.run("design.rollTo", json!({ "at": e["restore"] }))).and_then(|r| r) {
        crate::notices::error(app, err);
    }
}

/// An Edit Parameters replay job that ended without landing (cancelled or failed): the marker goes
/// back where it was, with the later steps' effects.
pub fn on_job_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) {
    let Some(i) = app.jobs.edit_steps.iter().position(|(job, _)| *job == e.id) else { return };
    let (_, edit) = app.jobs.edit_steps.remove(i);
    if matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_)) {
        return;
    }
    if let Err(err) = on_edited_document(app, &edit, |app| app.run("design.rollTo", json!({ "at": edit["restore"] }))).and_then(|r| r) {
        crate::notices::error(app, err);
    }
}

/// Run `f` on the document the Edit Parameters dialog `e` opened on (another may be active by
/// now: a background open activates its document), then reactivate the one that was active.
fn on_edited_document<T>(app: &mut PhotocraftApp, e: &Value, f: impl FnOnce(&mut PhotocraftApp) -> T) -> Result<T, String> {
    let index = |app: &PhotocraftApp, id: Option<u64>| app.session.documents().iter().position(|d| Some(d.doc.id.0) == id);
    let i = index(app, e["doc"].as_u64()).ok_or_else(|| photocraft_engine::EngineError::NoDocument.to_string())?;
    let was = app.session.active().map(|d| d.doc.id.0);
    app.session.set_active(i);
    let r = f(app);
    if let Some(back) = index(app, was) {
        app.session.set_active(back);
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
