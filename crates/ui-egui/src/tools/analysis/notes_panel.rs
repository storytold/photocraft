//! The Notes panel body.

use craft::behavior::{DocKey, IconId};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::text::TextField;
use serde_json::json;

use super::{note_alerts, show_note_options};
use crate::PhotocraftApp;
use crate::history_keys;

/// The text field's document key, the target of the focus request a note made with a click leaves.
const NOTE_TEXT: DocKey = DocKey::new("photocraft.note-text");

/// The panel body: the selected note's author and text, previous and next buttons with the position, and delete.
/// Nothing is selected until a note is clicked, so the text field is empty and disabled and the buttons are greyed.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (n, note) = match app.session.active() {
        Some(d) => (d.doc.notes.len(), Some(&d.doc.notes)),
        None => (0, None),
    };
    let shown = app.ui.analysis.note_selected.filter(|i| *i < n);
    // The shown note's author and text, owned so the edits below may run commands.
    let current = shown.and_then(|i| note.and_then(|notes| notes.get(i)).map(|note| (i, note.author.clone(), note.text.clone())));
    let (author, text) = current.as_ref().map_or((String::new(), String::new()), |(_, author, text)| (author.clone(), text.clone()));
    Text::new(Label::from(author)).show(ui);
    let field = TextField::new(Label::from(tl!("Notes")), Bind::given(text))
        .lines(6)
        .placeholder(Label::from(tl!("Click a note to view or edit")))
        .doc_key(NOTE_TEXT)
        .enabled(current.is_some());
    let events = field.show(ui).events;
    if current.is_some() && std::mem::take(&mut app.ui.analysis.focus_note_text) {
        let _ = focus::request(ui.ctx(), FocusTarget::Doc(NOTE_TEXT));
    }
    if let Some((i, ..)) = current {
        for event in events.iter() {
            let Some(value) = event.landed() else { continue };
            let mut params = json!({"index": i, "text": value});
            if let Some(key) = history_keys::coalesce(&format!("note-{i}"), event) {
                params["coalesce"] = json!(key);
            }
            let _ = app.run("notes.set", params);
        }
    }
    let mut moved = None;
    let mut delete = false;
    Row::new().show(ui, |ui, _| {
        let step = |ui: &mut egui::Ui, icon: &'static str, name: &str| {
            IconButton::new(IconId::new(icon)).label(Label::from(name.to_owned())).enabled(n > 1 && shown.is_some()).show(ui).clicked()
        };
        if step(ui, "chevrons_left", tl!("Previous note")) {
            moved = shown.map(|i| (i + n - 1) % n);
        }
        if step(ui, "chevrons_right", tl!("Next note")) {
            moved = shown.map(|i| (i + 1) % n);
        }
        Text::new(Label::from(note_counter(shown, n))).show(ui);
        delete = IconButton::new(IconId::new("trash")).label(Label::from(tl!("Delete note"))).enabled(shown.is_some()).show(ui).clicked();
    });
    if let Some(i) = moved {
        app.ui.analysis.note_selected = Some(i);
        show_note_options(app, i);
    }
    if delete && let Some(i) = shown {
        note_alerts::ask_delete_note(app, i);
    }
}

/// The counter: "N / M" for note N of M, blank with no note selected.
fn note_counter(shown: Option<usize>, n: usize) -> String {
    shown.map_or_else(String::new, |i| format!("{} / {n}", i + 1))
}
