//! The Notes panel body.

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use serde_json::json;

use super::{note_alerts, show_note_options};
use crate::PhotocraftApp;
use crate::history_keys;

/// The panel body: the shown note's author and text, previous and next buttons with the position, and delete.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (n, note) = match app.session.active() {
        Some(d) => (d.doc.notes.len(), Some(&d.doc.notes)),
        None => (0, None),
    };
    let shown = app.ui.analysis.note_selected.filter(|i| *i < n).or((n > 0).then_some(0));
    // The shown note's author and text, owned so the edits below may run commands.
    let current = shown.and_then(|i| note.and_then(|notes| notes.get(i)).map(|note| (i, note.author.clone(), note.text.clone())));
    match current {
        Some((i, author, text)) => {
            let author = if author.is_empty() { tl!("No author").to_string() } else { author };
            Text::new(Label::from(author)).show(ui);
            // PLAN: assumed `TextField::lines(usize)` sets the line count of a multi-line field.
            let events = TextField::new(Label::from(tl!("Notes")), Bind::given(text)).lines(6).show(ui).events;
            for event in events.iter() {
                let Some(value) = event.landed() else { continue };
                let mut params = json!({"index": i, "text": value});
                if let Some(key) = history_keys::coalesce(&format!("note-{i}"), event) {
                    params["coalesce"] = json!(key);
                }
                let _ = app.run("notes.set", params);
            }
        }
        None => {
            Text::new(Label::from(tl!("Click with the Note tool to add a note."))).show(ui);
        }
    }
    let mut moved = None;
    let mut delete = false;
    Row::new().show(ui, |ui, _| {
        let step = |ui: &mut egui::Ui, icon: &'static str, name: &str| {
            IconButton::new(IconId::new(icon)).label(Label::from(name.to_owned())).enabled(n > 1).show(ui).clicked()
        };
        if step(ui, "chevrons_left", tl!("Previous note")) {
            moved = shown.map(|i| (i + n - 1) % n);
        }
        if step(ui, "chevrons_right", tl!("Next note")) {
            moved = shown.map(|i| (i + 1) % n);
        }
        let template = "{index} of {total}";
        let position = crate::i18n::fmt(tl!(template), &[("index", &shown.map_or(0, |i| i + 1).to_string()), ("total", &n.to_string())]);
        Text::new(Label::from(position)).show(ui);
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
