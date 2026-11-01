//! History › New Snapshot (⌥-click the History panel's camera button, #2966): Photoshop's dialog
//! for `history.newSnapshot`, with the snapshot's Name and what it is taken From (the full
//! document, its layers merged, or only the current layer). The command stays directly callable
//! with the same params by agents and Actions.

use egui::{Align2, Sense, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "history.newSnapshot";
const MARK: &str = "__newSnapshot";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog on the active document, suggesting the name the camera button would give.
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let name = photocraft_engine::history_cmds::next_snapshot_name(app.session.active()?);
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("New Snapshot"));
    f.insert("name".into(), json!(name));
    f.insert("from".into(), json!("fullDocument"));
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run(COMMAND, json!({"name": f.get("name"), "from": f.get("from")}))
}

/// A caption in a fixed column that hugs the field it names: the box's right edge in a
/// left-to-right UI, its left edge in a right-to-left one.
fn caption_anchor(r: egui::Rect, rtl: bool) -> (egui::Pos2, Align2) {
    if rtl { (pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER) } else { (pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER) }
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::hover());
    let (pos, align) = caption_anchor(r, crate::direction::is_rtl());
    ui.painter().text(pos, align, text, egui::FontId::proportional(12.0), t.text_dim);
}

/// A single-line edit whose text reads right-to-left in a right-to-left UI (Arabic names align
/// to the field's right edge); a left-to-right UI keeps egui's default edit, unchanged.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    // One direction-aware body: in a left-to-right UI the layout is egui's default, so nothing
    // moves; in a right-to-left one the rows read from the right and every field follows its
    // caption.
    ui.with_layout(crate::direction::body_layout(), |ui| body_fields(ui, f));
}

/// The New Snapshot dialog's fields.
fn body_fields(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Name:"));
        let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut r = crate::widgets::rtl_singleline_add(ui, &mut name, |e| e.desired_width(260.0));
        r = r.accessible_name(tl!("Name:"));
        // Opens on the name, selected, so typing replaces it (Photoshop).
        if !ui.is_sizing_pass() && !f.contains_key("__named") {
            r.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), r.id);
            }
            f.insert("__named".into(), json!(true));
        }
        f.insert("name".into(), json!(name));
    });
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("From:"));
        let mut from = f.get("from").and_then(Value::as_str).unwrap_or("fullDocument").to_string();
        let opts = [
            ("fullDocument".to_string(), tl!("Full Document")),
            ("mergedLayers".to_string(), tl!("Merged Layers")),
            ("currentLayer".to_string(), tl!("Current Layer")),
        ];
        if crate::widgets::dropdown(ui, "new-snapshot-from", tl!("From"), &mut from, &opts, 170.0) {
            f.insert("from".into(), json!(from));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// The dialog's field state, plus the column it was drawn in.
    type State = (Map<String, Value>, egui::Rect);

    /// The body drawn in `dir`, with the column it draws in.
    fn harness(dir: crate::direction::Dir) -> (egui_kittest::Harness<'static, State>, egui::Rect) {
        let _dir = crate::direction::direction_scope(dir);
        let mut f = Map::new();
        f.insert("name".into(), json!("Snapshot 1"));
        f.insert("__named".into(), json!(true));
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(400.0, 120.0))
            .with_context_setup(|ctx| crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.1 = ui.max_rect();
                    body(ui, &mut s.0);
                },
                (f, egui::Rect::ZERO),
            );
        h.run_steps(2);
        let col = h.state().1;
        (h, col)
    }

    /// The name field's rect, the "Name:" caption's edge that hugs it, the column they are
    /// drawn in, and the "From:" dropdown's rect.
    fn probe(dir: crate::direction::Dir) -> (egui::Rect, f32, egui::Rect, egui::Rect) {
        let (h, col) = harness(dir);
        let field = h.get_by_role(egui::accesskit::Role::TextInput).rect();
        let from = h.get_by_label("From").rect();
        let (left, width) = h
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.text() == "Name:" => Some((t.pos.x, t.galley.size().x)),
                _ => None,
            })
            .expect("the caption is painted");
        // The caption hugs the field: its right edge in a left-to-right row, its left in a
        // mirrored one.
        let caption = if dir == crate::direction::Dir::Rtl { left } else { left + width };
        (field, caption, col, from)
    }

    /// Photoshop Arabic: the rows read from the right — the caption comes first in reading
    /// order and the field follows it, the exact mirror of the left-to-right dialog.
    #[test]
    fn the_rows_read_mirrored_under_rtl() {
        let (ltr_field, ltr_caption, ltr_col, ltr_from) = probe(crate::direction::Dir::Ltr);
        let (rtl_field, rtl_caption, rtl_col, rtl_from) = probe(crate::direction::Dir::Rtl);
        // The caption comes before the field in a left-to-right row and after it in a mirrored one.
        assert!(ltr_field.left() - ltr_col.left() > 50.0, "LTR: the caption comes before the field: {ltr_field:?} in {ltr_col:?}");
        assert!(rtl_col.right() - rtl_field.right() > 50.0, "RTL: the caption comes after the field: {rtl_field:?} in {rtl_col:?}");
        // The "From:" dropdown follows its caption the same way (~72 pt from the reading start).
        assert!(ltr_from.left() - ltr_col.left() > 50.0, "LTR: the caption comes before the dropdown: {ltr_from:?} in {ltr_col:?}");
        assert!(rtl_col.right() - rtl_from.right() > 50.0, "RTL: the caption comes after the dropdown: {rtl_from:?} in {rtl_col:?}");
        // The field and the caption mirror about the column's centre.
        let centre = ltr_col.left() + rtl_col.right();
        assert!((ltr_field.left() + rtl_field.right() - centre).abs() < 1.0, "the field mirrors: {ltr_field:?} / {rtl_field:?}");
        assert!((ltr_caption + rtl_caption - centre).abs() < 1.0, "the caption mirrors: {ltr_caption} / {rtl_caption}");
    }

    /// The caption in its fixed column hugs the field it names: the box's right edge in a
    /// left-to-right row, its left edge in a mirrored one.
    #[test]
    fn the_caption_hugs_its_field() {
        let r = egui::Rect::from_min_size(pos2(10.0, 0.0), vec2(64.0, 22.0));
        let (ltr_pos, ltr_align) = caption_anchor(r, false);
        assert!(matches!(ltr_align, Align2::RIGHT_CENTER), "the caption is right-aligned: {ltr_align:?}");
        assert!((ltr_pos.x - (r.right() - 6.0)).abs() < 0.01, "the caption sits at the right of its box: {ltr_pos:?}");
        let (rtl_pos, rtl_align) = caption_anchor(r, true);
        assert!(matches!(rtl_align, Align2::LEFT_CENTER), "the caption is left-aligned: {rtl_align:?}");
        assert!((rtl_pos.x - (r.left() + 6.0)).abs() < 0.01, "the caption sits at the left of its box: {rtl_pos:?}");
    }

    /// The dialog opened through the host (`crate::dialogs::show`) in `dir`; its frame's width.
    fn host_frame_width(dir: crate::direction::Dir) -> f32 {
        let _dir = crate::direction::direction_scope(dir);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut()).expect("the dialog opens");
        h.run_steps(4);
        h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog is on screen").width()
    }

    /// The body scope must not balloon the modal: the New Snapshot dialog opens as wide in a
    /// right-to-left UI as in a left-to-right one.
    #[test]
    fn the_dialog_opens_as_wide_in_rtl_as_in_ltr() {
        let ltr = host_frame_width(crate::direction::Dir::Ltr);
        let rtl = host_frame_width(crate::direction::Dir::Rtl);
        assert!((ltr - rtl).abs() < 1.0, "the frame width is the same in both directions: LTR {ltr} vs RTL {rtl}");
    }
}
