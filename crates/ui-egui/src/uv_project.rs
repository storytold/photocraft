//! Scene view metadata lives in the shell; the engine stores it with the editable assets.
use crate::{PhotocraftApp, uv_ui};
use photocraft_uv::preview::Channel;
use serde_json::{Value, json};

pub(crate) fn snapshot(app: &PhotocraftApp) -> Value {
    let state = &app.ui.uv_workbench;
    let mut settings = crate::uv_workspace::preferences(app);
    for (key, value) in [
        ("yaw", json!(state.yaw)),
        ("pitch", json!(state.pitch)),
        ("zoom", json!(state.zoom)),
        ("pan", json!(state.pan)),
        ("target", json!(state.target)),
        ("select_shells", json!(state.select_shells)),
    ] {
        settings[key] = value;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut views = Vec::new();
    for channel in Channel::MAPS {
        for (material, id) in app.session.uv.map_bindings(channel).iter().enumerate() {
            if let Some(id) = id
                && seen.insert(id.0)
                && let Some(index) = app.session.documents().iter().position(|st| st.doc.id == *id)
                && let Some(view) = app.ui.views.get(index)
            {
                views.push(json!({"material":material,"channel":channel,"view":view}));
            }
        }
    }
    json!({"settings":settings,"views":views,"environment":app.uv_cache.lighting.path})
}
pub(crate) fn restore(app: &mut PhotocraftApp, view: &Value) -> Result<(), String> {
    if view.is_null() {
        return Ok(());
    }
    if let Some(settings) = view.get("settings") {
        crate::uv_workspace::restore_preferences(app, Some(settings));
        if let Ok(saved) = serde_json::from_value::<uv_ui::State>(settings.clone()) {
            let s = &mut app.ui.uv_workbench;
            let finite = |v: f64, fallback: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { fallback };
            s.yaw = finite(saved.yaw, 0.65, -1e6, 1e6);
            s.pitch = finite(saved.pitch, 0.65, -1.55, 1.55);
            s.zoom = finite(saved.zoom, 1.0, 0.01, 1e6);
            s.pan = saved.pan.map(|v| finite(v, 0.0, -1e15, 1e15));
            s.target = saved.target.filter(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 1e15));
            s.select_shells = saved.select_shells;
        }
    }
    app.sync_views();
    if let Some(views) = view.get("views").and_then(Value::as_array) {
        for entry in views {
            let Some(material) = entry.get("material").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()) else { continue };
            let Some(channel) = entry.get("channel").cloned().and_then(|v| serde_json::from_value::<Channel>(v).ok()) else { continue };
            let Some(id) = app.session.uv.map_bindings(channel).get(material).copied().flatten() else { continue };
            let Some(index) = app.session.documents().iter().position(|st| st.doc.id == id) else { continue };
            let Some(mut saved) = entry.get("view").cloned().and_then(|v| serde_json::from_value::<crate::state::View>(v).ok()) else { continue };
            let Some(doc) = app.session.documents().get(index) else { continue };
            saved.doc_size = [doc.doc.size.width, doc.doc.size.height];
            saved.zoom = if saved.zoom.is_finite() { saved.zoom.clamp(0.001, 128.0) } else { 1.0 };
            for (axis, value) in saved.center.iter_mut().enumerate() {
                if !value.is_finite() || value.abs() > 1e9 {
                    *value = saved.doc_size.get(axis).copied().unwrap_or(0) as f32 * 0.5;
                }
            }
            saved.rotation = if saved.rotation.is_finite() { saved.rotation % 360.0 } else { 0.0 };
            if let Some(v) = app.ui.views.get_mut(index) {
                *v = saved;
            }
        }
    }
    crate::uv_lighting::select_environment(app, String::new())?;
    if let Some(path) = view.get("environment").and_then(Value::as_str).filter(|path| !path.is_empty()) {
        let has_embedded_environment =
            app.session.uv.environment.is_some() && app.session.uv.project_view.get("environment").and_then(Value::as_str) == Some(path);
        if has_embedded_environment {
            crate::uv_lighting::select_environment(app, path.to_owned())?;
        } else {
            app.ui.uv_workbench.status = tl!("This project does not contain its HDR/EXR environment map. Choose the map again.").into();
        }
    }
    Ok(())
}
pub(crate) fn save_to(app: &mut PhotocraftApp, path: String) -> Result<Value, String> {
    uv_ui::reveal(app)?;
    if crate::uv_lighting::pending(&app.uv_cache.lighting) {
        return Err(tl!("Wait for the environment map to finish preparing, then save the project again").into());
    }
    uv_ui::finish_paint(app);
    if let Some([x, y, _]) = app.drag.as_ref().and_then(|d| d.points.last()).copied() {
        crate::canvas::tool_event(app, crate::canvas::ToolEvent::Up { x, y }, egui::Modifiers::NONE);
    }
    let view = snapshot(app);
    let result = app.run("uv.project.save", json!({"path":path,"view":view}));
    match &result {
        Ok(_) => {
            app.ui.uv_workbench.project_path = path;
            app.ui.uv_workbench.status.clear();
        }
        Err(error) => app.ui.uv_workbench.status = crate::i18n::fmt(tl!("Could not save scene: {error}"), &[("error", error)]),
    }
    result
}
#[derive(Default)]
pub(crate) struct Ui {
    prompt: Option<(String, Value)>,
    skip_once: bool,
    exit_approved: bool,
    pub(crate) parked_view: Option<Value>,
}
pub(crate) fn dirty(app: &PhotocraftApp) -> bool {
    if app.session.uv.scene.is_none() {
        return false;
    }
    let docs = app.session.uv_documents();
    let saved = &app.session.uv.project_view;
    let view = if app.session.uv_workspace.active { Some(snapshot(app)) } else { app.uv_cache.project.parked_view.clone() };
    crate::uv_lighting::pending(&app.uv_cache.lighting)
        || crate::uv_mask_paint_preview::pending(app).is_some()
        || app.session.uv.has_unsaved_project(docs)
        || (!saved.is_null() && view.as_ref().is_some_and(|v| saved_view_changed(v, saved)))
}

/// View fit/fill are one-frame requests that should be saved and restored, but they do not make
/// an otherwise unchanged 3D project dirty while egui is waiting for a canvas layout.
fn saved_view_changed(view: &Value, saved: &Value) -> bool {
    fn without_pending_fit(value: &Value) -> Value {
        let mut value = value.clone();
        if let Some(views) = value.get_mut("views").and_then(Value::as_array_mut) {
            for entry in views {
                if let Some(view) = entry.get_mut("view")
                    && let Some(object) = view.as_object_mut()
                {
                    object.insert("fit_pending".into(), Value::Bool(false));
                    object.insert("fill_pending".into(), Value::Bool(false));
                }
            }
        }
        value
    }
    without_pending_fit(view) != without_pending_fit(saved)
}
pub(crate) fn close(app: &mut PhotocraftApp) {
    let in_3d = app.ui.workspace == "3D";
    uv_ui::finish_paint(app);
    if let Err(e) = app.run("uv.project.close", json!({})) {
        app.ui.status = e;
        return;
    }
    app.ui.uv_workbench.project_path.clear();
    app.uv_cache.project = Default::default();
    app.uv_cache.library = Default::default();
    if in_3d {
        app.ui.workspace = "Essentials".into();
        crate::menus::apply_workspace(app);
    } else if let Err(error) = crate::uv_workspace::sync_workspace(app) {
        // The engine close activates and clears the UV bank before this UI transition.
        // If ordinary-bank activation fails, keep the workspace label aligned with the bank.
        app.ui.workspace = "3D".into();
        app.ui.status = error;
        app.ui.status_error = true;
    }
    app.uv_cache.project = Default::default();
}
/// One project question replaces a sequence of texture-file questions.
pub(crate) fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> bool {
    let native_close = app.ui.workspace == "3D" && matches!(id, "file.close" | "file.closeAll");
    let destructive = id == "file.exit" || native_close || matches!(id, "file.uv.closeProject" | "file.uv.import" | "file.uv.openProject");
    if !destructive {
        return false;
    }
    let skip = std::mem::take(&mut app.uv_cache.project.skip_once);
    uv_ui::finish_paint(app);
    if !skip && dirty(app) {
        if app.uv_cache.project.prompt.is_none() {
            app.uv_cache.project.prompt = Some((if native_close { "file.uv.closeProject" } else { id }.to_owned(), params.clone()));
        }
        return true;
    }
    if native_close {
        close(app);
        return true;
    }
    if id == "file.exit" {
        app.uv_cache.project.exit_approved = true;
    }
    if id == "file.exit" && app.session.uv_workspace.active {
        // The project has already been answered; native prompts now cover only ordinary docs.
        let previous_workspace = app.ui.workspace.clone();
        app.ui.workspace = "Essentials".into();
        if let Err(error) = crate::uv_workspace::sync_workspace(app) {
            app.ui.workspace = previous_workspace;
            app.ui.status = error;
            app.ui.status_error = true;
            app.uv_cache.project.exit_approved = false;
            return true;
        }
        crate::menus::apply_workspace(app);
    }
    false
}
fn resume(app: &mut PhotocraftApp, ctx: &egui::Context, action: (String, Value)) {
    app.uv_cache.project.skip_once = true;
    if let Err(e) = crate::menus::invoke(app, ctx, &action.0, action.1) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
pub(crate) fn prompt(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.uv_cache.project.prompt.is_none() {
        return;
    }
    let mut answer = 0;
    egui::Modal::new(egui::Id::unique("uv-container-discard")).show(ctx, |ui| {
        ui.set_max_width(430.0);
        ui.heading(tl!("Save changes to the 3D project?"));
        ui.label(tl!("The .pcuv container saves layered textures, library images, the model and scene settings. Export Textures is not required."));
        ui.horizontal(|ui| {
            if ui.button(tl!("Save project")).clicked() {
                answer = 1;
            }
            if ui.button(tl!("Don't save")).clicked() {
                answer = 2;
            }
            if ui.button(tl!("Cancel")).clicked() {
                answer = 3;
            }
        });
    });
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        answer = 3;
    }
    if answer == 0 && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        answer = 1;
    }
    if answer == 0 {
        return;
    }
    let Some(action) = app.uv_cache.project.prompt.take() else {
        return;
    };
    match answer {
        1 => {
            let path = app.session.uv.project_path.clone();
            if !path.is_empty() {
                match save_to(app, path) {
                    Ok(_) => resume(app, ctx, action),
                    Err(e) => {
                        app.uv_cache.project.prompt = Some(action);
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            } else {
                let ctx = ctx.clone();
                let retry_action = action.clone();
                let open_error_action = action.clone();
                if let Err(e) = app.pick_save("model.pcuv", move |app, path| {
                    let path = if path.to_ascii_lowercase().ends_with(".pcuv") { path } else { format!("{path}.pcuv") };
                    match save_to(app, path) {
                        Ok(_) => {
                            resume(app, &ctx, action);
                            Ok(Value::Null)
                        }
                        Err(error) => {
                            app.uv_cache.project.prompt = Some(retry_action);
                            Err(error)
                        }
                    }
                }) {
                    app.uv_cache.project.prompt = Some(open_error_action);
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
        }
        2 => {
            // "Don't save" discards transient Paint/Fill previews with the project itself.
            // Save errors leave the preview intact so the user can retry or explicitly discard.
            crate::uv_mask_paint_preview::cancel(app);
            resume(app, ctx, action);
        }
        _ => {}
    }
}

pub(crate) fn guard_exit(app: &mut PhotocraftApp) -> bool {
    if std::mem::take(&mut app.uv_cache.project.exit_approved) { false } else { intercept(app, "file.exit", &Value::Null) }
}
pub(crate) fn cancel_exit(app: &mut PhotocraftApp) {
    app.uv_cache.project.exit_approved = false;
}

pub(crate) fn prompt_open(app: &PhotocraftApp) -> bool {
    app.uv_cache.project.prompt.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_save_restores_camera_pbr_texture_views_and_keeps_errors_visible() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut s = photocraft_engine::Session::new();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        s.execute("uv.model.import", json!({"path":source})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        app.sync_views();
        app.ui.uv_workbench.yaw = 1.1;
        app.ui.uv_workbench.pan = [0.2, 0.3];
        app.ui.uv_workbench.zoom = 3.0;
        app.ui.uv_workbench.target = Some([1.0, 2.0, 3.0]);
        app.ui.uv_workbench.split_view = true;
        app.ui.uv_workbench.lighting.brightness = 2.4;
        app.ui.uv_workbench.lighting.rotation = 37.0;
        app.ui.uv_workbench.lighting.pbr = false;
        let index = app.session.active_index().unwrap();
        app.ui.views[index].zoom = 5.0;
        app.ui.views[index].center = [31.0, 44.0];
        app.ui.views[index].fit_pending = false;
        let path = std::env::temp_dir().join(format!("uv-view-project-{}.pcuv", std::process::id()));
        save_to(&mut app, path.to_string_lossy().into_owned()).unwrap();
        app.ui.uv_workbench.yaw = 1.7;
        save_to(&mut app, path.to_string_lossy().into_owned()).unwrap();
        let result = app.run("uv.project.load", json!({"path":path})).unwrap();
        app.ui.uv_workbench = Default::default();
        restore(&mut app, &result["view"]).unwrap();
        assert_eq!(app.ui.uv_workbench.yaw, 1.7);
        assert_eq!(app.ui.uv_workbench.zoom, 3.0);
        assert_eq!(app.ui.uv_workbench.target, Some([1.0, 2.0, 3.0]));
        assert_eq!(app.ui.uv_workbench.pan, [0.2, 0.3]);
        assert_eq!(app.ui.uv_workbench.lighting.brightness, 2.4);
        assert_eq!(app.ui.uv_workbench.lighting.rotation, 37.0);
        assert!(!app.ui.uv_workbench.lighting.pbr);
        assert!(app.ui.uv_workbench.split_view);
        let index = app.session.active_index().unwrap();
        assert_eq!(app.ui.views[index].zoom, 5.0);
        assert_eq!(app.ui.views[index].center, [31.0, 44.0]);
        let invalid = path.join("missing.pcuv");
        assert!(save_to(&mut app, invalid.to_string_lossy().into_owned()).is_err());
        let message = app.ui.uv_workbench.status.clone();
        assert!(message.contains("Could not save"));
        app.run("uv.model.checkSource", json!({})).unwrap();
        assert_eq!(app.ui.uv_workbench.status, message);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;
    #[test]
    fn restoring_external_only_environment_uses_studio_without_opening_the_path() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let view = json!({"environment":"missing-project-resource.exr"});
        app.session.uv.project_view = view.clone();
        restore(&mut app, &view).unwrap();
        assert!(app.uv_cache.lighting.path.is_empty());
        assert!(!crate::uv_lighting::pending(&app.uv_cache.lighting), "metadata never starts a filesystem loading task");
        assert!(app.ui.uv_workbench.status.contains("Choose the map again"));
    }

    fn app() -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":16,"height":16})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        app.sync_views();
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.workspace.threeD", json!({})).unwrap();
        let mesh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        app.run("uv.model.import", json!({"path":mesh})).unwrap();
        app.run("uv.texture.create", json!({"width":32,"height":32})).unwrap();
        app
    }
    #[test]
    fn uv_container_save_quit_and_workspace_scope_preserve_normal_canvas() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = app();
        app.sync_views();
        let ctx = egui::Context::default();
        let uv = app.session.active().unwrap().doc.id;
        let normal = app.session.uv_workspace.parked()[0].doc.id;
        let i = app.session.active_index().unwrap();
        app.ui.views[i].zoom = 5.0;
        app.ui.views[i].fit_pending = false;
        let path = std::env::temp_dir().join(format!("uv-container-ui-{}.pcuv", std::process::id()));
        save_to(&mut app, path.to_string_lossy().into_owned()).unwrap();
        assert!(!dirty(&app));
        app.ui.views[i].fit_pending = true;
        app.ui.views[i].fill_pending = true;
        assert!(!dirty(&app), "one-frame fit/fill requests do not create a project change");
        app.ui.views[i].fit_pending = false;
        app.ui.views[i].fill_pending = false;
        crate::menus::invoke(&mut app, &ctx, "window.workspace.photography", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.id, normal);
        assert!(app.session.documents().iter().all(|st| st.doc.id != uv));
        assert!(!dirty(&app));
        crate::menus::invoke(&mut app, &ctx, "window.workspace.threeD", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.id, uv);
        assert_eq!(app.ui.views[app.session.active_index().unwrap()].zoom, 5.0);
        app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        assert!(dirty(&app));
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        assert!(!dirty(&app), "Ctrl+S saves the container");
        assert!(app.session.uv.needs_export(app.session.active().unwrap()));
        assert!(!intercept(&mut app, "file.exit", &json!({})));
        assert!(app.uv_cache.project.prompt.is_none());
        assert!(app.discard.is_none());
        assert_eq!(app.session.active().unwrap().doc.id, normal);
        assert!(!guard_exit(&mut app), "native close does not repeat project question");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn uv_hidden_unsaved_project_gets_one_container_question() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = app();
        app.ui.workspace = "Painting".into();
        crate::menus::apply_workspace(&mut app);
        assert!(dirty(&app));
        assert!(intercept(&mut app, "file.exit", &json!({})));
        assert!(app.discard.is_none());
        assert_eq!(app.uv_cache.project.prompt.as_ref().unwrap().0, "file.exit");
    }

    #[test]
    fn pending_fill_preview_prompts_before_close_and_stale_save_keeps_preview() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = app();
        app.run("layer.layerMask.hideAll", json!({})).unwrap();
        app.run("uv.maskPaint.add", json!({"kind":"fill","value":1.0})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let path = std::env::temp_dir().join(format!("uv-preview-close-{}.pcuv", std::process::id()));
        save_to(&mut app, path.to_string_lossy().into_owned()).unwrap();
        assert!(!dirty(&app));

        crate::uv_mask_paint_preview::set_fill(&mut app, layer, 0, [4, 4, 24, 24], 12.0);
        let preview = crate::uv_mask_paint_preview::pending(&app).expect("the Fill edit is transient");
        assert!(dirty(&app), "a pending Fill edit is part of the unsaved project");
        assert!(intercept(&mut app, "file.close", &json!({})), "native close must ask before dropping the project");
        assert_eq!(app.uv_cache.project.prompt.as_ref().unwrap().0, "file.uv.closeProject");
        assert_eq!(crate::uv_mask_paint_preview::pending(&app).unwrap().params(), preview.params());

        // Bypass PhotocraftApp::run to model a revision change from a concurrent document edit.
        app.session.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        let changed_revision = app.session.active().unwrap().revision;
        assert!(save_to(&mut app, path.to_string_lossy().into_owned()).is_err(), "a stale preview cannot be saved");
        assert_eq!(app.session.active().unwrap().revision, changed_revision);
        assert_eq!(crate::uv_mask_paint_preview::pending(&app).unwrap().params(), preview.params());
        assert!(dirty(&app), "the failed save keeps the project dirty and retryable");
        std::fs::remove_file(path).unwrap();
    }
}
