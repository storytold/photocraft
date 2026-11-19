//! A stop (a pause at a dialog, the time limit) unwinds the whole attempt: no `catch` and no
//! `finally` of the script runs, wherever the stopping native was called from.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_script_js::{Host, HostError, RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

use crate::run::Countdown;

#[derive(Default)]
struct Lines(Rc<RefCell<Vec<String>>>);

impl Host for Lines {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, line: &str) {
        self.0.borrow_mut().push(line.to_owned());
    }
}

/// Runs `src` to its first pause, checks nothing was written, answers and runs it to the end.
fn pause_then_finish(src: &str) -> Vec<String> {
    let lines = Lines::default();
    let out = lines.0.clone();
    let mut r = Runner::new(src, Box::new(lines), Box::new(Unlimited), RunOptions::default()).unwrap();
    let step = r.step();
    assert!(matches!(step, Step::Paused(_)), "{src}: {step:?}");
    assert!(out.borrow().is_empty(), "{src}: catch or finally ran at the pause: {:?}", out.borrow());
    r.answer(Value::Null).unwrap();
    assert_eq!(r.step(), Step::Done, "{src}");
    out.borrow().clone()
}

const GUARDED: &str = "try { alert('x'); } catch (e) { $.writeln('caught'); } finally { $.writeln('finally'); }";

#[test]
fn a_pause_skips_catch_and_finally_in_plain_code() {
    assert_eq!(pause_then_finish(GUARDED), ["finally"]);
}

#[test]
fn a_pause_skips_catch_and_finally_inside_a_getter() {
    let src = format!("var o = {{ get v() {{ {GUARDED} return 1; }} }}; try {{ o.v; }} catch (e) {{ $.writeln('outer'); }}");
    assert_eq!(pause_then_finish(&src), ["finally"]);
}

#[test]
fn a_pause_skips_catch_and_finally_inside_builtin_callbacks() {
    let map = format!("try {{ [1].map(function () {{ {GUARDED} }}); }} catch (e) {{ $.writeln('outer'); }}");
    assert_eq!(pause_then_finish(&map), ["finally"]);
    let json = format!("try {{ JSON.stringify({{ toJSON: function () {{ {GUARDED} return 1; }} }}); }} catch (e) {{ $.writeln('outer'); }}");
    assert_eq!(pause_then_finish(&json), ["finally"]);
}

#[test]
fn a_pause_skips_catch_and_finally_in_promise_jobs_and_async_functions() {
    let job = format!("Promise.resolve().then(function () {{ {GUARDED} }}).catch(function () {{ $.writeln('rejected'); }});");
    assert_eq!(pause_then_finish(&job), ["finally"]);
    let func = format!("(async function () {{ await 0; {GUARDED} }})().catch(function () {{ $.writeln('rejected'); }});");
    assert_eq!(pause_then_finish(&func), ["finally"]);
}

#[test]
fn the_time_limit_skips_finally_and_cannot_be_retried_from_catch() {
    let lines = Lines::default();
    let out = lines.0.clone();
    // Each `finally` would write a line after the stop, and the `catch` would loop forever.
    let src = "for (;;) { try { Math.random(); } catch (e) { for (;;) {} } finally { $.writeln('f'); } }";
    // Counted, not timed: the loop runs a few times on any machine before the limit.
    let mut r = Runner::new(src, Box::new(lines), Box::new(Countdown(64)), RunOptions::default()).unwrap();
    let Step::Failed(m) = r.step() else { panic!("stopped") };
    assert!(m.contains("time limit"), "{m}");
    let written = out.borrow().len();
    assert!(written > 0, "the loop ran");
    assert_eq!(r.step(), Step::Failed(m), "a finished run stays finished");
    assert_eq!(out.borrow().len(), written);
}

#[test]
fn a_time_limit_stop_inside_an_async_function_after_await_is_not_lost() {
    // After an `await` the body runs as a promise job; its stop must still end the run.
    let src = "(async function () { await 0; try { for (;;) { Math.random(); } } finally { $.writeln('f'); } })(); $.writeln('sync done');";
    let lines = Lines::default();
    let out = lines.0.clone();
    // Counted, not timed: the synchronous part always finishes, however slow the machine, and the
    // job's loop then spends the rest.
    let mut r = Runner::new(src, Box::new(lines), Box::new(Countdown(64)), RunOptions::default()).unwrap();
    let Step::Failed(m) = r.step() else { panic!("the stop in the job ended the run") };
    assert!(m.contains("time limit"), "{m}");
    assert_eq!(*out.borrow(), ["sync done"], "the finally after the stop did not run");
}
