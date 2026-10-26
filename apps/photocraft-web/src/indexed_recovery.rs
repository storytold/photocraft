//! Crash recovery in the browser (#6615): IndexedDB side of [`crate::recovery_bridge`], and the
//! `Services` the UI's existing autosave and recovery code calls.
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use web_sys::{Blob, IdbDatabase, IdbTransactionMode};

use crate::indexed_presets::{FILES, TIMEOUT_MS, before, js, open, request, write};
use crate::recovery_bridge::{MAX_ENTRIES, MAX_SNAPSHOT_BYTES, Recovery};
use photocraft_ui_egui::Services;

const DATABASE: &str = "photocraft.recovery";

/// Open the recovery database and read what an earlier page load left. Storage the browser
/// denies (private windows, blocked site data) returns the error; the app then runs without
/// recovery and says so when an autosave is attempted.
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

async fn read(db: &IdbDatabase, recovery: &Recovery, deadline: f64) -> Result<(), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readonly).map_err(js)?;
    let store = tx.object_store(FILES).map_err(js)?;
    // Both requests are issued before awaiting either, so the transaction stays active.
    let keys = request(&store.get_all_keys().map_err(js)?);
    let values = request(&store.get_all().map_err(js)?);
    let keys = js_sys::Array::from(&before(&keys.promise, deadline).await?);
    let values = js_sys::Array::from(&before(&values.promise, deadline).await?);
    for (key, value) in keys.iter().zip(values.iter()).take(MAX_ENTRIES * 2) {
        let Some(name) = key.as_string().filter(|n| n.len() <= 110) else { continue };
        let Ok(blob) = value.dyn_into::<Blob>() else { continue };
        // Inspect the size before copying a record into wasm memory.
        if blob.size() > MAX_SNAPSHOT_BYTES as f64 + 4096.0 {
            continue;
        }
        let buffer = before(&blob.array_buffer(), deadline).await?;
        recovery.load(&name, Uint8Array::new(&buffer).to_vec());
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
                let result = write(&db, &batch).await;
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
