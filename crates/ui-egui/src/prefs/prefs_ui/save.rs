use super::appearance::{canvas_style, sync_animations, sync_appearance, sync_display_scale, sync_tooltips};
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
    sync_animations(app, ctx);
    app.sync_recent();
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
            let base = &without_stale_migration(base, &theirs);
            merge_changes(base, ours, &mut theirs);
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
pub(super) fn merge_changes(base: &Value, ours: &Value, theirs: &mut Value) {
    if ours == base {
        return;
    }
    match (base, ours, theirs) {
        (Value::Object(b), Value::Object(o), Value::Object(t)) => {
            for (k, ov) in o {
                match (b.get(k), t.get_mut(k)) {
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
/// seconds while PhotoCraft is in front, and as soon as it comes back to the front (after saving in
/// another app), stamp the active document's linked files on a worker thread and re-render the
/// layers of the ones that changed ([`photocraft_engine::link_watch`]). Behind another app nothing
/// is stamped and no frame is asked for: a look every two seconds there redrew the interface and
/// started a thread for files that are looked at anyway the moment PhotoCraft is back.
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
    let focused = crate::os_window::in_front(ctx);
    let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
    ctx.data_mut(|d| d.insert_temp(focus_key, focused));
    let has_links = app.session.active().is_some_and(|d| !photocraft_engine::link_watch::linked_files(&d.doc).is_empty());
    if !has_links {
        return;
    }
    let last: f64 = ctx.data(|d| d.get_temp(last_key)).unwrap_or(f64::NEG_INFINITY);
    if focused && (now - last >= 2.0 || !was_focused) {
        ctx.data_mut(|d| d.insert_temp(last_key, now));
        app.session.start_link_scan();
    }
    if app.session.link_scan.is_some() {
        // The look doesn't wake the interface: a frame collects it.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    } else if focused {
        ctx.request_repaint_after(std::time::Duration::from_millis(2000));
    }
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

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod link_tests {
    use super::*;
    use serde_json::json;

    /// Save a solid-colour 10 × 10 PNG as `path`: written beside it, then moved over it, as an app
    /// saving the file does.
    fn write_png(path: &std::path::Path, color: &str) {
        let copy = path.with_extension("tmp.png");
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10, "background": color})).unwrap();
        s.execute("file.saveACopy", json!({"path": copy.to_string_lossy(), "layers": false})).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::rename(&copy, path).unwrap();
    }

    /// Linked smart objects follow their files every two seconds while PhotoCraft is in front, not
    /// at all behind another app (no look, no frame asked for), and a file saved meanwhile is looked
    /// at the moment PhotoCraft is back in front. The look times itself on the app's monotonic
    /// clock, so "two seconds later" moves the time of the last look back instead.
    #[test]
    fn linked_files_are_looked_at_in_front_and_at_once_on_coming_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        write_png(&path, "#ff0000");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.run("file.placeLinked", json!({"path": path.to_string_lossy()})).unwrap();
        let ctx = egui::Context::default();
        let last_key = egui::Id::unique("linkWatch.lastScan");
        // One frame with PhotoCraft in front or not; returns when the next one is asked for.
        let frame = |app: &mut PhotocraftApp, in_front: bool| {
            let mut input = egui::RawInput { focused: in_front, ..Default::default() };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(in_front);
            let mut out = ctx.run_ui(input, |ui| link_updates(app, ui.ctx()));
            out.textures_delta.clear();
            out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
        };
        // Until the look in progress is in (its frames come every 100 ms).
        let finish = |app: &mut PhotocraftApp| {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.session.link_scan.is_some() {
                assert!(std::time::Instant::now() < end, "the look never finished");
                std::thread::sleep(std::time::Duration::from_millis(5));
                frame(app, true);
            }
        };
        let two_seconds_pass = || ctx.data_mut(|d| d.insert_temp(last_key, f64::NEG_INFINITY));
        let pixel = |app: &mut PhotocraftApp| app.run("document.pixel", json!({"x": 20, "y": 20})).unwrap();

        // In front: looked at at once, and every two seconds.
        assert!(frame(&mut app, true) <= std::time::Duration::from_millis(100));
        assert!(app.session.link_scan.is_some(), "the first frame in front looks");
        finish(&mut app);
        let next = frame(&mut app, true);
        assert!(app.session.link_scan.is_none(), "no second look within two seconds");
        assert!(next > std::time::Duration::from_secs(1) && next <= std::time::Duration::from_secs(2), "the next look is two seconds away: {next:?}");
        two_seconds_pass();
        frame(&mut app, true);
        assert!(app.session.link_scan.is_some(), "two seconds later it looks again");
        finish(&mut app);
        let red = pixel(&mut app);

        // Behind another app: no look and no frame, however long; the file is saved meanwhile.
        for _ in 0..3 {
            two_seconds_pass();
            assert_eq!(frame(&mut app, false), std::time::Duration::MAX, "no frame asked for behind another app");
            assert!(app.session.link_scan.is_none(), "nothing looked at behind another app");
        }
        write_png(&path, "#0000ff");
        assert_eq!(frame(&mut app, false), std::time::Duration::MAX);
        assert!(app.session.link_scan.is_none());
        assert_eq!(pixel(&mut app), red, "not updated while in the back");

        // Back in front: looked at in that same frame, and the layer follows its file.
        assert!(frame(&mut app, true) <= std::time::Duration::from_millis(100));
        assert!(app.session.link_scan.is_some(), "coming back to the front looks at once");
        finish(&mut app);
        assert_ne!(pixel(&mut app), red, "the layer follows the file saved while PhotoCraft was in the back");
        assert!(app.ui.status.contains("Updated 1 linked smart object"), "{}", app.ui.status);

        // In front through another of its windows (a document window has the focus): still in
        // front.
        two_seconds_pass();
        let mut input = egui::RawInput { focused: false, ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(false);
        input.viewports.entry(egui::ViewportId::from_hash_of(("docwin", 1u64))).or_default().focused = Some(true);
        let mut out = ctx.run_ui(input, |ui| link_updates(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert!(app.session.link_scan.is_some(), "a document window in front counts as PhotoCraft in front");
        finish(&mut app);
    }
}
