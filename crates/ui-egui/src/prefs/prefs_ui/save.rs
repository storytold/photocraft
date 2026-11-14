use super::appearance::{canvas_style, sync_appearance, sync_display_scale, sync_tooltips};
use super::startup::{load, presets_store, recovery, sync_rulers};
use super::{DocId, HashMap, PhotocraftApp, Value, prefs};

/// Per-frame upkeep: theme sync, persistence, autosave and the history log.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.prefs_rt.loaded {
        load(app);
    }
    recovery(app);
    sync_rulers(app);
    sync_display_scale(app, ctx);
    sync_appearance(app, ctx);
    crate::theme::set_ui_font_size(ctx, app.session.prefs().interface.ui_font_size);
    presets_store(app);
    sync_tooltips(app, ctx);
    app.sync_recent();
    super::agents::sync_agent_link(app, ctx);
    // Monotonic, not `ctx.input(|i| i.time)`: logic also runs on hidden ticks, where egui's input
    // time is that of the last UI pass.
    if let Some(wait) = persist(app, crate::gpu_canvas::now_ms() / 1000.0) {
        ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wait).unwrap_or_default());
    }
    let style = canvas_style(app);
    if let Some(gpu) = app.gpu.as_ref()
        && app.prefs_rt.gpu_style != Some(style)
    {
        gpu.set_style(style);
        app.prefs_rt.gpu_style = Some(style);
    }
    autosave(app);
    #[cfg(not(target_arch = "wasm32"))]
    link_updates(app, ctx);
    history_log(app);
}

/// First retry delay after a failed preferences write, in seconds; it doubles with every further
/// failure up to [`SAVE_RETRY_MAX_S`]. A new change retries sooner, but no sooner than this.
pub(super) const SAVE_RETRY_S: f64 = 2.0;

pub(super) const SAVE_RETRY_MAX_S: f64 = 30.0;

/// Failed writes in a row before a notice says preferences aren't being saved.
const SAVE_NOTICE_AFTER: u32 = 3;

/// Retry state of a failed preferences write (times are egui input time, in seconds).
#[derive(Default)]
pub(super) struct SaveRetry {
    failures: u32,
    /// When the next retry is due.
    at: f64,
    /// When the last write was tried, and the revision it carried.
    tried_at: f64,
    tried_rev: u64,
    /// The "not saved" notice, while it's shown.
    notice: Option<u64>,
}

/// Save changed preferences. A failed write stays pending and is retried with a backoff (a newer
/// change retries sooner); the status bar reports the first failure and a notice a persistent
/// one. Returns the seconds until the next retry while one is pending.
pub(super) fn persist(app: &mut PhotocraftApp, now: f64) -> Option<f64> {
    let rev = app.session.prefs.rev();
    if rev == app.prefs_rt.saved_rev {
        return None;
    }
    let r = &app.prefs_rt.save_retry;
    if r.failures > 0 {
        let due = if rev == r.tried_rev { r.at } else { r.at.min(r.tried_at + SAVE_RETRY_S) };
        if now < due {
            return Some(due - now);
        }
    }
    let ours = app.session.prefs_value();
    // A change that was undone (or set a value to what it already was) leaves nothing to write.
    let written = if app.prefs_rt.saved_value.as_ref() == Some(&ours) { Ok(()) } else { write_prefs(app, &ours) };
    match written {
        Ok(()) => {
            app.prefs_rt.saved_rev = rev;
            app.prefs_rt.saved_value = Some(ours);
            let r = std::mem::take(&mut app.prefs_rt.save_retry);
            if r.failures > 0 {
                let message = "Preferences saved";
                app.ui.status = tl!(message).into();
                app.ui.status_error = false;
                if let Some(id) = r.notice {
                    app.ui.notices.retain(|n| n.id != id);
                }
            }
            None
        }
        Err(e) => {
            let r = &mut app.prefs_rt.save_retry;
            r.failures = r.failures.saturating_add(1);
            (r.tried_at, r.tried_rev) = (now, rev);
            let delay = (SAVE_RETRY_S * 2f64.powi(r.failures.min(16) as i32 - 1)).min(SAVE_RETRY_MAX_S);
            r.at = now + delay;
            let failures = r.failures;
            if failures == 1 {
                let template = "Couldn't save preferences: {error}. Retrying…";
                app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
                app.ui.status_error = true;
            } else if failures == SAVE_NOTICE_AFTER {
                let lines = vec![e, "PhotoCraft keeps retrying; until a save succeeds, preference changes are lost when it closes.".into()];
                let id = crate::notices::post(app, "Preferences can't be saved", lines, true, None);
                app.prefs_rt.save_retry.notice = Some(id);
            }
            Some(delay)
        }
    }
}

/// Write `ours` over the stored preferences, keeping what another instance (a second browser
/// tab, another app window) changed there since this one last loaded or saved them.
fn write_prefs(app: &mut PhotocraftApp, ours: &Value) -> Result<(), String> {
    let Some(save) = app.services.save_prefs.as_mut() else { return Ok(()) };
    let stored = app.services.load_prefs.as_mut().and_then(|load| load()).and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let text = match (stored, &app.prefs_rt.saved_value) {
        (Some(mut theirs @ Value::Object(_)), Some(base)) => {
            let mut base = without_stale_migration(base, &theirs);
            // Normalize legacy swatches through f32, as restore_colors does.
            // Otherwise JSON values such as 0.2 differ from restored f32 values.
            if let Some(colors) = base.get_mut("toolColors").and_then(Value::as_object_mut) {
                let defaults = crate::tool_memory::default_colors();
                for key in ["foreground", "background", "maskForeground", "maskBackground"] {
                    if let Some(default) = defaults.get(key) {
                        colors.entry(key).or_insert_with(|| default.clone());
                    }
                    if let Some(value) = colors.get_mut(key)
                        && let Ok(color) = serde_json::from_value::<[f32; 4]>(value.clone())
                        && color.iter().all(|v| v.is_finite())
                        && let Ok(normalized) = serde_json::to_value(color)
                    {
                        *value = normalized;
                    }
                }
            }
            // Older files have no toolColors object. Diff the first saved swatches against
            // their actual defaults so an unchanged field cannot overwrite another window's edit.
            if matches!(base.get("toolColors"), None | Some(Value::Null))
                && ours.get("toolColors").is_some_and(Value::is_object)
                && let Some(fields) = base.as_object_mut()
            {
                fields.insert("toolColors".into(), crate::tool_memory::default_colors());
            }
            // Legacy toolColors objects had no cmykRoles. Use the default slots
            // as the merge base so another window's new associations survive.
            if ours.get("toolColors").and_then(|c| c.get("cmykRoles")).is_some_and(Value::is_array)
                && let Some(colors) = base.get_mut("toolColors").and_then(Value::as_object_mut)
                && !colors.contains_key("cmykRoles")
            {
                let roles = crate::tool_memory::recover_legacy_cmyk_roles(&Value::Object(colors.clone()));
                colors.insert("cmykRoles".into(), serde_json::json!(roles));
            }
            // Legacy preferences may contain no pick history.
            // An empty baseline lets two windows merge their new picks.
            if let Some(colors) = base.get_mut("toolColors").and_then(Value::as_object_mut) {
                colors.entry("cmykPicks").or_insert_with(|| serde_json::json!([]));
            }

            merge_changes(&base, ours, &mut theirs);
            serde_json::to_string_pretty(&theirs)
        }
        _ => serde_json::to_string_pretty(ours),
    }
    .map_err(|e| e.to_string())?;
    save(&text)
}

/// The save base without the shift tool switch migration (#5647) once storage already holds a
/// version 2 file: another instance wrote it, so its Use Shift Key for Tool Switch is a choice,
/// not the old default this instance migrated. The base then holds the migrated value (on) instead
/// of the old file's off, so only a change made here since loading is written over that choice.
fn without_stale_migration(base: &Value, theirs: &Value) -> Value {
    let mut base = base.clone();
    let new_file = |v: &Value| v.get("version").and_then(Value::as_u64).is_some_and(|v| v >= 2);
    if new_file(theirs) && !new_file(&base) {
        let key = "useShiftKeyForToolSwitch";
        if let Some(tools) = base.get_mut("tools").and_then(Value::as_object_mut)
            && tools.get(key) == Some(&Value::Bool(false))
        {
            tools.insert(key.into(), Value::Bool(true));
        }
    }
    base
}

/// Three-way merge of preference trees: whatever `ours` changed since `base` goes over `theirs`
/// (what storage holds now) and the rest of `theirs` is kept. Objects merge key by key; any
/// other value, arrays included, is replaced whole.
/// Merge the four CMYK role associations independently. Each role's RGB/ink
/// pair is atomic; changing or clearing one role must not replace the others.
fn merge_cmyk_role_slots(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    let (Some(old), Some(new), Some(stored)) = (base.as_array(), ours.as_array(), theirs.as_array_mut()) else {
        return false;
    };

    if old.len() != 2 || new.len() != 2 || stored.len() != 2 {
        return false;
    }
    if !(0..2).all(|pair| {
        old[pair].as_array().is_some_and(|v| v.len() == 2)
            && new[pair].as_array().is_some_and(|v| v.len() == 2)
            && stored[pair].as_array().is_some_and(|v| v.len() == 2)
    }) {
        return false;
    }

    for pair in 0..2 {
        let Some(old_pair) = old[pair].as_array() else { return false };
        let Some(new_pair) = new[pair].as_array() else { return false };
        let Some(stored_pair) = stored[pair].as_array_mut() else { return false };

        for role in 0..2 {
            if new_pair[role] != old_pair[role] {
                stored_pair[role] = new_pair[role].clone();
            }
        }
    }
    true
}

/// Read one native CMYK association from the stored role matrix.
fn cmyk_role(value: &Value, pair: usize, role: usize) -> Option<&Value> {
    value.get("cmykRoles")?.get(pair)?.get(role)
}

/// Merge a swatch and its native CMYK association as one selection.
/// Independent pixel and mask roles can still be changed by separate windows.
fn merge_tool_colors(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    if !base.is_object() || !ours.is_object() || !theirs.is_object() {
        return false;
    }

    let remote = theirs.clone();

    for pair in 0..2 {
        for role in 0..2 {
            if cmyk_role(base, pair, role).is_none() || cmyk_role(ours, pair, role).is_none() || cmyk_role(&remote, pair, role).is_none() {
                return false;
            }
        }
    }

    // Merge unrelated fields, including the bounded CMYK pick history.
    merge_changes(base, ours, theirs);

    for (key, pair, role) in [("foreground", 0, 0), ("background", 0, 1), ("maskForeground", 1, 0), ("maskBackground", 1, 1)] {
        let (Some(old_rgb), Some(local_rgb), Some(remote_rgb), Some(old_inks), Some(local_inks), Some(remote_inks)) =
            (base.get(key), ours.get(key), remote.get(key), cmyk_role(base, pair, role), cmyk_role(ours, pair, role), cmyk_role(&remote, pair, role))
        else {
            continue;
        };

        // Local RGB changes carry their corresponding CMYK association.
        // If the remote RGB changed but only local inks changed,
        // retain the complete remote selection.
        let (rgb, inks) = if local_rgb != old_rgb {
            (local_rgb, local_inks)
        } else if remote_rgb != old_rgb {
            (remote_rgb, remote_inks)
        } else if local_inks != old_inks {
            (local_rgb, local_inks)
        } else {
            (remote_rgb, remote_inks)
        };

        if let Some(colors) = theirs.as_object_mut() {
            colors.insert(key.to_owned(), rgb.clone());
        }

        if let Some(slot) = theirs.get_mut("cmykRoles").and_then(|v| v.get_mut(pair)).and_then(|v| v.get_mut(role)) {
            *slot = inks.clone();
        }
    }

    true
}
/// Merge recently picked CMYK colors without discarding picks from another window.
/// Each RGB key retains its most recently supplied native inks.
fn merge_cmyk_picks(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    type Pick = ([f32; 3], [f32; 4]);

    let parse = |v: &Value| -> Option<Vec<Pick>> {
        let entries = v.as_array()?;
        let picks: Vec<Pick> = entries.iter().map(|entry| serde_json::from_value::<Pick>(entry.clone())).collect::<Result<_, _>>().ok()?;

        picks.iter().all(|(rgb, inks)| rgb.iter().all(|v| v.is_finite()) && inks.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))).then_some(picks)
    };

    let (Some(old), Some(local), Some(mut stored)) = (parse(base), parse(ours), parse(theirs)) else {
        return false;
    };

    if local == old {
        return true;
    }

    // Unchanged picks form a prefix in their original order. Everything after the
    // first moved/new pick was selected locally and must become recent in storage.
    let mut old_position = 0;
    let mut changed_suffix = false;
    for &(rgb, inks) in &local {
        if !changed_suffix && let Some(offset) = old.iter().skip(old_position).position(|pick| *pick == (rgb, inks)) {
            old_position += offset + 1;
            continue;
        }
        changed_suffix = true;
        stored.retain(|(existing_rgb, _)| *existing_rgb != rgb);
        stored.push((rgb, inks));
    }

    let extra = stored.len().saturating_sub(photocraft_engine::ToolState::CMYK_PICKS);
    stored.drain(..extra);

    if let Ok(value) = serde_json::to_value(stored) {
        *theirs = value;
        true
    } else {
        false
    }
}
pub(super) fn merge_changes(base: &Value, ours: &Value, theirs: &mut Value) {
    if ours == base {
        return;
    }
    match (base, ours, theirs) {
        (Value::Object(b), Value::Object(o), Value::Object(t)) => {
            for (k, ov) in o {
                match (b.get(k), t.get_mut(k)) {
                    (Some(bv), Some(tv)) if k == "toolColors" => {
                        if !merge_tool_colors(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) if k == "cmykPicks" => {
                        if !merge_cmyk_picks(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) if k == "cmykRoles" => {
                        if !merge_cmyk_role_slots(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) => merge_changes(bv, ov, tv),
                    // Removed there, unchanged here: it stays removed.
                    (Some(bv), None) if bv == ov => {}
                    _ => {
                        t.insert(k.clone(), ov.clone());
                    }
                }
            }
            for k in b.keys().filter(|k| !o.contains_key(*k)) {
                t.remove(k);
            }
        }
        (_, _, theirs) => *theirs = ours.clone(),
    }
}

/// Write the preferences now, through the same merge as [`persist`], for recovery choices that
/// need the result immediately. The pending revision stays unsaved when the write fails.
pub(crate) fn save_preferences(app: &mut PhotocraftApp) -> Result<(), String> {
    let rev = app.session.prefs.rev();
    let ours = app.session.prefs_value();
    write_prefs(app, &ours)?;
    app.prefs_rt.saved_rev = rev;
    app.prefs_rt.saved_value = Some(ours);
    if let Some(id) = std::mem::take(&mut app.prefs_rt.save_retry).notice {
        app.ui.notices.retain(|n| n.id != id);
    }
    Ok(())
}

/// Linked smart objects follow their files while the document is open, as in Photoshop: every two
/// seconds, and as soon as the window comes back to the front (after saving in another app), stamp
/// the active document's linked files on a worker thread and re-render the layers of the ones that
/// changed ([`photocraft_engine::link_watch`]).
#[cfg(not(target_arch = "wasm32"))]
fn link_updates(app: &mut PhotocraftApp, ctx: &egui::Context) {
    match app.session.poll_link_scan() {
        Some(Ok(r)) => {
            if let Some(n) = r["updated"].as_array().map(Vec::len).filter(|n| *n > 0) {
                app.ui.status = crate::i18n::trn(
                    crate::i18n::current(),
                    n as u64,
                    "Updated {n} linked smart object changed on disk",
                    "Updated {n} linked smart objects changed on disk",
                );
            }
        }
        Some(Err(e)) => app.ui.status = e.to_string(),
        None => {}
    }
    let (last_key, focus_key) = (egui::Id::unique("linkWatch.lastScan"), egui::Id::unique("linkWatch.focused"));
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    let focused = ctx.input(|i| i.focused);
    let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
    ctx.data_mut(|d| d.insert_temp(focus_key, focused));
    let has_links = app.session.active().is_some_and(|d| !photocraft_engine::link_watch::linked_files(&d.doc).is_empty());
    if !has_links {
        return;
    }
    let last: f64 = ctx.data(|d| d.get_temp(last_key)).unwrap_or(f64::NEG_INFINITY);
    if now - last >= 2.0 || (focused && !was_focused) {
        ctx.data_mut(|d| d.insert_temp(last_key, now));
        app.session.start_link_scan();
    }
    let wait = if app.session.link_scan.is_some() { 100 } else { 2000 };
    ctx.request_repaint_after(std::time::Duration::from_millis(wait));
}

/// Background autosave of documents with unsaved changes every N minutes (File Handling).
fn autosave(app: &mut PhotocraftApp) {
    let fh = &app.session.prefs().file_handling;
    let (on, minutes) = (fh.autosave, fh.autosave_minutes.max(1));
    if app.services.autosave.is_none() {
        return;
    }
    // A successful queue operation is not a successful disk write. Poll each frame,
    // including frames before the next autosave interval, so failures are visible.
    if let Some(results) = app.services.autosave_results.as_mut() {
        for (raw_id, revision, result) in results() {
            let id = DocId(raw_id);
            if app.prefs_rt.autosave_pending.get(&id) != Some(&revision) {
                continue;
            }
            app.prefs_rt.autosave_pending.remove(&id);
            match result {
                Ok(()) => {
                    app.prefs_rt.autosaved.insert(id, revision);
                }
                Err(e) => {
                    let template = "Autosave failed: {error}";
                    app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
                    app.ui.status_error = true;
                }
            }
        }
    }
    let now = crate::gpu_canvas::now_ms();
    // Saved or closed documents drop their recovery data.
    let live: HashMap<DocId, bool> = app.session.documents().iter().map(|d| (d.doc.id, d.is_dirty())).collect();
    let mut stale: Vec<DocId> = app.prefs_rt.autosaved.keys().filter(|id| live.get(id) != Some(&true)).copied().collect();
    stale.extend(app.prefs_rt.autosave_pending.keys().filter(|id| live.get(*id) != Some(&true) && !app.prefs_rt.autosaved.contains_key(*id)).copied());
    for id in stale {
        app.prefs_rt.autosaved.remove(&id);
        app.prefs_rt.autosave_pending.remove(&id);
        if let Some(d) = app.services.discard_autosave.as_mut() {
            d(id.0);
        }
    }
    if !on {
        return;
    }
    let interval = minutes as f64 * 60_000.0;
    if app.prefs_rt.next_autosave_ms == 0.0 || app.prefs_rt.next_autosave_ms > now + interval {
        app.prefs_rt.next_autosave_ms = now + interval;
        return;
    }
    if now < app.prefs_rt.next_autosave_ms {
        return;
    }
    app.prefs_rt.next_autosave_ms = now + interval;
    let jobs: Vec<_> = app
        .session
        .documents()
        .iter()
        .filter(|d| d.is_dirty() && app.prefs_rt.autosaved.get(&d.doc.id) != Some(&d.revision) && !app.prefs_rt.autosave_pending.contains_key(&d.doc.id))
        .map(|d| (d.doc.clone(), d.revision, d.path.clone()))
        .collect();
    for (doc, rev, path) in jobs {
        if let Some(save) = app.services.autosave.as_mut() {
            match save(&doc, rev, path.as_deref()) {
                Ok(()) => {
                    if app.services.autosave_results.is_some() {
                        app.prefs_rt.autosave_pending.insert(doc.id, rev);
                    } else {
                        // Synchronous or mocked services report completion on return.
                        app.prefs_rt.autosaved.insert(doc.id, rev);
                    }
                }
                Err(e) => {
                    let template = "Autosave failed: {error}";
                    app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
                    app.ui.status_error = true;
                }
            }
        }
    }
}

/// Force the autosave timer to fire on the next tick (tests, `prefs` changes).
pub fn autosave_now(app: &mut PhotocraftApp) {
    app.prefs_rt.next_autosave_ms = f64::MIN_POSITIVE;
}

/// History Log preference: append executed commands to a text file.
fn history_log(app: &mut PhotocraftApp) {
    let n = app.session.journal.len();
    if n <= app.prefs_rt.log_len {
        app.prefs_rt.log_len = n;
        return;
    }
    let hl = app.session.prefs().history_log.clone();
    let start = app.prefs_rt.log_len;
    app.prefs_rt.log_len = n;
    if !hl.enabled || hl.file_path.is_empty() || hl.destination == prefs::LogDestination::Metadata {
        return;
    }
    let Some(append) = app.services.append_text.as_mut() else { return };
    let mut text = String::new();
    for (id, params) in &app.session.journal[start..] {
        let label = photocraft_engine::commands::find(id).map_or(id.as_str(), |c| c.label).trim_end_matches('…');
        match hl.detail {
            prefs::LogDetail::SessionsOnly => {
                if matches!(id.as_str(), "file.new" | "file.open" | "file.close" | "file.openAs") {
                    text.push_str(&format!("{label}\n"));
                }
            }
            prefs::LogDetail::Concise => text.push_str(&format!("{label}\n")),
            prefs::LogDetail::Detailed => text.push_str(&format!("{label}  {id} {params}\n")),
        }
    }
    if !text.is_empty() {
        let _ = append(&hl.file_path, &text);
    }
}
