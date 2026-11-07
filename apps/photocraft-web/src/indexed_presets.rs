//! IndexedDB owns durable bytes; the engine sees only the bounded, Send memory bridge.
//! Each batch commits tips, groups and index together, and success means transaction completion.
use js_sys::{Array, Function, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, Event, EventTarget, IdbDatabase, IdbRequest, IdbTransactionMode};

use crate::preset_bridge::{Batch, Bridge, MAX_RECORDS, fits_total, record_limit};
use photocraft_engine::preset_store::MAX_STORE_BYTES;

const DATABASE: &str = "photocraft.presets";
pub const FILES: &str = "files";
pub const TIMEOUT_MS: f64 = 10_000.0;

pub fn js(e: JsValue) -> String {
    if let Some(exception) = e.dyn_ref::<web_sys::DomException>() {
        format!("{}: {}", exception.name(), exception.message())
    } else {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }
}

/// Listeners are detached even when startup or a write times out.
type Listener = (EventTarget, &'static str, Closure<dyn FnMut(Event)>);
struct Listeners(Vec<Listener>);
impl Listeners {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn add(&mut self, target: &EventTarget, name: &'static str, f: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
        let cb = Closure::<dyn FnMut(Event)>::new(f);
        target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref())?;
        self.0.push((target.clone(), name, cb));
        Ok(())
    }
}
impl Drop for Listeners {
    fn drop(&mut self) {
        for (target, name, cb) in &self.0 {
            let _ = target.remove_event_listener_with_callback(name, cb.as_ref().unchecked_ref());
        }
    }
}

pub struct Pending {
    pub promise: Promise,
    _listeners: Listeners,
}

pub fn request(request: &IdbRequest) -> Pending {
    let mut listeners = Listeners::new();
    let promise = Promise::new(&mut |resolve, reject| {
        let req = request.clone();
        let failed = reject.clone();
        if let Err(e) = listeners.add(request, "success", move |_| match req.result() {
            Ok(v) => {
                let _ = resolve.call1(&JsValue::NULL, &v);
            }
            Err(e) => {
                let _ = failed.call1(&JsValue::NULL, &e);
            }
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
        let req = request.clone();
        let failed = reject.clone();
        if let Err(e) = listeners.add(request, "error", move |_| {
            let e = req.error().ok().flatten().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("IndexedDB request failed"));
            let _ = failed.call1(&JsValue::NULL, &e);
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
    });
    Pending { promise, _listeners: listeners }
}

/// A single deadline bounds the whole startup read, not ten seconds per record.
pub async fn before(promise: &Promise, deadline: f64) -> Result<JsValue, String> {
    let window = web_sys::window().ok_or("no window")?;
    let mut timer = None;
    let mut callback = None;
    let timeout = Promise::new(&mut |_, reject| {
        let cb = Closure::<dyn FnMut()>::new(move || {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("browser storage timed out"));
        });
        timer = Some(window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            (deadline - js_sys::Date::now()).clamp(0.0, TIMEOUT_MS) as i32,
        ));
        callback = Some(cb);
    });
    let id = timer.ok_or("could not start storage timeout")?.map_err(js)?;
    let result = JsFuture::from(Promise::race(&Array::of2(promise, &timeout))).await.map_err(js);
    window.clear_timeout_with_handle(id);
    drop(callback);
    result
}

pub async fn open(database: &str, deadline: f64) -> Result<IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(js)?.ok_or("IndexedDB unavailable")?;
    let req = factory.open_with_u32(database, 1).map_err(js)?;
    let mut upgrade = Listeners::new();
    let upgrade_req = req.clone();
    upgrade
        .add(&req, "upgradeneeded", move |_| {
            let result = upgrade_req.result().and_then(|v| v.dyn_into::<IdbDatabase>()).and_then(|db| db.create_object_store(FILES));
            if result.is_err()
                && let Some(tx) = upgrade_req.transaction()
            {
                let _ = tx.abort();
            }
        })
        .map_err(js)?;
    let pending = request(&req);
    match before(&pending.promise, deadline).await {
        Ok(value) => value.dyn_into().map_err(|_| "invalid preset database".into()),
        Err(e) => {
            // An open request cannot be cancelled (another tab may block its upgrade). Keep
            // its handlers alive and close any late connection; never hydrate a live session.
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(value) = JsFuture::from(pending.promise.clone()).await
                    && let Ok(db) = value.dyn_into::<IdbDatabase>()
                {
                    db.close();
                }
                drop(pending);
                drop(upgrade);
            });
            Err(e)
        }
    }
}

pub async fn load() -> Result<(IdbDatabase, Bridge, Vec<String>), String> {
    let deadline = js_sys::Date::now() + TIMEOUT_MS;
    let db = open(DATABASE, deadline).await?;
    let result = read(&db, deadline).await;
    match result {
        Ok((bridge, warnings)) => Ok((db, bridge, warnings)),
        Err(e) => {
            db.close();
            Err(e)
        }
    }
}

async fn read(db: &IdbDatabase, deadline: f64) -> Result<(Bridge, Vec<String>), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readonly).map_err(js)?;
    let req = tx.object_store(FILES).map_err(js)?.open_cursor().map_err(js)?;
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut unreadable = Vec::new();
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut next = request(&req);
    loop {
        let value = before(&next.promise, deadline).await?;
        if value.is_null() {
            break;
        }
        let cursor = value.dyn_into::<web_sys::IdbCursorWithValue>().map_err(|_| "invalid preset cursor")?;
        seen += 1;
        if seen > MAX_RECORDS {
            return Err(format!("browser preset record limit ({MAX_RECORDS}) exceeded"));
        }
        let key = cursor.key().map_err(js)?;
        // Inspect key length in JS before allocating its Rust String.
        let name = if key.is_string() && js_sys::JsString::from(key.clone()).length() <= 133 { key.as_string().unwrap_or_default() } else { String::new() };
        let blob = cursor.value().map_err(js)?.dyn_into::<Blob>();
        let record = record_limit(&name).and_then(|limit| {
            let blob = blob.map_err(|_| "record is not a Blob".to_string())?;
            if blob.size() > limit as f64 {
                return Err("record is too large".into());
            }
            Ok(blob)
        });
        match record {
            Ok(blob) => {
                total = total.saturating_add(blob.size() as u64);
                if total > MAX_STORE_BYTES {
                    return Err("browser preset store exceeds size limit".into());
                }
                records.push((name, blob));
            }
            Err(e) => {
                // Leave an unreadable placeholder in the engine's file inventory. In
                // particular, a skipped group must protect its tip records from orphan GC.
                if record_limit(&name).is_ok() {
                    unreadable.push(name.clone());
                }
                if warnings.len() < 8 {
                    warnings.push(format!("Brush presets: {name} skipped: {e}"));
                }
            }
        }
        // Register the next callback before continuing the cursor. Blob reads happen after
        // enumeration, since awaiting them here would let this transaction become inactive.
        next = request(&req);
        cursor.continue_().map_err(js)?;
    }
    let bridge = Bridge::default();
    for name in unreadable {
        bridge.load(&name, Vec::new())?;
    }
    for (name, blob) in records {
        let buffer = before(&blob.array_buffer(), deadline).await?;
        bridge.load(&name, Uint8Array::new(&buffer).to_vec())?;
    }
    Ok((bridge, warnings))
}

pub async fn write(db: &IdbDatabase, batch: &Batch) -> Result<(), String> {
    write_capped(db, batch, None).await
}

/// [`write`] with a limit on the whole store. With `cap = Some((max, message))`, a batch that
/// stores anything first has the transaction read every stored record's size, and is aborted
/// with `message` if it would take the store past `max` bytes ([`fits_total`]). The check and
/// the commit are one transaction, so tabs sharing the database cannot each pass a private
/// check and together exceed the cap.
pub async fn write_capped(db: &IdbDatabase, batch: &Batch, cap: Option<(u64, &str)>) -> Result<(), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readwrite).map_err(js)?;
    let store = tx.object_store(FILES).map_err(js)?;
    let mut reads = None;
    let queued = (|| -> Result<(), String> {
        // Queued before the writes, so they see the store as it was. Their results arrive before
        // the transaction can commit, and aborting then discards the queued writes.
        if cap.is_some() && batch.values().any(Option::is_some) {
            let keys = store.get_all_keys().map_err(js)?;
            let values = store.get_all().map_err(js)?;
            reads = Some((request(&keys), request(&values)));
        }
        for (name, bytes) in batch {
            let key = JsValue::from_str(name);
            if let Some(bytes) = bytes {
                let blob = Blob::new_with_u8_array_sequence(&Array::of1(&Uint8Array::from(bytes.as_ref()))).map_err(js)?;
                store.put_with_key(&blob, &key).map_err(js)?;
            } else {
                store.delete(&key).map_err(js)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = queued {
        let _ = tx.abort();
        return Err(e);
    }
    let mut listeners = Listeners::new();
    let promise = Promise::new(&mut |resolve, reject| {
        if let Err(e) = listeners.add(&tx, "complete", move |_| {
            let _ = resolve.call0(&JsValue::NULL);
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
        for event in ["abort", "error"] {
            let tx = tx.clone();
            let failed: Function = reject.clone();
            let target: EventTarget = tx.clone().into();
            if let Err(e) = listeners.add(&target, event, move |_| {
                let e = tx.error().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("preset transaction aborted"));
                let _ = failed.call1(&JsValue::NULL, &e);
            }) {
                let _ = reject.call1(&JsValue::NULL, &e);
            }
        }
    });
    if let (Some((max, message)), Some((keys, values))) = (cap, reads) {
        let verdict = async {
            let deadline = js_sys::Date::now() + TIMEOUT_MS;
            let keys = Array::from(&before(&keys.promise, deadline).await?);
            let values = Array::from(&before(&values.promise, deadline).await?);
            let stored = keys.iter().zip(values.iter()).filter_map(|(k, v)| Some((k.as_string()?, v.dyn_ref::<Blob>().map_or(0, |b| b.size() as u64))));
            Ok::<bool, String>(fits_total(stored, batch, max))
        }
        .await;
        if !matches!(verdict, Ok(true)) {
            let _ = tx.abort();
            // Consume the abort so it is not reported as an unhandled rejection.
            let _ = before(&promise, js_sys::Date::now() + TIMEOUT_MS).await;
            return Err(verdict.err().unwrap_or_else(|| message.to_string()));
        }
    }
    let result = before(&promise, js_sys::Date::now() + TIMEOUT_MS).await.map(|_| ());
    if result.is_err() {
        let _ = tx.abort();
    }
    result
}
