//! Dialog previews under the automation policy.
//!
//! A control or MCP client confined to its automation roots may open a filter or adjustment
//! dialog and fill it. Confirming is judged by the automation policy, and so must be the live
//! preview, which runs the same command on a copy of the document every frame: otherwise a
//! Displace map or Color Lookup file the policy refuses would still be read, decoded and shown
//! on the canvas, where the client can screenshot it.
//!
//! The app remembers which dialogs automation opened or changed (a control request or a
//! synthetic input pass that added or edited one). Their previews, and the previews of dialogs
//! the synthetic input being handled opened or edited, pass the policy first. Dialogs only the
//! user touched preview as before.
//!
//! A pass is compared as a whole, so it can't tell who made a change within it: a dialog the user
//! edits in the same pass as injected input is marked too, and its preview stays judged (and
//! refused, if the policy refuses it) for as long as that dialog stays open.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::Value;

use crate::PhotocraftApp;

/// One open dialog as automation could have changed it: its id, the host's serial for this
/// opening (an id is its key's, so a dialog reopened under the same id is a new one) and its
/// state (the host's fields, plus an adjustment dialog's parameters, which it keeps apart).
type Entry = (DialogId, u64, Value);

/// The open dialogs before a control request or synthetic input pass, so [`note_changes`] can
/// tell which ones it opened or edited.
pub(crate) type Snapshot = Vec<Entry>;

/// The open dialogs now, taken before a control request or synthetic input pass.
pub(crate) fn snapshot(app: &PhotocraftApp) -> Snapshot {
    app.framework.dialogs.open_dialogs().into_iter().map(|d| entry(app, d.id, d.fields)).collect()
}

/// Dialog `id` (with the host's `fields` for it) as a snapshot entry.
fn entry(app: &PhotocraftApp, id: DialogId, fields: Option<Value>) -> Entry {
    let serial = app.framework.dialogs.open_serial(id).unwrap_or(0);
    let params = app.adjust_open.as_ref().filter(|open| open.id == id).and_then(|_| crate::adjust_dialog::open_params(app));
    (id, serial, serde_json::json!([fields, params]))
}

/// The open dialog `id` as a snapshot entry, or None when it is not open.
fn current(app: &PhotocraftApp, id: DialogId) -> Option<Entry> {
    let fields = app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id)?.fields;
    Some(entry(app, id, fields))
}

/// After a control request or synthetic input pass: mark every dialog it opened or changed as
/// automation-driven. A dialog it left as it was (a user's own) keeps its unjudged preview.
pub(crate) fn note_changes(app: &mut PhotocraftApp, before: &Snapshot) {
    let changed: Vec<(DialogId, u64)> = snapshot(app).into_iter().filter(|now| !before.contains(now)).map(|(id, serial, _)| (id, serial)).collect();
    mark(app, changed);
}

/// After synthetic input that began mid-pass, with no snapshot to compare: it could have reached
/// any open dialog, so all of them are marked.
pub(crate) fn note_open(app: &mut PhotocraftApp) {
    let open: Vec<(DialogId, u64)> = snapshot(app).into_iter().map(|(id, serial, _)| (id, serial)).collect();
    mark(app, open);
}

/// Add `dialogs` to the automation-driven ones and forget the ones that have closed.
fn mark(app: &mut PhotocraftApp, dialogs: Vec<(DialogId, u64)>) {
    for dialog in dialogs {
        if !app.automation_dialogs.contains(&dialog) {
            app.automation_dialogs.push(dialog);
        }
    }
    // Forget closed dialogs so the list stays as small as the open ones.
    let host = &app.framework.dialogs;
    app.automation_dialogs.retain(|(id, serial)| host.open_serial(*id) == Some(*serial));
}

/// Whether dialog `dialog`, in this opening, is one a control request or synthetic input marked.
fn marked(app: &PhotocraftApp, dialog: DialogId) -> bool {
    app.framework.dialogs.open_serial(dialog).is_some_and(|serial| app.automation_dialogs.contains(&(dialog, serial)))
}

/// Whether synthetic input being handled now could have opened or edited dialog `dialog`: it is
/// not in the pass's starting snapshot as it is now, or the input began mid-pass with no
/// snapshot. A user's dialog the pass left alone is not, so its preview is neither blanked nor
/// cached blank for the frames after the pass.
fn synthetic_reached(app: &PhotocraftApp, dialog: DialogId) -> bool {
    if !app.automation_input {
        return false;
    }
    let now = current(app, dialog);
    !app.automation_pass_dialogs.as_ref().is_some_and(|before| now.is_some_and(|now| before.contains(&now)))
}

/// What confirming dialog `dialog` is judged as: a parameter dialog whose key is a live command,
/// with its fields (as `ui.dialog.confirm` judges it). Keep in step with `hosted_command` in
/// `control/dialogs.rs`, which `ui.dialog.confirm` uses; it can't be called from here because
/// `control` is not built for wasm.
fn confirmed_as(app: &PhotocraftApp, dialog: DialogId) -> Option<(String, Value)> {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    let open = app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == dialog)?;
    let key = open.key.filter(|key| crate::menus::is_live(key))?;
    Some((key, open.fields.unwrap_or(Value::Null)))
}

/// Why the automation policy refuses to preview dialog `dialog` as `command` with `params`, or
/// None when the preview may run. Only automation-driven dialogs, including those the synthetic
/// input being handled opened or edited, are judged; the user's own dialogs never are.
pub(crate) fn refusal(app: &PhotocraftApp, dialog: DialogId, command: &str, params: &Value) -> Option<String> {
    if !(app.session.authorize.is_some() || marked(app, dialog) || synthetic_reached(app, dialog)) {
        return None;
    }
    // What confirming would be judged as (the dialog's command and fields), then what the
    // preview actually runs, which can differ (a smart filter's edit, a layer mask target).
    let confirmed = confirmed_as(app, dialog);
    let mut checks = confirmed.iter().map(|(id, p)| (id.as_str(), p.clone())).chain(std::iter::once((command, params.clone())));
    if let Some(authorize) = app.services.automation_command.as_ref() {
        return checks.find_map(|(id, p)| authorize(id, &p).err());
    }
    let gate = app.session.authorize.or(app.services.automation_authorize)?;
    checks.find_map(|(id, p)| gate(id, &p).err().map(|e| e.to_string()))
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::DialogId;
    use serde_json::{Value, json};

    use crate::PhotocraftApp;
    use crate::control::{ControlRequest, Reply, handle};

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

    /// An app whose automation gate and command callback both apply [`policy`].
    fn confined_app() -> (Box<PhotocraftApp>, egui::Context) {
        let services = crate::Services {
            automation_authorize: Some(policy),
            automation_command: Some(Box::new(|id, params| policy(id, params).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.background_jobs = false;
        let ctx = app.ctx().clone();
        (app, ctx)
    }

    /// Send one control request and return its immediate reply.
    fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, ctx, &req) {
            Reply::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    /// The open dialog a control reply named (`ui.dialog.open`'s `dialog`, a decimal string).
    fn dialog_of(app: &PhotocraftApp, reply: &Value) -> DialogId {
        let named = reply["result"]["dialog"].as_str().expect("the dialog's id is returned");
        app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id.value().to_string() == named).expect("the dialog is open").id
    }

    /// Set `field` of dialog `id` as the user's own edit (not through the control channel).
    fn set_field(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
        let ctx = app.ctx().clone();
        app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
    }

    /// Run the dialog host for a couple of passes, so a parameter dialog shows with Preview on.
    fn passes(app: &mut PhotocraftApp) {
        for _ in 0..2 {
            crate::dialogs::testing::show(app);
        }
    }

    /// Whether dialog `id` (in any opening) is marked as automation-driven.
    fn is_marked(app: &PhotocraftApp, id: DialogId) -> bool {
        app.automation_dialogs.iter().any(|(marked, _)| *marked == id)
    }

    /// A fresh scratch directory for this test process, tagged by `tag`.
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

    /// The active document's filter preview flattened to pixels, or None when none was computed.
    fn preview_pixels(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Vec<[f32; 4]>> {
        let idx = app.session.active_index().unwrap();
        crate::canvas::ensure_filter_preview(app, idx, ctx)?;
        app.filter_preview.as_ref().and_then(|p| p.result.as_ref()).map(|d| photocraft_compose::flatten(d).into_px())
    }

    /// A Displace dialog a client filled with a refused map previews nothing and reports it,
    /// while the user's own dialog with the same map still previews.
    #[test]
    fn displace_preview_of_an_automation_dialog_does_not_read_a_refused_map() {
        let dir = temp_dir("displace");
        let map = dir.join("map.png");
        std::fs::write(&map, displace_map()).unwrap();
        let (mut app, ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("filter.render.clouds", json!({"seed": 4})).unwrap();
        let original = photocraft_compose::flatten(&app.session.active().unwrap().doc).into_px();
        let values = [("mapPath", json!(map.to_string_lossy())), ("horizontal", json!(50)), ("vertical", json!(50))];

        // A confined client opens and fills the dialog: its preview is refused like OK is.
        let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "filter.distort.displace"}));
        let dialog = dialog_of(&app, &r);
        for (field, value) in &values {
            let reply = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": r["result"]["dialog"], "field": field, "value": value}));
            assert!(reply.get("error").is_none(), "{reply}");
        }
        passes(&mut app);
        assert_eq!(preview_pixels(&mut app, &ctx), None, "no preview is computed from the refused map");
        assert!(app.ui.status_error && app.ui.status.contains("filter.distort.displace"), "{}", app.ui.status);
        let confirm = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": r["result"]["dialog"]}));
        assert_eq!(confirm["ok"], false, "{confirm}");
        crate::dialogs::testing::cancel(&mut app, dialog).unwrap();

        // The user's own dialog previews the same map; a client only inspecting it changes nothing.
        let mine = crate::dialogs::open_command_dialog(&mut app, "filter.distort.displace", "Displace").unwrap();
        for (field, value) in values {
            set_field(&mut app, mine, field, value);
        }
        passes(&mut app);
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

    /// A synthetic input pass that leaves the user's dialog as it was doesn't mark it, so its
    /// preview stays unjudged; one that edits a field does.
    #[test]
    fn synthetic_pass_marks_only_the_dialogs_it_changed() {
        let (mut app, _ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mine = crate::dialogs::open_command_dialog(&mut app, "filter.distort.displace", "Displace").unwrap();
        passes(&mut app);
        let before = super::snapshot(&app);
        super::note_changes(&mut app, &before);
        assert!(!is_marked(&app, mine), "an untouched dialog stays the user's");
        let before = super::snapshot(&app);
        set_field(&mut app, mine, "mapPath", json!("elsewhere.png"));
        super::note_changes(&mut app, &before);
        assert!(is_marked(&app, mine), "a dialog the pass edited is automation-driven");
    }

    /// A preview drawn during a synthetic input pass is judged only for the dialogs that input
    /// opened or edited: the user's dialog it left alone keeps its preview, during the pass and
    /// after it, instead of a blank one cached under the same key.
    #[test]
    fn synthetic_pass_does_not_blank_the_users_untouched_preview() {
        let dir = temp_dir("pass");
        let map = dir.join("map.png");
        std::fs::write(&map, displace_map()).unwrap();
        let (mut app, ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("filter.render.clouds", json!({"seed": 4})).unwrap();
        let mine = crate::dialogs::open_command_dialog(&mut app, "filter.distort.displace", "Displace").unwrap();
        for (field, value) in [("mapPath", json!(map.to_string_lossy())), ("horizontal", json!(50)), ("vertical", json!(50))] {
            set_field(&mut app, mine, field, value);
        }
        passes(&mut app);

        // A synthetic pass that leaves the dialog as it was: the preview runs, and still runs after.
        app.automation_input = true;
        app.automation_pass_dialogs = Some(super::snapshot(&app));
        assert!(preview_pixels(&mut app, &ctx).is_some(), "the untouched user dialog still previews during the pass");
        app.automation_input = false;
        app.automation_pass_dialogs = None;
        assert!(preview_pixels(&mut app, &ctx).is_some(), "and after it");

        // The same pass editing the dialog: its preview is judged and refused.
        app.automation_input = true;
        app.automation_pass_dialogs = Some(super::snapshot(&app));
        set_field(&mut app, mine, "horizontal", json!(40));
        assert_eq!(preview_pixels(&mut app, &ctx), None, "a dialog the pass edited is judged");
        app.automation_input = false;
        app.automation_pass_dialogs = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Color Lookup dialog a client pointed at a refused LUT previews nothing, while the
    /// user's own dialog with the same LUT still previews.
    #[test]
    fn color_lookup_preview_of_an_automation_dialog_does_not_read_a_refused_lut() {
        let dir = temp_dir("lut");
        let lut = dir.join("swap.cube");
        std::fs::write(&lut, swap_cube()).unwrap();
        let (mut app, ctx) = confined_app();
        app.session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();

        let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "image.adjustments.colorLookup"}));
        let dialog = dialog_of(&app, &r);
        let set = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": r["result"]["dialog"], "field": "file", "value": lut.to_string_lossy()}));
        assert!(set.get("error").is_none(), "{set}");
        passes(&mut app);
        assert_eq!(preview_pixels(&mut app, &ctx), None, "no preview is computed from the refused LUT");
        assert!(app.ui.status_error && app.ui.status.contains("image.adjustments.colorLookup"), "{}", app.ui.status);
        crate::dialogs::testing::cancel(&mut app, dialog).unwrap();

        let mine = crate::dialogs::open_command_dialog(&mut app, "image.adjustments.colorLookup", "Color Lookup").unwrap();
        set_field(&mut app, mine, "file", json!(lut.to_string_lossy()));
        passes(&mut app);
        let px = preview_pixels(&mut app, &ctx).expect("the user's preview runs");
        let first = px.first().copied().unwrap();
        assert!(first[2] > 0.9 && first[0] < 0.1, "the LUT turned the user's red preview blue: {first:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
