//! Deferred questions and destinations retain both captured and current policies.
use super::*;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
    Ok(())
}

fn deny(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "actions.delete" || id == "actions.layout.classic" {
        Err(photocraft_engine::EngineError::Other("deferred operation denied".into()))
    } else {
        Ok(())
    }
}

#[test]
fn delete_question_cannot_replace_captured_or_live_policy() {
    for service in [false, true] {
        for captured_denies in [false, true] {
            let mut app = app();
            app.session.actions.list.push(Action::new("Retain me", Vec::new()));
            let captured: Gate = if captured_denies { deny } else { allow };
            let live: Gate = if captured_denies { allow } else { deny };
            app.session.authorize = Some(captured);
            let dialog = ask_delete(&mut app, Some(0), None).unwrap()["dialog"].as_u64().unwrap();
            app.session.authorize = if service { None } else { Some(live) };
            app.automation_input = service;
            app.services.automation_authorize = service.then_some(live);
            let error = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
            assert!(error.contains("authorization context changed"), "{error}");
            assert_eq!(app.session.actions.list.len(), 1);
            assert_eq!(app.session.actions.list[0].name, "Retain me");
            assert!(app.actions_runtime.delete.is_none());
            if service {
                assert!(app.session.authorize.is_none());
            } else {
                assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), live));
            }
        }
    }
}

#[test]
fn parked_navigation_cannot_replace_captured_or_live_policy() {
    for captured_denies in [false, true] {
        let mut app = app();
        let captured: Gate = if captured_denies { deny } else { allow };
        let live: Gate = if captured_denies { allow } else { deny };
        app.actions_runtime.pending = Some(("actions.layout.classic".into(), json!({}), Some(captured)));
        app.session.authorize = Some(live);
        assert!(finish_navigation(&mut app).is_err_and(|error| error.contains("authorization context changed")));
        assert_eq!(app.ui.actions_layout, ActionsLayout::Modern);
        assert!(app.actions_runtime.pending.is_none());
        assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), live));
    }
}

#[test]
fn matching_deferred_policy_still_deletes_and_navigates_and_restores_errors() {
    let mut app = app();
    app.session.authorize = Some(allow);
    app.session.actions.list.push(Action::new("Delete me", Vec::new()));
    let dialog = ask_delete(&mut app, Some(0), None).unwrap()["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.actions.list.is_empty());
    app.actions_runtime.pending = Some(("actions.layout.classic".into(), json!({}), Some(allow)));
    finish_navigation(&mut app).unwrap();
    assert_eq!(app.ui.actions_layout, ActionsLayout::Classic);
    app.actions_runtime.pending = Some(("not.a.command".into(), json!({}), Some(allow)));
    assert!(finish_navigation(&mut app).is_err());
    assert!(app.actions_runtime.pending.is_none());
    assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), allow as Gate));
}
