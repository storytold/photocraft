use super::*;
use crate::canvas_tool_menu::MenuKind;
use craft::automation::actions::{Action, ActionList, ActionStep};
use craft::color_ui::ui::{SampleSize, SampleSource};
use craft::keymap::CommandRef;
use photocraft_engine::actions_cmds::ActionsExt;

/// An action named `name` whose steps run the given commands.
// PLAN: assumed `app.session.actions.list` is craft-automation's `ActionList`
fn action(name: &str, steps: Vec<(&str, Value)>) -> Action {
    Action::new(name, steps.into_iter().map(|(id, params)| ActionStep::new(id, params)).collect())
}

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// `ui.pointer` as the canvas takes it: the request is handled on the app's own context
/// (`app.ctx()`; the tool host queues injected input per context), then canvas passes run until
/// the window host reports its queue idle. `ToolHost::inject` asks the host for its pass, so
/// `injected_idle` stays false until the canvas took the input. A held key is released on a
/// later pass, so the loop runs to idle rather than for one pass.
fn pointer(app: &mut PhotocraftApp, params: Value) -> Value {
    let ctx = app.ctx().clone();
    // The pointer maps document points through the view the canvas last showed, so the canvas shows one first.
    if crate::canvas::document_display::active_view(app).is_none() {
        crate::canvas::testing::frame(app);
    }
    let reply = call(app, &ctx, "ui.pointer", params);
    for _ in 0..8 {
        if nothing_queued(&ctx) {
            break;
        }
        crate::canvas::testing::frame(app);
    }
    assert!(nothing_queued(&ctx), "the canvas took all the queued input: {reply}");
    reply
}

/// The `ui.inspect` `dialogs` list: the hosted dialogs in stack order, the top last.
fn dialogs(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<Value> {
    call(app, ctx, "ui.inspect", json!({}))["result"]["dialogs"].as_array().cloned().unwrap_or_default()
}

/// The top hosted dialog as `ui.inspect` lists it, or `null` when none is open.
fn top_dialog(app: &mut PhotocraftApp, ctx: &egui::Context) -> Value {
    dialogs(app, ctx).last().cloned().unwrap_or(Value::Null)
}

/// #3674: a guessed name gets a pointer to the right one, in both directions; and every
/// method listed for those hints is served.
#[test]
fn unknown_names_suggest_the_right_method_or_command() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let mut error = |method: &str, params: Value| {
        let reply = call(&mut app, &ctx, method, params);
        assert_eq!(reply["ok"], false, "{reply}");
        reply["error"].as_str().unwrap_or_default().to_owned()
    };
    let e = error("engine.execute", json!({"command": "engine.commands"}));
    assert!(e.contains("did you mean `command.list`") && e.contains("protocol method"), "{e}");
    let e = error("engine.execute", json!({"command": "ui.inspect"}));
    assert!(e.contains("`ui.inspect` is a protocol method"), "{e}");
    let e = error("engine.execute", json!({"command": "edit.undoo"}));
    assert!(e.contains("did you mean `edit.undo`"), "{e}");
    let e = error("ui.inspct", json!({}));
    assert!(e.contains("did you mean `ui.inspect`"), "{e}");
    let e = error("edit.undo", json!({}));
    assert!(e.contains("engine.execute {\"command\": \"edit.undo\"}"), "{e}");
    for m in METHODS {
        let (req, _rx) = ControlRequest::new(*m, json!({}));
        if let Reply::Done(v) = handle(&mut app, &ctx, &req) {
            assert!(!v.to_string().contains("unknown method"), "{m} is listed but not served: {v}");
        }
    }
}

#[test]
fn shift_right_brush_does_not_open_presets_or_paint() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = crate::state::Tool::Brush;
    let before = app.session.journal.len();
    pointer(
        &mut app,
        json!({"button": "right", "shift": true,
        "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "up", "x": 20, "y": 20}]}),
    );
    let ctx = app.ctx().clone();
    assert!(inspect(&app, &ctx)["brushPicker"].is_null(), "Shift+right opens Mode, not Brush Presets");
    assert!(!crate::canvas::testing::in_gesture(&app));
    assert!(app.session.journal.iter().skip(before).all(|(id, _)| id != "paint.stroke"), "no stroke is painted");
    assert_eq!(app.session.journal.len(), before, "opening a menu does not paint or journal");
}

#[test]
fn perf_output_includes_slow_frames() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"], json!([]));
    app.slow_frames.set_threshold_ms(0.0);
    app.run("file.new", json!({"width": 64, "height": 32})).unwrap();
    app.finish_frame(&ctx, 1.0);
    let frames = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"].clone();
    let last = frames.as_array().and_then(|f| f.last()).expect("a slow frame").clone();
    assert_eq!(last["command"], "file.new", "{last}");
    for key in ["t", "frameMs", "inputMs", "commandMs", "damageMs", "compositeMs", "uploadMs", "gpuMs", "presentMs", "refresh", "refreshPx", "path"] {
        assert!(last.get(key).is_some(), "{key} in {last}");
    }
    // A nested run (`actions.play` → its view steps through `App::run`) is inside the outer
    // call's time: the frame counts the outer command once, not the steps again.
    app.session.actions_mut().list.push(action("Zoom", vec![("view.zoomIn", json!({})), ("view.zoomOut", json!({}))])).unwrap();
    let played = app.run("actions.play", json!({"action": "Zoom"})).unwrap();
    assert_eq!(played["ran"], 2, "{played}");
    let outer_ms = app.perf.command_ms;
    app.finish_frame(&ctx, 1.0);
    let frames = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"].clone();
    let last = frames.as_array().and_then(|f| f.last()).expect("a slow frame").clone();
    assert_eq!(last["command"], "actions.play", "{last}");
    let command_ms = last["commandMs"].as_f64().unwrap();
    assert!((command_ms - outer_ms).abs() < 1e-9, "the outer run's {outer_ms} ms, once: {last}");
}

#[test]
fn full_window_workspaces_are_modal_for_menu_invocations() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#808080"})).unwrap();
    app.run("select.rect", json!({"x":8,"y":8,"width":8,"height":8})).unwrap();
    for workspace in ["filter.liquify", "edit.contentAwareFill", "filter.filterGallery", "filter.adaptiveWideAngle", "select.selectAndMask"] {
        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":workspace}));
        assert_eq!(opened["ok"], true, "{workspace}: {opened}");
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(workspace));
        let doc = app.session.active().unwrap().doc.id;
        for request in [
            json!({"id":"window.document.0"}),
            json!({"id":"select.all"}),
            json!({"id":"view.zoomIn"}),
            json!({"id":"filter.cameraRaw"}),
            json!({"id":"filter.liquify"}),
            json!({"id":workspace,"params":{"ui":null}}),
        ] {
            let blocked = call(&mut app, &ctx, "ui.menu.invoke", request.clone());
            assert_eq!(blocked["ok"], false, "{workspace}: {request} → {blocked}");
            assert!(!crate::menus::allows(&app, &CommandRef::new(request["id"].as_str().unwrap())), "{workspace}: native menu greys {request}");
        }
        let listed = call(&mut app, &ctx, "ui.menu.list", json!({}));
        let enabled = |id: &str| listed["result"].as_array().unwrap().iter().find(|it| it["id"] == id).map(|it| it["enabled"].clone());
        assert_eq!(enabled("select.all"), Some(json!(false)), "{workspace}: ui.menu.list greys what invoke refuses");
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(workspace), "{workspace} stays open with its work");
        assert_eq!(app.session.active().unwrap().doc.id, doc);
        // An open dialog still takes precedence over the workspace's own requests.
        assert_eq!(call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "about"}))["ok"], true);
        let about = top_dialog(&mut app, &ctx)["id"].clone();
        let cancel = json!({"id":workspace,"params":{"ui":{"cancel":true}}});
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel.clone())["ok"], false);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": about}))["ok"], true);
        let cancelled = call(&mut app, &ctx, "ui.menu.invoke", cancel);
        assert_eq!(cancelled["ok"], true, "{workspace}: {cancelled}");
        assert_eq!(crate::distort_ui::modal_workspace(&app), None);
        assert!(crate::menus::allows(&app, &CommandRef::new("select.all")), "{workspace}: menus are free again");
        let listed = call(&mut app, &ctx, "ui.menu.list", json!({}));
        assert!(
            listed["result"].as_array().unwrap().iter().any(|it| it["id"] == "select.all" && it["enabled"] == true),
            "{workspace}: listed as enabled again"
        );
    }
    // Programmatic edits cannot bypass the active workspace ownership gate.
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.liquify"}))["ok"], true);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let executed = call(&mut app, &ctx, "engine.execute", json!({"command":"select.all"}));
    assert_eq!(executed["ok"], false, "{executed}");
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.liquify","params":{"ui":{"cancel":true}}}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "engine.execute", json!({"command":"select.all"}))["ok"], true);
}

#[test]
fn layer_style_picker_control_binds_previews_confirms_and_cancels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    let parent = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"layerStyle","effect":"gradientOverlay"}))["result"]["dialog"].clone();
    assert!(parent.is_string(), "a hosted dialog's id is the string ui.inspect lists: {parent}");
    let parent_fields = |app: &mut PhotocraftApp| {
        let listed = dialogs(app, &ctx);
        listed.into_iter().find(|d| d["id"] == parent).expect("the Layer Style dialog is listed")["fields"].clone()
    };
    let before = parent_fields(&mut app);
    // The Gradient Overlay's colours are its stops (#5177): `stop` picks one.
    for stop in [0, 1] {
        let reply = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":stop}));
        assert_eq!(reply["ok"], true, "{reply}");
        let picker = top_dialog(&mut app, &ctx)["id"].clone();
        assert_ne!(picker, parent, "the picker is its own hosted dialog above the Layer Style dialog");
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":parent}))["ok"], false);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":picker,"field":"color","value":"#123456"}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":picker}))["ok"], true);
        assert_eq!(parent_fields(&mut app)["effects"][0]["params"]["stops"][stop][1], "#123456");
    }
    assert_ne!(parent_fields(&mut app), before);
    let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":"x"}));
    assert_eq!(bad["ok"], false, "{bad}");
    let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":9}));
    assert_eq!(bad["ok"], false, "{bad}");
    call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":1}));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog":parent}))["ok"], true);
    assert!(dialogs(&mut app, &ctx).is_empty());
}

#[test]
fn layer_style_picker_control_rejects_bad_targets_without_opening_tool_picker() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    let parent = json!(crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap().value().to_string());
    for params in [
        json!({"kind":"colorPicker","parent":null,"effect":"fx1","field":"color"}),
        json!({"kind":"colorPicker","parent":"bad","effect":"fx1","field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":3,"field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"missing","field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"opacity"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"to"}),
        json!({"kind":"colorPicker","effect":"fx1","field":"color"}),
    ] {
        let before = dialogs(&mut app, &ctx);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.open", params)["ok"], false);
        assert_eq!(dialogs(&mut app, &ctx), before);
    }
}

#[test]
fn shape_stroke_defaults_are_drivable_and_validate_atomically() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let result =
        call(&mut app, &ctx, "ui.set", json!({"tool":"Triangle", "shapeStroke":{"width":5,"cap":"round","join":"bevel","dashes":[0,2,4,1],"dashOffset":-0.5}}));
    assert_eq!(result["ok"], true);
    assert_eq!(app.ui.tool_options.stroke_width, 5.0);
    assert_eq!(app.ui.tool_options.shape_stroke.dashes, [0.0, 2.0, 4.0, 1.0]);
    let before = app.ui.clone();
    let bad = call(&mut app, &ctx, "ui.set", json!({"tool":"Rectangle","shapeStroke":{"dashes":[0,0]}}));
    assert_eq!(bad["ok"], false);
    assert_eq!(app.ui, before);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"shapeStroke":{"colour":"#ff0000"}}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"shapeStroke":null}))["ok"], true);
    assert_eq!(app.ui.tool_options.stroke_width, 0.0);
}

/// A dialog on the framework's `DialogHost` is set, confirmed and cancelled through ui.dialog.* by
/// the id ui.inspect lists, whatever its type: a parameter dialog, then a prompt by each route.
#[test]
fn framework_dialogs_are_set_confirmed_and_cancelled_by_id() {
    use craft::ui::contract::Bind;
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let top = |app: &mut PhotocraftApp| call(app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"].as_array().and_then(|d| d.last().cloned());
    let ok = |app: &mut PhotocraftApp, method: &str, params: Value| call(app, &ctx, method, params)["ok"].clone();

    app.framework.dialogs.open(HostedParamDialog::new(&ctx, "gaussianBlur", DialogFields::from_pairs([("radius", 1.0)]), |fields| {
        ParamDialog::new("gaussianBlur", "Gaussian Blur", Param::parse_list(r#"{"radius":0.1..1000=1}"#)).values(Bind::bound(fields))
    }));
    let blur = top(&mut app).expect("the blur dialog is listed")["id"].clone();
    assert_eq!(ok(&mut app, "ui.dialog.set", json!({"dialog": blur, "field": "radius", "value": 4.5})), true);
    assert_eq!(ok(&mut app, "ui.dialog.set", json!({"dialog": blur, "field": "angle", "value": 1})), false, "no such field");
    assert_eq!(top(&mut app).expect("still open")["fields"]["radius"], 4.5);
    assert_eq!(ok(&mut app, "ui.dialog.confirm", json!({"dialog": blur})), true);
    // The request runs the answer itself (the family takes the result), so the dialog is gone.
    assert!(top(&mut app).is_none(), "the confirmed dialog closed");
    assert_eq!(ok(&mut app, "ui.dialog.cancel", json!({"dialog": blur})), false, "nothing left to cancel");

    // A real prompt family answers each route: the mask-trash prompt of a layer mask. A refused choice
    // leaves the prompt open, Cancel keeps the mask, Alternate deletes it.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let layer = app.session.active().and_then(|state| state.active_layer).expect("an active layer");
    let has_mask = |app: &PhotocraftApp| app.session.active().and_then(|s| s.doc.layer(layer)).is_some_and(|l| l.mask.is_some());
    assert!(has_mask(&app));
    // (method, extra params, accepted, the mask survives)
    let routes: [(&str, Value, bool, bool); 3] = [
        ("ui.dialog.confirm", json!({"choice": "maybe"}), false, true),
        ("ui.dialog.cancel", json!({}), true, true),
        ("ui.dialog.confirm", json!({"choice": "delete"}), true, false),
    ];
    for (method, extra, accepted, kept) in routes {
        let (command, mark) = crate::mask_trash_prompt::drop_action(layer.0, crate::mask_thumbs_ui::MaskKind::Pixel);
        crate::mask_trash_prompt::intercept(&mut app, &command, &mark).expect("the drop opens the prompt").unwrap();
        let prompt = top(&mut app).expect("the prompt is listed")["id"].clone();
        let mut params = extra.clone();
        params["dialog"] = prompt.clone();
        assert_eq!(ok(&mut app, method, params), accepted, "{method} {extra}");
        assert_eq!(has_mask(&app), kept, "{method} {extra}");
        if accepted {
            assert!(top(&mut app).is_none(), "{method} {extra}: an answered prompt closed");
        } else {
            assert_eq!(ok(&mut app, "ui.dialog.cancel", json!({"dialog": prompt})), true);
        }
        assert_eq!(ok(&mut app, "ui.dialog.cancel", json!({"dialog": prompt})), false, "nothing left to cancel");
    }
}

/// `"top"` names the topmost hosted dialog in `ui.dialog.*` (dl.K9), and a field keyed as the
/// production dialog keys it (cx.P7) reaches the hosted dialog's value.
#[test]
fn a_value_reaches_the_top_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let opened = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "filter.blur.gaussianBlur"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let set = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "top", "field": "radius", "value": 7.5}));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(top_dialog(&mut app, &ctx)["fields"]["radius"], 7.5);
    let by_id = top_dialog(&mut app, &ctx)["id"].clone();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": by_id}))["ok"], true);
    for dialog in [json!("top"), json!("no such dialog")] {
        let refused = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": "radius", "value": 2.0}));
        assert_eq!(refused["ok"], false, "{dialog}: {refused}");
    }
}

/// ui.inspect lists the hosted dialogs in their stack order, top last, addressed by one `dialog`
/// id; closing the top one gives the input back to the one below, and an open hosted dialog
/// counts as an open dialog.
#[test]
fn inspect_lists_hosted_dialogs_in_stack_order() {
    use craft::ui::contract::Bind;
    use craft::ui::dialog::{PromptKind, StandardPrompt};
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    // Each dialog takes its place when it opens, with no pass between the opens. The host keeps one dialog
    // per type, so the two are of different types.
    let lower = app.framework.dialogs.open(StandardPrompt::new(PromptKind::TrashMask));
    let upper = app.framework.dialogs.open(HostedParamDialog::new(&ctx, "gaussianBlur", DialogFields::from_pairs([("radius", 1.0)]), |fields| {
        ParamDialog::new("gaussianBlur", "Gaussian Blur", Param::parse_list(r#"{"radius":0.1..1000=1}"#)).values(Bind::bound(fields))
    }));
    let (lower_id, upper_id) = (json!(lower.value().to_string()), json!(upper.value().to_string()));
    let listed = |app: &mut PhotocraftApp| -> Vec<Value> { dialogs(app, &ctx).iter().map(|d| d["id"].clone()).collect() };
    assert_eq!(listed(&mut app), [lower_id.clone(), upper_id.clone()]);
    assert!(app.framework.dialogs.stack().is_top(upper));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": upper_id}))["ok"], true);
    assert_eq!(listed(&mut app), std::slice::from_ref(&lower_id));
    assert!(app.framework.dialogs.stack().is_top(lower), "the dialog below takes the input again");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": lower_id}))["ok"], true);
    assert!(listed(&mut app).is_empty());
    assert!(!app.dialog_open());
    app.framework.dialogs.open(StandardPrompt::new(PromptKind::TrashMask));
    assert!(app.dialog_open(), "a hosted dialog counts as an open dialog");
}

#[test]
fn shell_dialogs_are_listed_and_driven_through_ui_dialog() {
    // #1004: Window › Workspace and View dialogs are hosted dialogs, listed by ui.inspect and
    // driven by ui.dialog.* on the id it lists.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    // Listed, then cancelled.
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({})).unwrap();
    let d = top_dialog(&mut app, &ctx);
    assert_eq!(d["key"].as_str(), Some("workspace.new"));
    let id = d["id"].clone();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
    assert!(dialogs(&mut app, &ctx).is_empty());
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], false, "nothing left to cancel");
    // A field set over control reaches OK.
    crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", json!({})).unwrap();
    assert!(app.ui.view.show.notes);
    let id = top_dialog(&mut app, &ctx)["id"].clone();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "notes", "value": false}))["ok"], true);
    assert_eq!(top_dialog(&mut app, &ctx)["fields"]["notes"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    assert!(!app.ui.view.show.notes && dialogs(&mut app, &ctx).is_empty());
    // An OK that fails keeps the dialog open and says why.
    crate::menus::invoke(&mut app, &ctx, "view.pixelAspectRatio.custom", json!({})).unwrap();
    let id = top_dialog(&mut app, &ctx)["id"].clone();
    let r = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": id}));
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap_or("").contains("name"), "{r}");
    assert_eq!(dialogs(&mut app, &ctx).len(), 1);
    // Apply is Preferences-only; an id no dialog has is refused.
    assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "0"}))["ok"], false);
    assert_eq!(dialogs(&mut app, &ctx).len(), 1);
}

#[test]
fn opening_new_document_through_control_matches_clipboard_image_size() {
    // #2034: every way of opening New Document uses the clipboard-aware initial fields.
    let clipboard = std::sync::Arc::new(std::sync::Mutex::new(Some((100, 200, vec![255; 100 * 200 * 4]))));
    let image = clipboard.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services { clipboard_get_image: Some(Box::new(move || Ok(image.lock().unwrap().clone()))), ..Default::default() },
    );
    let ctx = app.ctx().clone();
    let opened = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"newDocument"}));
    assert_eq!(opened["ok"], true);
    let id = opened["result"]["dialog"].as_str().expect("the hosted dialog's id is returned").to_owned();
    let listed = dialogs(&mut app, &ctx);
    let dialog = listed.iter().find(|d| d["id"] == id.as_str()).expect("the dialog is listed by that id");
    let size = &dialog["fields"]["size"];
    assert_eq!((size["width"].as_u64(), size["height"].as_u64()), (Some(100), Some(200)));
}

#[test]
fn right_pointer_opens_agent_visible_selection_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    let result = pointer(
        &mut app,
        json!({
            "tool": "RectMarquee", "button": "secondary",
            "events": [{"kind": "down", "x": 4, "y": 4}, {"kind": "up", "x": 4, "y": 4}]
        }),
    );
    assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    assert!(entries.iter().any(|entry| entry.get("id") == Some(&json!("select.inverse")) && entry.get("enabled") == Some(&json!(true))));
    assert!(app.session.active().unwrap().doc.selection.is_some(), "right-click must not edit selection");
}

#[test]
fn agent_right_click_with_the_zoom_tool_opens_and_runs_the_view_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let result = pointer(&mut app, json!({"tool": "Zoom", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]}));
    assert_eq!(result["ok"], true);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().map(|m| m.pos), Some(screen_point(&app, 8.0, 8.0)));
    assert!(app.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::View)));
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    let ids: Vec<_> = entries.iter().filter_map(|row| row.get("id").and_then(Value::as_str)).collect();
    assert_eq!(ids, ["view.fitOnScreen", "view.fillScreen", "view.actualPixels", "view.twoHundredPercent", "view.printSize", "view.zoomIn", "view.zoomOut"]);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"select.all"}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"view.twoHundredPercent"}))["ok"], true);
    // The canvas runs the view command the row asked of it in its next frame.
    crate::canvas::testing::frame(&mut app);
    assert_eq!(app.ui.views[0].zoom, 2.0);
    assert!(app.ui.canvas_tool_menu.is_none());
}

/// #11271: an agent's right-click with Space held is the held Hand's: the view menu, whatever the
/// tool; with ⌘ as well (the held Zoom) it is not.
#[test]
fn agent_space_right_click_opens_the_hand_view_menu() {
    for tool in [crate::state::Tool::Move, crate::state::Tool::Brush, crate::state::Tool::RectMarquee, crate::state::Tool::Pen] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        let click = |space, command| json!({"space": space, "command": command, "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]});
        assert_eq!(pointer(&mut app, click(true, false))["ok"], true);
        let menu = app.ui.canvas_tool_menu.clone().unwrap_or_else(|| panic!("{tool:?}: no menu"));
        assert!(crate::canvas_tool_menu::rows(&menu) == crate::canvas_tool_menu::HAND_MENU, "{tool:?}");
        assert_eq!(menu.under, Some(tool), "{tool:?}");
        assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"view.twoHundredPercent"}))["ok"], true, "{tool:?}");
        // The canvas runs the view command the row asked of it in its next frame.
        crate::canvas::testing::frame(&mut app);
        assert_eq!(app.ui.views[0].zoom, 2.0, "{tool:?}");
        assert_eq!(app.ui.tool, tool);
        app.ui.canvas_tool_menu = None;
        assert_eq!(pointer(&mut app, click(true, true))["ok"], true, "{tool:?}");
        // ⌘ still makes it a ⌘/Ctrl+right-click, which lists the layers under the pointer (#307):
        // the Zoom held by ⌘Space has no menu of its own, so the only menu is the layer list.
        assert!(matches!(app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::Layers { .. })), "{tool:?}: ⌘+right-click lists the layers");
        app.ui.canvas_tool_menu = None;
        // ⌥Space holds the Zoom too: no view menu, and not the Move tool's layer list.
        let alt = json!({"space": true, "alt": true, "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]});
        assert_eq!(pointer(&mut app, alt)["ok"], true, "{tool:?}");
        assert!(app.ui.canvas_tool_menu.is_none(), "{tool:?}: ⌥Space opens no menu and doesn't list the layers");
    }
}

/// #11271: while type is edited Space types, so an agent's Space right-click stays the type
/// edit menu, not the Hand's view menu.
#[test]
fn agent_space_right_click_while_typing_keeps_the_type_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 20})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
    assert!(app.ui.text_edit.is_some());
    let click = json!({"space": true, "button": "secondary", "events": [{"kind":"down","x":8,"y":16},{"kind":"up","x":8,"y":16}]});
    assert_eq!(pointer(&mut app, click)["ok"], true);
    let menu = app.ui.canvas_tool_menu.clone().expect("the type edit menu");
    assert!(matches!(menu.kind, MenuKind::TypeEdit { .. }), "the type edit menu, not the view menu");
}

/// #6561: an agent's right-click on a pending crop opens the crop menu, as on the canvas.
#[test]
fn agent_right_click_on_a_pending_crop_opens_and_runs_the_crop_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Crop;
    app.ui.tool_options.crop.shield.classic_mode = true;
    app.ui.extras.snap = false;
    // The frame is drawn with the agent's own drags, as a user draws it.
    pointer(
        &mut app,
        json!({"tool": "Crop", "events": [{"kind":"down","x":4,"y":4},{"kind":"move","x":16,"y":12},{"kind":"move","x":28,"y":20},{"kind":"up","x":28,"y":20}]}),
    );
    let frame = crate::crop_ui::frame(&app).unwrap();
    let result = pointer(&mut app, json!({"tool": "Crop", "button": "secondary", "events": [{"kind":"down","x":16,"y":12},{"kind":"up","x":16,"y":12}]}));
    assert_eq!(result["ok"], true);
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(matches!(app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::Crop { .. })));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    assert!(entries.iter().any(|row| row.get("id") == Some(&json!("crop.commit")) && row.get("enabled") == Some(&json!(true))));
    assert_eq!(crate::crop_ui::frame(&app), Some(frame), "the right-click leaves the frame alone");
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"crop.commit"}))["ok"], true);
    let size = app.session.active().unwrap().doc.size;
    assert_eq!([f64::from(size.width), f64::from(size.height)], [frame.rect.width(), frame.rect.height()], "cropped to the frame");
}

#[test]
fn agent_can_inspect_and_choose_pen_make_selection_from_full_context_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[2,2],[20,2],[20,20]]}]}})).unwrap();
    let result = pointer(
        &mut app,
        json!({
            "tool": "Pen", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]
        }),
    );
    assert_eq!(result["ok"], true);
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(app.ui.canvas_tool_menu.as_ref().unwrap().pos, screen_point(&app, 8.0, 8.0));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    // #12915: (8,8) lies on the closed work path's diagonal, so the menu now leads with the
    // segment's Add Anchor Point entry above one more separator.
    assert_eq!(entries.iter().filter(|row| row.get("separator") == Some(&json!(true))).count(), 10);
    assert!(
        entries.iter().any(|row| row.get("id") == Some(&json!("path.addAnchor")) && row.get("enabled") == Some(&json!(true))),
        "the segment entry heads the menu"
    );
    assert!(entries.iter().any(|row| row.get("id") == Some(&json!("path.toSelection")) && row.get("enabled") == Some(&json!(true))));
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"file.new"}))["ok"], false);
    let chosen = call(&mut app, &ctx, "ui.context.choose", json!({"id":"path.toSelection"}));
    assert_eq!(chosen["ok"], true);
    let dialog = chosen["result"]["dialog"].clone();
    assert!(dialog.is_string(), "{chosen}");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn engine_execute_runs_with_defaults_but_menu_invoke_opens_the_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let rev = app.session.active().unwrap().revision;
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "filter.blur.gaussianBlur"}));
    assert!(!r.to_string().contains("\"dialog\""), "engine.execute opened a dialog: {r}");
    assert!(app.session.active().unwrap().revision > rev, "engine.execute didn't run the filter");
    assert!(dialogs(&mut app, &ctx).is_empty(), "engine.execute listed a dialog");
    let rev = app.session.active().unwrap().revision;
    let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
    assert!(r.to_string().contains("dialog"), "ui.menu.invoke should open the dialog: {r}");
    assert_eq!(dialogs(&mut app, &ctx).len(), 1, "the opened dialog is listed");
    assert_eq!(app.session.active().unwrap().revision, rev, "opening a dialog must not edit the document");
}

#[test]
fn menu_invoke_cannot_edit_document_behind_an_open_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();

    let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageSize"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let dialog = opened["result"]["dialog"].clone();
    let revision = app.session.active().unwrap().revision;
    let before = call(&mut app, &ctx, "ui.inspect", json!({}));
    let fields = before["result"]["dialogs"][0]["fields"].clone();

    for params in [json!({"id": "image.imageRotation.90cw"}), json!({"id": "image.imageRotation.90cw", "params": {}})] {
        let blocked = call(&mut app, &ctx, "ui.menu.invoke", params);
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(app.session.active().unwrap().revision, revision);
        let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
        assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
    }

    // View navigation stays permitted by the same policy as native menus and shortcuts.
    assert!(crate::menus::allows(&app, &CommandRef::new("view.zoomIn")));
    assert!(!crate::menus::allows(&app, &CommandRef::new("image.imageRotation.90cw")));

    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    let rotated = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageRotation.90cw"}));
    assert_eq!(rotated["ok"], true, "{rotated}");
    assert!(app.session.active().unwrap().revision > revision);
}

#[test]
fn engine_execute_cannot_change_document_behind_adjustment_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 60, "height": 40, "background": "#3366cc"})).unwrap();
    let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.adjustments.hueSaturation"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let dialog = opened["result"]["dialog"].clone();
    let revision = app.session.active().unwrap().revision;
    let fields = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"][0]["fields"].clone();

    for method in ["engine.execute", "ui.menu.invoke"] {
        let blocked = call(&mut app, &ctx, method, json!({"command": "image.adjustments.invert"}));
        assert_eq!(blocked["ok"], false, "{method}: {blocked}");
        assert_eq!(app.session.active().unwrap().revision, revision, "{method} changed the document");
        let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
        assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
    }

    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "engine.execute", json!({"command": "image.adjustments.invert"}))["ok"], true);
    assert!(app.session.active().unwrap().revision > revision);
}

#[test]
fn ui_set_rejects_unknown_fields_before_changing_anything() {
    use crate::theme::AppTheme;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.set_theme(&ctx, AppTheme::studio_light());
    // Not the default tool, so a rejected `"tool": "move"` that still applied would show.
    app.ui.tool = Tool::Brush;
    // `dark` was advertised to MCP clients but never read; a typo looked like success too.
    for params in [json!({"dark": true}), json!({"thme": "classic"}), json!({"tool": "move", "dark": true})] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert!(r["error"].as_str().unwrap().contains("unknown field"), "{r}");
    }
    assert_eq!(app.ui.theme, AppTheme::studio_light());
    assert_eq!(app.ui.tool, Tool::Brush, "a rejected call applies none of its fields");
    // Every field the method reads gets past the check (a bad value is its own error).
    for field in UI_SET_FIELDS {
        let r = call(&mut app, &ctx, "ui.set", json!({ field: null }));
        assert!(!r.to_string().contains("unknown field"), "{field}: {r}");
    }
    assert_eq!(call(&mut app, &ctx, "ui.set", Value::Null)["ok"], true);
}

#[test]
fn ui_set_drives_the_eyedropper_options() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let good = call(
        &mut app,
        &ctx,
        "ui.set",
        json!({"tool": "eyedropper", "eyedropperSampleSize": 11, "eyedropperSample": "currentAndBelow", "eyedropperRing": false}),
    );
    assert_eq!(good["ok"], true, "{good}");
    let o = &app.ui.tool_options.eyedropper;
    assert_eq!((o.size, o.source, o.show_ring), (SampleSize::Avg11, SampleSource::CurrentAndBelow, false));
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": "point"}))["ok"], true);
    assert_eq!(app.ui.tool_options.eyedropper.size, SampleSize::Point);
    for bad in [
        json!({"eyedropperSampleSize": 4}),
        json!({"eyedropperSampleSize": "big"}),
        json!({"eyedropperSample": "below"}),
        json!({"eyedropperSample": 2}),
        json!({"eyedropperRing": "yes"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    // A rejected call applies none of its fields.
    let r = call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": 101, "eyedropperSample": "nope"}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(app.ui.tool_options.eyedropper.size, SampleSize::Point);
}

/// #3823: the Crop tool's Fill over the control channel, reported by `ui.inspect`.
#[test]
fn ui_set_drives_the_crop_fill() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "crop", "cropFill": "contentAware"}))["ok"], true);
    // The fill's serialized name is the protocol's value.
    let fill = |app: &PhotocraftApp| serde_json::to_value(app.ui.tool_options.crop.fill).unwrap();
    assert_eq!(fill(&app), json!("contentAware"));
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"]["crop"]["fill"], json!("contentAware"));
    for bad in [json!("generativeExpand"), json!(true), json!(1)] {
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropFill": bad}))["ok"], false, "{bad}");
        assert_eq!(fill(&app), json!("contentAware"), "{bad}: unchanged");
    }
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropFill": "background"}))["ok"], true);
    assert_eq!(fill(&app), json!("background"));
}

/// #1919: the Crop tool's overlay menu over the control channel, reported by `ui.inspect`.
#[test]
fn ui_set_drives_the_crop_overlay_options() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let good = call(&mut app, &ctx, "ui.set", json!({"tool": "crop", "cropOverlay": "goldenSpiral", "cropOverlayShow": "always", "cropOverlayOrientation": 2}));
    assert_eq!(good["ok"], true, "{good}");
    // The serialized names are the protocol's values.
    let guide = |app: &PhotocraftApp| serde_json::to_value(app.ui.tool_options.crop.guide).unwrap();
    let o = &app.ui.tool_options.crop;
    assert_eq!((guide(&app), serde_json::to_value(o.guide_show).unwrap(), o.guide_orientation), (json!("goldenSpiral"), json!("always"), 2));
    let seen = call(&mut app, &ctx, "ui.inspect", json!({}));
    let t = &seen["result"]["toolOptions"]["crop"];
    assert_eq!((t["guide"].as_str(), t["guideShow"].as_str(), t["guideOrientation"].as_u64()), (Some("goldenSpiral"), Some("always"), Some(2)));
    for bad in [
        json!({"cropOverlay": "spiral"}),
        json!({"cropOverlay": 1}),
        json!({"cropOverlayShow": "sometimes"}),
        json!({"cropOverlayOrientation": 4}),
        json!({"cropOverlayOrientation": -1}),
        json!({"cropOverlayOrientation": "1"}),
        json!({"cropOverlay": "grid", "cropOverlayShow": "nope"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(guide(&app), json!("goldenSpiral"), "a rejected call applies none of its fields");
}

/// #1919: the Crop tool's gear menu (Show Cropped Area, crop shield) over the control channel.
#[test]
fn ui_set_drives_the_crop_shield_options() {
    use craft::ui::canvas::{CropShield, ShieldColor};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let good = call(&mut app, &ctx, "ui.set", json!({"cropShield": {"color": "custom", "custom_color": [255, 0, 0], "opacity": 40}}));
    assert_eq!(good["ok"], true, "{good}");
    let mut want = CropShield::default();
    want.color = ShieldColor::Custom;
    want.custom_color = [255, 0, 0];
    want.opacity = 40.0;
    // `CropShield` is non-exhaustive, so a variation is the value with one field set.
    let hidden_area = |shield: &CropShield| {
        let mut shield = shield.clone();
        shield.show_cropped_area = false;
        shield
    };
    assert_eq!(app.ui.tool_options.crop.shield, want);
    // A patch: the other fields stay.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropShield": {"show_cropped_area": false}}))["ok"], true);
    assert_eq!(app.ui.tool_options.crop.shield, hidden_area(&want));
    let t = &call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"]["crop"]["shield"];
    assert_eq!((t["color"].as_str(), t["opacity"].as_f64(), t["show_cropped_area"].as_bool()), (Some("custom"), Some(40.0), Some(false)));
    for bad in [
        json!({"cropShield": true}),
        json!({"cropShield": {"opacity": 101}}),
        json!({"cropShield": {"opacity": -1}}),
        json!({"cropShield": {"opacity": "50"}}),
        json!({"cropShield": {"color": "red"}}),
        json!({"cropShield": {"custom_color": [256, 0, 0]}}),
        json!({"cropShield": {"shield": true}}),
        json!({"cropShield": {"enabled": false}, "cropOverlay": "nope"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(app.ui.tool_options.crop.shield, hidden_area(&want), "a rejected call applies none of its fields");
}

#[test]
fn ui_set_opens_the_brush_preset_picker_and_sets_its_cards() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let parts = &app.ui.brush_picker_state.parts;
    assert!(parts.name && parts.stroke && !parts.tip, "the cards show the name and the stroke by default");
    let opened_at = |app: &PhotocraftApp| inspect(app, &ctx)["brushPicker"]["pos"].clone();
    let r = call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushPicker": [120, 80], "brushPickerStroke": false}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(opened_at(&app), json!([120.0, 80.0]));
    assert!(!app.ui.brush_picker_state.parts.stroke);
    for bad in [
        json!({"brushPicker": [1]}),
        json!({"brushPicker": "here"}),
        json!({"brushPickerStroke": "nope"}),
        json!({"brushPickerName": false, "brushPickerStroke": false, "brushPickerTip": false}),
    ] {
        assert_eq!(call(&mut app, &ctx, "ui.set", bad.clone())["ok"], false, "{bad}");
    }
    assert_eq!(opened_at(&app), json!([120.0, 80.0]), "a bad value leaves the picker alone");
    assert!(app.ui.brush_picker_state.parts.name, "a rejected call leaves the card parts alone");
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
    assert!(inspect(&app, &ctx)["brushPicker"].is_null());
}

#[test]
fn ui_inspect_tracks_brush_picker_open_and_closed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 128, "height": 128})).unwrap();

    let closed = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(closed["result"]["brushPicker"].is_null());

    // A right-click on the canvas with Brush opens the same picker as the options bar.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}))["ok"], true);
    let click = pointer(
        &mut app,
        json!({
            "button": "right",
            "events": [{"kind": "down", "x": 64, "y": 64}, {"kind": "up", "x": 64, "y": 64}]
        }),
    );
    assert_eq!(click["ok"], true, "{click}");
    let open = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(open["result"]["brushPicker"]["pos"].is_array(), "{open}");
    assert!(open["result"]["brushPicker"].get("size").is_none(), "the framework remembers the picker's size");
    assert_eq!(open["result"]["brushPicker"]["list"]["showName"], true);
    assert_eq!(open["result"]["brushPicker"]["list"]["showStroke"], true);
    assert_eq!(open["result"]["brushPicker"]["list"]["showTip"], false);

    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPickerTip": true}))["ok"], true);
    let changed = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(changed["result"]["brushPicker"]["list"]["showTip"], true);

    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
    let closed_again = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(closed_again["result"]["brushPicker"].is_null());
}

#[test]
fn ui_set_applies_all_fields_or_none() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let before = app.ui.tool;
    // The panel and dock fields are workspace actions: a refused call leaves the workspace equal.
    let docking_before = app.ui.docking.clone();
    // A late invalid field must not leave the earlier valid one applied - including the
    // journaled brush command, which is validated up front for exactly this reason.
    for params in [json!({"tool": "move", "theme": "nope"}), json!({"tool": "move", "brushSize": 1e300}), json!({"tool": "move", "colorPanel": {"nope": true}})]
    {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(app.ui.tool, before, "{params}: a rejected call applies none of its fields");
    }
    // Wrong types and unknown nested keys are errors, not silent no-ops with ok:true.
    for params in [
        json!({"panels": "x"}),
        json!({"panels": {"nope": true}}),
        json!({"dockTabs": {"nope": 1}}),
        json!({"dock": {"nope": 1}}),
        json!({"center": [1]}),
        json!({"center": [1, 2, 3]}),
        json!({"center": "here"}),
        json!({"colorPanel": "x"}),
        json!({"selectionMode": "add"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
    }
    assert_eq!(app.ui.docking, docking_before, "nothing was applied");
    // Non-numeric brushSize keeps the API's historical silent no-op (see the dispatch test).
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushSize": "large"}))["ok"], true);
}

#[test]
fn ui_set_rejects_view_numbers_beyond_f32_range() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let r = call(&mut app, &ctx, "ui.set", json!({"rotation": 30, "zoom": 2, "center": [3, 4], "dockWidth": 300}));
    assert_eq!(r["ok"], true, "{r}");
    // The width itself is `width_actions`' business; a refused call must leave the workspace equal.
    let docking = app.ui.docking.clone();
    let (rotation, zoom, center) = (app.ui.views[0].rotation, app.ui.views[0].zoom, app.ui.views[0].center);
    // Finite in f64 but infinite as f32: an error, not a silent reset to 0°, 100% or the minimum dock width.
    for params in [json!({"rotation": 1e39}), json!({"zoom": 1e39}), json!({"center": [1e39, 0]}), json!({"center": [0, -1e39]}), json!({"dockWidth": 1e39})] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(app.ui.views[0].rotation, rotation, "{params}");
        assert_eq!(app.ui.views[0].zoom, zoom, "{params}");
        assert_eq!(app.ui.views[0].center, center, "{params}");
        assert_eq!(app.ui.docking, docking, "{params}");
    }
    // Huge but representable still clamps to the top zoom level.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"zoom": 1e38}))["ok"], true);
    assert!(app.ui.views[0].zoom.is_finite() && app.ui.views[0].zoom > zoom, "{}", app.ui.views[0].zoom);
}

/// `ui.set {theme}` takes a suite theme id, shown in the mode on screen, or a name PhotoCraft took
/// before theme ids, shown in its own mode; a theme the suite doesn't have is refused.
#[test]
fn ui_set_theme_takes_ids_and_pre_id_names_and_refuses_unknown_themes() {
    use crate::theme::AppTheme;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    for unknown in ["nope", "Not an id!"] {
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": unknown}))["ok"], false, "{unknown}");
        assert_eq!(app.ui.theme, AppTheme::pro_medium(), "{unknown}");
    }
    for (name, shown) in [
        ("craft.studio", AppTheme::studio()),
        ("studioLight", AppTheme::studio_light()),
        ("Pro (Medium Gray)", AppTheme::pro_medium()),
        ("gnome", AppTheme::adwaita()),
        ("craft.pro", AppTheme::pro_light()),
    ] {
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": name}))["ok"], true, "{name}");
        assert_eq!(app.ui.theme, shown, "{name}");
    }
}

/// `ui.set {theme}` with a library theme the suite has in its library but hasn't loaded picks it
/// for the slot and answers ok with a note; the look stays until the suite's read shows it.
#[test]
fn ui_set_theme_picks_a_library_theme_the_suite_has_not_loaded() {
    use craft::theme_store::{
        Op,
        testing::{SAMPLE_THEME_ID, SAMPLE_THEME_JSON},
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.framework.suite().watch().request(Op::Import { bytes: std::sync::Arc::from(SAMPLE_THEME_JSON.as_bytes()), as_copy: false });
    app.framework.sync_theme(&mut app.session);
    let before = app.ui.theme.clone();

    let r = call(&mut app, &ctx, "ui.set", json!({"theme": SAMPLE_THEME_ID}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r.to_string().contains("still loading"), "{r}");
    assert_eq!(app.session.prefs().interface.dark_theme, SAMPLE_THEME_ID);
    crate::prefs_ui::sync_appearance(&mut app, &ctx);
    assert_eq!(app.ui.theme, before, "the look stays while the suite loads the theme");

    for _ in 0..4 {
        if app.framework.sync_theme(&mut app.session) {
            crate::prefs_ui::show_selected_theme(&mut app, &ctx);
        }
    }
    assert_eq!(app.ui.theme.id().as_str(), SAMPLE_THEME_ID);
    assert!(app.ui.theme.is_dark());
}

/// A slot naming a theme the suite doesn't know (a deleted library theme) shows the mode's
/// default, and once settled a frame resolves nothing.
#[test]
fn an_unknown_slot_theme_shows_the_mode_default_without_resolving_each_frame() {
    use std::sync::atomic::Ordering;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let frame = |app: &mut PhotocraftApp| {
        let changed = app.framework.sync_theme(&mut app.session);
        if changed {
            crate::prefs_ui::show_selected_theme(app, &ctx);
        }
        crate::prefs_ui::sync_appearance(app, &ctx);
        changed
    };
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": "craft.pro"}))["ok"], true);
    assert_eq!(app.ui.theme, crate::theme::AppTheme::pro());
    app.run("prefs.set", json!({"values": {"interface.darkTheme": "user.deleted"}})).unwrap();
    for _ in 0..10 {
        if !frame(&mut app) {
            break;
        }
    }
    assert_eq!(app.ui.theme, crate::theme::AppTheme::pro_medium(), "the dark mode's default");
    let resolves = app.framework.theme_resolves.load(Ordering::Relaxed);
    assert!(!frame(&mut app));
    assert_eq!(app.framework.theme_resolves.load(Ordering::Relaxed), resolves, "a settled frame resolves nothing");
}

#[test]
fn preferences_apply_is_available_over_control_without_closing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.preferences.interface"}));
    // The Preferences window is held by the app; control addresses it by the id `ui.inspect` lists,
    // and the reply names it by that id (#15544).
    let listed = dialogs(&mut app, &ctx);
    let window = listed.iter().find(|d| d["key"] == "preferences").expect("the Preferences window is listed");
    assert_eq!(r["result"]["dialog"], window["id"], "{r}");
    let id = window["id"].clone();
    let mut values = window["fields"]["values"].clone();
    values["interface"]["uiScale"] = json!("200");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true, "Apply left the window open");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
    for params in [json!({}), json!({"dialog": "invalid"}), json!({"dialog": u64::MAX})] {
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", params)["ok"], false);
    }
}

#[test]
fn preferences_apply_respects_the_automation_command_policy() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| if id == "prefs.set" { Err("preference changes denied".into()) } else { Ok(()) })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    let before = app.session.prefs().to_json();
    crate::prefs_ui::open_preferences(&mut app, "interface");
    // The draft is edited through `ui.dialog.set`, the protocol path.
    let listed = dialogs(&mut app, &ctx);
    let window = listed.iter().find(|d| d["key"] == "preferences").expect("the Preferences window is listed");
    let id = window["id"].clone();
    let mut values = window["fields"]["values"].clone();
    values["interface"]["uiScale"] = json!("200");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
    let r = call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}));
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"], "preference changes denied");
    assert_eq!(app.session.prefs().to_json(), before);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true, "the window stays open after the refusal");
}

#[test]
fn synthetic_shortcuts_use_the_automation_command_policy() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| if id.starts_with("file.") { Err("filesystem command denied".into()) } else { Ok(()) })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.automation_input = true;
    let error = crate::menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err();
    assert_eq!(error, "filesystem command denied");
}

/// `app.save` takes the Targa bits per pixel (`tgaBits`) and never stops at Targa Options.
#[test]
fn automation_save_passes_targa_bits_without_asking() {
    use std::cell::RefCell;
    use std::rc::Rc;
    // The bits each export was asked for, and each path written.
    let asked: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w) = (asked.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push(s.tga_bits.map(photocraft_io::TgaBits::bits));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in [json!({"path": "a.tga"}), json!({"path": "b.tga", "tgaBits": 24}), json!({"path": "c.tga", "tgaBits": 32})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.parked_save.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [None, Some(24), Some(32)]);
    assert_eq!(*written.borrow(), ["a.tga", "b.tga", "c.tga"]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "d.tga", "tgaBits": 16}));
    assert!(r["error"].as_str().unwrap().contains("tgaBits"), "{r}");
    assert_eq!(written.borrow().len(), 3, "a bad tgaBits writes nothing");
}

/// #8064: `app.save` takes `maximizeCompatibility` for a PSD/PSB's merged image over Preferences ›
/// File Handling; unset, the preference decides. Anything but a boolean writes nothing.
#[test]
fn app_save_takes_maximize_compatibility() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let asked: Rc<RefCell<Vec<bool>>> = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w) = (asked.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push(s.psd_maximize_compatibility);
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let saves = [
        json!({"path": "a.psd"}),
        json!({"path": "b.psd", "maximizeCompatibility": false}),
        json!({"path": "c.psd", "maximizeCompatibility": true}),
        json!({"path": "d.psd", "maximizeCompatibility": null}),
    ];
    for params in &saves {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
    }
    app.session.edit_prefs(|p| p.file_handling.maximize_psd_compatibility = photocraft_engine::prefs::Ask::Never);
    for params in [json!({"path": "e.psd"}), json!({"path": "f.psd", "maximizeCompatibility": true})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
    }
    assert_eq!(*asked.borrow(), [true, false, true, true, false, true]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "g.psd", "maximizeCompatibility": "no"}));
    assert!(r["error"].as_str().unwrap().contains("maximizeCompatibility"), "{r}");
    assert_eq!(written.borrow().len(), 6, "a bad maximizeCompatibility writes nothing");
}

/// `app.save` takes JPEG Options (`quality` 0–12, `jpegFormat`) and never stops at the prompt;
/// a value it can't read is an error that writes nothing.
#[test]
fn automation_save_passes_jpeg_options_without_asking() {
    use photocraft_codecs::JpegFormat;
    use std::cell::RefCell;
    use std::rc::Rc;
    // The JPEG quality (1–100) and format each export was asked for.
    type Asked = Rc<RefCell<Vec<(Option<u8>, JpegFormat)>>>;
    let asked: Asked = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w) = (asked.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push((s.jpeg_quality, s.jpeg_format));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in
        [json!({"path": "a.jpg"}), json!({"path": "b.jpg", "quality": 12, "jpegFormat": "progressive"}), json!({"path": "c.jpg", "jpegFormat": "optimized"})]
    {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.parked_save.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [(None, JpegFormat::Baseline), (Some(100), JpegFormat::Progressive), (None, JpegFormat::Optimized)]);
    for bad in [
        json!({"path": "d.jpg", "quality": 13}),
        json!({"path": "d.jpg", "quality": "high"}),
        json!({"path": "d.jpg", "jpegFormat": "fast"}),
        json!({"path": "d.jpg", "jpegFormat": 2}),
    ] {
        let r = call(&mut app, &ctx, "app.save", bad.clone());
        assert!(r["error"].is_string(), "{bad}: {r}");
    }
    assert_eq!(written.borrow().len(), 3, "a bad JPEG option writes nothing");
}

/// `app.save` takes the DDS options (`ddsCompression`, `ddsMipmaps`) and never stops at DDS
/// Options.
#[test]
fn automation_save_passes_dds_options_without_asking() {
    use photocraft_io::{DdsCompression, DdsOptions};
    use std::cell::RefCell;
    use std::rc::Rc;
    let asked: Rc<RefCell<Vec<DdsOptions>>> = Rc::default();
    let a = asked.clone();
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push(s.dds);
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in [json!({"path": "a.dds"}), json!({"path": "b.dds", "ddsCompression": "bc1", "ddsMipmaps": true})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.parked_save.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [DdsOptions::default(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: true }]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "c.dds", "ddsCompression": "bc9"}));
    assert!(r["error"].as_str().unwrap().contains("ddsCompression"), "{r}");
    assert_eq!(asked.borrow().len(), 2, "a bad ddsCompression writes nothing");
}

#[test]
fn an_automation_open_without_a_location_is_not_listed_in_open_recent() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;

    let services = crate::Services {
        import: Some(Box::new(|name: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
        })),
        automation_read: Some(Box::new(|path: &str| {
            Ok(crate::AutomationReadResult {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                bytes: b"x".to_vec(),
                save_path: Some(path.to_string()),
            })
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    let r = call(&mut app, &ctx, "app.open", json!({"path": "shots/gk_25.png"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("shots/gk_25.png"), "the save-back target stays the root-relative path");
    // A root-relative path names no file for Open Recent or Home, so nothing is listed.
    assert!(app.ui.recent_files.is_empty(), "{:?}", app.ui.recent_files);
}

#[test]
fn automation_open_and_save_use_the_roots_and_reply_with_warnings() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    use std::cell::RefCell;
    use std::rc::Rc;

    // Without granted roots both fail closed, even with an exporter and an ambient writer.
    let services = crate::Services {
        export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
    assert!(r.to_string().contains("automation read authority is not configured"), "{r}");
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
    assert!(r.to_string().contains("automation write authority is not configured"), "{r}");

    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let services = crate::Services {
        import: Some(Box::new(|name: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), vec!["Adjustment layer flattened".to_string()]))
        })),
        export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), vec!["Layers were flattened".to_string()])))),
        automation_read: Some(Box::new(|path: &str| {
            Ok(crate::AutomationReadResult {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                bytes: b"x".to_vec(),
                save_path: Some(path.to_string()),
            })
        })),
        automation_locate: Some(Box::new(|path: &str| Some(format!("/work/{path}")))),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
    assert_eq!(r["result"]["warnings"], json!(["Adjustment layer flattened"]), "{r}");
    assert_eq!(r["result"]["name"], "warn.psd");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("in/warn.psd"));
    // Open Recent lists where the file is, not the root-relative request path, which names nothing outside automation.
    assert_eq!(*app.ui.recent_files, vec!["/work/in/warn.psd".to_string()]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
    assert_eq!(r["result"], json!({"path": "out.png", "warnings": ["Layers were flattened"]}), "{r}");
    assert_eq!(*written.borrow(), vec!["out.png".to_string()]);
    // A flat export is a copy: the document keeps its name and file (#2579).
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("warn.psd", Some("in/warn.psd")));
    // Without `path`, only a layered file is written back, like File › Save (#416).
    call(&mut app, &ctx, "app.open", json!({"path": "in/flat.jpg"}));
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
    assert_eq!(written.borrow().len(), 1, "nothing written over the JPEG");
    call(&mut app, &ctx, "app.open", json!({"path": "in/layered.psd"}));
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert_eq!(r["result"]["path"], "in/layered.psd", "{r}");
    assert_eq!(written.borrow().last().map(String::as_str), Some("in/layered.psd"));
    // A template opens untitled, so a save without `path` never writes over it.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/card.psdt"}));
    assert_eq!(r["result"]["name"], "Untitled-1", "{r}");
    assert_eq!(app.session.active().unwrap().path, None);
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
}

fn deny_ambient_file(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
    if id.starts_with("file.") && id != "file.new" {
        Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
    } else {
        Ok(())
    }
}

/// An app whose automation gates refuse file commands, with a document open, and the paths
/// its writer was asked to write.
fn gated_app_recording_writes() -> (Box<PhotocraftApp>, egui::Context, std::rc::Rc<std::cell::RefCell<Vec<String>>>, u64) {
    let written: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::default();
    let w = written.clone();
    let services = crate::Services {
        automation_authorize: Some(deny_ambient_file),
        automation_command: Some(Box::new(|id, params| deny_ambient_file(id, params).map_err(|e| e.to_string()))),
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let doc = app.session.active().unwrap().doc.id.0;
    let ctx = app.ctx().clone();
    (app, ctx, written, doc)
}

/// A handler that writes files itself (the disk-change prompt's save) is judged as the file
/// command it stands for, through a control request or synthetic input alike.
#[test]
fn automation_confirming_a_file_writing_dialog_is_judged_as_its_file_command() {
    let (mut app, ctx, written, _) = gated_app_recording_writes();
    let command = |app: &mut PhotocraftApp| crate::dialogs::open_command_dialog(app, "file.save", "Save").expect("the file-writing dialog opens");
    // A hosted dialog that writes files is a file command: its confirm is authorised as one.
    let prompt = command(&mut app);
    let r = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": prompt.value().to_string()}));
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(r["error"].as_str().is_some_and(|e| e.contains("file.save")), "{r}");
    // Synthetic input (Enter or OK) reaches the same handler without the control pre-check.
    let synthetic = command(&mut app);
    app.automation_input = true;
    let r = crate::dialogs::confirm(&mut app, &ctx, synthetic);
    app.automation_input = false;
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(r.is_err_and(|e| e.contains("file.save")));
}

/// The desktop policy's verdict on file commands (crates/automation/src/workspace.rs): automation
/// may close documents, but every file command that reads or writes a file, File › Save
/// included, is refused.
fn desktop_file_policy(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id.starts_with("file.") && !matches!(id, "file.new" | "file.close" | "file.closeAll" | "file.closeOthers") {
        Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
    } else {
        Ok(())
    }
}

/// An app under the desktop file policy whose dirty document (its own file outside any
/// automation root) a control request asked to close: the unsaved-changes prompt is up.
/// Returns the app, its context, the paths written so far and the document's own file.
fn dirty_close_prompted() -> (Box<PhotocraftApp>, egui::Context, std::rc::Rc<std::cell::RefCell<Vec<String>>>, String) {
    let (mut app, ctx, written, _) = gated_app_recording_writes();
    app.services.automation_authorize = Some(desktop_file_policy);
    app.services.automation_command = Some(Box::new(|id, params| desktop_file_policy(id, params).map_err(|e| e.to_string())));
    let own_file = std::env::temp_dir().join("outside").join("x.psd").to_string_lossy().into_owned();
    app.session.active_mut().unwrap().path = Some(own_file.clone());
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"command": "file.close"}));
    // The frame after the request opens the question.
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(!dialogs(&mut app, &ctx).is_empty(), "closing a dirty document asks first: {r}");
    (app, ctx, written, own_file)
}

/// Answers the open unsaved-changes prompt with Save, as synthetic input (the drawn prompt's
/// answer with `automation_input` set) or as a control client's `ui.dialog.confirm`. Returns
/// the refusal, if any.
fn answer_save(app: &mut PhotocraftApp, ctx: &egui::Context, synthetic: bool) -> Result<(), String> {
    let prompt = top_dialog(app, ctx)["id"].clone();
    if synthetic {
        assert_eq!(prompt, crate::discard_ui::CONTROL_ID, "the prompt is listed as discard");
        let id = crate::discard_ui::host_id(app).expect("the prompt is open");
        app.automation_input = true;
        let r = crate::dialogs::confirm(app, ctx, id);
        app.automation_input = false;
        return r;
    }
    let r = call(app, ctx, "ui.dialog.confirm", json!({"dialog": prompt, "choice": "save"}));
    match r["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    }
}

/// Answering the unsaved-changes prompt with Save writes the document back like File › Save,
/// so automation is judged for `file.save` there too, through a control request or synthetic
/// input alike: a client allowed to close a document can't write it over its own file. A
/// policy that allows the Save lets it write and close, and a user's Save still writes and
/// closes.
#[test]
fn automation_saving_from_the_unsaved_changes_prompt_is_judged_as_file_save() {
    for synthetic in [false, true] {
        let (mut app, ctx, written, _) = dirty_close_prompted();
        let error = answer_save(&mut app, &ctx, synthetic).err().unwrap_or_default();
        assert!(written.borrow().is_empty(), "synthetic={synthetic}: nothing is written: {:?}", written.borrow());
        assert!(error.contains("file.save"), "synthetic={synthetic}: {error:?}");
        assert!(!dialogs(&mut app, &ctx).is_empty(), "synthetic={synthetic}: the prompt stays up");
        assert_eq!(app.session.documents().len(), 1, "synthetic={synthetic}: the document stays open");
    }
    // A policy that allows `file.save` lets the automation Save write the document and close it.
    /// A policy that allows every command.
    fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    for synthetic in [false, true] {
        let (mut app, ctx, written, own_file) = dirty_close_prompted();
        app.services.automation_authorize = Some(allow);
        app.services.automation_command = Some(Box::new(|id, params| allow(id, params).map_err(|e| e.to_string())));
        let r = answer_save(&mut app, &ctx, synthetic);
        assert!(r.is_ok(), "synthetic={synthetic}: {r:?}");
        assert_eq!(*written.borrow(), vec![own_file], "synthetic={synthetic}: the document is written back");
        assert!(dialogs(&mut app, &ctx).is_empty() && app.session.documents().is_empty(), "synthetic={synthetic}: the prompt and the document close");
    }
    // The user answering Save writes the document back and closes it.
    let (mut app, ctx, written, own_file) = dirty_close_prompted();
    let prompt = app.framework.dialogs.stack().top().expect("the prompt is the top dialog");
    crate::dialogs::confirm(&mut app, &ctx, prompt).unwrap();
    assert_eq!(*written.borrow(), vec![own_file]);
    assert!(dialogs(&mut app, &ctx).is_empty());
    assert!(app.session.documents().is_empty());
}

/// A control client's Save on the unsaved-changes prompt is judged as `file.save` whatever part
/// of the automation policy its services carry: only the gate, or only the command callback
/// (which installs no session gate for the request). With no policy at all, synthetic input and
/// a control answer alike fail closed. Nothing is written, and the prompt and document stay.
#[test]
fn automation_saving_from_the_unsaved_changes_prompt_needs_a_policy_to_judge_it() {
    for (gate, callback) in [(true, false), (false, true), (false, false)] {
        for synthetic in [false, true] {
            let (mut app, ctx, written, _) = dirty_close_prompted();
            if !gate {
                app.services.automation_authorize = None;
            }
            if !callback {
                app.services.automation_command = None;
            }
            let error = answer_save(&mut app, &ctx, synthetic).err().unwrap_or_default();
            let case = format!("gate={gate} callback={callback} synthetic={synthetic}");
            assert!(error.contains("file.save"), "{case}: {error:?}");
            assert!(written.borrow().is_empty(), "{case}: nothing is written: {:?}", written.borrow());
            assert!(!dialogs(&mut app, &ctx).is_empty(), "{case}: the prompt stays up");
            assert_eq!(app.session.documents().len(), 1, "{case}: the document stays open");
        }
    }
}

#[test]
fn app_open_opens_a_directory_pcraft_and_falls_back_for_the_zip_form() {
    use photocraft_doc::Document;
    use serde_json::json;
    let services = crate::Services {
        automation_open_pcraft: Some(Box::new(|path: &str| {
            if path == "native.pcraft" {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                let doc = Document::new("Native", photocraft_doc::Size::new(16, 12), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
                Ok(Some((name, doc)))
            } else if path == "zipped.pcraft" {
                Ok(None) // a regular file: the bytes path reads the zip form
            } else {
                Err(format!("corrupt bundle: {path}"))
            }
        })),
        import: Some(Box::new(|name: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), Vec::new()))
        })),
        automation_read: Some(Box::new(|path: &str| {
            if path == "zipped.pcraft" {
                Ok(crate::AutomationReadResult { name: path.rsplit('/').next().unwrap_or(path).to_string(), bytes: b"zip-bytes".to_vec(), save_path: None })
            } else {
                Err(format!("no such path: {path}"))
            }
        })),
        automation_locate: Some(Box::new(|path: &str| Some(format!("/work/{path}")))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();

    let r = call(&mut app, &ctx, "app.open", json!({"path": "native.pcraft"}));
    assert_eq!(r["result"], json!({"path": "native.pcraft", "name": "native.pcraft", "warnings": []}), "{r}");
    assert_eq!(app.session.active().unwrap().doc.name, "Native");
    assert_eq!(*app.ui.recent_files, vec!["/work/native.pcraft".to_string()], "the open is recorded in recent files, where it is on disk");
    assert_eq!(app.session.active().unwrap().path, None, "no save-back target: the automation writer replaces files, never directories");
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "an in-place save is refused up front: {r}");

    // The single-file form takes the bytes path through the same `.pcraft` extension.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "zipped.pcraft"}));
    assert_eq!(r["result"]["name"], "zipped.pcraft", "{r}");
    assert_eq!(app.session.active().unwrap().doc.name, "zipped.pcraft", "the import served the zip form");

    // A real opener error propagates instead of being masked by the fallback.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "gone.pcraft"}));
    assert!(r["error"].as_str().unwrap().contains("corrupt bundle"), "{r}");
}

#[test]
fn app_save_runs_as_a_background_job_and_the_final_reply_carries_the_path() {
    use photocraft_engine::automate_cmds::ScriptBinding;
    use serde_json::json;
    let saved: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let record = saved.clone();
    let services = crate::Services {
        automation_save: Some(std::sync::Arc::new(move |_doc, path: &str, _settings, _ctl| {
            record.lock().unwrap().push(path.to_string());
            Ok(vec!["Layers were flattened".to_string()])
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = true;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    // A "Save Document" binding that would leave a trace: automation saves never fire it.
    app.session.edit_prefs(|prefs| {
        prefs.script_events.enabled = true;
        prefs.script_events.bindings = vec![ScriptBinding {
            event: "saveDocument".into(),
            script: None,
            steps: Some(json!([["layer.new.layer", {"name": "From the binding"}]])),
            name: "Binding".into(),
        }];
    });
    let ctx = app.ctx().clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.with_control(rx);
    let mut app = app;

    let (mut req, reply) = ControlRequest::new("app.save", json!({"path": "out.psd"}));
    req.deadline = None;
    tx.send(req).unwrap();
    app.drain_control(&ctx);
    // Started in the background: no reply yet, the job is on the document.
    assert!(reply.try_recv().is_err(), "the reply waits for the save job");
    assert!(app.saving(), "the save runs as a background job");
    for _ in 0..100 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.saving(), "the save finished");
    assert_eq!(*saved.lock().unwrap(), vec!["out.psd".to_string()]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out.psd"), "the save was recorded");
    assert_eq!(app.session.active().unwrap().doc.layer_count(), 1, "the saveDocument binding did not run for an automation save");
    let r = reply.recv().unwrap();
    assert_eq!(r["result"], json!({"path": "out.psd", "warnings": ["Layers were flattened"]}), "{r}");

    // An interactive save (File › Save's worker) still fires the binding.
    let save = std::sync::Arc::new(|_doc: &photocraft_doc::Document, path: &str, _settings: &crate::ExportSettings, _ctl: &crate::jobs_ui::SaveCtl| {
        let _ = path;
        Ok(Vec::new())
    }) as crate::SaveFileFn;
    crate::jobs_ui::start_save(&mut app, "user.psd".into(), crate::ExportSettings::default(), false, save, false).unwrap();
    for _ in 0..100 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.saving(), "the interactive save finished");
    assert_eq!(app.session.active().unwrap().doc.layer_count(), 2, "the binding runs for an interactive save");
}

#[test]
fn ui_inspect_replies_while_a_save_job_encodes() {
    // #2488: the control channel must keep answering while a large save runs.
    use serde_json::json;
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wait_for_gate = gate.clone();
    let saved: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let record = saved.clone();
    let services = crate::Services {
        automation_save: Some(std::sync::Arc::new(move |_doc, path: &str, _settings, ctl| {
            while !wait_for_gate.load(std::sync::atomic::Ordering::Acquire) {
                ctl.check()?;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            record.lock().unwrap().push(path.to_string());
            Ok(Vec::new())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = true;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let ctx = app.ctx().clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.with_control(rx);
    let mut app = app;

    let (mut save_req, save_reply) = ControlRequest::new("app.save", json!({"path": "out.png", "wait": false}));
    save_req.deadline = None;
    tx.send(save_req).unwrap();
    let (mut inspect_req, inspect_reply) = ControlRequest::new("ui.inspect", json!({}));
    inspect_req.deadline = None;
    tx.send(inspect_req).unwrap();
    app.drain_control(&ctx);

    let pending = save_reply.recv_timeout(std::time::Duration::from_secs(5)).expect("the unwaited save replies at once");
    assert_eq!(pending["result"]["pending"], true, "{pending}");
    assert_eq!(pending["result"]["path"], "out.png", "{pending}");
    assert!(pending["result"]["job"].is_u64(), "the caller can poll or wait: {pending}");
    // While the save is still encoding, the channel answers other requests.
    assert!(saved.lock().unwrap().is_empty(), "the save is still encoding");
    let inspected = inspect_reply.recv_timeout(std::time::Duration::from_secs(5)).expect("ui.inspect is not blocked by the save");
    assert_eq!(inspected["ok"], true, "{inspected}");

    gate.store(true, std::sync::atomic::Ordering::Release);
    for _ in 0..500 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(*saved.lock().unwrap(), vec!["out.png".to_string()], "the save completed after the gate opened");
}

fn deny_smart_object_paths(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    // The production policy lives in the automation crate. This gate exercises the control
    // session's state-derived path check without adding that dependency to the UI crate.
    if matches!(id, "layer.smartObjects.editContents" | "layer.smartObjects.convertToLayers" | "layer.smartObjects.saveContents")
        && params.get("path").and_then(Value::as_str).is_some()
    {
        Err(photocraft_engine::EngineError::Other("ambient smart-object path denied".into()))
    } else {
        Ok(())
    }
}

fn smart_control_app() -> (Box<PhotocraftApp>, egui::Context, photocraft_doc::LayerId) {
    let services = crate::Services {
        automation_authorize: Some(deny_smart_object_paths),
        automation_command: Some(Box::new(|id, params| deny_smart_object_paths(id, params).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let ctx = app.ctx().clone();
    (app, ctx, photocraft_doc::LayerId(id))
}

#[test]
fn control_edits_nested_embedded_smart_objects_and_converts_an_explicit_target() {
    use photocraft_doc::{LayerContent, SmartSource};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    let (mut app, ctx, smart) = smart_control_app();
    // Embed this one-smart-layer document inside the outer object: two Edit Contents
    // requests must reach the raster layer, as in a nested product-photo template.
    let nested = photocraft_engine::smart_cmds::encode_source(&app.session.active().unwrap().doc).unwrap();
    app.session
        .edit("Nested fixture", |doc, _| {
            let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
            sm.source = SmartSource::Embedded { file_name: "product.pcraft".into(), bytes: Arc::new(nested) };
            Ok(())
        })
        .unwrap();
    let other = app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(app.session.active().unwrap().active_layer, Some(smart));
    for expected_document in [1, 2] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.editContents", "params": {"layer": smart.0}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["document"], expected_document, "{r}");
        assert!(app.session.active().unwrap().path.is_none(), "contents open in memory");
        assert!(app.session.authorize.is_none(), "the request gate must not remain installed");
    }
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "edit.fill", "params": {"color": "#0000ff"}}));
    assert_eq!(r["ok"], true, "{r}");
    for expected_documents in [2, 1] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.saveContents"}));
        assert_eq!(r["ok"], true, "{r}");
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "file.close"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.documents().len(), expected_documents);
    }
    assert_eq!(app.session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(other)));
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [0.0, 0.0, 1.0, 1.0]);
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.convertToLayers", "params": {"layer": smart.0}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(other)).is_some(), "the unrelated active layer remains");
    assert!(app.session.authorize.is_none());

    let written = Rc::new(RefCell::new(Vec::new()));
    let output = written.clone();
    app.services.export =
        Some(Box::new(|doc, _, _| photocraft_engine::smart_cmds::encode_source(doc).map(|bytes| (bytes, Vec::new())).map_err(|e| e.to_string())));
    app.services.automation_write = Some(Box::new(move |path, bytes| {
        output.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    app.services.write = Some(Box::new(|_, _| Err("ambient writer used".into())));
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out/template.pcraft"}));
    assert_eq!(r["ok"], true, "{r}");
    let written = written.borrow();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, "out/template.pcraft");
    let saved = photocraft_engine::smart_cmds::decode_source(&written[0].0, &written[0].1).unwrap();
    assert_eq!(photocraft_compose::flatten(&saved).px[0], [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn control_refuses_linked_smart_objects_in_commands_and_actions_but_local_editing_still_works() {
    use photocraft_doc::{LayerContent, SmartSource};
    use std::sync::Arc;

    let (mut app, ctx, smart) = smart_control_app();
    let path = std::env::temp_dir().join(format!("pc-control-smart-linked-{}.pcraft", std::process::id()));
    let st = app.session.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(smart).unwrap().content else { panic!("smart fixture") };
    let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("embedded fixture") };
    std::fs::write(&path, bytes.as_slice()).unwrap();
    app.session
        .edit("Linked fixture", |doc, _| {
            let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
            sm.source = SmartSource::Linked { path: path.to_string_lossy().into_owned() };
            Ok(())
        })
        .unwrap();
    app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active().unwrap().active_layer;
    for command in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers"] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": command, "params": {"layer": smart.0}}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("ambient smart-object path denied"), "{r}");
        app.session.actions_mut().list = ActionList::new();
        app.session.actions_mut().list.push(action("Linked contents", vec![(command, json!({"layer": smart.0}))])).unwrap();
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Linked contents"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["ran"], 0, "{r}");
        assert_eq!(r["result"]["failed"]["id"], command, "{r}");
        assert_eq!(app.session.documents().len(), 1);
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before), "a refused operation must not edit the document");
        assert_eq!(app.session.active().unwrap().active_layer, active);
        assert!(app.session.authorize.is_none(), "a refused request must restore the local session");
    }
    // A synthetic thumbnail action uses the same temporary authorization as control calls.
    app.automation_input = true;
    let error = crate::menus::invoke(&mut app, &ctx, "layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap_err();
    assert!(error.contains("ambient smart-object path denied"), "{error}");
    assert!(app.session.authorize.is_none());
    app.automation_input = false;
    app.run("layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "a human can still open the existing linked file");
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [1.0, 0.0, 0.0, 1.0]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn actions_play_checks_each_step_over_control() {
    let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.actions_mut().list.push(action("Open", vec![("file.open", json!({"path": "/etc/passwd"})), ("layer.new.layer", json!({}))])).unwrap();
    let ctx = app.ctx().clone();
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Open"}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["ran"], 0, "{r}");
    assert_eq!(r["result"]["failed"]["id"], "file.open", "{r}");
    assert!(r["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{r}");
    assert!(app.session.documents().is_empty());
    assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the request");
}

#[test]
fn actions_play_checks_each_step_for_synthetic_input() {
    let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.actions_mut().list.push(action("Open", vec![("file.open", json!({"path": "/etc/passwd"})), ("layer.new.layer", json!({}))])).unwrap();
    app.automation_input = true;
    let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
    assert_eq!(r["ran"], 0, "{r}");
    assert_eq!(r["failed"]["id"], "file.open", "{r}");
    assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the command");

    // A local play (no automation input) still runs recorded steps.
    app.automation_input = false;
    app.session.actions_mut().list.delete_step(0, 0).unwrap();
    app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
    assert_eq!(r["ran"], 1, "{r}");
}

/// An injected shortcut acts for the automation client: Cmd+J asks `automation_authorize` before it
/// runs, and a refusal leaves the document as it was.
#[test]
fn an_injected_shortcut_asks_the_automation_policy() {
    thread_local!(static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    fn deny_via_copy(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
        if id == "layer.new.layerViaCopy" {
            ASKED.set(true);
            Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
        } else {
            Ok(())
        }
    }
    let services = crate::Services { automation_authorize: Some(deny_via_copy), ..Default::default() };
    let (mut h, transport) = crate::lifecycle_tests::app_harness(services, 1);
    let layers = h.state().session.active().unwrap().doc.layers.len();
    let reply = crate::lifecycle_tests::answered(&mut h, &transport, "ui.key", json!({"key": "J", "command": true}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(ASKED.get(), "the shortcut's command asked the policy");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "the refused command added no layer");
}

/// Frames with the framework runtime, whose window host injects queued input one step per pass.
fn input_frames() -> (craft::ui::testing::Frames, egui::Context) {
    let frames = craft::ui::testing::Frames::new();
    let ctx = frames.ctx.clone();
    (frames, ctx)
}

/// Runs passes until the window host has injected everything queued; each pass's events.
fn delivered(frames: &mut craft::ui::testing::Frames) -> Vec<Vec<egui::Event>> {
    let host = craft::ui::runtime::windows::WindowHost::of(&frames.ctx.clone());
    let mut passes = Vec::new();
    for _ in 0..4 * MAX_CLICKS {
        if host.injected_idle() {
            break;
        }
        let mut events = Vec::new();
        frames.frame(Vec::new(), |ui| events = ui.input(|i| i.events.clone()));
        passes.push(events);
    }
    assert!(host.injected_idle(), "the queued input ran");
    passes.retain(|events| !events.is_empty());
    passes
}

/// `method` accepted as synthetic input; the steps it delivered, one pass each.
fn queued(app: &mut PhotocraftApp, frames: &mut craft::ui::testing::Frames, method: &str, params: Value) -> Vec<Vec<egui::Event>> {
    let (req, _rx) = ControlRequest::new(method, params.clone());
    assert!(matches!(handle(app, &frames.ctx.clone(), &req), Reply::AfterInput), "{method} {params}");
    delivered(frames)
}

fn nothing_queued(ctx: &egui::Context) -> bool {
    craft::ui::runtime::windows::WindowHost::of(ctx).injected_idle()
}

/// What the window holds after each family of synthetic input, as the next pass sees it: held
/// presses and keys until `ui.release`, a move's and a wheel turn's modifiers, a click's let go,
/// Caps Lock held for the canvas, and document points mapped through the main canvas.
#[test]
fn synthetic_input_families_hold_what_they_name() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let (mut frames, ctx) = input_frames();
    let held = |frames: &mut craft::ui::testing::Frames| {
        let mut seen = (false, false, egui::Modifiers::NONE);
        frames.frame(Vec::new(), |ui| seen = ui.input(|i| (i.pointer.primary_down(), i.keys_down.contains(&egui::Key::Space), i.modifiers)));
        seen
    };
    // (method, params, primary down, Space down, alt held) after the call.
    let cases = [
        ("ui.click", json!({"x": 40, "y": 40, "hold": true}), true, false, false),
        ("ui.move", json!({"x": 60, "y": 40}), true, false, false),
        ("ui.release", json!({}), false, false, false),
        ("ui.move", json!({"x": 60, "y": 40, "alt": true}), false, false, true),
        ("ui.wheel", json!({"x": 60, "y": 40, "dy": -40, "alt": true}), false, false, true),
        ("ui.click", json!({"x": 60, "y": 40}), false, false, false),
        ("ui.key", json!({"key": "Space", "hold": true}), false, true, false),
        ("ui.click", json!({"x": 60, "y": 40, "hold": true}), true, true, false),
        ("ui.release", json!({"key": "Space"}), true, false, false),
        ("ui.release", json!({"button": "left"}), false, false, false),
        ("ui.key", json!({"key": "Space"}), false, false, false),
    ];
    for (method, params, primary, space, alt) in cases {
        queued(&mut app, &mut frames, method, params.clone());
        let (down, key, mods) = held(&mut frames);
        assert_eq!((down, key, mods.alt), (primary, space, alt), "after {method} {params}");
    }
    let host = craft::ui::runtime::windows::WindowHost::of(&ctx);
    for (on, held) in [(json!(true), Some(true)), (json!(false), Some(false)), (Value::Null, None)] {
        assert_eq!(call_after_input(&mut app, &ctx, "ui.capsLock", json!({ "on": on })), Value::Null);
        assert_eq!(host.injected_caps_lock(), held, "ui.capsLock {on}");
    }
    // Document points need a canvas; with one they land where the main canvas shows them.
    let r = call(&mut app, &ctx, "ui.move", json!({"x": 4, "y": 4, "coordinates": "document"}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("no document canvas")), "{r}");
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // The canvas shows the view in its first pass.
    crate::canvas::testing::frame(&mut app);
    let Some(view) = crate::canvas::document_display::active_view(&app) else { panic!("the new document has a view") };
    queued(&mut app, &mut frames, "ui.move", json!({"x": 16, "y": 8, "coordinates": "document"}));
    assert_eq!(ctx.input(|i| i.pointer.latest_pos()), Some(view.to_screen(craft::behavior::kurbo::Point::new(16.0, 8.0))));
}

/// The reply of an accepted request that waits for input, once the host has run it.
fn call_after_input(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params.clone());
    match handle(app, ctx, &req) {
        Reply::AfterInput => Value::Null,
        Reply::Done(v) => panic!("{method} {params}: {v}"),
        _ => panic!("{method} {params}: expected to wait for input"),
    }
}

/// #2449: the error of a rejected synthetic input call, after checking it changed nothing.
fn rejected(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value, names: &str) -> String {
    let (tool, revision) = (app.ui.tool, app.session.active().map(|d| d.revision));
    let r = call(app, ctx, method, params.clone());
    assert_eq!(r["ok"], false, "{method} {params}: {r}");
    let error = r["error"].as_str().unwrap().to_string();
    assert!(error.contains(names), "{method} {params}: `{error}` should name `{names}`");
    assert!(nothing_queued(ctx), "{method} {params}: a rejected call queues nothing");
    if method == "ui.pointer" {
        // The tool host's queue has no read: a pass after the refusal starts no gesture if
        // nothing was injected. `ctx` is the canvas's own context, where the host queues.
        crate::canvas::testing::frame(app);
        assert!(!crate::canvas::testing::in_gesture(app), "{method} {params}: a rejected call injects nothing");
    }
    assert_eq!(app.ui.tool, tool, "{method} {params}: a rejected call keeps the tool");
    assert_eq!(app.session.active().map(|d| d.revision), revision, "{method} {params}: a rejected call edits nothing");
    error
}

#[test]
fn ui_pointer_rejects_bad_params_before_acting() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let stroke = json!([{"kind": "down", "x": 5, "y": 5}, {"kind": "move", "x": 30, "y": 30}, {"kind": "up", "x": 30, "y": 30}]);
    // The same unknown tool is the same error as `ui.set`'s.
    let set = call(&mut app, &ctx, "ui.set", json!({"tool": "noSuchTool"}));
    let error = rejected(&mut app, &ctx, "ui.pointer", json!({"tool": "noSuchTool", "events": stroke}), "noSuchTool");
    assert_eq!(json!(error), set["error"]);
    let event = |extra: Value| {
        let mut e = json!({"kind": "down", "x": 5, "y": 5});
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        // A valid stroke first: a later bad event must not let the earlier ones run.
        json!({"tool": "move", "events": [stroke[0], stroke[1], stroke[2], e]})
    };
    let cases = [
        // Unknown fields, top-level, under `modifiers` and in an event.
        (json!({"tol": "move", "events": stroke}), "tol"),
        (json!({"modifiers": {"shfit": true}, "events": stroke}), "shfit"),
        (event(json!({"presure": 0.5})), "presure"),
        // Wrong JSON types.
        (json!({"tool": 3, "events": stroke}), "tool"),
        (json!({"events": "down"}), "events"),
        (json!({}), "events"),
        (json!({"events": [5]}), "events[0]"),
        (event(json!({"x": "abc"})), "x"),
        (event(json!({"y": true})), "y"),
        (event(json!({"kind": 1})), "kind"),
        (json!({"shift": "yes", "events": stroke}), "shift"),
        (json!({"modifiers": {"alt": 1}, "events": stroke}), "alt"),
        (json!({"modifiers": true, "events": stroke}), "modifiers"),
        (json!({"button": 2, "events": stroke}), "button"),
        (json!([1, 2]), "params"),
        // Values outside the accepted set.
        (json!({"button": "ritgh", "events": stroke}), "button"),
        (event(json!({"kind": "dwon"})), "kind"),
        // Numbers outside their documented range.
        (event(json!({"pressure": 99})), "pressure"),
        (event(json!({"pressure": -0.1})), "pressure"),
        (event(json!({"tiltX": 91})), "tiltX"),
        (event(json!({"tiltY": -90.5})), "tiltY"),
        (event(json!({"rotation": 361})), "rotation"),
        (event(json!({"rotation": -1})), "rotation"),
    ];
    for (params, names) in cases {
        rejected(&mut app, &ctx, "ui.pointer", params, names);
    }
    let error = rejected(&mut app, &ctx, "ui.pointer", json!({"button": "ritgh", "events": stroke}), "button");
    assert!(error.contains("right") && error.contains("secondary"), "the error lists the buttons: {error}");
    let error = rejected(&mut app, &ctx, "ui.pointer", event(json!({"kind": "dwon"})), "kind");
    assert!(error.contains("down") && error.contains("move") && error.contains("up"), "the error lists the kinds: {error}");

    // Valid calls still run: an omitted kind is a move, null is an omitted field, and the
    // documented range ends are accepted.
    let before = app.session.active().unwrap().revision;
    let pen = json!({"kind": "down", "x": 5, "y": 5, "pressure": 0, "tiltX": -90, "tiltY": 90, "rotation": 360});
    let valid = json!({"tool": "brush", "button": null, "modifiers": {"shift": false}, "events": [pen, {"x": 20, "y": 20, "pressure": 1}, {"kind": "up", "x": 20, "y": 20}]});
    let r = pointer(&mut app, valid);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(app.session.active().unwrap().revision > before, "the stroke painted");
    for button in ["left", "middle", "right", "secondary"] {
        let r = pointer(&mut app, json!({"tool": "move", "button": button, "events": [{"kind": "down", "x": 5, "y": 5}, {"kind": "up", "x": 5, "y": 5}]}));
        assert_eq!(r["ok"], true, "{button}: {r}");
    }
    assert!(matches!(app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind), Some(MenuKind::Layers { .. })), "a right click with Move still opens the layer menu");
}

#[test]
fn ui_screenshot_rejects_bad_params_before_capturing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    for (params, names) in [
        (json!({"focus": "yes"}), "focus"),
        (json!({"path": 5}), "path"),
        (json!({"pth": "shot.png"}), "pth"),
        // An unknown window id is refused naming `window`.
        (json!({"window": "no-such-window"}), "window"),
        (json!({"window": 3}), "window"),
    ] {
        rejected(&mut app, &ctx, "ui.screenshot", params, names);
    }
    let error = rejected(&mut app, &ctx, "ui.screenshot", json!({"pth": "shot.png"}), "pth");
    assert!(error.contains("window"), "the parameter list names `window`: {error}");
    for (params, path) in [(json!({}), None), (Value::Null, None), (json!({"path": "shot.png", "focus": false}), Some("shot.png"))] {
        let (req, _rx) = ControlRequest::new("ui.screenshot", params.clone());
        let Reply::Screenshot { path: queued, .. } = handle(&mut app, &ctx, &req) else { panic!("{params}: expected a capture") };
        assert_eq!(queued.as_deref(), path, "{params}");
    }
}

/// A key press's modifiers, as `ui.key` delivered them.
fn key_modifiers(app: &mut PhotocraftApp, frames: &mut craft::ui::testing::Frames, params: Value) -> egui::Modifiers {
    let passes = queued(app, frames, "ui.key", params.clone());
    match passes.iter().flatten().find(|e| matches!(e, egui::Event::Key { pressed: true, .. })) {
        Some(egui::Event::Key { modifiers, .. }) => *modifiers,
        other => panic!("{params}: expected a key event, got {other:?}"),
    }
}

#[test]
fn ui_key_ctrl_reaches_shortcuts_like_a_real_ctrl_press() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let (mut frames, _) = input_frames();
    let ctrl = key_modifiers(&mut app, &mut frames, json!({"key":"N","ctrl":true}));
    let command = key_modifiers(&mut app, &mut frames, json!({"key":"N","command":true}));
    let grouped = key_modifiers(&mut app, &mut frames, json!({"key":"N","modifiers":{"ctrl":true}}));
    assert_eq!(ctrl, grouped, "a grouped flag must match the top-level one");
    if cfg!(target_os = "macos") {
        // Two different keys: Control is not ⌘, and only ⌘ matches a shortcut.
        assert!(ctrl.ctrl && !ctrl.command && !ctrl.mac_cmd, "{ctrl:?}");
        assert!(command.command && command.mac_cmd && !command.ctrl, "{command:?}");
    } else {
        // One key under two names, and shortcuts are matched on `command`, so asking for
        // either must reach them: Ctrl+N has to open New Document, not do nothing.
        assert_eq!(ctrl, command, "ctrl and command are the same key off the Mac");
        assert!(ctrl.command && ctrl.ctrl && !ctrl.mac_cmd, "{ctrl:?}");
    }
    let plain = key_modifiers(&mut app, &mut frames, json!({"key":"N"}));
    assert_eq!(plain, egui::Modifiers::default(), "{plain:?}");
    let shift_alt = key_modifiers(&mut app, &mut frames, json!({"key":"N","shift":true,"alt":true}));
    assert!(shift_alt.shift && shift_alt.alt && !shift_alt.command && !shift_alt.ctrl, "{shift_alt:?}");
}

/// FILE-215-3: `filterMaskTarget` on a layer without a filter mask is refused before anything
/// changes, so the layer mask stays targeted.
#[test]
fn ui_set_filter_mask_target_needs_a_smart_object_with_filters() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": true}))["ok"], true);
    let bad = call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}));
    assert_eq!(bad["ok"], false, "{bad}");
    assert!(app.ui.mask_target, "the layer mask stays targeted");
    assert_eq!(app.ui.filter_mask_target, None);
    // A smart object with smart filters takes it.
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    // Not while a single colour channel is targeted: nothing would end up targeted.
    app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Color(0);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], false);
    app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], true);
    assert!(app.filter_mask_targeted());
    // Untargeting it ends its view, which would otherwise keep it targeted.
    app.run("paint.stroke", json!({"points": [[4, 4]], "size": 4, "hardness": 1, "color": "#000000", "target": "filterMask"})).unwrap();
    app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": false}))["ok"], true);
    assert!(!app.filter_mask_targeted());
    assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none());
    // Targeting the pixels untargets it too.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": false}))["ok"], true);
    assert!(!app.filter_mask_targeted());
    assert_eq!(crate::canvas::paint_target(&app), json!("pixels"));
}

/// FILE-215-3: with the filter mask shown (⌥-click its thumbnail), targeting the layer mask
/// ends that view, and the brush paints the layer mask.
#[test]
fn ui_set_mask_target_ends_a_filter_mask_view() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("paint.stroke", json!({"points": [[4, 4]], "size": 4, "hardness": 1, "color": "#000000", "target": "filterMask"})).unwrap();
    app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    app.sync_mask_targets();
    assert!(app.filter_mask_targeted());
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": true}))["ok"], true);
    assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none(), "the filter mask view ends");
    assert!(app.layer_mask_targeted());
    assert!(!app.filter_mask_targeted());
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"));
    let target = crate::canvas::paint_target(&app);
    app.run("paint.stroke", json!({"points": [[20, 20]], "size": 4, "hardness": 1, "color": "#000000", "target": target})).unwrap();
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(l.mask.as_ref().unwrap().value(20, 20), 0.0, "the stroke went into the layer mask");
    let photocraft_doc::LayerContent::Smart(sm) = &l.content else { panic!("smart object") };
    assert_eq!(sm.filter_mask.as_ref().unwrap().value(20, 20), 1.0, "not into the filter mask");
}

#[test]
fn automation_library_open_requires_explicit_save_as_before_implicit_save() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    use std::cell::RefCell;
    use std::rc::Rc;

    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let writes = written.clone();
    let services = crate::Services {
        import: Some(Box::new(|name: &str, _: &[u8], _: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
        })),
        export: Some(Box::new(|_: &Document, _: &str, _: &crate::ExportSettings| Ok((b"saved".to_vec(), Vec::new())))),
        automation_read: Some(Box::new(|path: &str| {
            Ok(crate::AutomationReadResult {
                name: "same.psd".into(),
                bytes: b"source".to_vec(),
                save_path: path.starts_with("work/").then(|| path.to_owned()),
            })
        })),
        automation_locate: Some(Box::new(|path: &str| Some(format!("/roots/{path}")))),
        automation_write: Some(Box::new(move |path: &str, _: &[u8]| {
            if path == "failed.psd" {
                return Err("test write failure".into());
            }
            writes.borrow_mut().push(path.to_owned());
            Ok(())
        })),
        write: Some(Box::new(|_, _| Err("ambient writer used".into()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    let opened = call(&mut app, &ctx, "app.open", json!({"path": "library/same.psd"}));
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(app.session.active().unwrap().path, None);
    assert_eq!(*app.ui.recent_files, vec!["/roots/library/same.psd"]);
    app.run("layer.new.layer", json!({"name": "edit"})).unwrap();
    assert!(app.session.active().unwrap().is_dirty());
    let refused = call(&mut app, &ctx, "app.save", json!({}));
    assert!(refused["error"].as_str().unwrap().contains("pass `path`"), "{refused}");
    assert!(written.borrow().is_empty());
    assert!(app.session.active().unwrap().is_dirty());
    let flat = call(&mut app, &ctx, "app.save", json!({"path": "copy.png"}));
    assert_eq!(flat["ok"], true, "{flat}");
    assert_eq!(app.session.active().unwrap().path, None);
    assert!(app.session.active().unwrap().is_dirty());
    let failed = call(&mut app, &ctx, "app.save", json!({"path": "failed.psd"}));
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(app.session.active().unwrap().path, None);
    assert!(app.session.active().unwrap().is_dirty());
    let saved = call(&mut app, &ctx, "app.save", json!({"path": "copy.psd"}));
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("copy.psd"));
    assert!(!app.session.active().unwrap().is_dirty());
    let again = call(&mut app, &ctx, "app.save", json!({}));
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(*written.borrow(), vec!["copy.png", "copy.psd", "copy.psd"]);
    let opened = call(&mut app, &ctx, "app.open", json!({"path": "work/same.psd"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let saved = call(&mut app, &ctx, "app.save", json!({}));
    assert_eq!(saved["result"]["path"], "work/same.psd", "{saved}");
}

fn shift_right(app: &mut PhotocraftApp, tool: &str) {
    pointer(
        app,
        json!({"tool": tool, "button": "right", "shift": true,
        "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "up", "x": 20, "y": 20}]}),
    );
}

/// Whether the canvas tool menu open now is the brush mode menu.
fn brush_mode_menu_open(app: &PhotocraftApp) -> bool {
    app.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::BrushMode { .. }))
}

/// `ui.context.choose` picks a Shift+right-click row by label; a bad or greyed one is an error
/// that changes nothing.
#[test]
fn shift_right_menu_choose_rejects_bad_choices_and_sets_the_tool() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    // Nothing open yet.
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Multiply"}))["ok"], false);
    shift_right(&mut app, "brush");
    let menu = inspect(&app, &ctx)["brushModeMenu"].clone();
    assert_eq!(menu["tool"], json!("Brush"));
    assert_eq!(menu["mode"], json!("Normal"));
    assert_eq!(menu["items"][0], json!({"label": "Edit Brush…", "id": "Edit Brush…", "enabled": true, "checked": false}));
    assert_eq!(menu["items"][1], json!({"separator": true}));
    // The Background has no transparency: Behind and Clear are greyed.
    assert!(menu["items"].as_array().unwrap().iter().any(|i| *i == json!({"label": "Clear", "id": "Clear", "enabled": false, "checked": false})));
    let journal = app.session.journal.len();
    for bad in [json!({"id": "Bogus"}), json!({"id": "Clear"}), json!({"id": "Behind"}), json!({"id": 3}), json!({}), json!({"id": "Desaturate"})] {
        let reply = call(&mut app, &ctx, "ui.context.choose", bad.clone());
        assert_eq!(reply["ok"], false, "{bad}: {reply}");
        assert!(brush_mode_menu_open(&app), "{bad}: a bad choice leaves the menu open");
    }
    assert_eq!(app.session.journal.len(), journal, "bad choices change nothing");
    assert!(!app.ui.tool_options.brush_clear && !app.session.tools.brush.behind);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Multiply"}))["ok"], true);
    assert!(!brush_mode_menu_open(&app));
    assert_eq!(app.session.tools.brush.mode, photocraft_doc::BlendMode::Multiply);
    assert_eq!(app.session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Multiply", "behind": false}})));
    // Another tool's rows.
    shift_right(&mut app, "eraser");
    assert_eq!(inspect(&app, &ctx)["brushModeMenu"]["choices"], json!(["Brush", "Pencil", "Block"]));
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Multiply"}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Pencil"}))["ok"], true);
    assert_eq!(app.ui.tool_options.eraser_mode, "pencil");
    shift_right(&mut app, "cloneStamp");
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "Edit Brush…"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!inspect(&app, &ctx)["brushPicker"].is_null() && !brush_mode_menu_open(&app));
    // A tool change closes the menu: its rows are no longer chosen.
    shift_right(&mut app, "dodge");
    app.ui.tool = crate::state::Tool::Sponge;
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Saturate"}))["ok"], false);
    assert_eq!(app.ui.tool_options.sponge_mode, "desaturate");
    // Magic Eraser has no menu.
    shift_right(&mut app, "magicEraser");
    assert!(inspect(&app, &ctx)["brushModeMenu"].is_null());
}

/// A fresh app with a document, as a control client finds it, and the context the control channel runs on.
fn app_with_document() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// The id `ui.menu.invoke {id, wait: false}` replied for the dialog it opened.
fn invoked_dialog(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) -> Value {
    let reply = call(app, ctx, "ui.menu.invoke", json!({"id": id, "wait": false}));
    assert_eq!(reply["ok"], true, "{id}: {reply}");
    crate::canvas::testing::frame(app);
    reply["result"]["dialog"].clone()
}

/// #15544: Keyboard Shortcuts and Menus is listed under the id its menu items reply with, and
/// `ui.dialog.cancel` with that id closes it, so a control client is never stuck behind it.
#[test]
fn keyboard_shortcuts_is_listed_and_cancelled_by_the_id_invoke_replies() {
    for command in ["edit.keyboardShortcuts", "edit.menus"] {
        let (mut app, ctx) = app_with_document();
        let id = invoked_dialog(&mut app, &ctx, command);
        assert!(id.is_string(), "{command}: the reply names the dialog: {id}");
        let listed = dialogs(&mut app, &ctx);
        assert!(listed.iter().any(|d| d["id"] == id), "{command}: {id} is listed in {listed:?}");
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "select.all"}))["ok"], false, "{command}: the dialog is modal");
        let cancelled = call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}));
        assert_eq!(cancelled["ok"], true, "{command}: {cancelled}");
        assert!(!crate::prefs_ui::shortcuts_open(&app), "{command}: Cancel closed it");
        assert!(dialogs(&mut app, &ctx).is_empty());
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], false, "{command}: nothing left to cancel");
        let next = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "select.all"}));
        assert_eq!(next["ok"], true, "{command}: commands run again: {next}");
    }
}

/// #15544: Preferences replies with the id `ui.inspect` lists it under, and cancelling that id closes it.
#[test]
fn preferences_replies_with_the_id_it_is_listed_under() {
    let (mut app, ctx) = app_with_document();
    let id = invoked_dialog(&mut app, &ctx, "edit.preferences.interface");
    let listed = dialogs(&mut app, &ctx);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["id"], id, "the reply is the listed id");
    assert_eq!(listed[0]["key"], "preferences");
    let cancelled = call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    crate::canvas::testing::frame(&mut app);
    assert!(dialogs(&mut app, &ctx).is_empty());
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "select.all"}))["ok"], true);
}

/// #15544: Select › Color Range… replies with the bare id, as every other dialog does, not a `Result` around it.
#[test]
fn color_range_replies_with_the_bare_dialog_id() {
    let (mut app, ctx) = app_with_document();
    let id = invoked_dialog(&mut app, &ctx, "select.colorRange");
    assert!(id.is_string(), "a bare id, not a wrapper: {id}");
    assert_eq!(top_dialog(&mut app, &ctx)["id"], id);
    assert_eq!(top_dialog(&mut app, &ctx)["key"], "select.colorRange");
    let cancelled = call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert!(dialogs(&mut app, &ctx).is_empty());
}

/// #15544: the unsaved-changes prompt is listed as `"discard"` with the command it holds back, the document it
/// asks about, how many are left and the choices, and `ui.dialog.confirm` / `ui.dialog.cancel` take that id
/// (docs/control-protocol.md).
#[test]
fn the_unsaved_changes_prompt_is_listed_as_discard_and_answered_by_that_id() {
    let (mut app, ctx) = app_with_document();
    app.run("layer.new.layer", json!({})).unwrap();
    let prompt = |app: &mut PhotocraftApp| dialogs(app, &ctx).into_iter().find(|d| d["id"] == "discard");
    assert!(prompt(&mut app).is_none());
    // Close the changed document: the prompt is listed; Cancel keeps the document.
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "file.close"}))["ok"], true);
    crate::canvas::testing::frame(&mut app);
    let listed = prompt(&mut app).unwrap_or_else(|| panic!("listed as discard: {:?}", dialogs(&mut app, &ctx)));
    let name = app.session.active().unwrap().doc.name.clone();
    assert_eq!(listed["fields"]["command"], "file.close");
    assert_eq!(listed["fields"]["document"], name.as_str());
    assert_eq!(listed["fields"]["remaining"], 1);
    assert_eq!(listed["fields"]["choices"], json!(["save", "dontSave", "cancel"]));
    // Its rect is keyed by that id too, as `rects` keys every hosted dialog by its `dialogs` id; others keep theirs.
    let host = crate::discard_ui::host_id(&app).expect("the prompt is open");
    assert_eq!(super::inspect::dialog_key(&app, host), "discard");
    let other = egui::Id::unique("another dialog");
    assert_eq!(super::inspect::dialog_key(&app, other), other.value().to_string());
    let cancelled = call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "discard"}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    crate::canvas::testing::frame(&mut app);
    assert!(prompt(&mut app).is_none());
    assert_eq!(app.session.documents().len(), 1, "Cancel keeps the document");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "discard"}))["ok"], false, "nothing left to answer");
    // Don't Save by that id closes it.
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "file.close"}))["ok"], true);
    crate::canvas::testing::frame(&mut app);
    let answered = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "discard", "choice": "dontSave"}));
    assert_eq!(answered["ok"], true, "{answered}");
    crate::canvas::testing::frame(&mut app);
    assert!(app.session.documents().is_empty(), "Don't Save closed the document");
}
