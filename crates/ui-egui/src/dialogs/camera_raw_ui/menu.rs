use super::controls::{commit, redo_edit, remember_edit, undo_edit};
use super::open::{cancel, open, open_smart_filter};
use super::sidecar::{load_settings, save_settings};
use super::{CameraRaw, LayerId, PhotocraftApp, Value, json};

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "filter.cameraRaw" {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("RAW Develop params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(open(app, ctx).map(|_| super::describe(app).unwrap_or(Value::Null)));
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(super::describe(app).unwrap_or(Value::Null))
        })();
        return Some(opened);
    }
    let ui = params.get("ui")?;
    // (the brush's Auto Mask from its preference before this request changes it)
    super::controls::sync_auto_mask(app);
    let Some(fields) = ui.as_object() else {
        return Some(Err("RAW Develop ui must be an object".into()));
    };
    for (key, value) in fields {
        match key.as_str() {
            "set" | "scope" | "view" | "demosaic" | "filmstrip" | "develop" | "local" => {}
            "before" | "commit" | "cancel" | "done" | "saveSettings" | "loadSettings" | "undo" | "redo" | "auto" | "expandAll" if value.is_boolean() => {}
            "before" | "commit" | "cancel" | "done" | "saveSettings" | "loadSettings" | "undo" | "redo" | "auto" | "expandAll" => {
                return Some(Err(format!("RAW Develop {key} must be a boolean")));
            }
            _ => return Some(Err(format!("unknown RAW Develop ui property {key}"))),
        }
    }
    let ends = ["commit", "cancel", "done"].into_iter().filter(|k| ui.get(*k).and_then(Value::as_bool) == Some(true)).count();
    if ends > 1 {
        return Some(Err("RAW Develop takes one of commit, cancel and done".into()));
    }
    for (key, run) in [("saveSettings", save_settings as fn(&mut PhotocraftApp) -> Result<Value, String>), ("loadSettings", load_settings)] {
        if ui.get(key).and_then(Value::as_bool) == Some(true) {
            if fields.len() > 1 {
                return Some(Err(format!("RAW Develop {key} must be sent on its own")));
            }
            return Some(run(app));
        }
    }
    // A filmstrip click may show another raw (a new dialog), so it comes on its own.
    if let Some(film) = ui.get("filmstrip") {
        if fields.len() > 1 {
            return Some(Err("Camera Raw filmstrip must be sent on its own".into()));
        }
        return Some(super::filmstrip::request(app, ctx, film).map(|()| super::describe(app).unwrap_or(Value::Null)));
    }
    let was_closed = app.camera_raw.is_none();
    let previous_view = app.ui.camera_raw_scope.clone();
    let previous_navigation = app.ui.camera_raw_preview;
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let result = menu_update(app, ctx, ui);
    if was_closed && result.as_ref().is_some_and(Result::is_err) {
        app.camera_raw = None;
        app.ui.camera_raw_scope = previous_view;
        app.ui.camera_raw_preview = previous_navigation;
    }
    result
}

/// The control channel's `develop` (a partial of the shared develop settings, merged as
/// LightCraft's `develop.merge` merges), checked: an error for a dialog without develop settings
/// or a partial that doesn't merge.
fn develop_partial(d: &super::CameraRawDialog, v: Option<&Value>) -> Result<Option<Value>, String> {
    let Some(v) = v else { return Ok(None) };
    let e = d.develop.as_ref().ok_or("develop settings apply only to a raw the shared develop engine develops")?;
    if !v.is_object() {
        return Err("bad develop settings: expected an object".into());
    }
    e.settings.merged(v).map_err(|err| format!("bad develop settings: {err}"))?;
    Ok(Some(v.clone()))
}

/// Merges partial `v` into the dialog's develop settings, as one undo step.
fn merge_develop(d: &mut super::CameraRawDialog, v: &Value) {
    d.sync_develop();
    let Some(e) = d.develop.as_ref() else { return };
    let Ok(next) = e.settings.merged(v) else { return };
    if next == e.settings {
        return;
    }
    remember_edit(d);
    if let Some(e) = d.develop.as_mut() {
        e.settings = next;
        d.params = e.camera_raw(&d.params);
    }
    d.dirty = true;
}

fn menu_update(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &Value) -> Option<Result<Value, String>> {
    // Stage both filter and presentation updates. No texture, preference or dialog mutation
    // occurs until every part of the request has passed validation.
    let d = app.camera_raw.as_ref()?;
    if ui.get("done").and_then(Value::as_bool) == Some(true) && d.target != super::open::Target::OpenRaw {
        return Some(Err("Done is for raws being opened".into()));
    }
    let mut next_params = d.params.clone();
    let auto = ui.get("auto").and_then(Value::as_bool) == Some(true);
    // `develop`: a partial of the shared develop settings (a raw the shared engine develops),
    // merged over them as LightCraft's `develop.merge` merges; checked here, applied below.
    let develop_partial = match develop_partial(d, ui.get("develop")) {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    // Auto (the Light panel's button): the tone sliders from the image's histogram (with the
    // shared engine, the engine's Auto).
    if auto && let Some(p) = d.develop_auto() {
        next_params = p;
    } else if auto {
        photocraft_algo::camera_raw::auto_tone(&d.proxy, &mut next_params);
    }
    if let Some(set) = ui.get("set") {
        if !set.is_object() {
            return Some(Err("bad RAW Develop settings: expected an object".into()));
        }
        let mut cur = serde_json::to_value(&d.params).unwrap_or(json!({}));
        if let (Value::Object(c), Value::Object(s)) = (&mut cur, set) {
            for (k, v) in s {
                if !c.contains_key(k) {
                    return Some(Err(format!("unknown RAW Develop setting {k}")));
                }
                c.insert(k.clone(), v.clone());
            }
        }
        next_params = match serde_json::from_value::<CameraRaw>(cur) {
            Ok(p) => p,
            Err(e) => return Some(Err(format!("bad RAW Develop settings: {e}"))),
        };
        if let Err(e) = next_params.validate_changes(&d.original) {
            return Some(Err(format!("bad RAW Develop settings: {e}")));
        }
    }
    let scope_update = match ui.get("scope") {
        Some(scope) => match crate::camera_raw_scope_ui::prepare_control(&next_params, d.selection.is_some(), &app.ui.camera_raw_scope, scope) {
            Ok(update) => Some(update),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let navigation = match ui.get("view") {
        Some(view) => match crate::camera_raw_preview_ui::prepare(app.ui.camera_raw_preview, view) {
            Ok(next) => Some(next),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let demosaic = match ui.get("demosaic") {
        None => None,
        Some(_) if d.raw_open.is_none() => return Some(Err("demosaic applies only while a raw file opens".into())),
        Some(v) => match v.as_str().and_then(photocraft_io::raw::Demosaic::from_id) {
            Some(m) => Some(m),
            None => return Some(Err("demosaic must be ahd, mhc or bilinear".into())),
        },
    };
    let d = app.camera_raw.as_mut()?;
    if let (Some(m), Some(raw)) = (demosaic, d.raw_open.as_mut()) {
        raw.demosaic = m;
    }
    if let Some(view) = navigation {
        app.ui.camera_raw_preview = view;
    }
    if d.params.point_curve != next_params.point_curve {
        d.curve_state = Default::default();
    }
    if d.params != next_params {
        remember_edit(d);
        d.dirty = true;
        d.params = next_params;
    }
    if let Some(v) = &develop_partial {
        merge_develop(d, v);
    }
    if let Some(on) = ui.get("expandAll").and_then(Value::as_bool) {
        d.develop_panel.expand_all = on;
    }
    if let Some(v) = ui.get("local")
        && let Err(e) = local_update(d, v)
    {
        return Some(Err(e));
    }
    super::controls::sync_auto_mask(app);
    let Some(d) = app.camera_raw.as_mut() else { return Some(Err("RAW Develop closed".into())) };
    if ui.get("auto").and_then(Value::as_bool) == Some(true) {
        d.auto_all = true;
    }
    if let Some(b) = ui.get("before").and_then(Value::as_bool) {
        d.show_before = b;
    }
    if let Some(update) = scope_update {
        update.apply(d, &mut app.ui.camera_raw_scope);
    }
    if ui.get("undo").and_then(Value::as_bool) == Some(true) {
        undo_edit(d);
    }
    if ui.get("redo").and_then(Value::as_bool) == Some(true) {
        redo_edit(d);
    }
    if d.dirty {
        d.render(ctx);
    }
    // The control channel can read the status before the next frame draws.
    d.clear_recovered_error(&mut app.ui);
    if app.ui.camera_raw_scope.vectorscope
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.ensure_scope(app.ui.camera_raw_scope.selected_region);
    }
    crate::camera_raw_scope_ui::persist(app, ctx);
    // The edit reaches the other raws selected in the filmstrip before Open or Done reads them.
    super::filmstrip::sync_selected(app);
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        cancel(app);
        return Some(Ok(json!({"cancelled": true})));
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return Some(commit(app));
    }
    if ui.get("done").and_then(Value::as_bool) == Some(true) {
        return Some(Ok(super::filmstrip::done(app)));
    }
    super::filmstrip::refresh_thumbnails(app, true);
    super::filmstrip::refresh_shown_thumbnail(app);
    Some(Ok(super::describe(app).unwrap_or(Value::Null)))
}

/// A point `[x, y]` on the shown frame (0..1 across it).
fn frame_point(v: &Value) -> Option<craft_geom::Point> {
    let a = v.as_array()?;
    let (x, y) = (a.first()?.as_f64()?, a.get(1)?.as_f64()?);
    (x.is_finite() && y.is_finite()).then(|| craft_geom::Point::new(x, y))
}

/// The control channel's `local`: the develop view's tools, as the rail and their panels and
/// gestures drive them. `tool` ("edit", "heal", "masking", "redEye"), `sections` (the Edit panel's
/// open sections and flyouts), `newMask` (a kind: brush, linear, radial, luminanceRange,
/// colorRange), `addComponent` ({kind, op: add | subtract}), `selectMask` (an id, or null),
/// `overlay` (show the selected mask's overlay), `brush` ({points: [[x, y]…] on the shown frame,
/// size, feather, flow, erase, autoMask}: a stroke painted into the selected brush component, or
/// with the Brush armed and no mask selected into a new brush mask; without points only the
/// brush's settings change), `heal`
/// ({at: [x, y], mode: heal | clone | remove}: a spot placed as a click places it), `redEye`
/// ({at: [x, y], pet}). Checked whole before anything changes; one undo step.
fn local_update(d: &mut super::CameraRawDialog, v: &Value) -> Result<(), String> {
    use super::local;
    let o = v.as_object().ok_or("RAW Develop local must be an object")?;
    let edit = d.develop.as_ref().ok_or("the develop view's tools apply only to a raw the shared develop engine develops")?;
    let mut settings = edit.settings.clone();
    let mut st = d.local.clone();
    let (mut tool, mut sections) = (d.tool, None);
    let map = st.view_map.clone();
    let result = (|| -> Result<(), String> {
        for (key, value) in o {
            match key.as_str() {
                "tool" => tool = value.as_str().and_then(super::rail::Tool::from_id).ok_or("tool must be edit, heal, masking or redEye")?,
                "sections" => {
                    let list = value.as_array().ok_or("sections must be a list of section ids")?;
                    sections = Some(
                        list.iter().map(|s| s.as_str().map(str::to_string).ok_or("sections must be a list of section ids")).collect::<Result<Vec<_>, _>>()?,
                    );
                }
                "newMask" => {
                    local::add_mask(&mut st, &mut settings, value.as_str().ok_or("newMask must be a mask kind")?)?;
                }
                "addComponent" => {
                    let kind = value.get("kind").and_then(Value::as_str).ok_or("addComponent needs a kind")?;
                    let op = match value.get("op").and_then(Value::as_str).unwrap_or("add") {
                        "add" => craft_develop_model::MaskOp::Add,
                        "subtract" => craft_develop_model::MaskOp::Subtract,
                        _ => return Err("addComponent op must be add or subtract".into()),
                    };
                    local::add_component(&mut st, &mut settings, kind, op)?;
                }
                "selectMask" => {
                    let id = match value {
                        Value::Null => None,
                        v => {
                            let id = v.as_u64().and_then(|i| u32::try_from(i).ok()).ok_or("selectMask must be a mask id")?;
                            Some(settings.masks.iter().any(|m| m.id == id).then_some(id).ok_or(format!("no mask {id}"))?)
                        }
                    };
                    local::select(&mut st, &settings, id);
                }
                "overlay" => st.overlay = value.as_bool().ok_or("overlay must be a boolean")?,
                "brush" => {
                    let f = |k: &str| value.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
                    if let Some(v) = f("size") {
                        st.brush.size = v.clamp(0.0025, 0.25);
                    }
                    if let Some(v) = f("feather") {
                        st.brush.feather = v.clamp(0.0, 100.0);
                    }
                    if let Some(v) = f("flow") {
                        st.brush.flow = v.clamp(1.0, 100.0);
                    }
                    if let Some(e) = value.get("erase").and_then(Value::as_bool) {
                        st.brush.erase = e;
                    }
                    if let Some(a) = value.get("autoMask") {
                        st.brush.auto_mask = a.as_bool().ok_or("brush autoMask must be a boolean")?;
                    }
                    // without points: only the brush's settings
                    let Some(pts) = value.get("points") else { continue };
                    let pts = pts.as_array().ok_or("brush points must be a list of [x, y] pairs")?;
                    let map = map.as_ref().ok_or("the preview has not rendered yet")?;
                    // an armed Brush's stroke makes a new mask (on the photo, as every new one is)
                    let on_photo = match local::selected_mask(&st, &mut settings) {
                        Some(m) => m.on_photo,
                        None if st.armed() => craft_develop_model::Mask::default().on_photo,
                        None => return Err("no mask is selected".into()),
                    };
                    let pts = pts
                        .iter()
                        .take(4096)
                        .map(|p| frame_point(p).map(|u| map.to_mask(on_photo, u)))
                        .collect::<Option<Vec<_>>>()
                        .ok_or("brush points must be [x, y] pairs")?;
                    local::paint_stroke(&mut st, &mut settings, pts)?;
                }
                "heal" => {
                    let map = map.as_ref().ok_or("the preview has not rendered yet")?;
                    let at = value.get("at").and_then(frame_point).ok_or("heal needs at: [x, y]")?;
                    if let Some(mode) = value.get("mode") {
                        st.heal.mode = serde_json::from_value(mode.clone()).map_err(|_| "heal mode must be heal, clone, remove or fill")?;
                    }
                    let n = map.to_norm(at);
                    let (sx, _) = map.long_scale();
                    let dx = 2.5 * st.heal.size / sx.max(1e-9);
                    local::add_spot(&mut st, &mut settings, n, Some(craft_geom::Point::new(if n.x + dx <= 1.0 { dx } else { -dx }, 0.0)));
                }
                "redEye" => {
                    let map = map.as_ref().ok_or("the preview has not rendered yet")?;
                    let at = value.get("at").and_then(frame_point).ok_or("redEye needs at: [x, y]")?;
                    st.pet = value.get("pet").and_then(Value::as_bool).unwrap_or(st.pet);
                    local::add_eye(&mut st, &mut settings, map.to_norm(at), 0.02, 0.015);
                }
                other => return Err(format!("unknown RAW Develop local property {other}")),
            }
        }
        Ok(())
    })();
    result?;
    let overlay_was = d.overlay_mask();
    d.local = st;
    if tool != d.tool {
        let was = d.tool;
        d.tool = tool;
        d.tool_changed(was);
    }
    if let Some(open) = sections {
        d.develop_panel.open = open;
    }
    if d.develop.as_ref().is_some_and(|e| e.settings != settings) {
        remember_edit(d);
        if let Some(e) = d.develop.as_mut() {
            e.settings = settings;
            d.params = e.camera_raw(&d.params);
        }
        d.dirty = true;
    }
    if d.overlay_mask() != overlay_was {
        d.dirty = true;
    }
    Ok(())
}
