//! Record Again uses the real editor while retaining the selected step and permissions. These trials read the dialog
//! host (`app.framework`), so they live in the crate.
use std::sync::atomic::{AtomicUsize, Ordering};

use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands};
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::commands::CommandRef;
use egui_kittest::Harness;
use photocraft_engine::actions_cmds::{Action, ActionStep};
use photocraft_engine::{EngineError, Session};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};

/// Draw the actual dialog host over one selected command, without a GPU or file service.
fn harness(command: &str, params: Value) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":8,"height":6})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let mut action = Action::new("A", vec![ActionStep::new(command, params), ActionStep::new("select.all", json!({}))]);
    action.disabled.insert(0);
    action.dialogs.insert(0);
    app.session.actions.list.push(action).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Step(0, 0));
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1000.0, 760.0)).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// One control request through the public channel; the reply is the request's answer.
fn control(h: &mut Harness<'static, Box<PhotocraftApp>>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let reply = match handle(h.state_mut(), &ctx, &request) {
        Reply::Done(reply) => reply,
        _ => panic!("{method} answers at once"),
    };
    h.run_steps(3);
    reply
}

/// Open by firing the panel menu's Record Again row, preserving its selected-step and enablement checks.
fn open(h: &mut Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    let ctx = h.ctx.clone();
    assert!(super::menu::run_fired(h.state_mut(), &ctx, &CommandRef::from(ActionsPanelCommands::RecordAgain)), "the Actions panel owns the row");
    h.run_steps(3);
    h.state().framework.dialogs.stack().top().expect("the editor opened")
}

/// Sets one field of dialog `id` as an agent would.
fn set_field(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, field: &str, value: Value) -> Value {
    control(h, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}))
}

/// Answers OK on dialog `id`; the error is the reason the dialog refused.
fn confirm(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> Result<(), String> {
    let reply = control(h, "ui.dialog.confirm", json!({"dialog": id}));
    if reply["ok"] == true { Ok(()) } else { Err(reply["error"].as_str().unwrap_or("refused").to_string()) }
}

/// Cancels dialog `id`.
fn cancel(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) {
    control(h, "ui.dialog.cancel", json!({"dialog": id}));
}

/// The number the open dialog `id` lists under `key`, whatever kind of hosted dialog it is.
fn number(h: &Harness<'static, Box<PhotocraftApp>>, id: DialogId, key: &str) -> Option<f64> {
    h.state().framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id)?.fields?.get(key)?.as_f64()
}

/// Whether dialog `id` is still open.
fn is_open(h: &Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> bool {
    h.state().framework.dialogs.stack().contains(id)
}

/// A resize with different recorded and document sizes exposes seeding from the wrong source.
fn sizing() -> Harness<'static, Box<PhotocraftApp>> {
    harness("image.imageSize", json!({"width":4,"height":3,"resample":"nearest"}))
}

#[test]
fn indexed_path_record_again_keeps_the_recorded_target() {
    for params in [json!({"pathIndex":1,"translateX":2.0}), json!({"pathIndex":1,"matrix":[1,0,0,1,2,0]})] {
        let mut h = harness("path.transform", params);
        for (name, x) in [("First", 1), ("Second", 4)] {
            h.state_mut().run("path.set", json!({"name":name,"path":{"subpaths":[{"knots":[[x,1],[x+1,2]]}]}})).unwrap();
        }
        for p in &mut std::sync::Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc).paths {
            p.name = "Same".into();
        }
        h.state_mut().sync_views();
        h.state_mut().ui.selected_path = Some(crate::state::PathSelection::Saved { index: 0 });
        let before = h.state().session.active().unwrap().doc.paths.clone();
        let dialog = open(&mut h);
        assert_eq!(number(&h, dialog, "pathIndex"), Some(1.0));
        let authorized = crate::control::dialogs::hosted_command(h.state(), dialog).unwrap();
        assert_eq!(authorized.1["pathIndex"].as_f64(), Some(1.0));
        confirm(&mut h, dialog).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths[0], before[0]);
        assert_eq!(
            h.state().session.active().unwrap().doc.paths[1].path.subpaths[0].knots[0].anchor.x,
            6.0,
            "params={:?}; status={}",
            authorized.1,
            h.state().ui.status
        );
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths, before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths[1].path.subpaths[0].knots[0].anchor.x, 6.0);
    }
}

/// Cancel preserves the step and document; OK runs only that step and stores its new settings.
#[test]
fn seeded_editor_cancels_or_updates_only_the_original_step() {
    let mut h = sizing();
    let original = h.state().session.actions.list.clone();
    let revision = h.state().session.active().unwrap().revision;
    let dialog = open(&mut h);
    assert_eq!(number(&h, dialog, "width"), Some(4.0));
    assert_eq!(number(&h, dialog, "height"), Some(3.0));
    cancel(&mut h, dialog);
    assert_eq!(h.state().session.actions.list, original);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert!(!is_open(&h, dialog));
    let dialog = open(&mut h);
    set_field(&mut h, dialog, "constrain", json!(false));
    assert_eq!(set_field(&mut h, dialog, "width", json!(6))["ok"], true);
    assert_eq!(set_field(&mut h, dialog, "height", json!(5))["ok"], true);
    confirm(&mut h, dialog).unwrap();
    assert!(!h.state().dialog_open());
    let action = h.state().session.actions.list.action(0).unwrap();
    assert_eq!(action.steps.len(), 2);
    assert_eq!(action.steps[0].params["width"].as_f64(), Some(6.0));
    assert!(action.disabled.contains(&0));
    assert!(action.dialogs.contains(&0));
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_engine::doc::Size::new(6, 5));
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

/// Permit initial settings while denying a later edit even if the current gate is removed.
fn limited_resize(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" && params.get("width").and_then(Value::as_f64).is_some_and(|w| w >= 10.0) {
        Err(EngineError::Other("resize denied".into()))
    } else {
        Ok(())
    }
}

/// Changed definitions/documents, retained denials, invalid settings and retargeting cannot overwrite.
#[test]
fn stale_denied_or_failed_edit_preserves_the_stored_step() {
    for mode in ["action-stale", "document-stale", "denied", "retargeted", "failed"] {
        let mut h = sizing();
        if mode == "denied" {
            h.state_mut().session.authorize = Some(limited_resize);
        }
        let dialog = open(&mut h);
        // Whether the dialog took the edit: a value its row cannot show is refused when it is set.
        let mut accepted = true;
        match mode {
            "action-stale" => {
                h.state_mut().run("actions.rename", json!({"action":"A","name":"Changed"})).unwrap();
            }
            "document-stale" => {
                h.state_mut().run("file.new", json!({"width":3,"height":3})).unwrap();
            }
            "denied" => {
                h.state_mut().session.authorize = None;
                assert_eq!(set_field(&mut h, dialog, "width", json!(10))["ok"], true);
            }
            "retargeted" => {
                let other = ActionStep::new("select.all", json!({}));
                h.state_mut().session.actions.list.replace_step(0, 0, other).unwrap();
            }
            _ => {
                accepted = set_field(&mut h, dialog, "percent", json!(-1))["ok"] == true;
            }
        }
        let original = h.state().session.actions.list.clone();
        let revision = h.state().session.active().unwrap().revision;
        if accepted {
            assert!(confirm(&mut h, dialog).is_err(), "{mode}");
        }
        assert_eq!(h.state().session.actions.list, original, "{mode}");
        assert_eq!(h.state().session.active().unwrap().revision, revision, "{mode}");
        cancel(&mut h, dialog);
        let retry = open(&mut h);
        cancel(&mut h, retry);
    }
}

/// Confirmation of an impostor dialog leaves the original editor's private state intact.
#[test]
fn another_dialog_cannot_consume_the_selected_step() {
    let mut h = sizing();
    let original = h.state().session.actions.list.clone();
    let real = open(&mut h);
    let key = h.state().framework.dialogs.open_dialogs().into_iter().find(|open| open.id == real).and_then(|open| open.key).expect("the editor has a key");
    // A second hosted dialog with the same key and values, as another opener could make it.
    let impostor_key = key.clone();
    let ctx = h.ctx.clone();
    let fake = h.state_mut().framework.dialogs.open(
        HostedParamDialog::new(&ctx, key, DialogFields::new(), move |fields: &mut DialogFields| {
            let rows = vec![
                Param::new("width", "Width", ParamKind::number(None, None, 4.0, 1.0, 0)),
                Param::new("height", "Height", ParamKind::number(None, None, 3.0, 1.0, 0)),
            ];
            ParamDialog::command(impostor_key.clone(), "Impostor", rows).values(Bind::bound(fields))
        })
        .instance("impostor"),
    );
    h.run_steps(3);
    let _ = confirm(&mut h, fake);
    assert_eq!(h.state().session.actions.list, original, "the impostor's answer does not touch the stored step");
    assert!(is_open(&h, real), "the real editor is still waiting for its own answer");
    confirm(&mut h, real).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_engine::doc::Size::new(4, 3));
}

/// One pass of the preview routes the canvas draws: the filter route, then the adjustment route. The canvas asks them
/// both before it draws, and each checks the retained caller itself.
fn preview_routes(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::canvas::ensure_filter_preview(app, 0, ctx);
    crate::dialogs::adjust_preview::display_doc(app, 0);
}

static PREVIEW_CHECKS: AtomicUsize = AtomicUsize::new(0);

/// Count actual preview authorization and refuse LUT paths or edited blur and gamma settings.
fn preview_gate(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if matches!(id, "filter.blur.gaussianBlur" | "image.adjustments.levels" | "image.adjustments.colorLookup") {
        PREVIEW_CHECKS.fetch_add(1, Ordering::SeqCst);
        if params.get("file").is_some() || ["radius", "gamma"].iter().any(|key| params.get(key).and_then(Value::as_f64).is_some_and(|v| v > 2.0)) {
            return Err(EngineError::Other("preview denied".into()));
        }
    }
    Ok(())
}

/// Both real preview routes check the retained caller before evaluating changed settings or LUT files.
#[test]
fn filter_and_adjustment_previews_retain_permissions_before_evaluation() {
    for command in ["filter.blur.gaussianBlur", "image.adjustments.levels", "image.adjustments.colorLookup"] {
        let params = if command.starts_with("filter.") {
            json!({"radius":1})
        } else {
            photocraft_engine::adjust_params::to_params(
                &photocraft_engine::adjust_params::default_for(command.strip_prefix("image.adjustments.").unwrap(), photocraft_engine::doc::ColorMode::Rgb)
                    .unwrap(),
            )
        };
        let mut h = harness(command, params);
        h.state_mut().session.authorize = Some(preview_gate);
        let dialog = open(&mut h);
        h.state_mut().session.authorize = None;
        let context = h.ctx.clone();
        PREVIEW_CHECKS.store(0, Ordering::SeqCst);
        preview_routes(h.state_mut(), &context);
        assert!(PREVIEW_CHECKS.load(Ordering::SeqCst) > 0, "the permitted real preview request is authorized: {command}");
        if command == "image.adjustments.levels" {
            assert!(crate::dialogs::adjust_preview::display_doc(h.state_mut(), 0).is_some(), "the permitted adjustment preview is evaluated");
        }
        let original = h.state().session.actions.list.clone();
        let revision = h.state().session.active().unwrap().revision;
        if command.starts_with("filter.") {
            set_field(&mut h, dialog, "radius", json!(8));
        } else if command == "image.adjustments.levels" {
            set_field(&mut h, dialog, "gamma", json!(8));
            // This route parses settings even with Preview off; authorization must precede it.
            set_field(&mut h, dialog, "preview", json!(false));
        } else {
            set_field(&mut h, dialog, "file", json!("/outside/forbidden.cube"));
        }
        PREVIEW_CHECKS.store(0, Ordering::SeqCst);
        preview_routes(h.state_mut(), &context);
        assert!(PREVIEW_CHECKS.load(Ordering::SeqCst) > 0, "the changed actual request was refused before evaluation: {command}");
        assert_eq!(h.state().session.actions.list, original);
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        let answer = confirm(&mut h, dialog);
        assert!(answer.is_err(), "OK with the changed settings is refused: {command}");
        assert_eq!(h.state().session.actions.list, original);
    }
}

#[test]
fn recorded_matrix_numeric_edit_changes_translation_and_preserves_shear() {
    let mut h = harness("path.transform", json!({"pathIndex":0,"matrix":[1,0,0.5,1,2,0],"translateX":-20}));
    h.state_mut().run("path.set", json!({"name":"A","path":{"subpaths":[{"knots":[[4,2],[5,3]]}]}})).unwrap();
    let dialog = open(&mut h);
    // Legacy actions can contain shadowed scalar values. The opening rows show the live matrix.
    assert_eq!(number(&h, dialog, "translateX"), Some(2.0));
    assert_eq!(set_field(&mut h, dialog, "translateX", json!(8.0))["ok"], true);
    let (_, authorized) = crate::control::dialogs::hosted_command(h.state(), dialog).unwrap();
    assert_eq!(authorized["matrix"][4].as_f64(), Some(8.0));
    confirm(&mut h, dialog).unwrap();
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.paths[0].path.subpaths[0].knots[0].anchor.x, 13.0);
    let recorded = &h.state().session.actions.list.action(0).unwrap().steps[0].params;
    assert_eq!(recorded["matrix"][2].as_f64(), Some(0.5));
    assert_eq!(recorded["matrix"][4].as_f64(), Some(8.0));
}

#[test]
fn recorded_distortions_hide_scalar_controls_and_replay_the_original_geometry() {
    let mesh = photocraft_geom::warp::BezierMesh::identity([10.0, 10.0, 30.0, 20.0], 1, 1).map_points(|[x, y]| [x + 1.0, y]);
    for params in [
        json!({"pathIndex":0,"quad":[[12,8],[40,5],[45,30],[5,25]]}),
        json!({"pathIndex":0,"warp":{"style":"arc","bend":50,"bounds":[10,10,30,20]},"translateX":80}),
        json!({"pathIndex":0,"style":"arc","bend":50}),
        json!({"pathIndex":0,"mesh":mesh}),
    ] {
        let mut h = harness("path.transform", params.clone());
        h.state_mut().run("path.set", json!({"name":"A","path":{"subpaths":[{"closed":true,"knots":[[10,10],[30,10],[30,20],[10,20]]}]}})).unwrap();
        let before = h.state().session.active().unwrap().doc.paths.clone();
        let dialog = open(&mut h);
        for field in ["translateX", "translateY", "scaleX", "scaleY", "angle"] {
            assert_eq!(set_field(&mut h, dialog, field, json!(80))["ok"], false, "{field} accepted an ineffective edit");
        }
        assert_eq!(crate::control::dialogs::hosted_command(h.state(), dialog).unwrap().1, params);
        confirm(&mut h, dialog).unwrap();
        let recorded = h.state().session.actions.list.action(0).unwrap().steps[0].clone();
        assert_eq!(recorded.params, params);
        let after = h.state().session.active().unwrap().doc.paths.clone();
        assert_ne!(before, after);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths, before);

        let mut playback = Action::new("Replay", vec![recorded]);
        playback.dialogs.insert(0);
        h.state_mut().session.actions.list.push(playback).unwrap();
        let reply = h.state_mut().run("actions.play", json!({"action":"Replay"})).unwrap();
        assert!(reply.get("paused").is_some(), "{reply}");
        h.run_steps(3);
        let paused = h.state().framework.dialogs.stack().top().unwrap();
        assert_eq!(crate::control::dialogs::hosted_command(h.state(), paused).unwrap().1, params);
        confirm(&mut h, paused).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.paths, after);
        assert_eq!(h.state().session.actions.playing, 0);
    }
}
