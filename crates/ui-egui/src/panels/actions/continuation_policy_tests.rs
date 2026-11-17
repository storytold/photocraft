//! Exercise the actual Layer Style confirmation consumer at a paused-step boundary.
use super::*;

fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
    Ok(())
}

fn deny_style(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id.starts_with("layer.layerStyle.") { Err(photocraft_engine::EngineError::Other("layer style denied".into())) } else { Ok(()) }
}

fn other_allow(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "unrelated.denied.command" { Err(photocraft_engine::EngineError::Other("unrelated policy".into())) } else { Ok(()) }
}

fn fixture(captured: Option<Gate>) -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":8,"height":6})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session.actions.list.push(Action::new("Style owner", Vec::new()));
    let frame = Frame::library(&app.session, &json!({"action":0})).unwrap();
    let dialog = crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap();
    app.session.authorize = captured;
    let token = pause_frame(&mut app, &frame, 0, dialog).unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert(RESUME.into(), json!(token));
    (app, dialog)
}

#[test]
fn layer_style_confirmation_retains_both_policies_and_restores_on_denial() {
    for mode in 0..3 {
        let captured: Gate = if mode == 0 { deny_style } else { allow };
        let live: Gate = if mode == 1 {
            deny_style
        } else if mode == 2 {
            other_allow
        } else {
            allow
        };
        let (mut app, dialog) = fixture(Some(captured));
        app.session.authorize = Some(live);
        app.actions_runtime.step_gate = Some(None);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let error = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
        assert!(error.contains(if mode == 2 { "authorization context changed" } else { "layer style denied" }), "{error}");
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert!(app.paused_playbacks.paused.is_empty());
        assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), live));
        assert!(matches!(app.actions_runtime.step_gate, Some(None)));
        assert!(app.ui.status_error);
    }
}

#[test]
fn layer_style_confirmation_allows_matching_policy_and_manual_dialog() {
    for paused in [false, true] {
        let (mut app, dialog) = fixture(Some(allow));
        if !paused {
            app.ui.dialog_mut(dialog).unwrap().fields.remove(RESUME);
            app.paused_playbacks.paused.clear();
        }
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let state = app.session.active().unwrap();
        assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().effects.items.len(), 1);
        assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), allow as Gate));
        assert!(app.actions_runtime.step_gate.is_none());
    }
}
