//! Migration-only transaction liveness while immutable Blob bytes are compared.
//! Blob futures publish a decision; only an active IDB request callback queues mutations.
use std::{cell::RefCell, rc::Rc};

use super::*;

type Callback = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

#[derive(Default)]
struct Decision {
    ready: Option<Result<bool, String>>,
    stopped: bool,
    error: Option<String>,
}

pub(super) struct Scope {
    decision: Rc<RefCell<Decision>>,
    current: Rc<RefCell<Option<IdbRequest>>>,
    callback: Callback,
}

impl Scope {
    pub(super) fn error(&self) -> Option<String> {
        self.decision.borrow().error.clone()
    }
}

impl Drop for Scope {
    /// Stop the migration decision and detach its live request callback so late Blob reads cannot write.
    fn drop(&mut self) {
        self.decision.borrow_mut().stopped = true;
        if let Some(request) = self.current.borrow_mut().take() {
            request.set_onsuccess(None);
        }
        self.callback.borrow_mut().take();
    }
}

/// Replace the single liveness request and detach its predecessor's success handler.
fn next(store: &web_sys::IdbObjectStore, current: &Rc<RefCell<Option<IdbRequest>>>, callback: &Callback) -> Result<(), String> {
    let request = store.count().map_err(js)?;
    let borrowed = callback.borrow();
    let handler = borrowed.as_ref().ok_or("migration callback was released")?;
    request.set_onsuccess(Some(handler.as_ref().unchecked_ref()));
    if let Some(previous) = current.borrow_mut().replace(request) {
        previous.set_onsuccess(None);
    }
    Ok(())
}

/// Compare bounded Blob bytes before the shared deadline; only undefined means absent.
async fn bytes(value: JsValue, deadline: f64) -> Result<Option<Vec<u8>>, String> {
    if value.is_undefined() {
        return Ok(None);
    }
    let blob = value.dyn_into::<Blob>().map_err(|_| "migration record is not a Blob")?;
    if blob.size() > photocraft_engine::preset_store::MAX_ACTIONS_BYTES as f64 {
        return Err("migration record exceeds the Actions file limit".into());
    }
    let buffer = before(&blob.array_buffer(), deadline).await?;
    Ok(Some(Uint8Array::new(&buffer).to_vec()))
}

/// Compare the exact migration tuple inside one live read/write transaction.
/// Blob futures publish decisions; active request callbacks alone queue mutations.
/// Dropping the scope detaches the callback so late Blob results cannot write.
pub(super) fn start(tx: &web_sys::IdbTransaction, store: &web_sys::IdbObjectStore, batch: &Batch, deadline: f64) -> Result<Scope, String> {
    let migration = batch.migration.clone().ok_or("missing migration identity")?;
    let mut reads = Vec::new();
    let promises = Array::new();
    for name in [ACTIONS_FILE, LEGACY_ACTIONS_FILE, MIGRATED_ACTIONS_FILE] {
        let pending = request(&store.get(&JsValue::from_str(name)).map_err(js)?);
        promises.push(&pending.promise);
        reads.push(pending);
    }
    let decision = Rc::new(RefCell::new(Decision::default()));
    let current = Rc::new(RefCell::new(None));
    let callback: Callback = Rc::new(RefCell::new(None));
    let weak = Rc::downgrade(&callback);
    let state = decision.clone();
    let active = current.clone();
    let transaction = tx.clone();
    let files = store.clone();
    let owned = batch.clone();
    *callback.borrow_mut() = Some(Closure::new(move |_: Event| {
        let result = {
            let mut state = state.borrow_mut();
            if state.stopped {
                return;
            }
            if js_sys::Date::now() >= deadline { Some(Err("browser preset storage timed out".into())) } else { state.ready.take() }
        };
        let result = match result {
            Some(result) => {
                // This callback is an active IDB request event. No Blob future issues writes.
                state.borrow_mut().stopped = true;
                result.and_then(|committed| queue_batch(&files, &owned, committed))
            }
            None => weak.upgrade().ok_or_else(|| "migration callback was released".to_string()).and_then(|callback| next(&files, &active, &callback)),
        };
        if let Err(error) = result {
            let mut state = state.borrow_mut();
            state.stopped = true;
            state.error = Some(error);
            let _ = transaction.abort();
        }
    }));
    let scope = Scope { decision: decision.clone(), current, callback };
    next(store, &scope.current, &scope.callback)?;
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let values = before(&Promise::all(&promises), deadline).await?;
            let values = Array::from(&values);
            let atn = bytes(values.get(0), deadline).await?;
            let legacy = bytes(values.get(1), deadline).await?;
            let backup = bytes(values.get(2), deadline).await?;
            migration.already_committed(atn.as_deref(), legacy.as_deref(), backup.as_deref())
        }
        .await;
        // A timeout or abort may have finished the attempt. Late reads have no IDB effects.
        let mut state = decision.borrow_mut();
        if !state.stopped {
            state.ready = Some(result);
        }
        drop(reads);
    });
    Ok(scope)
}
