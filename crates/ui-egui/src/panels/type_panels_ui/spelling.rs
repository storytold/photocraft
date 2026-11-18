//! Edit › Check Spelling: the framework's `CheckSpellingDialog` hosted on the dialog host, and the pump that runs
//! its actions against the engine.

use craft::text::ui::spelling::{CheckSpellingDialog, SpellingAction, SpellingItem};
use photocraft_doc::LayerId;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::run;
use crate::PhotocraftApp;

/// The `dialog` that Check Spelling's results and a playback paused at its step name.
pub const SPELLING: &str = "checkSpelling";

/// What the Check Spelling session keeps beside the hosted dialog (which holds the list, the position, the ignored
/// words and the Change To text).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellDialog {
    /// Check All Layers as the dialog last had it.
    pub all_layers: bool,
    /// Single-layer checks remain bound to their opening layer across relists and toggles.
    #[serde(skip)]
    pub layer: Option<LayerId>,
    /// The token of the action playback that opened the dialog at a Check Spelling step: Done
    /// continues it ([`crate::actions::resume`]).
    #[serde(skip)]
    pub resume: Option<u64>,
}

/// The engine's misspellings, as the raw JSON items of `edit.checkSpelling`. A single-layer check without a layer
/// lists nothing: an absent opening layer must not adopt the newly active one.
fn list(app: &mut PhotocraftApp, all_layers: bool, layer: Option<LayerId>, ignore: &[String]) -> Vec<Value> {
    let mut p = json!({ "action": "list", "allLayers": all_layers, "ignore": ignore, "suggestions": 8 });
    if !all_layers {
        let Some(layer) = layer else { return Vec::new() };
        p["layer"] = json!(layer.0);
    }
    run(app, "edit.checkSpelling", p).and_then(|r| r["misspellings"].as_array().cloned()).unwrap_or_default()
}

/// The text of a text layer of the active document (empty for any other layer).
fn text_of(app: &PhotocraftApp, id: u64) -> String {
    let Some(layer) = app.session.active().and_then(|st| st.doc.layer(LayerId(id))) else { return String::new() };
    match &layer.content {
        photocraft_doc::LayerContent::Text(text) => text.text.clone(),
        _ => String::new(),
    }
}

/// The byte range of `text` that the engine's character range covers.
fn byte_range(text: &str, chars: Range<usize>) -> Range<usize> {
    let at = |i: usize| text.char_indices().nth(i).map_or(text.len(), |(b, _)| b);
    at(chars.start)..at(chars.end)
}

/// The engine's character range for a byte range of `text`.
fn char_range(text: &str, bytes: Range<usize>) -> Range<usize> {
    let at = |b: usize| text.char_indices().take_while(|(i, _)| *i < b).count();
    at(bytes.start)..at(bytes.end)
}

/// The dialog's items for the engine's JSON items. The engine counts characters; the items hold byte ranges.
fn to_items(app: &PhotocraftApp, raw: &[Value]) -> Vec<SpellingItem> {
    raw.iter()
        .map(|i| {
            let layer = i["layer"].as_u64().unwrap_or(0);
            let start = i["start"].as_u64().unwrap_or(0) as usize;
            let end = i["end"].as_u64().unwrap_or(0) as usize;
            let suggestions = i["suggestions"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect();
            let range = byte_range(&text_of(app, layer), start..end);
            SpellingItem::new(layer, range, i["word"].as_str().unwrap_or(""), suggestions)
        })
        .collect()
}

/// Lists the misspellings again with the hosted dialog's Check All Layers and ignored words, and keeps the session's
/// `all_layers` at what the dialog has. Single-layer checks stay on the opening layer.
fn relist(app: &mut PhotocraftApp) -> Vec<SpellingItem> {
    let Some(dialog) = app.framework.dialogs.get::<CheckSpellingDialog>() else { return Vec::new() };
    let (checked, ignored) = (dialog.all_layers_checked(), dialog.ignored().to_vec());
    let Some(spell) = app.ui.type_panels.spell.as_mut() else { return Vec::new() };
    spell.all_layers = checked;
    let layer = spell.layer;
    let raw = list(app, checked, layer, &ignored);
    to_items(app, &raw)
}

/// Opens Edit › Check Spelling (returns the misspellings). Opening it again while an action waits
/// at a Check Spelling step keeps that action's token, so Done still continues it.
pub fn open_spelling(app: &mut PhotocraftApp, all_layers: bool) -> Result<Value, String> {
    let resume = app.ui.type_panels.spell.as_ref().and_then(|d| d.resume);
    let layer = if resume.is_some() { app.ui.type_panels.spell.as_ref().and_then(|d| d.layer) } else { app.session.active().and_then(|st| st.active_layer) };
    let mut params = json!({ "action": "list", "allLayers": all_layers });
    if !all_layers {
        params["layer"] = json!(layer.ok_or("no layer was selected for this spelling check")?.0);
    }
    app.session.execute("edit.checkSpelling", params).map_err(|e| e.to_string())?;
    let raw = list(app, all_layers, layer, &[]);
    app.ui.type_panels.spell = Some(SpellDialog { all_layers, layer, resume });
    let items = to_items(app, &raw);
    // Opening over an open dialog replaces it and leaves the replaced one's Cancel as a result; that Cancel is not
    // the new dialog's, so it is dropped.
    let replaced = app.framework.dialogs.is_open::<CheckSpellingDialog>();
    app.framework.dialogs.open(CheckSpellingDialog::new(items).all_layers(all_layers));
    if replaced {
        let _ = app.framework.dialogs.take::<CheckSpellingDialog>();
    }
    Ok(json!({ "dialog": SPELLING, "misspellings": raw }))
}

/// Done: an action being recorded gets one Check Spelling step with Check All Layers as the dialog
/// has it, and an action paused at a Check Spelling step continues on the document it was playing
/// on. When recording the step fails, the status keeps that error with the playback's outcome.
/// A dialog still open is cancelled through the host, which releases its layers.
pub(crate) fn close_spelling(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.framework.dialogs.is_open::<CheckSpellingDialog>() {
        app.framework.dialogs.cancel::<CheckSpellingDialog>(ctx);
        let _ = app.framework.dialogs.take::<CheckSpellingDialog>();
    }
    let Some(d) = app.ui.type_panels.spell.take() else { return };
    let mut recording_error = None;
    if app.session.actions.recording.is_some() && run(app, "edit.checkSpelling", json!({ "allLayers": d.all_layers })).is_none() {
        recording_error = Some(app.ui.status.clone());
    }
    if let Some(token) = d.resume {
        let _ = crate::actions::resume_spelling(app, token);
        if let Some(error) = recording_error {
            app.ui.status = format!("{error} ({})", app.ui.status);
            app.ui.status_error = true;
        }
    }
}

/// Reports an engine refusal in the status line.
fn report(app: &mut PhotocraftApp, result: Result<Value, String>) {
    if let Err(error) = result {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Replaces the one listed occurrence.
fn change(app: &mut PhotocraftApp, item: &SpellingItem, to: &str) {
    let chars = char_range(&text_of(app, item.object), item.range.clone());
    let p = json!({ "action": "change", "layer": item.object, "start": chars.start, "end": chars.end, "word": item.word, "replace": to });
    let result = crate::actions::spelling_change(app, p);
    report(app, result);
}

/// Replaces the listed item's word: in its own text object, or in every text object when `all_layers`.
fn change_all(app: &mut PhotocraftApp, item: &SpellingItem, to: &str, all_layers: bool) {
    let mut p = json!({ "action": "changeAll", "word": item.word, "replace": to, "allLayers": all_layers });
    if !all_layers {
        p["layer"] = json!(item.object);
    }
    let result = crate::actions::spelling_change(app, p);
    report(app, result);
}

/// Carries out one button of the dialog. The list is read again by the caller.
fn run_action(app: &mut PhotocraftApp, action: SpellingAction) {
    match action {
        SpellingAction::Change { item, to, .. } => change(app, &item, &to),
        SpellingAction::ChangeAll { item, to, all_layers, .. } => change_all(app, &item, &to, all_layers),
        SpellingAction::Add(word) => {
            let _ = run(app, "edit.checkSpelling", json!({ "action": "addToDictionary", "word": word }));
        }
        // The relist after the actions reads the dialog's Check All Layers and ignored words.
        SpellingAction::Relist => {}
        _ => {}
    }
}

/// The result pump (`dialogs::TAKERS`): while the dialog is open, runs its actions and lists again; a result of any
/// kind ends the session. A second pass in a frame finds no action left, so it applies nothing twice.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut acted = false;
    while let Some(action) = app.framework.dialogs.get_mut::<CheckSpellingDialog>().and_then(|d| d.take_action()) {
        run_action(app, action);
        acted = true;
    }
    if acted {
        let items = relist(app);
        if let Some(dialog) = app.framework.dialogs.get_mut::<CheckSpellingDialog>() {
            dialog.set_items(items);
        }
    }
    if app.framework.dialogs.take::<CheckSpellingDialog>().is_some() {
        close_spelling(app, ctx);
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::StackDialog as _;
    use photocraft_engine::actions_cmds::ActionStep;

    use super::*;

    fn app_with_text(text: &str) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 300, "height": 120, "background": "white"})).unwrap();
        app.run("type.create", json!({"x": 10, "y": 60, "text": text})).unwrap();
        app.sync_views();
        app
    }

    fn items(app: &PhotocraftApp) -> Vec<SpellingItem> {
        app.framework.dialogs.get::<CheckSpellingDialog>().unwrap().items().to_vec()
    }

    fn layer_text(app: &PhotocraftApp, id: LayerId) -> String {
        let photocraft_doc::LayerContent::Text(text) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("not text") };
        text.text.clone()
    }

    #[test]
    /// Single-layer Change All edits the listed text layer even after the host selects another layer.
    fn change_all_keeps_the_listed_layer_after_a_session_selection_change() {
        for all_layers in [false, true] {
            let mut app = app_with_text("Teh cat");
            let listed = app.session.active().unwrap().active_layer.unwrap();
            app.run("type.create", json!({"x":10,"y":90,"text":"Teh dog"})).unwrap();
            let other = app.session.active().unwrap().active_layer.unwrap();
            app.session.select_layer(listed).unwrap();
            open_spelling(&mut app, false).unwrap();
            app.session.select_layer(other).unwrap();
            let item = relist(&mut app).remove(0);
            assert_eq!(item.object, listed.0);
            // PLAN: `SpellingAction::ChangeAll` is `#[non_exhaustive]`, so a test outside craft-text cannot build it;
            // the test drives `change_all`, which `run_action` calls for that action.
            change_all(&mut app, &item, "The", all_layers);
            let items = relist(&mut app);
            assert!(items.iter().all(|i| i.object != listed.0), "the listed layer is corrected");
            for (id, expected) in [(listed, "The cat"), (other, if all_layers { "The dog" } else { "Teh dog" })] {
                assert_eq!(layer_text(&app, id), expected);
            }
        }
    }

    #[test]
    /// Relisting and toggling Check All Layers keep the original single-layer receiver, including the absent-layer case.
    fn spelling_refresh_and_all_layers_toggles_keep_the_opening_layer() {
        let mut app = app_with_text("Teh cat");
        let ctx = app.ctx().clone();
        let original = app.session.active().unwrap().active_layer.unwrap();
        app.run("type.create", json!({"x":10,"y":90,"text":"Teh dog"})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        app.session.select_layer(original).unwrap();
        open_spelling(&mut app, false).unwrap();
        app.session.select_layer(other).unwrap();
        for all_layers in [false, true, false] {
            let dialog = app.framework.dialogs.get_mut::<CheckSpellingDialog>().unwrap();
            dialog.set_json_field(&ctx, "allLayers", json!(all_layers)).unwrap();
            take(&mut app, &ctx);
            // A second pass in the same frame finds nothing left to apply.
            take(&mut app, &ctx);
            let listed = items(&app);
            assert!(listed.iter().any(|item| item.object == original.0));
            assert_eq!(listed.iter().any(|item| item.object == other.0), all_layers);
        }
        app.session.restore_layer_selection(None, &[], None).unwrap();
        open_spelling(&mut app, true).unwrap();
        assert!(app.ui.type_panels.spell.as_ref().unwrap().layer.is_none());
        app.session.select_layer(other).unwrap();
        let dialog = app.framework.dialogs.get_mut::<CheckSpellingDialog>().unwrap();
        dialog.set_json_field(&ctx, "allLayers", json!(false)).unwrap();
        take(&mut app, &ctx);
        assert!(items(&app).is_empty(), "a missing opening target must not adopt the new active layer");
    }

    #[test]
    fn recording_the_dialog_records_one_step_with_its_dialog_on() {
        let mut app = app_with_text("Teh cat");
        let ctx = app.ctx().clone();
        app.run("actions.record", json!({"name": "Spell"})).unwrap();
        open_spelling(&mut app, true).unwrap();
        let item = items(&app).remove(0);
        // Change, as the dialog's Change button runs it, then untick Check All Layers and press Done.
        change(&mut app, &item, "The");
        let dialog = app.framework.dialogs.get_mut::<CheckSpellingDialog>().unwrap();
        dialog.set_json_field(&ctx, "allLayers", json!(false)).unwrap();
        take(&mut app, &ctx);
        let id = app.framework.dialogs.get::<CheckSpellingDialog>().unwrap().id();
        crate::dialogs::confirm(&mut app, &ctx, id).expect("Done is accepted");
        take(&mut app, &ctx);
        app.run("actions.stop", json!({})).unwrap();
        assert!(app.ui.type_panels.spell.is_none());
        assert!(!app.framework.dialogs.is_open::<CheckSpellingDialog>());
        let action = app.session.actions.list.action(0).unwrap();
        assert_eq!(action.steps, vec![ActionStep::new("edit.checkSpelling", json!({"allLayers": false}))]);
        assert_eq!(action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
    }
}
