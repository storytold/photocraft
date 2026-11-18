use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

type H = Harness<'static, PhotocraftApp>;

/// One 4 × 2 inch document at 100 ppi, with the rulers in `unit`, under a harness whose passes run the dialog host.
fn harness(unit: &str) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
    app.sync_views();
    with_app(app)
}

fn with_app(app: PhotocraftApp) -> H {
    Harness::<'static, PhotocraftApp>::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui: &mut egui::Ui, app: &mut PhotocraftApp| crate::dialogs::run(app, ui), app)
}

/// Opens New Guide and shows its first pass, which recalls the remembered values.
fn open_new(h: &mut H) -> DialogId {
    let id = open(h.state_mut());
    h.run_steps(3);
    id
}

fn open_edit_dialog(h: &mut H) -> DialogId {
    let id = open_edit(h.state_mut()).unwrap();
    h.run_steps(3);
    id
}

fn set(h: &mut H, id: DialogId, field: &str, value: serde_json::Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
    h.run_steps(3);
}

/// Sets the orientation and the position (a number in the ruler unit).
fn set_guide(h: &mut H, id: DialogId, orientation: &str, position: f64) {
    set(h, id, "orientation", json!(orientation));
    set(h, id, "position", json!(position));
}

fn confirm(h: &mut H, id: DialogId) -> bool {
    let accepted = crate::dialogs::testing::confirm(h.state_mut(), id).is_ok();
    h.run_steps(3);
    accepted
}

fn cancel(h: &mut H, id: DialogId) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, id);
    h.run_steps(3);
}

fn fields(h: &H, id: DialogId) -> serde_json::Value {
    h.state().framework.dialogs.get_by_id::<HostedNewGuideDialog>(id).unwrap().json_fields().unwrap()
}

/// Committed guide positions, excluding any draft preview.
fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.horizontal.clone(), g.vertical.clone())
}

fn color_of(app: &PhotocraftApp, vertical: bool, position: f64) -> Option<[u8; 3]> {
    app.session.active().unwrap().doc.guides.color(vertical, position)
}

fn rgb8(color: &serde_json::Value) -> [u8; 3] {
    ["r", "g", "b"].map(|c| (color[c].as_f64().unwrap() * 255.0).round() as u8)
}

fn magenta() -> serde_json::Value {
    json!({"r": 1.0, "g": 0.0, "b": 1.0, "a": 1.0})
}

fn history(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// The position reads in the ruler unit, and a typed position reaches the command as pixels.
#[test]
fn opens_in_the_ruler_unit_and_takes_photoshop_units() {
    let mut h = harness("cm");
    let id = open_new(&mut h);
    let f = fields(&h, id);
    assert_eq!((f["orientation"].as_str(), f["position"].as_f64()), (Some("horizontal"), Some(0.0)));
    // 5.08 cm at 100 ppi is 200 px.
    set_guide(&mut h, id, "vertical", 5.08);
    assert!(confirm(&mut h, id));
    let vertical = guides(h.state()).1;
    assert!(vertical.len() == 1 && (vertical[0] - 200.0).abs() < 1e-6, "{vertical:?}");
    // The last guide opens next time, in the ruler unit.
    let id = open_new(&mut h);
    let f = fields(&h, id);
    assert_eq!(f["orientation"].as_str(), Some("vertical"));
    assert!((f["position"].as_f64().unwrap() - 5.08).abs() < 1e-6, "{f}");
    cancel(&mut h, id);
    // One inch is 100 px at 100 ppi.
    h.state_mut().run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "inches"})).unwrap();
    let id = open_new(&mut h);
    set_guide(&mut h, id, "horizontal", 1.0);
    assert!(confirm(&mut h, id));
    assert_eq!(guides(h.state()).0, vec![100.0]);
}

/// Percent rulers open at 0 % and a position is a share of the height for a horizontal guide.
#[test]
fn percent_rulers_open_at_zero_percent() {
    let mut h = harness("percent");
    let id = open_new(&mut h);
    assert_eq!(fields(&h, id)["position"].as_f64(), Some(0.0));
    set_guide(&mut h, id, "horizontal", 25.0);
    assert!(confirm(&mut h, id));
    assert_eq!(guides(h.state()).0, vec![50.0], "25% of the 200 px height");
}

/// The guide stores the chosen colour; Cyan is the default and the last choice opens next time.
#[test]
fn guides_store_the_chosen_colour_cyan_by_default() {
    let mut h = harness("pixels");
    let id = open_new(&mut h);
    assert_eq!(rgb8(&fields(&h, id)["color"]), [0x4a, 0xff, 0xff]);
    set_guide(&mut h, id, "vertical", 10.0);
    assert!(confirm(&mut h, id));
    // Stored on the guide itself, so a later change of the preference colour leaves it alone.
    assert_eq!(color_of(h.state(), true, 10.0), Some([0x4a, 0xff, 0xff]));
    let id = open_new(&mut h);
    set_guide(&mut h, id, "vertical", 20.0);
    set(&mut h, id, "color", magenta());
    assert!(confirm(&mut h, id));
    assert_eq!(color_of(h.state(), true, 20.0), Some([0xff, 0x00, 0xff]));
    let id = open_new(&mut h);
    assert_eq!(rgb8(&fields(&h, id)["color"]), [0xff, 0x00, 0xff], "the last colour opens next time");
    set_guide(&mut h, id, "horizontal", 30.0);
    set(&mut h, id, "color", json!({"r": 0xab as f64 / 255.0, "g": 0xcd as f64 / 255.0, "b": 0xef as f64 / 255.0, "a": 1.0}));
    assert!(confirm(&mut h, id));
    assert_eq!(color_of(h.state(), false, 30.0), Some([0xab, 0xcd, 0xef]));
}

/// A remembered pixel position is displayed in each current ruler unit.
#[test]
fn the_last_position_reopens_in_whatever_the_ruler_unit_is_now() {
    let mut h = harness("pixels");
    let id = open_new(&mut h);
    set_guide(&mut h, id, "horizontal", 100.0);
    assert!(confirm(&mut h, id));
    for (unit, shown) in [("pixels", 100.0), ("inches", 1.0), ("mm", 25.4), ("points", 72.0), ("picas", 6.0), ("percent", 50.0)] {
        h.state_mut().run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
        let id = open_new(&mut h);
        let position = fields(&h, id)["position"].as_f64().unwrap();
        assert!((position - shown).abs() < 1e-6, "{unit}: {position}");
        cancel(&mut h, id);
    }
}

#[test]
fn focusing_and_recolouring_preserve_the_exact_guide_position() {
    for (rulers, original) in [("pixels", 12.5), ("cm", 123.45678901234567), ("percent", -1e-100)] {
        for recolour in [false, true] {
            let mut h = harness(rulers);
            h.state_mut().run(COMMAND, json!({"orientation": "vertical", "position": original})).unwrap();
            h.state_mut().run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": original + 1.0})).unwrap();
            assert!(h.state_mut().session.undo());
            crate::guide_select::select(h.state_mut(), true, 0);
            let before = history(&h);
            let id = open_edit_dialog(&mut h);
            h.get_by_label("Position").click();
            h.run_steps(3);
            if recolour {
                set(&mut h, id, "color", magenta());
            }
            let doc = &h.state().session.active().unwrap().doc;
            assert_eq!(edit_preview(h.state(), doc).unwrap().vertical, vec![original]);
            assert_eq!(guides(h.state()).1, vec![original], "preview must not modify the document");
            assert!(confirm(&mut h, id));
            assert_eq!(guides(h.state()).1, vec![original]);
            if recolour {
                assert_eq!(color_of(h.state(), true, original), Some([255, 0, 255]));
                assert_eq!(history(&h), before + 1);
                assert!(h.state_mut().session.undo());
                assert_eq!(guides(h.state()).1, vec![original]);
            } else {
                assert_eq!(history(&h), before);
                assert!(h.state().session.active().unwrap().history.can_redo(), "focus alone must preserve Redo");
            }
        }
    }
}

#[test]
fn retyping_the_rounded_readout_is_an_intentional_position_edit() {
    let mut h = harness("pixels");
    h.state_mut().run(COMMAND, json!({"orientation": "vertical", "position": 12.5})).unwrap();
    crate::guide_select::select(h.state_mut(), true, 0);
    let id = open_edit_dialog(&mut h);
    // The field reads 13; entering 13 is a move to 13.
    set(&mut h, id, "position", json!(13.0));
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(edit_preview(h.state(), doc).unwrap().vertical, vec![13.0]);
    assert!(confirm(&mut h, id));
    assert_eq!(guides(h.state()).1, vec![13.0]);
    assert!(h.state_mut().session.undo());
    assert_eq!(guides(h.state()).1, vec![12.5]);
}

/// Two guides at 10 and 20 px, the first being edited and proposed to move to 30 px.
fn edit_fixture() -> (H, DialogId) {
    let mut h = harness("pixels");
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    crate::guide_select::select(h.state_mut(), true, 0);
    let id = open_edit_dialog(&mut h);
    set_guide(&mut h, id, "vertical", 30.0);
    (h, id)
}

fn draft_dialog(h: &H) -> DialogId {
    h.state().guide_edit.as_ref().expect("a draft is open").dialog
}

/// A document switch rejects the old draft without touching the new document.
#[test]
fn draft_cannot_edit_a_guide_in_another_document() {
    let (mut h, id) = edit_fixture();
    h.state_mut().run("file.new", json!({"width": 400, "height": 200})).unwrap();
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    let before = history(&h);
    confirm(&mut h, id);
    assert_eq!(guides(h.state()).1, vec![10.0]);
    assert_eq!(history(&h), before);
    assert!(h.state().guide_edit.is_none());
}

/// Deleting the target invalidates its draft even when another guide reuses the index.
#[test]
fn draft_cannot_edit_the_guide_that_reused_a_deleted_index() {
    let (mut h, id) = edit_fixture();
    h.state_mut().run("view.deleteGuide", json!({"orientation": "vertical", "index": 0})).unwrap();
    let before = history(&h);
    confirm(&mut h, id);
    assert_eq!(guides(h.state()).1, vec![20.0]);
    assert_eq!(history(&h), before);
}

/// Confirming unchanged values records no command and retains the redo stack.
#[test]
fn draft_unchanged_confirmation_preserves_redo() {
    let (mut h, id) = edit_fixture();
    cancel(&mut h, id);
    h.state_mut().run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 15})).unwrap();
    assert!(h.state_mut().session.undo());
    crate::guide_select::select(h.state_mut(), true, 0);
    let id = open_edit_dialog(&mut h);
    let before = history(&h);
    assert!(confirm(&mut h, id));
    assert_eq!(history(&h), before);
    assert!(h.state().session.active().unwrap().history.can_redo());
}

/// Opening another edit retires the earlier draft; cancellation removes the draft.
#[test]
fn draft_reopening_retires_the_previous_dialog() {
    let (mut h, first) = edit_fixture();
    let second = open_edit_dialog(&mut h);
    assert_eq!(draft_dialog(&h), second);
    let open = h.state().framework.dialogs.open_dialogs();
    assert_eq!(open.len(), 1, "one guide dialog stays open (was {first:?})");
    cancel(&mut h, second);
    assert!(h.state().guide_edit.is_none());
    assert_eq!(guides(h.state()).1, vec![10.0, 20.0]);
}

/// Locking guides invalidates an open edit before it can record a command.
#[test]
fn draft_cannot_commit_after_guides_are_locked() {
    let (mut h, id) = edit_fixture();
    h.state_mut().ui.extras.lock_guides = true;
    confirm(&mut h, id);
    assert_eq!(guides(h.state()).1, vec![10.0, 20.0]);
    assert!(h.state().guide_edit.is_none());
}

fn deny_edits(h: &mut H, message: &'static str) {
    h.state_mut().automation_input = true;
    h.state_mut().services.automation_command = Some(Box::new(move |id, _| if id == "view.editGuide" { Err(message.into()) } else { Ok(()) }));
}

/// A late policy refusal retains the typed proposal and history until a successful retry.
#[test]
fn refused_edit_command_preserves_the_draft_for_retry() {
    let (mut h, id) = edit_fixture();
    cancel(&mut h, id);
    h.state_mut().run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 15})).unwrap();
    assert!(h.state_mut().session.undo());
    crate::guide_select::select(h.state_mut(), true, 0);
    let mut id = open_edit_dialog(&mut h);
    set(&mut h, id, "position", json!(30.0));
    let before = history(&h);
    let revision = h.state().session.active().unwrap().revision;
    deny_edits(&mut h, "guide edits are denied");
    for _ in 0..2 {
        confirm(&mut h, id);
        assert_eq!(guides(h.state()).1, vec![10.0, 20.0]);
        assert_eq!(history(&h), before);
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert!(h.state().session.active().unwrap().history.can_redo());
        assert_eq!(h.state().session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 0);
        id = draft_dialog(&h);
        let again = fields(&h, id);
        assert_eq!((again["position"].as_f64(), again["orientation"].as_str()), (Some(30.0), Some("vertical")));
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(edit_preview(h.state(), doc).unwrap().vertical, vec![30.0, 20.0]);
    }
    h.state_mut().services.automation_command = None;
    assert!(confirm(&mut h, id));
    assert_eq!(guides(h.state()).1, vec![30.0, 20.0]);
    assert_eq!(history(&h), before + 1);
    assert_eq!(h.state().session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 1);
    assert!(h.state().guide_edit.is_none());
    assert!(h.state_mut().session.undo());
    assert_eq!(guides(h.state()).1, vec![10.0, 20.0]);
    assert!(h.state_mut().session.redo());
    assert_eq!(guides(h.state()).1, vec![30.0, 20.0]);
}

/// Restoring a refused proposal must not bind it to a different document at retry time.
#[test]
fn refused_edit_draft_cannot_follow_a_document_switch() {
    let (mut h, id) = edit_fixture();
    deny_edits(&mut h, "denied");
    confirm(&mut h, id);
    let id = draft_dialog(&h);
    h.state_mut().services.automation_command = None;
    h.state_mut().run("file.new", json!({"width": 400, "height": 200})).unwrap();
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    let before = history(&h);
    confirm(&mut h, id);
    assert_eq!(guides(h.state()).1, vec![10.0]);
    assert_eq!(history(&h), before);
    assert!(h.state().guide_edit.is_none());
    assert!(!h.state().framework.dialogs.is_open::<HostedNewGuideDialog>());
}
