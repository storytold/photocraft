use super::open::{open, open_smart_filter};
use super::{LayerId, PhotocraftApp, Value, describe};
#[cfg(not(target_arch = "wasm32"))]
use super::{dismiss, take_with};

/// The menu id of the RAW Develop workspace.
const ID: &str = "filter.cameraRaw";

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    #[cfg(not(target_arch = "wasm32"))]
    let request = workspace_request;
    #[cfg(target_arch = "wasm32")]
    let request = browser_workspace_request;
    menu_with_workspace_request(app, ctx, id, params, request)
}

fn menu_with_workspace_request(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    id: &str,
    params: &Value,
    request: impl FnOnce(&mut PhotocraftApp, &egui::Context, &Value) -> Option<Result<Value, String>>,
) -> Option<Result<Value, String>> {
    if id != ID {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("RAW Develop params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(open(app, ctx).map(|_| describe(app)));
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(describe(app))
        })();
        return Some(opened);
    }
    request(app, ctx, params)
}

#[cfg(any(target_arch = "wasm32", test))]
fn browser_workspace_request(_app: &mut PhotocraftApp, _ctx: &egui::Context, params: &Value) -> Option<Result<Value, String>> {
    // A workspace control must never fall through to an engine edit with default parameters.
    params.get("ui").map(|_| Err("Workspace controls are unavailable in the browser".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn workspace_request(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Option<Result<Value, String>> {
    let ui = params.get("ui")?;
    app.raw_reply = None;
    let was_closed = app.raw.is_none();
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let applied = crate::control::dialogs::workspace_ui(app, ctx, ID, ui);
    // A commit, cancel or done in the request lands now, so its reply is this call's: the confirm or cancel ran the
    // result pump, which left the reply.
    let ended = take_with(app, ctx).or_else(|| app.raw_reply.take());
    if let Err(e) = applied {
        // A request that opened a closed RAW Develop and was refused leaves it closed again.
        if was_closed {
            dismiss(app, ctx);
        }
        return Some(Err(e));
    }
    Some(ended.unwrap_or_else(|| Ok(describe(app))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn browser_workspace_controls_are_consumed_without_editing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let document = app.session.active().unwrap().doc.clone();
        let before = app.session.active().unwrap().history.past_len();
        for ui in [Value::Null, json!({}), json!({"cancel": true}), json!({"commit": true})] {
            let reply = menu_with_workspace_request(&mut app, &ctx, ID, &json!({"ui": ui}), browser_workspace_request);
            assert!(matches!(reply, Some(Err(ref error)) if error.contains("unavailable in the browser")), "{reply:?}");
            assert!(app.raw.is_none());
            assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), before);
        }
        assert!(menu_with_workspace_request(&mut app, &ctx, ID, &json!({"exposure": 0.5}), browser_workspace_request).is_none());
        assert!(menu_with_workspace_request(&mut app, &ctx, "unrelated", &json!({"ui": {}}), browser_workspace_request).is_none());
        assert!(menu_with_workspace_request(&mut app, &ctx, ID, &json!({"smartFilter": {}, "ui": {}}), browser_workspace_request).unwrap().is_err());
        assert!(app.raw.is_none(), "mixed smartFilter/control requests still refuse before opening");
        assert!(menu_with_workspace_request(&mut app, &ctx, ID, &json!({}), browser_workspace_request).unwrap().is_ok());
        assert!(app.raw.is_some(), "ordinary menu invocation still opens the workspace");
        let reply = menu_with_workspace_request(&mut app, &ctx, ID, &json!({"ui": {"cancel": true}}), browser_workspace_request);
        assert!(matches!(reply, Some(Err(_))));
        assert!(app.raw.is_some(), "refused controls leave an open workspace untouched");
        assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
    }
}
