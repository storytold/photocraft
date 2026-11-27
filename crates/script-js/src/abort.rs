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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use boa_engine::{Context, JsResult, JsValue, NativeFunction, Source, js_string};

    use super::{EngineError, RuntimeLimitError, uncatchable};

    thread_local! {
        /// The handlers entered, recorded by the VM itself: no host and no stop checkpoint in
        /// between that could hide one.
        static ENTERED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn stop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
        Err(uncatchable())
    }

    fn mark(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let what = args.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
        ENTERED.with(|e| e.borrow_mut().push(what));
        Ok(JsValue::undefined())
    }

    /// The engine itself enters no `catch` or `finally` on the way out of an uncatchable error,
    /// in plain code, a getter or a builtin's callback.
    #[test]
    fn the_vm_enters_no_handler_on_an_uncatchable_error() {
        let guarded = "try { stop(); } catch (e) { mark('catch'); } finally { mark('finally'); }";
        for src in [
            guarded.to_owned(),
            format!("var o = {{ get v() {{ {guarded} return 1; }} }}; try {{ o.v; }} catch (e) {{ mark('outer'); }}"),
            format!("try {{ [1].map(function () {{ {guarded} }}); }} catch (e) {{ mark('outer'); }} finally {{ mark('outer finally'); }}"),
        ] {
            ENTERED.with(|e| e.borrow_mut().clear());
            let mut ctx = Context::default();
            ctx.register_global_callable(js_string!("stop"), 0, NativeFunction::from_fn_ptr(stop)).unwrap();
            ctx.register_global_callable(js_string!("mark"), 1, NativeFunction::from_fn_ptr(mark)).unwrap();
            // The stop itself, not an error before it (a parse or setup error enters no handler either).
            let error = ctx.eval(Source::from_bytes(src.as_str())).expect_err(&src);
            assert_eq!(error.as_engine(), Some(&EngineError::RuntimeLimit(RuntimeLimitError::LoopIteration)), "{src}: {error}");
            ENTERED.with(|e| assert!(e.borrow().is_empty(), "{src}: entered {:?}", e.borrow()));
        }
        // A catchable error enters both, so the marks do record handlers.
        let mut ctx = Context::default();
        ctx.register_global_callable(js_string!("mark"), 1, NativeFunction::from_fn_ptr(mark)).unwrap();
        ENTERED.with(|e| e.borrow_mut().clear());
        ctx.eval(Source::from_bytes("try { throw 1; } catch (e) { mark('catch'); } finally { mark('finally'); }")).unwrap();
        ENTERED.with(|e| assert_eq!(*e.borrow(), ["catch", "finally"]));
    }
}
