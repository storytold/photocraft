//! Edit › Preferences support for Generative Fill: the dialog that asks the server for its models,
//! the Integrations model list, and the worker-thread lookups behind both.

use super::{Map, PhotocraftApp, Value, json};

/// Where the Preferences dialog publishes the Generative Fill model list for [`field_rows`]
/// (`(models, note)`), and the flag its Refresh button raises.
pub(super) const GF_MODELS_ID: &str = "prefs-gf-models";
pub(super) const GF_REFRESH_ID: &str = "prefs-gf-refresh";

/// A Generative Fill model lookup running on a worker thread; its reply lands here when it ends.
pub(super) type LookupSlot = std::sync::Arc<std::sync::Mutex<Option<Result<Value, String>>>>;

/// The Preferences section's one lookup in flight, as `(backend|host it asked about, slot)`.
const GF_PREF_LOOKUP_ID: &str = "prefs-gf-pref-lookup";
/// The fill dialogs' lookups in flight, as `(dialog id, backend|host asked about, slot)`.
pub(super) const GF_DIALOG_LOOKUPS_ID: &str = "prefs-gf-dialog-lookups";
/// How long a typed backend or host must stay unchanged before the Preferences list asks again.
const GF_TYPING_PAUSE: f64 = 0.6;

/// Ask a server for its models on a worker thread. The lookup waits on the network and resolves
/// the host name, so it must not run on the interface's thread: the window stays responsive,
/// Cancel included, while a slow or silent server is asked.
pub(super) fn start_lookup(backend: String, host: String) -> LookupSlot {
    let slot = LookupSlot::default();
    let out = slot.clone();
    let work = move || {
        let reply = photocraft_engine::generative_fill_cmds::lookup_models(&backend, &host).map_err(|e| e.to_string());
        *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reply);
    };
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("model-lookup".into()).spawn(work).is_err() {
        *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Err("couldn't start a worker thread".into()));
    }
    // The browser has no threads; the lookup refuses at once there.
    #[cfg(target_arch = "wasm32")]
    work();
    slot
}

pub(super) fn take_reply(slot: &LookupSlot) -> Option<Result<Value, String>> {
    slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

/// The model names in a lookup reply and the one-line note about it.
pub(super) fn lookup_summary(reply: &Result<Value, String>) -> (Vec<String>, String) {
    match reply {
        Ok(v) => {
            let names: Vec<String> = v["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            let server = v["server"].as_str().unwrap_or_default();
            let note = if names.is_empty() { format!("{server} has no models installed") } else { format!("{} models found on {server}", names.len()) };
            (names, note)
        }
        Err(e) => (Vec::new(), format!("Couldn't list models: {e}")),
    }
}

/// Keep the Generative Fill model list of the Integrations section current. The server is the one
/// named by the (unsaved) backend and host in the section: it is asked once when the section is
/// shown, again on Refresh, and again when either changes (after a short pause for typing), so
/// the list never belongs to a server other than the one shown. The asking happens on a worker
/// thread; until it answers the note says so and the model row is a text box. A server that
/// doesn't answer leaves an empty list and the reason.
pub(super) fn refresh_generative_models(ctx: &egui::Context, f: &mut Map<String, Value>) {
    if f.get("section").and_then(Value::as_str) != Some("integrations") {
        return;
    }
    let draft = f.get("values").and_then(|v| v.get("integrations")).cloned().unwrap_or(Value::Null);
    let backend = draft["generativeFillBackend"].as_str().unwrap_or("comfyui").to_string();
    let typed = draft["generativeFillHost"].as_str().unwrap_or_default().trim().to_string();
    // A blank host stands for the usual one on this machine.
    let host = if typed.is_empty() { photocraft_engine::generative_fill_cmds::default_host_for(&backend).unwrap_or_default() } else { typed };
    let want = format!("{backend}|{host}");
    let id = egui::Id::unique(GF_PREF_LOOKUP_ID);
    let asked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(GF_REFRESH_ID))).unwrap_or(false);
    let now = ctx.input(|i| i.time);

    // 1. A lookup that has finished: keep its answer only if it was for the server shown now.
    let mut running: Option<(String, LookupSlot)> = ctx.data(|d| d.get_temp::<Vec<(String, LookupSlot)>>(id)).and_then(|v| v.into_iter().next());
    if let Some((asked_of, slot)) = running.clone() {
        if let Some(reply) = take_reply(&slot) {
            running = None;
            if asked_of == want {
                let (models, note) = lookup_summary(&reply);
                f.insert("__gfModels".into(), json!(models));
                f.insert("__gfNote".into(), json!(note));
                f.insert("__gfKey".into(), json!(asked_of));
            }
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // 2. Ask again when the list isn't for the server shown, or on Refresh.
    if asked {
        f.remove("__gfKey");
    }
    let first = !f.contains_key("__gfWant");
    if f.get("__gfWant").and_then(Value::as_str) != Some(want.as_str()) {
        f.insert("__gfWant".into(), json!(want));
        f.insert("__gfWantAt".into(), json!(now));
    }
    let stale = f.get("__gfKey").and_then(Value::as_str) != Some(want.as_str());
    if stale && running.as_ref().is_none_or(|(of, _)| *of != want) {
        let since = now - f.get("__gfWantAt").and_then(Value::as_f64).unwrap_or(now);
        if asked || first || since >= GF_TYPING_PAUSE {
            running = Some((want.clone(), start_lookup(backend, host.clone())));
            f.insert("__gfModels".into(), json!(Vec::<String>::new()));
            f.insert("__gfNote".into(), json!(format!("Looking for models on {host}…")));
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, running.into_iter().collect::<Vec<_>>()));

    let models: Vec<String> = f.get("__gfModels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let note = f.get("__gfNote").and_then(Value::as_str).unwrap_or_default().to_string();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(GF_MODELS_ID), (models, note)));
}

/// The server a Generative Fill dialog's `backend` and `host` fields name now, as the same
/// `backend|host` key the Integrations lookup uses (a blank host means the usual one).
pub(super) fn dialog_server(fields: &Map<String, Value>) -> (String, String) {
    let backend = fields.get("backend").and_then(Value::as_str).unwrap_or("comfyui").to_string();
    let typed = fields.get("host").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let host = if typed.is_empty() { photocraft_engine::generative_fill_cmds::default_host_for(&backend).unwrap_or_default() } else { typed };
    (backend, host)
}

/// Keep the model list of every open Generative Fill dialog true to the server its fields name
/// (called every frame). A lookup that finishes fills the model in (a menu of what the server has,
/// or the reason there is none); one whose server is no longer the one named is dropped, so a
/// list from another server never shows. When the backend or host is changed, the new server is
/// asked after a short pause for typing and the model goes back to a text box meanwhile. A dialog
/// closed in the meantime is left alone.
pub(super) fn poll_model_lookups(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let id = egui::Id::unique(GF_DIALOG_LOOKUPS_ID);
    let pending: Vec<(u64, String, LookupSlot)> = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let open: Vec<u64> =
        app.ui.dialogs.iter().filter(|d| d.fields.get("__command").and_then(Value::as_str) == Some("edit.generativeFill")).map(|d| d.id).collect();
    if pending.is_empty() && open.is_empty() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let key_of = |app: &PhotocraftApp, dialog: u64| -> Option<String> {
        let dm = app.ui.dialogs.iter().find(|d| d.id == dialog)?;
        let (backend, host) = dialog_server(&dm.fields);
        Some(format!("{backend}|{host}"))
    };
    let mut waiting: Vec<(u64, String, LookupSlot)> = Vec::new();
    for (dialog, key, slot) in pending {
        if !open.contains(&dialog) {
            continue;
        }
        match take_reply(&slot) {
            None => waiting.push((dialog, key, slot)),
            Some(reply) if key_of(app, dialog).as_deref() == Some(key.as_str()) => apply_dialog_lookup(app, dialog, &key, &reply),
            // The fields name another server by now: this answer is for the wrong one.
            Some(_) => {}
        }
    }
    let mut changing = false;
    for dialog in open {
        let Some(want) = key_of(app, dialog) else { continue };
        let Some(dm) = app.ui.dialog_mut(dialog) else { continue };
        if dm.fields.get("__gfKey").and_then(Value::as_str) == Some(want.as_str()) || waiting.iter().any(|(d, k, _)| *d == dialog && *k == want) {
            continue;
        }
        changing = true;
        if dm.fields.get("__gfWant").and_then(Value::as_str) != Some(want.as_str()) {
            dm.fields.insert("__gfWant".into(), json!(want));
            dm.fields.insert("__gfWantAt".into(), json!(now));
        }
        let since = now - dm.fields.get("__gfWantAt").and_then(Value::as_f64).unwrap_or(now);
        if since < GF_TYPING_PAUSE {
            continue;
        }
        let (backend, host) = dialog_server(&dm.fields);
        // The list shown belongs to the old server: back to a text box while the new one is asked.
        if let Some(spec) = photocraft_engine::commands::find("edit.generativeFill") {
            dm.fields.insert("__spec".into(), json!(spec.params));
        }
        dm.fields.insert("__note".into(), json!(format!("Looking for models on {host}…")));
        dm.fields.remove("__gfKey");
        waiting.retain(|(d, _, _)| *d != dialog);
        waiting.push((dialog, want, start_lookup(backend, host)));
    }
    let more = !waiting.is_empty() || changing;
    ctx.data_mut(|d| d.insert_temp(id, waiting));
    if more {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Apply a finished lookup (for `key`, the server the dialog names) to its dialog: the model
/// becomes a menu of what the server has. A model the user typed that the server doesn't list is
/// kept as typed, in the text box, rather than swapped for another one; the model prefilled from
/// Preferences is replaced by the server's default when the server lacks it.
pub(super) fn apply_dialog_lookup(app: &mut PhotocraftApp, dialog: u64, key: &str, reply: &Result<Value, String>) {
    let saved_model = app.session.prefs().integrations.generative_fill_model.clone();
    let Some(dm) = app.ui.dialog_mut(dialog) else { return };
    let (all, mut note) = lookup_summary(reply);
    dm.fields.insert("__gfKey".into(), json!(key));
    // Names the dialog's notation can't hold (`|` separates choices) are left out of the menu.
    let names: Vec<String> = all.into_iter().filter(|n| !n.contains(['|', '"'])).collect();
    if let Ok(v) = reply
        && !names.is_empty()
    {
        let typed = dm.fields.get("model").and_then(Value::as_str).unwrap_or_default().to_string();
        let prefilled = dm.fields.get("__gfSeed").and_then(Value::as_str).unwrap_or_default() == typed;
        let listed = names.contains(&typed);
        if !typed.is_empty() && !listed && !prefilled {
            note = format!("{note} ({typed} isn't one of them)");
        } else {
            let default = v["default"].as_str().unwrap_or_default();
            let chosen =
                [typed.as_str(), saved_model.as_str(), default].into_iter().find(|m| names.iter().any(|n| n == m)).unwrap_or(names[0].as_str()).to_string();
            if let Some(spec) = dm.fields.get("__spec").and_then(Value::as_str) {
                let spec = spec.replace("\"model\":text", &format!("\"model\":\"{}\"=\"{chosen}\"", names.join("|")));
                dm.fields.insert("__spec".into(), json!(spec));
            }
            // A model picked here, not by the user, stands in for "no choice yet": when the server
            // changes it is replaced by the new server's default instead of being kept as typed.
            // A listed model the user typed or picked (`typed` differs from the seed) stays theirs.
            if prefilled {
                dm.fields.insert("__gfSeed".into(), json!(chosen));
            }
            dm.fields.insert("model".into(), json!(chosen));
        }
    }
    dm.fields.insert("__note".into(), json!(note));
}

/// The Generative Fill dialog: opens at once, starting from the server a fill would use (the
/// parameters, then the environment, then Preferences › Integrations), and asks that server for its
/// models on a worker thread. When it answers (see [`poll_model_lookups`]) the model becomes a menu
/// of what the server has (the one chosen if the server has it, else an inpainting checkpoint, else
/// the first); when it doesn't, the model stays a text box and the note says why. The dialog's
/// backend and host fields are the server that was asked, so a fill goes to the server whose
/// models are shown.
pub(super) fn generative_fill_dialog(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) -> Option<u64> {
    let saved = app.session.prefs().integrations.clone();
    let spec = photocraft_engine::commands::find(id)?.params.to_string();
    let (backend, mut host) = photocraft_engine::generative_fill_cmds::effective_server(&app.session, &json!({}))
        .unwrap_or_else(|_| (saved.generative_fill_backend.name().to_string(), saved.generative_fill_host.clone()));
    if host.trim().is_empty() {
        host = photocraft_engine::generative_fill_cmds::default_host_for(&backend).unwrap_or_default();
    }
    let d = crate::dialogs::filter_dialog::open_with_spec(app, id, "Generative Fill…", &spec, serde_json::Map::new());
    if let Some(dm) = app.ui.dialog_mut(d) {
        // No live preview: a fill is a long request to a server, not a filter.
        dm.fields.remove("__preview");
        dm.fields.insert("backend".into(), json!(backend));
        dm.fields.insert("host".into(), json!(host));
        if !saved.generative_fill_model.is_empty() {
            dm.fields.insert("model".into(), json!(saved.generative_fill_model));
            // Told apart later from a model the user types: this one yields to the server's default.
            dm.fields.insert("__gfSeed".into(), json!(saved.generative_fill_model));
        }
        dm.fields.insert("__note".into(), json!(format!("Looking for models on {host}…")));
    }
    let key = format!("{backend}|{host}");
    let slot = start_lookup(backend, host);
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Vec<(u64, String, LookupSlot)>>(egui::Id::unique(GF_DIALOG_LOOKUPS_ID)).push((d, key, slot)));
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    Some(d)
}
