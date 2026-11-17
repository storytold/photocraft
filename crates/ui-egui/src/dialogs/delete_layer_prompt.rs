//! Confirmation for a Layers-panel trash-button click; other deletion routes stay immediate.
use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

pub(crate) const PREF: &str = "layer.delete.confirmation";
const COMMAND: &str = "layer.delete";

/// What the prompt asked about: the document, its selection and the command's parameters. The
/// answer runs only while the document and the selection are still these.
pub(crate) struct PendingDelete {
    doc: u64,
    selected: Vec<u64>,
    params: Value,
}

pub fn intercept(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if app.automation_input || command != COMMAND || params.get("__trash").and_then(Value::as_bool) != Some(true) || alerts::dismissed(app, PREF) {
        return None;
    }
    let state = app.session.active()?;
    let selected = state.selected_layers();
    let ids = if let Some(id) = params.get("layer").and_then(Value::as_u64) { vec![photocraft_doc::LayerId(id)] } else { selected.clone() };
    if ids.is_empty() {
        return None;
    }
    let names: Vec<String> = ids.iter().filter_map(|id| state.doc.layer(*id).map(|layer| layer.name.clone())).take(4).collect();
    let tag = PendingDelete { doc: state.doc.id.0, selected: selected.iter().map(|id| id.0).collect(), params: params.clone() };
    let prompt = StandardPrompt::confirm_delete(names, ids.len()).dont_show_again().tag(tag);
    Some(Ok(match alerts::open_prompt(app, prompt) {
        Ok(id) => json!({"dialog": id.value(), "confirmationPending": true}),
        Err(_) => json!({"confirmationPending": true}),
    }))
}

/// The `PROMPTS` entry: Delete runs the held command when the document and selection are unchanged
/// and keeps a ticked "Don't show again" after it succeeded. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(pending) = answer.take_tag::<PendingDelete>() else { return false };
    if answer.choice != PromptChoice::Proceed {
        return true;
    }
    match confirm(app, pending) {
        Ok(_) if answer.skip => alerts::remember_skip_or(app, PREF, tl!("Layer deleted, but confirmation preference could not be saved: {error}")),
        Ok(_) => {}
        Err(error) => alerts::report_error(app, error),
    }
    true
}

fn confirm(app: &mut PhotocraftApp, pending: PendingDelete) -> Result<Value, String> {
    let state = app.session.active().ok_or(tl!("The document is closed"))?;
    if pending.doc != state.doc.id.0 || pending.selected != state.selected_layers().iter().map(|id| id.0).collect::<Vec<_>>() {
        return Err(tl!("The document or selected layers changed. Please request deletion again.").into());
    }
    app.run_command(COMMAND, pending.params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use std::sync::Arc;

    fn session(layer: &str) -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        s.execute("layer.new.layer", json!({"name":layer})).unwrap();
        s
    }

    fn make_app() -> PhotocraftApp {
        PhotocraftApp::new(session("Ink"), crate::Services::default())
    }

    /// An app whose preferences live in `files`, as one launch of the program.
    fn launch(files: &Arc<craft::settings::testing::MemoryFiles>, session: photocraft_engine::Session) -> PhotocraftApp {
        let scratch = std::env::temp_dir().join("photocraft-delete-layer-prompt");
        let options = craft::settings::OpenOptions::new(craft::settings::Location::Dir(scratch)).files(files.clone());
        PhotocraftApp::new(session, crate::Services { settings: Some(craft::settings::SettingsStore::open_with(options)), ..Default::default() })
    }

    fn pending(app: &PhotocraftApp) -> craft::ui::dialog::DialogId {
        alerts::prompt_tagged::<PendingDelete>(app).expect("the delete question is open")
    }

    #[test]
    fn confirmation_deletes_once_and_remembers_across_restart() {
        let files = Arc::new(craft::settings::testing::MemoryFiles::new());
        let mut app = launch(&files, session("Ink"));
        let ctx = app.ctx().clone();
        let target = app.session.active().unwrap().active_layer.unwrap();
        let before = app.session.active().unwrap().history.past_len();
        app.run(COMMAND, json!({"__trash":true})).unwrap();
        let id = pending(&app);
        app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        assert!(app.session.active().unwrap().doc.layer(target).is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        assert!(app.framework.dialogs.is_empty());
        app.session.undo();
        assert!(app.session.active().unwrap().doc.layer(target).is_some());
        assert!(alerts::dismissed(&app, PREF));
        let mut fresh = launch(&files, session("Fresh"));
        let fresh_layer = fresh.session.active().unwrap().active_layer.unwrap();
        fresh.run(COMMAND, json!({"__trash":true})).unwrap();
        assert!(fresh.framework.dialogs.is_empty());
        assert!(fresh.session.active().unwrap().doc.layer(fresh_layer).is_none(), "a remembered skip deletes without asking");
    }

    #[test]
    fn changed_document_refuses_confirmation_and_clear_does_not_prompt() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        app.run(COMMAND, json!({"__trash":true})).unwrap();
        let id = pending(&app);
        app.session.execute("file.new", json!({"width":8,"height":8})).unwrap();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        assert!(app.ui.status_error, "the stale answer reports why nothing was deleted");
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2);
        app.run("edit.clear", json!({})).unwrap();
        assert!(app.framework.dialogs.is_empty());
    }

    #[test]
    fn only_trash_click_prompts_and_default_delete_binding_is_unchanged() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            // PLAN: assumed `PhotocraftApp::with_context(ctx, session, services)` (hub-w1)
            PhotocraftApp::with_context(cc.egui_ctx.clone(), session("Ink"), crate::Services::default())
        });
        h.run_steps(4);
        let id = h.state().session.active().unwrap().active_layer.unwrap();
        h.get_by_label("Delete layer").click();
        h.run_steps(4);
        let question = pending(h.state());
        assert!(h.state().session.active().unwrap().doc.layer(id).is_some());
        let ctx = h.ctx.clone();
        crate::dialogs::cancel(h.state_mut(), &ctx, question).unwrap();
        h.state_mut().run(COMMAND, json!({})).unwrap();
        assert!(h.state().framework.dialogs.is_empty());
        assert!(h.state().session.active().unwrap().doc.layer(id).is_none());
        // The Delete key is the framework's `edit.delete` binding, which `shortcut_dispatch` runs as Clear, so Delete
        // Layer carries no key of its own.
        let delete = photocraft_engine::commands::find(COMMAND).unwrap();
        assert_eq!(delete.shortcut, None);
    }
}
