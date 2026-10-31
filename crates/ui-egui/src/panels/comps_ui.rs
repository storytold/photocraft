//! Window › Layer Comps panel (a tab of the History card).
//!
//! Rows: the Last Document State, then each comp with its apply marker, name, warning triangle
//! (layers deleted since capture) and the three apply toggles (visibility, position,
//! appearance). Footer: previous / next comp, update, new, delete. Every action is a
//! `layerComp.*` engine command; the selection is `UiState::layer_comp_selected`.

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The comp footer and row actions act on: the panel selection if still valid, else the last
/// applied comp.
pub fn target(app: &PhotocraftApp) -> Option<u32> {
    let doc = &app.session.active()?.doc;
    app.ui.layer_comp_selected.filter(|id| doc.comp(*id).is_some()).or(doc.last_applied_comp.filter(|id| doc.comp(*id).is_some()))
}

/// The row's warning triangle: Clear Layer Comp Warning for that comp only (Photoshop keeps
/// Clear All Layer Comp Warnings as a separate command).
fn clear_warning_action(comp: u32) -> (&'static str, Value) {
    ("layerComp.updateWarnings", json!({"clear": true, "comp": comp}))
}

/// The comp whose name is being edited in place, kept in egui's temp memory under its row id.
#[derive(Clone)]
struct RenameComp {
    text: String,
    focused: bool,
}

/// The in-place name field of a comp. Enter, Tab or clicking elsewhere returns the new name to
/// commit; Escape cancels and keeps the old name.
fn rename_field(ui: &mut egui::Ui, id: egui::Id, rect: Rect) -> Option<String> {
    let mut r = ui.data(|d| d.get_temp::<RenameComp>(id))?;
    ui.data_mut(|d| d.remove::<RenameComp>(id));
    let font = egui::FontId::proportional(12.0);
    let mut layouter = crate::widgets::rtl_singleline_layouter(font.clone());
    let te = ui.put(rect, crate::widgets::rtl_singleline(&mut r.text, &mut layouter).id(id).font(font)).accessible_name(tl!("Name"));
    if !r.focused {
        te.request_focus();
        // Like Photoshop, the whole name starts selected so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        r.focused = true;
        ui.data_mut(|d| d.insert_temp(id, r));
        return None;
    }
    let (enter, tab, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        Some(r.text)
    } else {
        ui.data_mut(|d| d.insert_temp(id, r));
        None
    }
}

fn run(app: &mut PhotocraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[derive(Clone, Copy)]
struct DragComp {
    document: photocraft_doc::DocId,
    comp: u32,
}

fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let slot = target + usize::from(after);
    if from < slot { slot - 1 } else { slot }
}

/// The x anchors of a comp row's fixed elements: the apply marker, the name and the three apply
/// toggles in their logical order (eye, move, sparkles). Every anchor mirrors about the row's
/// centre between the directions, and the toggles pack from the reading end.
fn row_anchors(rect: Rect) -> (f32, f32, [f32; 3]) {
    (
        crate::direction::start_x(rect, 14.0),
        crate::direction::start_x(rect, 30.0),
        [crate::direction::end_x(rect, 58.0), crate::direction::end_x(rect, 36.0), crate::direction::end_x(rect, 14.0)],
    )
}

/// The row's apply-marker square: 12×12, centred on the row's reading-start anchor.
fn marker_rect(rect: Rect) -> Rect {
    let (x, _, _) = row_anchors(rect);
    Rect::from_center_size(pos2(x, rect.center().y), vec2(12.0, 12.0))
}

/// The row's apply marker: the stroked square at the reading start; a check inside when applied.
fn apply_marker(ui: &egui::Ui, ctx: &RowCtx<'_>, rect: Rect, checked: bool, check_tint: Color32) {
    let marker = marker_rect(rect);
    ui.painter().rect_stroke(marker, 2.0, Stroke::new(1.0, ctx.t.text_faint), egui::StrokeKind::Inside);
    if checked {
        crate::icons::paint(ui, marker, "check", 10.0, check_tint);
    }
}

/// The "Last Document State" row: its apply marker, label and separator.
fn last_state_row(ui: &mut egui::Ui, ctx: &RowCtx<'_>, applied: Option<u32>) -> Option<(&'static str, Value)> {
    let row_h = 26.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    if applied.is_none() {
        ui.painter().rect_filled(rect, 0.0, ctx.t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, ctx.t.hover.gamma_multiply(0.5));
    }
    apply_marker(ui, ctx, rect, applied.is_none(), ctx.t.icon);
    let (_, label_x, _) = row_anchors(rect);
    ui.painter().text(
        pos2(label_x, rect.center().y),
        crate::direction::start_align(),
        tl!("Last Document State"),
        egui::FontId::proportional(12.0),
        ctx.t.text_dim,
    );
    let mut action = None;
    if resp.clicked() && ctx.doc.last_document_state.is_some() && applied.is_some() {
        action = Some(("layerComp.restoreLastDocumentState", json!({})));
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, ctx.t.separator));
    action
}

/// The shared state of a comp row: the theme, the document and the reading direction.
struct RowCtx<'a> {
    t: &'a Tokens,
    doc: &'a photocraft_doc::Document,
    rtl: bool,
}

/// The three apply toggles, packed from the reading end of the row. Returns the cursor left for
/// the name, whether a toggle was hit and the command it asked for.
fn draw_toggles(
    ui: &mut egui::Ui,
    ctx: &RowCtx<'_>,
    rect: Rect,
    resp: &egui::Response,
    c: &photocraft_doc::LayerComp,
) -> (f32, bool, Option<(&'static str, Value)>) {
    let toggles = [
        ("eye", tl!("Visibility"), c.apply_visibility, "visibility"),
        ("move", tl!("Position"), c.apply_position, "position"),
        ("sparkles", tl!("Appearance (Layer Style)"), c.apply_appearance, "appearance"),
    ];
    let (_, _, toggle_xs) = row_anchors(rect);
    let mut toggle_hit = false;
    let mut action = None;
    for ((icon, tip, on, key), &x) in toggles.iter().zip(toggle_xs.iter()) {
        let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(20.0, 20.0));
        let hov = ui.rect_contains_pointer(r);
        if hov {
            ui.painter().rect_filled(r, 3.0, ctx.t.hover);
        }
        crate::icons::paint(ui, r, icon, 13.0, if *on { ctx.t.icon } else { ctx.t.text_faint.gamma_multiply(0.45) });
        if hov && resp.clicked() {
            toggle_hit = true;
            action = Some(("layerComp.setOptions", json!({"comp": c.id, *key: !on})));
        }
        if hov {
            let template = "Apply {option}: {state}";
            let state = if *on { "on" } else { "off" };
            resp.clone().on_hover_text(crate::i18n::fmt(tl!(template), &[("option", tip), ("state", tl!(state))]));
        }
    }
    // The cursor for the name: one step past the eye toggle, on the name's side of the cluster.
    let cursor = toggle_xs[0] + if ctx.rtl { 22.0 } else { -22.0 };
    (cursor, toggle_hit, action)
}

/// The comp name, the missing-layer warning and the in-place rename field. Returns the command
/// the name asked for, the rect the name occupies and whether the warning was hit.
fn draw_name(
    ui: &mut egui::Ui,
    ctx: &RowCtx<'_>,
    rect: Rect,
    toggles_x: f32,
    rename_id: egui::Id,
    resp: &egui::Response,
    c: &photocraft_doc::LayerComp,
) -> (Option<(&'static str, Value)>, Rect, bool) {
    let missing = c.missing_layers(ctx.doc).len();
    let (_, mut name_x, _) = row_anchors(rect);
    let mut action = None;
    let mut hit = false;
    if missing > 0 {
        let r = Rect::from_center_size(pos2(if ctx.rtl { name_x - 7.0 } else { name_x + 7.0 }, rect.center().y), vec2(14.0, 14.0));
        crate::icons::paint(ui, r, "triangle-alert", 12.0, ctx.t.warning);
        if ui.rect_contains_pointer(r) {
            resp.clone().on_hover_text(crate::i18n::trn(
                crate::i18n::current(),
                missing as u64,
                "{n} layer recorded by this comp was deleted. Update the comp, or clear the warning.",
                "{n} layers recorded by this comp were deleted. Update the comp, or clear the warning.",
            ));
            if resp.clicked() {
                hit = true;
                action = Some(clear_warning_action(c.id));
            }
        }
        name_x += if ctx.rtl { -18.0 } else { 18.0 };
    }
    let galley_w = (if ctx.rtl { name_x - toggles_x } else { toggles_x - name_x }).max(20.0);
    let name = ui.painter().layout(c.name.clone(), egui::FontId::proportional(12.0), ctx.t.text, galley_w);
    let name_rect = Rect::from_min_size(pos2(if ctx.rtl { name_x - name.size().x } else { name_x }, rect.center().y - name.size().y / 2.0), name.size());
    if ui.data(|d| d.get_temp::<RenameComp>(rename_id)).is_some() {
        let field = if ctx.rtl {
            Rect::from_min_max(pos2((toggles_x - 10.0).min(name_x - 60.0), rect.top() + 3.0), pos2(name_x + 3.0, rect.bottom() - 3.0))
        } else {
            Rect::from_min_max(pos2(name_x - 3.0, rect.top() + 3.0), pos2((toggles_x + 10.0).max(name_x + 60.0), rect.bottom() - 3.0))
        };
        if let Some(text) = rename_field(ui, rename_id, field) {
            let text = text.trim();
            if !text.is_empty() && text != c.name {
                action = Some(("layerComp.rename", json!({"comp": c.id, "name": text})));
            }
        }
    } else {
        ui.painter().galley(name_rect.min, name, ctx.t.text);
    }
    (action, name_rect, hit)
}

/// The drag-and-drop reorder of a comp row: the drop line and the reorder command.
fn comp_dnd(ui: &mut egui::Ui, ctx: &RowCtx<'_>, rect: Rect, resp: &egui::Response, index: usize) -> (Option<u32>, Option<(&'static str, Value)>) {
    let Some(source) = resp.dnd_hover_payload::<DragComp>() else {
        return (None, None);
    };
    if source.document != ctx.doc.id {
        return (None, None);
    }
    let Some(from) = ctx.doc.layer_comps.iter().position(|c| c.id == source.comp) else {
        return (None, None);
    };
    let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= rect.center().y));
    let to = drop_index(from, index, after);
    let y = if after { rect.bottom() } else { rect.top() };
    ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, ctx.t.accent));
    if resp.dnd_release_payload::<DragComp>().is_some() && from != to {
        return (Some(source.comp), Some(("layerComp.reorder", json!({"comp":source.comp, "index":to}))));
    }
    (None, None)
}

/// The row's context menu: move the comp up or down in the list.
fn comp_context_menu(resp: &egui::Response, ctx: &RowCtx<'_>, c: &photocraft_doc::LayerComp, index: usize) -> (Option<u32>, Option<(&'static str, Value)>) {
    let mut select = None;
    let mut action = None;
    resp.context_menu(|ui| {
        if ui.add_enabled(index > 0, egui::Button::new(tl!("Move Up"))).clicked() {
            action = Some(("layerComp.reorder", json!({"comp":c.id, "index":index.saturating_sub(1)})));
            select = Some(c.id);
            ui.close();
        }
        if ui.add_enabled(index + 1 < ctx.doc.layer_comps.len(), egui::Button::new(tl!("Move Down"))).clicked() {
            action = Some(("layerComp.reorder", json!({"comp":c.id, "index":index + 1})));
            select = Some(c.id);
            ui.close();
        }
    });
    (select, action)
}

/// One comp row: apply marker, name, warning and toggles. Returns the selection and the
/// command the row asked for.
fn comp_row(
    ui: &mut egui::Ui,
    ctx: &RowCtx<'_>,
    c: &photocraft_doc::LayerComp,
    index: usize,
    selected: Option<u32>,
    applied: Option<u32>,
) -> (Option<u32>, Option<(&'static str, Value)>) {
    let mut action: Option<(&'static str, Value)> = None;
    let mut select: Option<u32> = None;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected == Some(c.id), &c.name));
    resp.dnd_set_drag_payload(DragComp { document: ctx.doc.id, comp: c.id });
    let (dnd_select, dnd_action) = comp_dnd(ui, ctx, rect, &resp, index);
    if let Some(s) = dnd_select {
        select = Some(s);
    }
    if let Some(a) = dnd_action {
        action = Some(a);
    }
    let (menu_select, menu_action) = comp_context_menu(&resp, ctx, c, index);
    if let Some(s) = menu_select {
        select = Some(s);
    }
    if let Some(a) = menu_action {
        action = Some(a);
    }
    if selected == Some(c.id) {
        ui.painter().rect_filled(rect, 0.0, ctx.t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, ctx.t.hover.gamma_multiply(0.5));
    }
    apply_marker(ui, ctx, rect, applied == Some(c.id), ctx.t.accent);
    let (toggles_x, mut toggle_hit, toggle_action) = draw_toggles(ui, ctx, rect, &resp, c);
    if let Some(a) = toggle_action {
        action = Some(a);
    }
    let rename_id = egui::Id::unique(("comp-rename", ctx.doc.id.0, c.id));
    let (name_action, name_rect, name_hit) = draw_name(ui, ctx, rect, toggles_x, rename_id, &resp, c);
    toggle_hit = toggle_hit || name_hit;
    if let Some(a) = name_action {
        action = Some(a);
    }
    if !c.comment.is_empty() && resp.hovered() {
        resp.clone().on_hover_text(&c.comment);
    }
    if resp.clicked() && !toggle_hit {
        select = Some(c.id);
        let marker = marker_rect(rect);
        let band = if ctx.rtl { marker.left() - 4.0 } else { marker.right() + 4.0 };
        let on_marker = |p: egui::Pos2| if ctx.rtl { p.x > band } else { p.x < band };
        if resp.interact_pointer_pos().is_some_and(on_marker) {
            action = Some(("layerComp.apply", json!({"comp": c.id})));
        }
    }
    if resp.double_clicked() && !toggle_hit {
        if resp.interact_pointer_pos().is_some_and(|p| name_rect.expand2(vec2(4.0, 13.0)).contains(p)) {
            ui.data_mut(|d| d.insert_temp(rename_id, RenameComp { text: c.name.clone(), focused: false }));
        } else {
            action = Some(("layerComp.apply", json!({"comp": c.id})));
        }
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, ctx.t.separator.gamma_multiply(0.5)));
    (select, action)
}

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.scope_builder(egui::UiBuilder::new().layout(crate::direction::body_layout()), |ui| {
            ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint).size(11.5));
        });
        return;
    };
    let doc = st.doc.clone();
    let selected = target(app);
    let applied = doc.last_applied_comp;
    let max_h = (ui.available_height() - 70.0).clamp(80.0, 320.0);
    let mut action: Option<(&'static str, Value)> = None;
    let mut select: Option<u32> = None;
    let rtl = crate::direction::is_rtl();
    let ctx = RowCtx { t: &t, doc: &doc, rtl };
    egui::ScrollArea::vertical().id_salt("comps-rows").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        // Last Document State.
        if let Some(a) = last_state_row(ui, &ctx, applied) {
            action = Some(a);
        }

        if doc.layer_comps.is_empty() {
            ui.scope_builder(egui::UiBuilder::new().layout(crate::direction::body_layout()), |ui| {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(tl!("Capture the visibility, position and style of every layer with + below, then switch between versions."))
                        .color(t.text_faint)
                        .size(11.5),
                );
            });
        }
        for (index, c) in doc.layer_comps.iter().enumerate() {
            let (sel, act) = comp_row(ui, &ctx, c, index, selected, applied);
            if let Some(s) = sel {
                select = Some(s);
            }
            if let Some(a) = act {
                action = Some(a);
            }
        }
    });
    // Footer: apply previous, apply next | update, new, delete (Photoshop's order).
    ui.add_space(4.0);
    let has = !doc.layer_comps.is_empty();
    // The footer row mirrors like egui's `horizontal`, but with the direction pinned: the dock
    // body is a vertical layout, so a nested row would not follow the reading order on its own.
    let footer_size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(footer_size, crate::direction::row(), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if crate::icons::button(ui, "chevrons-left", 24.0, false, tl!("Apply previous selected layer comp")).clicked() && has {
            action = Some(("layerComp.previous", json!({})));
        }
        if crate::icons::button(ui, "chevrons-right", 24.0, false, tl!("Apply next selected layer comp")).clicked() && has {
            action = Some(("layerComp.next", json!({})));
        }
        ui.with_layout(crate::direction::trailing(), |ui| {
            if crate::icons::button(ui, "trash", 24.0, false, tl!("Delete layer comp")).clicked()
                && let Some(id) = selected
            {
                action = Some(("layerComp.delete", json!({"comp": id})));
            }
            if crate::icons::button(ui, "file-plus", 24.0, false, tl!("Create new layer comp")).clicked() {
                action = Some(("layerComp.new", json!({})));
            }
            if crate::icons::button(ui, "rotate-cw", 24.0, false, tl!("Update layer comp (visibility, position and appearance)")).clicked()
                && let Some(id) = selected
            {
                action = Some(("layerComp.update", json!({"comp": id})));
            }
        });
    });
    if let Some(id) = select {
        app.ui.layer_comp_selected = Some(id);
    }
    if let Some((cmd, p)) = action {
        let created = cmd == "layerComp.new";
        run(app, cmd, p);
        if created {
            app.ui.layer_comp_selected = app.session.active().and_then(|d| d.doc.last_applied_comp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    #[test]
    fn dragging_a_comp_row_reorders_without_applying_it() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        for name in ["First", "Second", "Third"] {
            app.run("layerComp.new", json!({"name":name})).unwrap();
        }
        let applied = app.session.active().unwrap().doc.last_applied_comp;
        let first = app.session.active().unwrap().doc.layer_comps[0].id;
        let mut h = Harness::builder().with_size(vec2(360.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let from = h.get_by_label("First").rect().center();
        let row = h.get_by_label("Third").rect();
        let to = pos2(row.center().x, row.bottom() - 2.0);
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
        h.run_steps(2);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        let app = h.state();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.layer_comps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Second", "Third", "First"]);
        assert_eq!(doc.last_applied_comp, applied);
        assert_eq!(app.ui.layer_comp_selected, Some(first));
    }

    #[test]
    fn comp_drop_slots_account_for_the_removed_source_row() {
        assert_eq!(drop_index(0, 2, false), 1);
        assert_eq!(drop_index(0, 2, true), 2);
        assert_eq!(drop_index(2, 0, false), 0);
        assert_eq!(drop_index(2, 0, true), 1);
        assert_eq!(drop_index(1, 1, false), 1);
        assert_eq!(drop_index(1, 1, true), 1);
    }

    #[test]
    fn warning_icon_clears_only_its_own_comp() {
        // Regression for #4197.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let layer = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let c1 = s.execute("layerComp.new", json!({"name": "One"})).unwrap()["comp"].as_u64().unwrap() as u32;
        let c2 = s.execute("layerComp.new", json!({"name": "Two"})).unwrap()["comp"].as_u64().unwrap() as u32;
        s.execute("layer.delete", json!({"layer": layer})).unwrap();
        let (id, p) = clear_warning_action(c1);
        assert_eq!(p["comp"], c1);
        s.execute(id, p).unwrap();
        let doc = &s.active().unwrap().doc;
        assert!(doc.comp(c1).unwrap().missing_layers(doc).is_empty());
        assert_eq!(doc.comp(c2).unwrap().missing_layers(doc).iter().map(|l| l.0).collect::<Vec<_>>(), vec![layer]);
    }

    #[test]
    fn double_clicking_a_comp_name_renames_it_in_place() {
        // #6496.
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let first = app.run("layerComp.new", json!({"name": "Comp number one"})).unwrap()["comp"].as_u64().unwrap() as u32;
        let second = app.run("layerComp.new", json!({"name": "Two"})).unwrap()["comp"].as_u64().unwrap() as u32;
        // 60 fps steps, so two clicks a frame apart count as a double-click.
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        // The footer starts at the panel's left edge; the first comp is the second 26 px row. The
        // name starts at left + 30 and is as wide as its text, so the click stays near its start
        // to hit the short "Hero" too.
        let left = h.get_by_label("Apply previous selected layer comp").rect().left();
        let (name, rest) = (pos2(left + 35.0, left + 39.0), pos2(left + 190.0, left + 39.0));
        let double_click = |h: &mut Harness<'_, PhotocraftApp>, pos: egui::Pos2| {
            h.event(egui::Event::PointerMoved(pos));
            // Wait out the double-click window of the previous attempt.
            h.run_steps(30);
            for pressed in [true, false, true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
        };
        let type_name = |h: &mut Harness<'_, PhotocraftApp>, text: &str, key: egui::Key| {
            double_click(h, name);
            h.event(egui::Event::Text(text.into()));
            h.run_steps(1);
            let fields: Vec<_> = h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.accesskit_node().value()).collect();
            assert_eq!(fields, vec![Some(text.to_string())], "the name field opened and holds {text:?}");
            h.key_press(key);
            h.run_steps(3);
        };
        let comp_name = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.comp(first).unwrap().name.clone();
        let applied = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.last_applied_comp;
        // Whitespace keeps the old name.
        type_name(&mut h, "  ", egui::Key::Enter);
        assert_eq!(comp_name(&h), "Comp number one");
        type_name(&mut h, "Hero", egui::Key::Enter);
        assert_eq!(comp_name(&h), "Hero");
        assert_eq!(applied(&h), Some(second), "renaming must not apply the comp");
        type_name(&mut h, "Lost", egui::Key::Escape);
        assert_eq!(comp_name(&h), "Hero", "Escape cancels");
        assert_eq!(applied(&h), Some(second), "Escape applies nothing");
        // Double-clicking the rest of the row still applies the comp.
        double_click(&mut h, rest);
        assert_eq!(applied(&h), Some(first));
        assert_eq!(comp_name(&h), "Hero");
    }

    #[test]
    fn a_rename_left_open_does_not_carry_over_to_another_document() {
        // Comp ids restart in every document, so the pending name must stay with its own document.
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let a = app.run("layerComp.new", json!({"name": "Alpha comp"})).unwrap()["comp"].as_u64().unwrap() as u32;
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let b = app.run("layerComp.new", json!({"name": "Beta comp"})).unwrap()["comp"].as_u64().unwrap() as u32;
        assert_eq!(a, b, "the two documents' comps share an id");
        app.session.set_active(0);
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let left = h.get_by_label("Apply previous selected layer comp").rect().left();
        let name = pos2(left + 35.0, left + 39.0);
        h.event(egui::Event::PointerMoved(name));
        h.run_steps(3);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.event(egui::Event::Text("Leak".into()));
        h.run_steps(1);
        let fields: Vec<_> = h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.accesskit_node().value()).collect();
        assert_eq!(fields, vec![Some("Leak".to_string())], "the name field opened in the first document");
        h.state_mut().session.set_active(1);
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.comp(b).unwrap().name, "Beta comp");
    }

    #[test]
    fn row_anchors_mirror_about_the_row_centre() {
        let rect = Rect::from_min_max(pos2(100.0, 10.0), pos2(400.0, 36.0));
        let mirror = |x: f32| rect.left() + rect.right() - x;
        let (marker_ltr, name_ltr, toggles_ltr) = {
            let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
            row_anchors(rect)
        };
        let (marker_rtl, name_rtl, toggles_rtl) = {
            let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
            row_anchors(rect)
        };
        assert_eq!(marker_rtl, mirror(marker_ltr), "the apply marker mirrors");
        assert_eq!(name_rtl, mirror(name_ltr), "the name anchors mirror");
        for (ltr, rtl) in toggles_ltr.iter().zip(toggles_rtl.iter()) {
            assert_eq!(*rtl, mirror(*ltr), "every toggle anchor mirrors");
        }
        // The marker leads the row and the name follows it in reading order.
        assert!(marker_ltr < name_ltr, "LTR: the marker comes before the name");
        assert!(marker_rtl > name_rtl, "RTL: the marker comes before the name");
        // The toggles keep their logical order (eye, move, sparkles): the eye leads in reading
        // order in both directions.
        assert!(toggles_ltr[0] < toggles_ltr[1] && toggles_ltr[1] < toggles_ltr[2], "LTR: the eye toggle leads");
        assert!(toggles_rtl[0] > toggles_rtl[1] && toggles_rtl[1] > toggles_rtl[2], "RTL: the eye toggle leads");
    }

    #[test]
    fn the_footer_reads_in_the_interface_direction() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut h = Harness::builder().with_size(vec2(360.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
        let labels = [
            "Apply previous selected layer comp",
            "Apply next selected layer comp",
            "Delete layer comp",
            "Create new layer comp",
            "Update layer comp (visibility, position and appearance)",
        ];
        let rects = |h: &Harness<'_, PhotocraftApp>| labels.map(|l| h.get_by_label(l).rect());
        h.run_steps(3);
        let [prev, next, delete, new, update] = rects(&h);
        assert!(prev.left() < next.left(), "LTR: the previous button comes first, at the left");
        // LTR: the trailing cluster sits at the far right, in reading order update, new, delete.
        assert!(
            next.right() < update.left() && update.left() < new.left() && new.left() < delete.left(),
            "LTR: update, new, delete pack at the right end in reading order"
        );
        {
            let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
            h.run_steps(3);
            let [prev, next, delete, new, update] = rects(&h);
            assert!(prev.right() > next.right(), "RTL: the previous button comes first, at the right");
            // RTL: the same reading order update, new, delete, packed from the far left.
            assert!(
                update.right() < prev.left() && delete.left() < new.left() && new.left() < update.left(),
                "RTL: update, new, delete pack at the left end in reading order"
            );
        }
    }

    #[test]
    fn the_rename_field_mirrors_about_the_row_centre_in_rtl() {
        // Pins the mirrored row geometry through a real direction scope, via the one queryable
        // row element: the rename TextInput's rect.
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layerComp.new", json!({"name": "Hero"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(360.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let double_click = |h: &mut Harness<'_, PhotocraftApp>, pos: egui::Pos2| {
            h.event(egui::Event::PointerMoved(pos));
            // Wait out the double-click window of the previous attempt.
            h.run_steps(30);
            for pressed in [true, false, true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
        };
        let field_rect = |h: &mut Harness<'_, PhotocraftApp>| h.query_all_by_role(egui::accesskit::Role::TextInput).next().map(|n| n.rect()).unwrap();
        let row = h.get_by_label("Hero").rect();
        double_click(&mut h, pos2(row.left() + 35.0, row.center().y));
        let ltr = field_rect(&mut h);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        let rtl_field;
        {
            let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
            h.run_steps(3);
            double_click(&mut h, pos2(row.right() - 45.0, row.center().y));
            rtl_field = field_rect(&mut h);
        }
        let mirror = row.left() + row.right() - ltr.center().x;
        assert!((rtl_field.center().x - mirror).abs() < 0.5, "the rename field mirrors about the row centre: LTR {ltr:?}, RTL {rtl_field:?}");
        assert!(rtl_field.left() > ltr.left() && rtl_field.right() > ltr.right(), "RTL: the field hugs the reading-start (right) side of the row");
    }
}
