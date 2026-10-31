//! History › New Snapshot (⌥-click the History panel's camera button, #2966): Photoshop's dialog
//! for `history.newSnapshot`, with the snapshot's Name and what it is taken From (the full
//! document, its layers merged, or only the current layer). The command stays directly callable
//! with the same params by agents and Actions.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

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

/// OK: takes the snapshot with `history.newSnapshot`, passing the Name and From fields.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run(COMMAND, json!({"name": f.get("name"), "from": f.get("from")}))
}

/// Label column bounds for [`crate::widgets::label_column_width`]: the English labels' width, and what the 260 pt name field leaves of the dialog.
const LABEL_MIN_W: f32 = 64.0;
const LABEL_MAX_W: f32 = 160.0;
/// Every label in the column, measured in the current language.
const LABELS: [&str; 2] = ["Name:", "From:"];

/// The Name field (focused and selected when the dialog opens) and the From menu, their labels
/// in one column measured over [`LABELS`].
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let label_w = crate::widgets::label_column_width(ui, &LABELS.map(crate::i18n::t), LABEL_MIN_W, LABEL_MAX_W);
    let label = |ui: &mut egui::Ui, text: &str| {
        crate::widgets::column_label(ui, text, label_w);
    };
    ui.horizontal(|ui| {
        label(ui, tl!("Name:"));
        let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
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
    ui.horizontal(|ui| {
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
