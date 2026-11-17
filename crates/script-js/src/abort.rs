//! Ending an attempt with an error the script cannot catch.
//!
//! Boa 0.22 has no interrupt hook, but it treats every engine error as uncatchable: when a
//! native returns one, the VM unwinds all script frames up to the Rust caller without running
//! `catch` or `finally`, also when the native was reached through a getter, a callback of a
//! builtin (`Array.prototype.map`, `JSON.stringify`) or a promise job. A stop (a pause at a
//! dialog, the time limit, a host fault) therefore returns a runtime-limit engine error from the
//! native that decided to stop. The reason is kept in the run state; the error itself carries no
//! meaning.
//!
//! An attempt that ended this way leaves its context mid-unwind, so a context is never reused
//! after one: every attempt starts in a fresh context.

use boa_engine::JsError;
use boa_engine::error::{EngineError, RuntimeLimitError};

/// An uncatchable error that unwinds the whole attempt.
pub fn uncatchable() -> JsError {
    EngineError::RuntimeLimit(RuntimeLimitError::LoopIteration).into()
}
