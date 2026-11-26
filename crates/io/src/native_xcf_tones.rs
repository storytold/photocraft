//! Explicit per-request authority for optional XCF native tone preparation.
//!
//! Ordinary IO imports keep source effects inert. An engine import entrypoint
//! may enter this scope for an admitted import request after capturing its
//! policy; source acquisition retains the entrypoint's existing checks. The callback is consulted for every native command
//! before any stage is evaluated; nothing in source data can install a scope.
use serde_json::Value;
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizationError {
    #[error("native XCF tone command is not authorized")]
    Denied,
    #[error("cancelled")]
    Cancelled,
}
pub type Authorize = fn(&str, &Value) -> Result<(), AuthorizationError>;

thread_local! {
    static AUTHORITY: Cell<Option<Authorize>> = const { Cell::new(None) };
}

/// Restores exactly the preceding request on drop/unwind. A scope cannot move
/// to another thread; a worker must explicitly enter its own admitted request.
pub struct ImportScope {
    previous: Option<Authorize>,
    _not_send: PhantomData<Rc<()>>,
}
impl Drop for ImportScope {
    fn drop(&mut self) {
        AUTHORITY.with(|current| current.set(self.previous));
    }
}

/// Enter one explicit authorized import. This does not authorize source
/// acquisition, other importers, ambient filesystem reads or future requests.
pub fn enter_import(authorize: Authorize) -> ImportScope {
    ImportScope { previous: AUTHORITY.with(|current| current.replace(Some(authorize))), _not_send: PhantomData }
}

pub(crate) fn current() -> Option<Authorize> {
    AUTHORITY.with(Cell::get)
}

#[cfg(any(test, feature = "testgen"))]
thread_local! { static PREPARATIONS: Cell<usize> = const { Cell::new(0) }; }
#[cfg(any(test, feature = "testgen"))]
pub(crate) fn note_preparation() {
    PREPARATIONS.with(|count| count.set(count.get().saturating_add(1)));
}
/// Test support for proving denied/default imports never enter materialization.
#[cfg(any(test, feature = "testgen"))]
pub fn preparation_count() -> usize {
    PREPARATIONS.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn allow(_: &str, _: &Value) -> Result<(), AuthorizationError> {
        Ok(())
    }
    fn deny(_: &str, _: &Value) -> Result<(), AuthorizationError> {
        Err(AuthorizationError::Denied)
    }
    #[test]
    fn explicit_scope_restores_nested_policy_and_never_enters_a_worker_implicitly() {
        assert!(current().is_none());
        let outer = enter_import(deny);
        assert_eq!(current().unwrap()("tone", &Value::Null), Err(AuthorizationError::Denied));
        {
            let _inner = enter_import(allow);
            assert!(current().unwrap()("tone", &Value::Null).is_ok());
            #[cfg(not(target_arch = "wasm32"))]
            assert!(std::thread::spawn(|| current().is_none()).join().unwrap());
        }
        assert_eq!(current().unwrap()("tone", &Value::Null), Err(AuthorizationError::Denied));
        drop(outer);
        assert!(current().is_none());
    }
}

/// Small independent sources for native import policy regressions.
#[cfg(any(test, feature = "testgen"))]
pub mod fixtures;
