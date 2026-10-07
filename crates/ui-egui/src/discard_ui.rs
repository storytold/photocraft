//! Save / Don't Save / Cancel prompt for actions that would throw away unsaved changes: closing
//! documents, Revert, and leaving the app (File › Exit or the window's close button).
//!
//! egui is immediate-mode, so the action is parked in [`Prompt`] while the modal is up and re-run
//! once every affected document has been answered. Cancel at any point drops it. Documents are
//! tracked by id, not tab index, so closing one elsewhere while the prompt is up can't retarget it.

use egui::Key;
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const EXIT: &str = "file.exit";

/// An action waiting on the user, and the dirty documents still to ask about.
pub struct Prompt {
    id: String,
    params: Value,
    /// The document the command was aimed at (`document` param, else the active one), if it has one.
    target: Option<DocId>,
    docs: Vec<DocId>,
}

fn index_of(app: &PhotocraftApp, id: DocId) -> Option<usize> {
    app.session.documents().iter().position(|d| d.doc.id == id)
}

/// Which documents a command discards, relative to its target (the `document` param, else the active one).
#[derive(Clone, Copy)]
enum Reach {
    Target,
    AllButTarget,
    All,
}

/// Every command that can throw away unsaved changes. A new one only needs a row here.
const DISCARDING: &[(&str, Reach)] = &[
    ("file.close", Reach::Target),
    ("file.revert", Reach::Target),
    ("file.closeOthers", Reach::AllButTarget),
    ("file.closeAll", Reach::All),
    (EXIT, Reach::All),
];

/// The command's target document and the unsaved documents it would discard (empty for commands
/// that discard nothing).
fn discarded(app: &PhotocraftApp, id: &str, params: &Value) -> (Option<DocId>, Vec<DocId>) {
    let docs = app.session.documents();
    let target = params.get("document").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index());
    let affected: Vec<usize> = match DISCARDING.iter().find(|(c, _)| *c == id) {
        Some((_, Reach::Target)) => target.into_iter().collect(),
        Some((_, Reach::AllButTarget)) => (0..docs.len()).filter(|&i| Some(i) != target).collect(),
        Some((_, Reach::All)) => (0..docs.len()).collect(),
        None => Vec::new(),
    };
    let dirty = affected.into_iter().filter_map(|i| docs.get(i)).filter(|d| d.is_dirty()).map(|d| d.doc.id).collect();
    (target.and_then(|i| docs.get(i)).map(|d| d.doc.id), dirty)
}

/// Park `id` behind a prompt if it would discard unsaved work. Returns whether it did.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> bool {
    let (target, docs) = discarded(app, id, params);
    if docs.is_empty() {
        return false;
    }
    let prompt = Prompt { id: id.to_string(), params: params.clone(), target, docs };
    match &app.discard {
        None => app.discard = Some(prompt),
        // Quitting overrides whatever is pending: it covers every document, so nothing is lost.
        Some(open) if id == EXIT && open.id != EXIT => app.discard = Some(prompt),
        // Repeated quit requests (the X pressed again) keep the prompt, and the answers so far.
        Some(open) if open.id == id => {}
        Some(_) => {
            app.ui.status = tl!("Answer the unsaved-changes prompt first").into();
            app.ui.status_error = true;
        }
    }
    true
}

/// Called once per frame: holds back a window close request while there is unsaved work.
pub fn guard_window_close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.allow_close || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    if intercept(app, EXIT, &Value::Null) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
}

/// Moves on to the next document, or runs the parked action once none are left.
fn advance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.discard.as_mut() else { return };
    if !p.docs.is_empty() {
        p.docs.remove(0);
    }
    if !p.docs.is_empty() {
        return;
    }
    let Some(Prompt { id, mut params, target, .. }) = app.discard.take() else { return };
    if id == EXIT {
        app.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    if let Some(target) = target {
        // The tab may have moved since the command was issued; aim it at the same document.
        let Some(i) = index_of(app, target) else { return };
        params = json!({"document": i});
    }
    if let Err(e) = crate::menus::invoke_unguarded(app, ctx, &id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Saves `doc` in place; false when it is gone, the save failed or its file dialog was cancelled.
fn save(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) -> bool {
    let Some(i) = index_of(app, doc) else { return false };
    app.session.set_active(i);
    match crate::menus::invoke_unguarded(app, ctx, "file.save", json!({})) {
        Ok(_) => true,
        // Backing out of the file dialog is the user's choice, not an error.
        Err(e) if e == "cancelled" => false,
        Err(e) => {
            app.ui.status = format!("Couldn't save: {e}");
            app.ui.status_error = true;
            false
        }
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &app.discard else { return };
    let Some(&doc) = p.docs.first() else { return };
    let (exits, reverts) = (p.id == EXIT, p.id == "file.revert");
    let Some(name) = index_of(app, doc).map(|i| app.session.documents()[i].doc.name.clone()) else {
        // Closed by something else while the prompt was up: nothing left to ask about.
        advance(app, ctx);
        return;
    };
    let (title, message) = if reverts {
        ("Revert", crate::i18n::fmt(tl!("Revert “{name}” to the last saved version? Your changes will be lost."), &[("name", &name)]))
    } else {
        let template = if exits {
            tl!("Do you want to save the changes you made to “{name}” before quitting?")
        } else {
            tl!("Do you want to save the changes you made to “{name}” before closing?")
        };
        (tl!("Unsaved changes"), crate::i18n::fmt(template, &[("name", &name)]))
    };
    // Left to right, as drawn: (label, key, primary, min width, answer).
    let buttons: &[(&str, Key, bool, f32, Answer)] = if reverts {
        &[("Cancel", Key::C, false, 84.0, Answer::Cancel), ("Revert", Key::R, true, 84.0, Answer::Discard)]
    } else {
        &[("Don't Save", Key::D, false, 100.0, Answer::Discard), ("Cancel", Key::C, false, 84.0, Answer::Cancel), ("Save", Key::S, true, 84.0, Answer::Save)]
    };
    let mut answer = ctx.input_mut(|i| buttons.iter().find(|b| i.consume_key(egui::Modifiers::NONE, b.1)).map(|b| b.4));
    // egui's Tab order follows the right-to-left layout below; walk the buttons left to right instead.
    let step = ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, Key::Tab) {
            -1
        } else if i.consume_key(egui::Modifiers::NONE, Key::Tab) {
            1
        } else {
            0
        }
    });
    if step != 0 {
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    let modal = egui::Modal::new(egui::Id::new("discard-prompt")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!(&title)).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(message);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let mut drawn: Vec<_> = buttons
                .iter()
                .rev()
                .map(|&(label, key, primary, width, a)| {
                    let label = mnemonic(label, key);
                    let r = if primary { crate::widgets::primary_button(ui, &label, width) } else { crate::widgets::secondary_button(ui, &label, width) };
                    if r.clicked() {
                        answer = Some(a);
                    }
                    r
                })
                .collect();
            drawn.reverse();
            if step != 0 {
                let n = drawn.len() as i32;
                let at = drawn.iter().position(|r| r.has_focus()).map_or(if step > 0 { -1 } else { n }, |i| i as i32);
                if let Some(r) = drawn.get((at + step).rem_euclid(n) as usize) {
                    r.request_focus();
                }
            }
        });
    });
    if modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    match answer {
        Some(Answer::Cancel) => app.discard = None,
        Some(Answer::Discard) => advance(app, ctx),
        Some(Answer::Save) if save(app, ctx, doc) => advance(app, ctx),
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// "(S)ave": the key in parentheses, or appended ("Guardar (S)") when the translation doesn't start with it.
fn mnemonic(label: &str, key: Key) -> String {
    let (label, k) = (tl!(label), key.name());
    match label.split_at_checked(1) {
        Some((first, rest)) if first.eq_ignore_ascii_case(k) => format!("({first}){rest}"),
        _ => format!("{label} ({k})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_docs(n: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..n {
            app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        }
        app.sync_views();
        app
    }

    fn make_dirty(app: &mut PhotocraftApp, doc: usize) {
        app.session.set_active(doc);
        app.run("layer.new.layer", json!({})).unwrap();
    }

    fn doc_id(app: &PhotocraftApp, i: usize) -> DocId {
        app.session.documents()[i].doc.id
    }

    #[test]
    fn clean_documents_close_without_asking() {
        let mut app = app_with_docs(2);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
        assert!(app.discard.is_none());
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn closing_a_dirty_document_waits_for_an_answer() {
        let mut app = app_with_docs(2);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 1);
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 1})).unwrap();
        assert_eq!(app.session.documents().len(), 2, "nothing closes before the user answers");
        assert!(app.discard.is_some());
        advance(&mut app, &ctx);
        assert!(app.discard.is_none());
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn only_documents_that_would_be_discarded_are_asked_about() {
        let mut app = app_with_docs(3);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 2);
        let ids = [doc_id(&app, 0), doc_id(&app, 2)];
        assert_eq!(discarded(&app, "file.closeOthers", &json!({"document": 0})), (Some(ids[0]), vec![ids[1]]));
        assert_eq!(discarded(&app, "file.closeAll", &json!({})).1, ids);
        assert_eq!(discarded(&app, EXIT, &json!({})).1, ids);
        assert!(discarded(&app, "view.zoomIn", &json!({})).1.is_empty());
    }

    #[test]
    fn close_all_asks_per_dirty_document_then_runs() {
        let mut app = app_with_docs(3);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 2);
        crate::menus::invoke(&mut app, &ctx, "file.closeAll", json!({})).unwrap();
        advance(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 3, "still waiting on the second document");
        advance(&mut app, &ctx);
        assert!(app.session.documents().is_empty());
    }

    #[test]
    fn the_parked_close_still_hits_its_document_after_tabs_move() {
        let mut app = app_with_docs(3);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 2);
        let doomed = doc_id(&app, 2);
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 2})).unwrap();
        // Another document closes while the prompt is up, shifting the tab to index 1.
        app.run("file.close", json!({"document": 0})).unwrap();
        advance(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        assert!(index_of(&app, doomed).is_none());
    }

    #[test]
    fn quitting_replaces_a_pending_close_but_other_requests_are_refused() {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        assert!(intercept(&mut app, "file.close", &json!({"document": 0})));
        assert!(intercept(&mut app, "file.closeAll", &Value::Null));
        assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some(("file.close", 1)));
        assert!(app.ui.status_error);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some((EXIT, 2)));
        // Pressing the window's X again must not restart the questions.
        app.discard.as_mut().unwrap().docs.remove(0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert_eq!(app.discard.as_ref().unwrap().docs.len(), 1);
    }

    #[test]
    fn exit_is_held_back_until_confirmed() {
        let mut app = app_with_docs(1);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert!(!app.allow_close);
        advance(&mut app, &ctx);
        assert!(app.allow_close);
    }

    #[test]
    fn mnemonic_labels_bracket_the_key() {
        assert_eq!(mnemonic("Don't Save", Key::D), "(D)on't Save");
        assert_eq!(mnemonic("Guardar", Key::S), "Guardar (S)");
        assert_eq!(mnemonic("保存", Key::S), "保存 (S)");
    }

    #[test]
    fn tab_walks_the_buttons_left_to_right_and_letters_answer() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        assert!(intercept(h.state_mut(), "file.closeAll", &Value::Null));
        h.run_steps(2);
        let focused = |h: &Harness<'_, PhotocraftApp>| ["(D)on't Save", "(C)ancel", "(S)ave"].into_iter().find(|l| h.get_by_label(l).is_focused());
        for want in ["(D)on't Save", "(C)ancel", "(S)ave", "(D)on't Save"] {
            h.key_press(Key::Tab);
            h.run_steps(2);
            assert_eq!(focused(&h), Some(want));
        }
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
        h.run_steps(2);
        assert_eq!(focused(&h), Some("(S)ave"));
        h.key_press(Key::D);
        h.run_steps(2);
        assert_eq!(h.state().discard.as_ref().map(|p| p.docs.len()), Some(1), "D answered the first document");
        h.key_press(Key::C);
        h.run_steps(2);
        assert!(h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 2, "Cancel closed nothing");
    }

    /// One frame with the window's close button pressed; whether the guard cancelled the close.
    fn press_window_close(app: &mut PhotocraftApp) -> bool {
        let mut info = egui::ViewportInfo::default();
        info.events.push(egui::ViewportEvent::Close);
        let mut input = egui::RawInput::default();
        input.viewports.insert(egui::ViewportId::ROOT, info);
        let mut out = egui::Context::default().run_ui(input, |ui| guard_window_close(app, ui.ctx()));
        out.textures_delta.clear();
        out.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose))
    }

    #[test]
    fn window_close_is_cancelled_while_work_is_unsaved() {
        let mut app = app_with_docs(1);
        make_dirty(&mut app, 0);
        assert!(press_window_close(&mut app));
        assert!(app.discard.is_some());
    }

    #[test]
    fn window_close_goes_through_when_nothing_is_unsaved_or_it_was_confirmed() {
        let mut app = app_with_docs(1);
        assert!(!press_window_close(&mut app));
        make_dirty(&mut app, 0);
        app.allow_close = true;
        assert!(!press_window_close(&mut app));
        assert!(app.discard.is_none());
    }
}
