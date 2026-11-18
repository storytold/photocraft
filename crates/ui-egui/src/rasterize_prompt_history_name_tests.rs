//! #12888: OK on the type-layer prompt a painting tool raises adds one history state named
//! "Rasterize Layer", as the reference application does; Layer › Rasterize › Type, the filter
//! prompt and the shape, Smart Object and fill prompts keep their step names.

use serde_json::{Value, json};

use super::tests::{BLUR, active_content, answer, app_with, click, menu, past, prompt};
use super::*;

fn step_name(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.undo_label().map(str::to_string)
}

fn active_layer_content(app: &PhotocraftApp) -> photocraft_doc::LayerContent {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
}

/// A `kind` layer whose prompt is raised by a press of `tool`, or by Gaussian Blur from the menu
/// when `tool` is `None`, and answered with OK. Returns the app and the history length before.
fn answered(kind: Kind, tool: Option<Tool>) -> (PhotocraftApp, usize) {
    let mut app = app_with(kind);
    let before = past(&app);
    match tool {
        Some(tool) => {
            app.ui.tool = tool;
            click(&mut app, 30.0, 20.0);
        }
        None => {
            menu(&mut app, BLUR).unwrap();
        }
    }
    let id = prompt(&app).unwrap_or_else(|| panic!("{kind:?}/{tool:?}: the prompt opens"));
    answer(&mut app, id);
    assert!(prompt(&app).is_none(), "{kind:?}/{tool:?}: OK closes the prompt");
    (app, before)
}

/// Each trigger's step name: a painting tool on a type layer is "Rasterize Layer", the filter prompt
/// keeps "Rasterize Type" (#4303, not measured), and the other kinds keep their command's own step.
#[test]
fn ok_adds_one_rasterize_state_named_for_its_trigger() {
    for tool in [Tool::PatternStamp, Tool::Brush, Tool::HistoryBrush] {
        let (app, before) = answered(Kind::Type, Some(tool));
        assert_eq!(past(&app), before + 1, "{tool:?}: exactly one state");
        assert_eq!(step_name(&app).as_deref(), Some("Rasterize Layer"), "{tool:?}");
        assert_eq!(active_content(&app), "Pixel", "{tool:?}: the layer is rasterized");
        // Nothing painted: the pixels are the plain rasterized type.
        let mut plain = app_with(Kind::Type);
        plain.run("layer.rasterize.type", json!({})).unwrap();
        assert!(active_layer_content(&app) == active_layer_content(&plain), "{tool:?}: the press painted nothing");
        // The name belongs to the history state: undone, it is the state to redo.
        let mut app = app;
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.redo_labels().next(), Some("Rasterize Layer"), "{tool:?}");
    }
    for kind in [Kind::Shape, Kind::SmartObject, Kind::Fill] {
        let mut direct = app_with(kind);
        direct.run(kind.command(), json!({})).unwrap();
        let (app, before) = answered(kind, Some(Tool::Brush));
        assert_eq!(past(&app), before + 1, "{kind:?}");
        assert_eq!(step_name(&app), step_name(&direct), "{kind:?}");
        assert_ne!(step_name(&app).as_deref(), Some("Rasterize Layer"), "{kind:?}");
    }
    let (app, before) = answered(Kind::Type, None);
    assert_eq!(past(&app), before + 1, "the filter prompt's Rasterize");
    assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"));
}

#[test]
fn layer_rasterize_type_keeps_its_step_name() {
    let mut app = app_with(Kind::Type);
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "layer.rasterize.type", json!({})).unwrap();
    assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"), "the menu");

    let ctx = app.ctx().clone();
    for (method, params) in [("ui.menu.invoke", json!({"id": "layer.rasterize.type"})), ("engine.execute", json!({"command": "layer.rasterize.type"}))] {
        let mut app = app_with(Kind::Type);
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        let crate::control::Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("{method}: deferred") };
        assert_ne!(r.get("ok"), Some(&Value::Bool(false)), "{method}: {r}");
        assert_eq!(active_content(&app), "Pixel", "{method}");
        assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"), "{method}");
    }
}
