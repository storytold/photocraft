//! Crash recovery in the browser (#6615): IndexedDB side of [`crate::recovery_bridge`], and the
//! `Services` the UI's existing autosave and recovery code calls.
//!
//! Several tabs share the database, so every page load writes under its own session id and
//! holds a Web Lock named after it for as long as the page lives. At launch a page lists the
//! database's keys, and for each session that is not its own tries to take that session's lock
//! "if available": only a page whose writer is gone gets it, and only one page wins. Entries of
//! sessions it could not lock belong to a page that may be open; they are never listed, adopted
//! or deleted (see [`Recovery::claim`]). Without the Web Locks API (insecure contexts), no
//! earlier entries are offered, which is the safe side.
use js_sys::{Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Blob, IdbDatabase, IdbTransactionMode};

use crate::indexed_presets::{FILES, TIMEOUT_MS, before, js, open, request, write, write_capped};
use crate::recovery_bridge::{MAX_ENTRIES, MAX_SNAPSHOT_BYTES, Recovery, recover_on_launch, session_of};
use photocraft_ui_egui::Services;

const DATABASE: &str = "photocraft.recovery";
/// Most recovery data kept at once, across documents and tabs. Enforced inside the write
/// transaction (`indexed_presets::write_capped`), against what is actually stored.
const MAX_TOTAL_BYTES: u64 = 768 << 20;
/// Shown as "Autosave failed" when the total cap stops a write.
const TOTAL_LIMIT_MSG: &str = "the browser's recovery storage limit is reached; save or close a document";
const PREFS_KEY: &str = "photocraft.preferences";
/// All the per-record reads together may not hold up startup longer than this, whatever the count.
const SCAN_MS: f64 = 15_000.0;

/// Open the recovery database, take this page's lock and read what earlier page loads left.
/// Storage the browser denies (private windows, blocked site data) returns the error; the app
/// then runs without recovery and says so when an autosave is attempted. A slow or unreadable
/// record is skipped, not fatal.
pub async fn load() -> Result<(IdbDatabase, Recovery), String> {
    let deadline = js_sys::Date::now() + TIMEOUT_MS;
    let db = open(DATABASE, deadline).await?;
    let session = format!("{:x}{:x}", js_sys::Date::now() as u64, (js_sys::Math::random() * 4_294_967_296.0) as u64);
    let recovery = Recovery::new(session);
    match read(&db, &recovery, deadline).await {
        Ok(()) => Ok((db, recovery)),
        Err(e) => {
            db.close();
            Err(e)
        }
    }
}

fn lock_name(session: &str) -> String {
    format!("photocraft.recovery.{session}")
}

/// Take Web Lock `name` and keep it until the page goes away (the callback's promise never
/// settles). With `if_available`, `Ok(false)` means another page holds it. The answer is awaited
/// until the absolute `deadline`, so callers that take several locks can share one.
async fn hold(name: &str, if_available: bool, deadline: f64) -> Result<bool, String> {
    let navigator = web_sys::window().ok_or("no window")?.navigator();
    let locks = Reflect::get(&navigator, &JsValue::from_str("locks")).map_err(js)?;
    if locks.is_undefined() || locks.is_null() {
        return Err("Web Locks are unavailable".into());
    }
    let request = Reflect::get(&locks, &JsValue::from_str("request")).map_err(js)?.dyn_into::<Function>().map_err(|_| "Web Locks are unavailable")?;
    let options = Object::new();
    Reflect::set(&options, &JsValue::from_str("ifAvailable"), &JsValue::from_bool(if_available)).map_err(js)?;
    let mut failed = None;
    let granted = Promise::new(&mut |resolve, reject| {
        let callback = Closure::once_into_js(move |lock: JsValue| -> Promise {
            let got = !lock.is_null();
            let _ = resolve.call1(&JsValue::NULL, &JsValue::from_bool(got));
            if got { Promise::new(&mut |_, _| {}) } else { Promise::resolve(&JsValue::UNDEFINED) }
        });
        if let Err(e) = request.call3(&locks, &JsValue::from_str(name), &options, &callback) {
            failed = Some(e);
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("lock request failed"));
        }
    });
    if let Some(e) = failed {
        return Err(js(e));
    }
    Ok(before(&granted, deadline).await?.as_bool().unwrap_or(false))
}

/// One record, read in its own transaction (and under its own deadline) so one slow record
/// cannot take the others, or the database, down with it.
async fn get_blob(db: &IdbDatabase, name: &str, scan_end: f64) -> Result<Option<Blob>, String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readonly).map_err(js)?;
    let req = tx.object_store(FILES).map_err(js)?.get(&JsValue::from_str(name)).map_err(js)?;
    let pending = request(&req);
    Ok(before(&pending.promise, (js_sys::Date::now() + TIMEOUT_MS).min(scan_end)).await?.dyn_into::<Blob>().ok())
}

async fn read(db: &IdbDatabase, recovery: &Recovery, deadline: f64) -> Result<(), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readonly).map_err(js)?;
    let keys = request(&tx.object_store(FILES).map_err(js)?.get_all_keys().map_err(js)?);
    let names: Vec<String> =
        js_sys::Array::from(&before(&keys.promise, deadline).await?).iter().filter_map(|k| k.as_string().filter(|n| n.len() <= 110)).collect();
    // This page's lock comes before any write, so no other page can ever see its entries as
    // abandoned. Without locks nothing is claimed.
    let own = recovery.session();
    let locked = match hold(&lock_name(&own), false, js_sys::Date::now() + TIMEOUT_MS).await {
        Ok(got) => got,
        Err(e) => {
            log::warn!("Crash recovery cannot tell live tabs from closed ones, so earlier entries are not offered: {e}");
            false
        }
    };
    let stored = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).and_then(|s| s.get_item(PREFS_KEY).ok().flatten());
    let mut claimed: Vec<&str> = Vec::new();
    if locked && recover_on_launch(stored.as_deref()) {
        // One deadline for every candidate: if the lock manager stalls, the claims together (not
        // each in turn) may hold up startup for at most this long. Claims that were granted
        // promptly are kept.
        let claim_deadline = js_sys::Date::now() + TIMEOUT_MS;
        for session in names.iter().filter_map(|n| session_of(n)).filter(|s| *s != own) {
            if js_sys::Date::now() >= claim_deadline {
                log::warn!("Crash recovery stopped claiming earlier sessions: the lock manager is slow");
                break;
            }
            if claimed.len() < MAX_ENTRIES && !claimed.contains(&session) && hold(&lock_name(session), true, claim_deadline).await.unwrap_or(false) {
                recovery.claim(session);
                claimed.push(session);
            }
        }
    }
    let scan_end = js_sys::Date::now() + SCAN_MS;
    // Only a claimed page's sidecars and bundles are read. The record limit applies after that
    // filter: IndexedDB lists keys in name order, so limiting the names first could leave a claimed
    // session's pair past the first 128 names unread on every launch. The total cap is not tracked
    // here: the write transaction checks it against what is actually stored.
    let eligible = names.iter().filter(|n| session_of(n).is_some_and(|s| claimed.contains(&s)) && (n.ends_with(".pcraft") || n.ends_with(".json")));
    for name in eligible.take(MAX_ENTRIES * 2) {
        if js_sys::Date::now() >= scan_end {
            log::warn!("Crash recovery stopped reading records: the scan took too long");
            break;
        }
        let blob = match get_blob(db, name, scan_end).await {
            Ok(Some(blob)) => blob,
            Ok(None) => continue,
            Err(e) => {
                log::warn!("Skipping recovery record {name}: {e}");
                continue;
            }
        };
        if blob.size() > MAX_SNAPSHOT_BYTES as f64 + 4096.0 {
            log::warn!("Skipping recovery record {name}: it is larger than the autosave limit");
        } else {
            match before(&blob.array_buffer(), (js_sys::Date::now() + TIMEOUT_MS).min(scan_end)).await {
                Ok(buffer) => recovery.load(name, Uint8Array::new(&buffer).to_vec()),
                Err(e) => log::warn!("Skipping recovery record {name}: {e}"),
            }
        }
    }
    Ok(())
}

/// The UI's autosave and recovery hooks over `db`. `db` is `None` when storage was unavailable.
pub fn services(db: Option<IdbDatabase>, recovery: Recovery) -> Services {
    let (r1, r2, r3, r4, r5) = (recovery.clone(), recovery.clone(), recovery.clone(), recovery.clone(), recovery);
    let (db1, db2) = (db.clone(), db);
    Services {
        autosave: Some(Box::new(move |doc, revision, path| {
            let db = db1.clone().ok_or("browser storage is unavailable, so recovery data cannot be kept")?;
            let batch = r1.snapshot(doc, revision, path, (js_sys::Date::now() / 1000.0) as u64)?;
            let (id, done) = (doc.id.0, r1.clone());
            wasm_bindgen_futures::spawn_local(async move {
                // A quota error lands here, and the UI shows it as "Autosave failed".
                // The total cap is enforced inside the transaction, against what every tab stored.
                let result = write_capped(&db, &batch, Some((MAX_TOTAL_BYTES, TOTAL_LIMIT_MSG))).await;
                done.finish(id, revision, result);
            });
            Ok(())
        })),
        autosave_results: Some(Box::new(move || r2.take_completed())),
        flush_autosaves: Some(Box::new(move || r3.take_completed())),
        discard_autosave: Some(Box::new(move |id| {
            if let (Some(db), Some(batch)) = (db2.clone(), r4.discard(id)) {
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) = write(&db, &batch).await {
                        log::warn!("Could not remove recovery data: {e}");
                    }
                });
            }
        })),
        recover: Some(Box::new({
            let r = r5.clone();
            move || r.recoverable()
        })),
        adopt_autosave: Some(Box::new(move |id, key| r5.adopt(id, key))),
        ..Default::default()
    }
}
