//! Dialog previews under the automation policy.
//!
//! A control or MCP client confined to its automation roots may open a filter or adjustment
//! dialog and fill it. Confirming is judged by the automation policy, and so must be the live
//! preview, which runs the same command on a copy of the document every frame: otherwise a
//! Displace map or Color Lookup file the policy refuses would still be read, decoded and shown
//! on the canvas, where the client can screenshot it.
//!
//! The app remembers which dialogs automation opened or changed (a control request that added
//! or edited one, or a synthetic input pass while it was open). Their previews, and every
//! preview computed while automation input is being handled, pass the policy first. Dialogs
//! only the user touched preview as before.

use crate::PhotocraftApp;
use serde_json::{Map, Value};

/// The open dialogs' ids and fields before a control request, so [`note_changes`] can tell
/// which ones the request opened or edited.
pub(crate) type Snapshot = Vec<(u64, Map<String, Value>)>;

pub(crate) fn snapshot(app: &PhotocraftApp) -> Snapshot {
    app.ui.dialogs.iter().map(|d| (d.id, d.fields.clone())).collect()
}

/// After a control request: mark every dialog it opened or changed as automation-driven.
pub(crate) fn note_changes(app: &mut PhotocraftApp, before: &Snapshot) {
    let changed: Vec<u64> = app.ui.dialogs.iter().filter(|d| !before.iter().any(|(id, fields)| *id == d.id && *fields == d.fields)).map(|d| d.id).collect();
    mark(app, changed);
}

/// After a synthetic input pass: the input could have reached any open dialog.
pub(crate) fn note_open(app: &mut PhotocraftApp) {
    let open: Vec<u64> = app.ui.dialogs.iter().map(|d| d.id).collect();
    mark(app, open);
}

fn mark(app: &mut PhotocraftApp, ids: Vec<u64>) {
    for id in ids {
        if !app.automation_dialogs.contains(&id) {
            app.automation_dialogs.push(id);
        }
    }
    // Forget closed dialogs so the list stays as small as the open ones.
    let open = &app.ui.dialogs;
    app.automation_dialogs.retain(|id| open.iter().any(|d| d.id == *id));
}

/// Why the automation policy refuses to preview dialog `dialog` (its `fields`) as `command` with
/// `params`, or None when the preview may run. Only automation-driven dialogs, or previews
/// computed while automation input is handled, are judged; the user's own dialogs never are.
pub(crate) fn refusal(app: &PhotocraftApp, dialog: u64, fields: &Map<String, Value>, command: &str, params: &Value) -> Option<String> {
    if !(app.automation_input || app.session.authorize.is_some() || app.automation_dialogs.contains(&dialog)) {
        return None;
    }
    // What confirming would be judged as (the dialog's command and fields), then what the
    // preview actually runs, which can differ (a smart filter's edit, a layer mask target).
    let dialog_command = fields.get("__command").and_then(Value::as_str).map(|c| (c, Value::Object(fields.clone())));
    let mut checks = dialog_command.into_iter().chain(std::iter::once((command, params.clone())));
    if let Some(authorize) = app.services.automation_command.as_ref() {
        return checks.find_map(|(id, p)| authorize(id, &p).err());
    }
    let gate = app.session.authorize.or(app.services.automation_authorize)?;
    checks.find_map(|(id, p)| gate(id, &p).err().map(|e| e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::PhotocraftApp;
    use crate::control::{ControlRequest, Outcome, handle};
    use serde_json::{Value, json};

    /// The part of the desktop policy (`photocraft_automation::workspace`) these previews meet:
    /// a Displace `mapPath` or a Color Lookup `file` names a file outside the session.
    fn policy(id: &str, params: &Value) -> photocraft_engine::Result<()> {
        let key = match id {
            "filter.distort.displace" => "mapPath",
            "image.adjustments.colorLookup" => "file",
            _ => return Ok(()),
        };
        match params.get(key).and_then(Value::as_str) {
            Some(path) if !path.is_empty() => Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled"))),
            _ => Ok(()),
        }
    }

    fn confined_app() -> (PhotocraftApp, egui::Context) {
        let services = crate::Services {
            automation_authorize: Some(policy),
            automation_command: Some(Box::new(|id, params| policy(id, params).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.background_jobs = false;
        (app, egui::Context::default())
    }

    fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, ctx, &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pc-automation-preview-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A map whose left half is yellow (shift right and down) and right half black.
    fn displace_map() -> Vec<u8> {
        let rgba: Vec<u8> = (0..64u32).flat_map(|_| (0..64u32).flat_map(|x| if x < 32 { [255, 255, 0, 255] } else { [0, 0, 0, 255] })).collect();
        let image = photocraft_codecs::Image::from_u8(64, 64, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
        photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    }

    /// A size-2 .cube that swaps red and blue.
    fn swap_cube() -> String {
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    cube.push_str(&format!("{b} {g} {r}\n"));
                }
            }
        }
        cube
    }

    fn preview_pixels(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Vec<[f32; 4]>> {
        let idx = app.session.active_index().unwrap();
        crate::canvas::ensure_filter_preview(app, idx, ctx)?;
        app.filter_preview.as_ref().and_then(|p| p.result.as_ref()).map(|d| photocraft_compose::flatten(d).px)
    }

    #[test]
    fn displace_preview_of_an_automation_dialog_does_not_read_a_refused_map() {
        let dir = temp_dir("displace");
        let map = dir.join("map.png");
        std::fs::write(&map, displace_map()).unwrap();
        let (mut app, ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("filter.render.clouds", json!({"seed": 4})).unwrap();
        let original = photocraft_compose::flatten(&app.session.active().unwrap().doc).px;
        let values = [("mapPath", json!(map.to_string_lossy())), ("horizontal", json!(50)), ("vertical", json!(50))];

        // A confined client opens and fills the dialog: its preview is refused like OK is.
        let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "filter.distort.displace"}));
        let dialog = r["result"]["dialog"].as_u64().unwrap();
        for (field, value) in &values {
            let r = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": field, "value": value}));
            assert!(r.get("error").is_none(), "{r}");
        }
        assert_eq!(preview_pixels(&mut app, &ctx), None, "no preview is computed from the refused map");
        assert!(app.ui.status_error && app.ui.status.contains("filter.distort.displace"), "{}", app.ui.status);
        let confirm = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": dialog}));
        assert_eq!(confirm["ok"], false, "{confirm}");
        crate::dialogs::cancel(&mut app, dialog).unwrap();

        // The user's own dialog previews the same map; a client only inspecting it changes nothing.
        let mine = crate::dialogs::open_command_dialog(&mut app, "filter.distort.displace", "Displace");
        for (field, value) in values {
            app.ui.dialog_mut(mine).unwrap().fields.insert(field.into(), value);
        }
        assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["ok"], true);
        let px = preview_pixels(&mut app, &ctx).expect("the user's preview runs");
        assert_ne!(px, original, "the map displaced the user's preview");
        // Synthetic input reaches whatever dialog is open: the policy judges its preview too.
        app.automation_input = true;
        app.session.execute("filter.render.clouds", json!({"seed": 5})).unwrap();
        assert_eq!(preview_pixels(&mut app, &ctx), None);
        app.automation_input = false;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn color_lookup_preview_of_an_automation_dialog_does_not_read_a_refused_lut() {
        let dir = temp_dir("lut");
        let lut = dir.join("swap.cube");
        std::fs::write(&lut, swap_cube()).unwrap();
        let (mut app, ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();

        let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "image.adjustments.colorLookup"}));
        let dialog = r["result"]["dialog"].as_u64().unwrap();
        let r = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": "file", "value": lut.to_string_lossy()}));
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(preview_pixels(&mut app, &ctx), None, "no preview is computed from the refused LUT");
        assert!(app.ui.status_error && app.ui.status.contains("image.adjustments.colorLookup"), "{}", app.ui.status);
        crate::dialogs::cancel(&mut app, dialog).unwrap();

        let mine = crate::dialogs::open_command_dialog(&mut app, "image.adjustments.colorLookup", "Color Lookup");
        app.ui.dialog_mut(mine).unwrap().fields.insert("file".into(), json!(lut.to_string_lossy()));
        let px = preview_pixels(&mut app, &ctx).expect("the user's preview runs");
        let first = px.first().copied().unwrap();
        assert!(first[2] > 0.9 && first[0] < 0.1, "the LUT turned the user's red preview blue: {first:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
