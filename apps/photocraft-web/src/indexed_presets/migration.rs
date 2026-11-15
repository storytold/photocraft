//! Migration-only transaction liveness while immutable Blob bytes are compared.
//! Blob futures publish a decision; only an active IDB request callback queues mutations.
use std::{cell::RefCell, rc::Rc};

use super::{
    ACTIONS_FILE, ActionWrite, Array, Closure, Event, IdbRequest, JsCast, JsValue, LEGACY_ACTIONS_FILE, MIGRATED_ACTIONS_FILE, Promise, WriteError,
    action_record, before, js, queue_migration, request,
};

type Callback = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

#[derive(Default)]
struct Decision {
    ready: Option<Result<(bool, JsValue), WriteError>>,
    stopped: bool,
    error: Option<WriteError>,
}

pub(super) struct Scope {
    decision: Rc<RefCell<Decision>>,
    current: Rc<RefCell<Option<IdbRequest>>>,
    callback: Callback,
}

impl Scope {
    pub(super) fn error(&self) -> Option<WriteError> {
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

/// Compare the exact migration tuple inside one live read/write transaction.
/// Blob futures publish decisions; active request callbacks alone queue mutations.
/// Dropping the scope detaches the callback so late Blob results cannot write.
pub(super) fn start(tx: &web_sys::IdbTransaction, store: &web_sys::IdbObjectStore, action: &ActionWrite, deadline: f64) -> Result<Scope, String> {
    let migration = action.migration.clone().ok_or("missing migration identity")?;
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
    let owned = action.clone();
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
                result.and_then(|(committed, legacy)| queue_migration(&files, &owned, &legacy, committed))
            }
            None => weak
                .upgrade()
                .ok_or_else(|| "migration callback was released".to_string())
                .and_then(|callback| next(&files, &active, &callback))
                .map_err(WriteError::Aborted),
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
    let action = action.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let values = before(&Promise::all(&promises), deadline).await?;
            let values = Array::from(&values);
            let atn = action_record(values.get(0), deadline).await?;
            let legacy = action_record(values.get(1), deadline).await?;
            let backup = action_record(values.get(2), deadline).await?;
            let committed = migration.already_committed(
                atn.as_ref().and_then(|record| record.definitions.as_deref()),
                legacy.as_ref().and_then(|record| record.definitions.as_deref()),
                backup.as_ref().and_then(|record| record.definitions.as_deref()),
            )?;
            migration.validate_pairs(
                &action,
                committed,
                atn.as_ref().map(|record| (record.revision, record.keys.clone())),
                legacy.as_ref().and_then(|record| record.pair()),
                backup.as_ref().and_then(|record| record.pair()),
            )?;
            Ok((committed, values.get(1)))
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
