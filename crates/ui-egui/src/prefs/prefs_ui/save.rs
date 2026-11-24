use photocraft_engine::link_watch::LinkWatchExt;
use photocraft_engine::prefs::SavedPrefsExt;
use std::time::Instant;

use craft::platform::autosave::{AutosaveDoc, interval_for_minutes};

use super::appearance::{canvas_style, sync_appearance, sync_display_scale, sync_tooltips};
use super::startup::{load, presets_store, recovery, sync_rulers};
use super::{PhotocraftApp, prefs};

/// The notice a failed preferences write posts; a later successful write removes it.
const SAVE_FAILED_TITLE: &str = "Preferences can't be saved";

/// Per-frame upkeep: theme sync, persistence, autosave and the history log. `now` is the
/// caller's monotonic clock (logic also runs on hidden ticks, where egui's input time is that of
/// the last UI pass).
///
/// Nothing here reads the frame's input, so a second call in one frame does nothing new: the
/// preferences revision is already handed over, the store and the autosave timer have nothing
/// due at the same `now`, and the autosave results were drained by the first call.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context, now: Instant) {
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
    #[cfg(not(target_arch = "wasm32"))]
    super::agents::sync_agent_link(app, ctx);
    let save_at = store_preferences(app, now);
    let style = canvas_style(app);
    if let Some(gpu) = app.gpu.as_ref()
        && app.prefs_rt.gpu_style != Some(style)
    {
        gpu.set_style(style);
        app.prefs_rt.gpu_style = Some(style);
    }
    autosave(app, now);
    if let Some(at) = [save_at, app.autosave.deadline()].into_iter().flatten().min() {
        ctx.request_repaint_after(at.saturating_duration_since(now));
    }
    #[cfg(not(target_arch = "wasm32"))]
    link_updates(app, ctx);
    history_log(app);
}

/// Hands a changed preferences revision to the settings store (`now`: the store's change time;
/// `None` reads the clock).
#[cfg(not(target_arch = "wasm32"))]
fn hand_over(app: &mut PhotocraftApp, now: Option<Instant>) {
    let rev = app.session.prefs.rev();
    if rev == app.prefs_rt.saved_rev || app.services.settings.is_none() {
        return;
    }
    let saved = app.session.saved_prefs();
    if let Some((store, _)) = app.services.settings.as_mut() {
        match now {
            Some(now) => store.update_at(now, |p| *p = saved),
            None => store.update(|p| *p = saved),
        };
    }
    app.prefs_rt.saved_rev = rev;
}

/// Saves the preferences through the store: hands over a change, ticks the store (its debounce and
/// retry backoff), and reports a failed write once and its recovery. Returns when to tick again.
#[cfg(not(target_arch = "wasm32"))]
fn store_preferences(app: &mut PhotocraftApp, now: Instant) -> Option<Instant> {
    hand_over(app, Some(now));
    let (store, _) = app.services.settings.as_mut()?;
    let result = store.tick(now);
    // Another instance changed the file and the store merged it: show its values here too.
    let merged = store.take_merged_change().then(|| store.get().clone());
    match result.status {
        craft::settings::SaveStatus::Failed(e) => save_failed(app, &e.to_string()),
        craft::settings::SaveStatus::Saved => saved(app),
        craft::settings::SaveStatus::Idle | craft::settings::SaveStatus::Pending => {}
    }
    if let Some(merged) = merged {
        app.session.set_saved_prefs(merged);
        app.prefs_rt.saved_rev = app.session.prefs.rev();
    }
    result.next_deadline
}

/// The web build saves through `Services::save_prefs` (browser storage) until it opens its own
/// settings store (FW-264): the whole document on every change, no retry.
#[cfg(target_arch = "wasm32")]
fn store_preferences(app: &mut PhotocraftApp, _now: Instant) -> Option<Instant> {
    let rev = app.session.prefs.rev();
    if rev == app.prefs_rt.saved_rev {
        return None;
    }
    let text = app.session.prefs_to_json();
    let Some(save) = app.services.save_prefs.as_mut() else { return None };
    match save(&text) {
        Ok(()) => {
            app.prefs_rt.saved_rev = rev;
            saved(app);
        }
        Err(e) => save_failed(app, &e),
    }
    None
}

/// A preferences write failed; the store retries it. The first failure in a row sets the status
/// line and posts the notice; the notice being shown keeps later failures quiet.
fn save_failed(app: &mut PhotocraftApp, error: &str) {
    if crate::notices::shown_titled(app, SAVE_FAILED_TITLE) {
        return;
    }
    let template = "Couldn't save preferences: {error}. Retrying…";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("error", error)]);
    app.ui.status_error = true;
    let lines = vec![error.to_string(), "PhotoCraft keeps retrying; until a save succeeds, preference changes are lost when it closes.".into()];
    crate::notices::post(app, SAVE_FAILED_TITLE, lines, craft::theme::variant::Tone::Bad, None);
}

/// A preferences write succeeded: after a failure, say so and take the notice down.
fn saved(app: &mut PhotocraftApp) {
    if !crate::notices::shown_titled(app, SAVE_FAILED_TITLE) {
        return;
    }
    crate::notices::remove_titled(app, SAVE_FAILED_TITLE);
    let message = "Preferences saved";
    app.ui.status = tl!(message).into();
    app.ui.status_error = false;
}

/// Write the preferences now, for choices that need the result immediately (recovery, "don't
/// show again"). The change stays pending in the store, and is retried, when the write fails.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_preferences(app: &mut PhotocraftApp) -> Result<(), String> {
    hand_over(app, None);
    let Some((store, _)) = app.services.settings.as_mut() else { return Ok(()) };
    store.flush().map_err(|e| e.to_string())?;
    saved(app);
    Ok(())
}

/// Write the preferences now (the web build's storage).
#[cfg(target_arch = "wasm32")]
pub(crate) fn save_preferences(app: &mut PhotocraftApp) -> Result<(), String> {
    let rev = app.session.prefs.rev();
    let text = app.session.prefs_to_json();
    let Some(save) = app.services.save_prefs.as_mut() else { return Ok(()) };
    save(&text)?;
    app.prefs_rt.saved_rev = rev;
    saved(app);
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
    let now = ctx.input(|i| i.time);
    let focused = ctx.input(|i| i.focused);
    let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
    ctx.data_mut(|d| d.insert_temp(focus_key, focused));
    let has_links = app.session.active().is_some_and(|d| app.doc_queries.has_linked_files(&app.session, d));
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

/// Background autosave of documents with unsaved changes every N minutes (File Handling). The
/// timer (`app.autosave`) decides which documents are due and which recovery copies to discard;
/// the write goes through `Services::autosave`.
///
/// A saved or closed document needs no `Autosave::forget` here: the timer's next `due` finds it
/// clean or gone and lists it in `discard` (craft-platform autosave/timer.rs:124-132).
fn autosave(app: &mut PhotocraftApp, now: Instant) {
    if app.services.autosave.is_none() {
        return;
    }
    let fh = &app.session.prefs().file_handling;
    let interval = fh.autosave.then(|| interval_for_minutes(prefs::nearest_autosave_minutes(fh.autosave_minutes))).flatten();
    app.autosave.set_interval(interval);
    // A successful queue operation is not a successful disk write. Poll each frame, including
    // frames before the next autosave interval, so failures are visible.
    if let Some(results) = app.services.autosave_results.as_mut() {
        for (id, revision, result) in results() {
            if let Some(failed) = app.autosave.finished(id, revision, result) {
                autosave_failed(app, failed);
            }
        }
    }
    // Apply Data Set's Preview is not the document: autosave waits until the preview ends, and
    // the timer keeps what is due for the next tick.
    if crate::panels::apply_data_set_ui::previewing(app) {
        return;
    }
    let docs: Vec<AutosaveDoc> = app.session.documents().iter().map(|d| AutosaveDoc { id: d.doc.id.0, revision: d.revision, dirty: d.is_dirty() }).collect();
    let due = app.autosave.due(now, docs);
    if let Some(discard) = app.services.discard_autosave.as_mut() {
        for id in &due.discard {
            discard(*id);
        }
    }
    let jobs: Vec<_> = due
        .save
        .iter()
        .filter_map(|id| app.session.documents().iter().find(|d| d.doc.id.0 == *id))
        .map(|d| (d.doc.clone(), d.revision, d.path.clone()))
        .collect();
    let reports_later = app.services.autosave_results.is_some();
    for (doc, revision, path) in jobs {
        let Some(save) = app.services.autosave.as_mut() else { return };
        let queued = save(&doc, revision, path.as_deref());
        // Synchronous or mocked services report completion on return; a failure to queue is the
        // write's failure either way.
        if (queued.is_err() || !reports_later)
            && let Some(failed) = app.autosave.finished(doc.id.0, revision, queued)
        {
            autosave_failed(app, failed);
        }
    }
}

/// The status line of a failed autosave (`Autosave::finished`'s text); the next interval retries.
fn autosave_failed(app: &mut PhotocraftApp, text: String) {
    app.ui.status = text;
    app.ui.status_error = true;
}

/// Makes the next tick autosave every dirty document that has none in flight (tests, `prefs`
/// changes).
pub fn autosave_now(app: &mut PhotocraftApp) {
    app.autosave.autosave_now();
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
