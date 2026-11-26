//! End-to-end runs through the public API.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::ManualClock;
use crate::session_host::SessionHost;
use photocraft_engine::Session;
use photocraft_script_js::{Budget, Host, HostError, Modal, ModalKind, RunOptions, Runner, Unlimited, WallClock};
use serde_json::{Value, json};

type Log = Rc<RefCell<Vec<String>>>;

/// Counts commands, keeps output and the dialogs shown, answering every command with an empty
/// session (or `HostError("refused")` for the `refuse` id) and each dialog with the next of
/// `answers` (`null` once they run out).
#[derive(Default)]
struct Probe {
    calls: Log,
    lines: Log,
    dialogs: Rc<RefCell<Vec<Modal>>>,
    answers: VecDeque<Value>,
    refuse: Option<&'static str>,
}

impl Host for Probe {
    fn execute(&mut self, id: &str, _: Value) -> Result<Value, HostError> {
        self.calls.borrow_mut().push(id.to_owned());
        if self.refuse == Some(id) { Err(HostError("refused".into())) } else { Ok(json!({"active": null, "documents": []})) }
    }

    fn write_line(&mut self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }

    fn modal(&mut self, kind: ModalKind, args: &Value) -> Result<Value, HostError> {
        self.dialogs.borrow_mut().push(Modal { kind, args: args.clone() });
        Ok(self.answers.pop_front().unwrap_or(Value::Null))
    }
}

/// What a run did, as its host saw it.
struct Seen {
    result: Result<(), String>,
    calls: Vec<String>,
    lines: Vec<String>,
    dialogs: Vec<Modal>,
}

fn run_probe(src: &str, probe: Probe, budget: Box<dyn Budget>, options: RunOptions) -> Seen {
    let (calls, lines, dialogs) = (probe.calls.clone(), probe.lines.clone(), probe.dialogs.clone());
    let result = Runner::new(src, Box::new(probe), budget, options).unwrap().run();
    let (calls, lines, dialogs) = (calls.borrow().clone(), lines.borrow().clone(), dialogs.borrow().clone());
    Seen { result, calls, lines, dialogs }
}

/// `src` with every dialog answered from `answers` in turn.
fn answering(src: &str, answers: &[Value]) -> Seen {
    run_probe(src, Probe { answers: answers.iter().cloned().collect(), ..Probe::default() }, Box::new(Unlimited), RunOptions::default())
}

fn lines_of(src: &str) -> Vec<String> {
    let seen = answering(src, &[]);
    assert_eq!(seen.result, Ok(()), "{src}");
    seen.lines
}

#[test]
fn app_name_defaults_and_takes_the_run_override() {
    assert_eq!(lines_of("$.writeln(app.name, ' ', app.version)"), ["PhotoCraft 15.0.0"]);
    let options = RunOptions { app_name: Some("Tagged".into()), ..RunOptions::default() };
    let seen = run_probe("$.writeln(app.name)", Probe::default(), Box::new(Unlimited), options);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["Tagged"]);
}

#[test]
fn app_version_is_a_three_part_compatibility_level() {
    let src = "$.writeln(typeof app.version, ' ', parseInt(app.version) === 15, ' ', app.version.split('.').length === 3);";
    assert_eq!(lines_of(src), ["string true true"]);
}

#[test]
fn dialogs_wait_for_their_answers_in_order() {
    let src = "$.writeln('before'); var ok = confirm('Go?'); var n = prompt('Name', 'x'); alert('hi'); $.writeln(ok, ' ', n);";
    let seen = answering(src, &[json!(true), json!("Ann"), Value::Null]);
    assert_eq!(seen.result, Ok(()));
    let shown: Vec<(ModalKind, Value)> = seen.dialogs.into_iter().map(|m| (m.kind, m.args)).collect();
    assert_eq!(shown, [(ModalKind::Confirm, json!(["Go?"])), (ModalKind::Prompt, json!(["Name", "x"])), (ModalKind::Alert, json!(["hi"]))]);
    assert_eq!(seen.lines, ["before", "true Ann"]);
}

#[test]
fn prompt_cancel_is_null() {
    let seen = answering("$.writeln(prompt('q') === null)", &[Value::Null]);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["true"]);
}

#[test]
fn host_errors_are_catchable() {
    let probe = Probe { refuse: Some("session.inspect"), ..Probe::default() };
    let src = "var m = 'no error'; try { app.documents } catch (e) { m = (e instanceof Error) + ' ' + e.message } $.writeln(m);";
    let seen = run_probe(src, probe, Box::new(Unlimited), RunOptions::default());
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["true refused"]);
    assert_eq!(seen.calls, ["session.inspect"]);
    assert_eq!(lines_of("try { $.writeln('\\uD800') } catch (e) { $.writeln(e instanceof TypeError) }"), ["true"]);
}

#[test]
fn the_time_limit_stops_a_loop_that_catches_errors() {
    // Finite, so a stop the script could catch fails the test instead of hanging it.
    let src = "for (var i = 0; i < 1000; i++) { try { $.writeln('x') } catch (e) {} }";
    // A counted budget, not a clock: room for a few writes whatever the machine's speed.
    let seen = run_probe(src, Probe::default(), Box::new(Countdown(8)), RunOptions::default());
    assert!(seen.result.as_ref().is_err_and(|m| m.contains("time limit")), "{:?}", seen.result);
    assert!(!seen.lines.is_empty());
}

#[test]
fn unsupported_include_is_reported_up_front() {
    let err = Runner::new("#include \"a.jsx\"", Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err();
    assert_eq!(err.as_deref(), Some("line 1: #include is not supported yet"));
}

#[test]
fn script_errors_fail_the_run() {
    assert!(answering("null.x", &[]).result.is_err());
}

#[test]
fn script_errors_name_the_file_and_a_position() {
    let m = failure_of("null.x");
    assert!(m.contains("script:1:"), "{m}");
    assert!(!m.contains("unknown at"), "{m}");
    let options = RunOptions { script_name: Some("export.jsx".into()), ..RunOptions::default() };
    let Err(m) = run_probe("null.x", Probe::default(), Box::new(Unlimited), options).result else { panic!("failed") };
    assert!(m.contains("export.jsx:1:"), "{m}");
}

fn session_with_two_documents() -> Rc<RefCell<Session>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "name": "One"})).unwrap();
    s.execute("file.new", json!({"width": 20, "height": 10, "name": "Two"})).unwrap();
    Rc::new(RefCell::new(s))
}

#[test]
fn documents_and_active_document_against_a_real_session() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s.clone());
    let out = host.output();
    let src = "var d = app.documents; $.writeln(d.length, ' ', d[0].name, ' ', d[0].resolution, ' ', d[0].saved);
               $.writeln(app.activeDocument.name); app.activeDocument = d[0]; $.writeln(app.activeDocument.name);";
    let r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()));
    assert_eq!(*out.borrow(), ["2 One 72 false", "Two", "One"]);
    assert_eq!(s.borrow().active_index(), Some(0));
}

#[test]
fn the_same_document_is_the_same_object() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s);
    let out = host.output();
    let src = "$.writeln(app.activeDocument == app.documents[1]);
               $.writeln(app.documents[0] == app.documents[0]);
               $.writeln(app.documents[0] == app.documents[1]);
               var a = app.documents[0];
               $.writeln(a == app.documents[0]);";
    let r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()));
    assert_eq!(*out.borrow(), ["true", "true", "false", "true"]);
}

#[test]
fn a_planted_global_cannot_replace_document_objects() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s);
    let out = host.output();
    let src = "var before = Object.getOwnPropertyNames(globalThis).length;
               app.documents[0].name; $.writeln(Object.getOwnPropertyNames(globalThis).length - before);
               before = Object.getOwnPropertyNames(globalThis).length;
               var fake = new Proxy({}, { get: function () { return { name: 'fake' }; } });
               globalThis['\\u200bdocuments'] = fake; Object.defineProperty(globalThis, 'documents', { value: fake });
               $.writeln(app.activeDocument.name, ' ', app.documents[0].name, ' ', app.documents[0] == app.documents[0]);
               $.writeln(Object.getOwnPropertyNames(globalThis).length - before);";
    let r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()), "{}", out.borrow().join(" | "));
    // `documents` is a global already (`app.documents` named bare): planted over, it adds no name.
    assert_eq!(*out.borrow(), ["0", "Two One true", "1"]);
}

#[test]
fn a_new_document_is_not_saved() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s);
    let out = host.output();
    let src = "$.writeln(app.documents[0].saved, ' ', app.documents[1].saved, ' ', app.activeDocument.saved);";
    let r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()));
    assert_eq!(*out.borrow(), ["false false false"]);
}

#[test]
fn alert_of_a_document_shows_its_name() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s);
    let (out, dialogs) = (host.output(), host.dialogs());
    let src = "$.writeln(String(app.activeDocument)); alert(app.activeDocument);";
    assert_eq!(Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap().run(), Ok(()));
    assert_eq!(*dialogs.borrow(), [json!(["Two"])]);
    assert_eq!(*out.borrow(), ["Two"]);
}

#[test]
fn time_at_a_dialog_does_not_count_against_the_limit() {
    let (clock, lines) = (Arc::new(ManualClock::default()), Log::default());
    let host = SlowDialogs { clock: clock.clone(), wait: Duration::from_millis(400), lines: lines.clone() };
    // The clock moves only at the dialog, so the run is charged nothing else: a charged dialog
    // wait alone would be past the limit.
    let budget = WallClock::with_clock(Duration::from_millis(200), clock);
    let r = Runner::new("alert('a'); $.writeln('after');", Box::new(host), Box::new(budget), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()));
    assert_eq!(*lines.borrow(), ["after"]);
}

#[test]
fn fifty_alerts_fit_under_a_time_limit() {
    let seen = run_probe("for (var i = 0; i < 50; i++) { alert(i); }", Probe::default(), Box::new(Countdown(10_000)), RunOptions::default());
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs.len(), 50);
}

#[test]
fn a_document_handle_follows_its_document_across_a_dialog() {
    let s = session_with_two_documents();
    // While the dialog is open the user moves the tabs.
    let host = SessionHost::new(s.clone()).at_dialog(|s| drop(s.execute("document.move", json!({"document": 0, "to": 1})).unwrap()));
    let out = host.output();
    let src = "var one = app.documents[0], two = app.documents[1]; alert('wait');
               $.writeln(one.name, ' ', two.name); app.activeDocument = one; $.writeln(app.activeDocument.name);";
    assert_eq!(Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap().run(), Ok(()));
    assert_eq!(*out.borrow(), ["One Two", "One"]);
    assert_eq!(s.borrow().documents()[s.borrow().active_index().unwrap()].doc.name, "One");
}

#[test]
fn a_handle_to_a_closed_document_is_a_script_error() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s.clone()).at_dialog(|s| drop(s.close(0).unwrap()));
    let out = host.output();
    let src = "var one = app.documents[0], before = app.activeDocument.name; alert('wait');
               try { $.writeln(one.name) } catch (e) { $.writeln(e.message) }
               try { $.writeln(one.saved) } catch (e) { $.writeln(e.message) }
               try { app.activeDocument = one; $.writeln('activated') } catch (e) { $.writeln(e.message, ' ', app.activeDocument.name, ' ', app.activeDocument.name === before) }";
    assert_eq!(Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap().run(), Ok(()));
    assert_eq!(*out.borrow(), ["the document is no longer open", "the document is no longer open", "Object expected Two true"]);
    assert_eq!(s.borrow().documents()[s.borrow().active_index().unwrap()].doc.name, "Two");
}

#[test]
fn reading_active_document_with_none_open_is_no_such_element() {
    let src = "try {
                 try { app.activeDocument; $.writeln('opened') }
                 catch (e) {
                   $.writeln((e instanceof Error) + ' ' + e.message + ' ' + e.number + ' ' + (e.number === 1302));
                   throw e;
                 }
               } catch (e2) {
                 $.writeln((e2 instanceof Error) + ' ' + e2.message + ' ' + e2.number + ' ' + (e2.number === 1302));
               }";
    assert_eq!(lines_of(src), ["true No such element 1302 true", "true No such element 1302 true"]);
}

/// The error's `number` is its own: a setter a script put on `Error.prototype` neither swallows
/// it nor replaces the error.
#[test]
fn an_error_number_is_its_own_property() {
    let src = "Object.defineProperty(Error.prototype, 'number', { set: function (v) { throw new TypeError('setter') }, get: function () { return 7 }, configurable: true });
               try { app.activeDocument } catch (e) { $.writeln(e.message + ' ' + e.number + ' ' + e.hasOwnProperty('number')) }";
    assert_eq!(lines_of(src), ["No such element 1302 true"]);
}

#[test]
fn setting_active_document_to_a_non_document_is_object_expected() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s.clone());
    let out = host.output();
    let src = "var before = app.activeDocument.name;
               var one = app.documents[0];
               var proto = Object.getPrototypeOf(one);
               function assign(v, label) {
                 try { app.activeDocument = v; $.writeln(label, ' activated ', app.activeDocument.name) }
                 catch (e) { $.writeln(label, ' ', e.message, ' ', app.activeDocument.name === before) }
               }
               assign(null, 'null');
               assign(undefined, 'undef');
               assign({}, 'obj');
               assign(new Proxy({}, {}), 'proxy');
               assign(new Proxy(one, {}), 'proxyDoc');
               assign(Object.create(proto), 'create');
               $.writeln(app.activeDocument.name);";
    let r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Ok(()));
    assert_eq!(
        *out.borrow(),
        [
            "null Object expected true",
            "undef Object expected true",
            "obj Object expected true",
            "proxy Object expected true",
            "proxyDoc Object expected true",
            "create Object expected true",
            "Two"
        ]
    );
    assert_eq!(s.borrow().documents()[s.borrow().active_index().unwrap()].doc.name, "Two");
}

/// Operators are the engine's own: the script text is run as written, with nothing rewritten.
#[test]
fn operators_run_unchanged_on_the_script_text() {
    let src = "var o = { k3: 'x' }, i = 3, n = 0, length = 4, a = [10, 20, 30, 40], b = [0, 2, 0];
               $.writeln(o['k' + i], ' ', a[length-2], ' ', a[b[n+1]-1]);
               var v = 1; var arr = [v + 1]; $.writeln(arr[0]);
               var j = 1, c = [0, 0, 0]; c[j++] += 5; $.writeln(j, ' ', c[1]);
               var calls = 0, box = { v: 1 }; function f() { calls++; return box; } f().v += 1; $.writeln(calls, ' ', box.v);
               var d = [0, 0, 0]; d[j-1] = 7; d[j+0] -= 1; $.writeln(d.join(','));
               $.writeln(String(-5n), ' ', String(~5n));
               function g(x) { return x + 1; } $.writeln(g.toString());";
    assert_eq!(lines_of(src), ["x 30 20", "2", "2 5", "1 2", "0,7,-1", "-5 -6", "function g(x) { return x + 1; }"]);
}

#[test]
fn long_operator_chains_and_many_lines_finish_quickly() {
    let chain = format!("var s = 1{}; $.writeln(s);", " + 1".repeat(4999));
    let lines = format!("var x = 0;\n{}$.writeln(x);", "x = x + 1;\n".repeat(10_000));
    let started = std::time::Instant::now();
    let out = on_script_stack(move || {
        let mut out = lines_of(&chain);
        out.extend(lines_of(&lines));
        out
    });
    assert_eq!(out, ["5000", "10000"]);
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

/// After an `await` the body runs as a promise job; a time-limit stop there still ends the run
/// with its reason, not as a success.
#[test]
fn a_time_limit_stop_after_an_await_ends_the_run_by_name() {
    // Finite, so a job that stopped charging the budget fails the test instead of hanging it.
    let src = "(async function () { await 0; for (var i = 0; i < 1000; i++) { app.documents.length; } })(); $.writeln('sync done');";
    // Counted, not timed: the synchronous part always finishes, and the job's loop spends the rest.
    let seen = run_probe(src, Probe::default(), Box::new(Countdown(64)), RunOptions::default());
    assert!(seen.result.as_ref().is_err_and(|m| m.contains("time limit")), "{:?}", seen.result);
    assert_eq!(seen.lines, ["sync done"]);
}

/// Allows `n` charges, then reports the time limit: a time limit that doesn't depend on how fast
/// the machine is.
struct Countdown(u32);

impl Budget for Countdown {
    fn charge(&mut self) -> bool {
        self.0.checked_sub(1).map(|n| self.0 = n).is_some()
    }
}

/// The first argument of the first dialog the script opens.
fn first_dialog_text(src: &str) -> String {
    let seen = answering(src, &[]);
    let Some(m) = seen.dialogs.first() else { panic!("no dialog: {src} {:?}", seen.result) };
    assert_eq!(seen.result, Ok(()), "{src}");
    m.args[0].as_str().unwrap().to_owned()
}

#[test]
fn dialog_arguments_are_shown_as_text() {
    for (arg, shown) in [
        ("'hi'", "hi"),
        ("function f() { return 1; }", "function f() { return 1; }"),
        ("{ toString: function () { return 'T'; } }", "T"),
        ("{ toJSON: function () { return 1; } }", "[object Object]"),
        ("{ get a() { return 1; } }", "[object Object]"),
        ("new Proxy({}, {})", "[object Object]"),
        ("[1, [2, 3], null, undefined]", "1,2,3,,"),
        ("NaN", "NaN"),
        ("-Infinity", "-Infinity"),
        ("-0", "0"),
        ("10n", "10"),
        ("undefined", "undefined"),
        ("null", "null"),
        ("true", "true"),
        ("new Error('boom')", "Error: boom"),
    ] {
        assert_eq!(first_dialog_text(&format!("alert({arg});")), shown, "{arg}");
    }
    // A date shows like String(date), in the local time zone.
    let seen = answering("$.writeln(String(new Date(0))); alert(new Date(0));", &[]);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs[0].args, json!([seen.lines[0]]));
    // Every argument is text.
    let seen = answering("confirm('Sure?', true); ", &[]);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs[0].args, json!(["Sure?", "true"]));
}

#[test]
fn a_failing_conversion_is_catchable_at_the_dialog() {
    let src = "try { alert({ toString: function () { throw new Error('boom'); } }); } catch (e) { $.writeln(e.message); }
               try { confirm({ get toString() { throw new Error('getter'); } }); } catch (e) { $.writeln(e.message); }
               try { prompt(Symbol('s')); } catch (e) { $.writeln(e instanceof TypeError); }";
    let seen = answering(src, &[]);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["boom", "getter", "true"]);
    assert!(seen.calls.is_empty() && seen.dialogs.is_empty());
}

#[test]
fn conversion_code_runs_once_per_call() {
    // toString writes and opens a dialog of its own.
    let src = "var n = 0;
               var o = { toString: function () { n++; $.writeln('conv ' + n); return 'msg ' + n; } };
               alert(o); confirm(o);
               alert({ toString: function () { return 'outer ' + confirm('inner'); } });
               $.writeln('done');";
    let seen = answering(src, &[Value::Null, json!(true), json!(true), Value::Null]);
    assert_eq!(seen.result, Ok(()));
    let shown: Vec<&str> = seen.dialogs.iter().map(|m| m.args[0].as_str().unwrap()).collect();
    assert_eq!(shown, ["msg 1", "msg 2", "inner", "outer true"]);
    assert_eq!(seen.lines, ["conv 1", "conv 2", "done"]);
}

/// Each of these used to overflow the native stack inside Array.prototype.join and abort.
#[test]
fn deep_arrays_are_a_named_error_when_converted_to_text() {
    for call in ["$.writeln(a)", "alert(a)", "confirm('x', a)"] {
        for depth in [20_000, 100_000] {
            let src = format!(
                "var a = []; for (var i = 0; i < {depth}; i++) {{ a = [a]; }}
                 try {{ {call}; }} catch (e) {{ $.writeln(e instanceof RangeError, ' ', e.message); }}"
            );
            assert_eq!(lines_of(&src), ["true value is nested too deeply to convert to text (more than 128 levels)"], "{call} {depth}");
        }
    }
    // A deep object has no nested text: it is shown as [object Object].
    assert_eq!(first_dialog_text("var o = {}; for (var i = 0; i < 100000; i++) { o = { a: o }; } alert(o);"), "[object Object]");
    // The limit is exact: 128 levels pass, 129 do not.
    assert_eq!(lines_of("var a = 1; for (var i = 0; i < 128; i++) { a = [a]; } $.writeln(a);"), ["1"]);
    let src = "var a = 1; for (var i = 0; i < 129; i++) { a = [a]; } try { $.writeln(a); } catch (e) { $.writeln(e instanceof RangeError); }";
    assert_eq!(lines_of(src), ["true"]);
    // The intrinsic toString or join as `Symbol.toPrimitive` is held to the same bound.
    for method in ["Array.prototype.toString", "Array.prototype.join"] {
        let src = format!(
            "var a = 1; for (var i = 0; i < 129; i++) {{ a = [a]; }} a[Symbol.toPrimitive] = {method};
             try {{ $.writeln(a); }} catch (e) {{ $.writeln(e instanceof RangeError, ' ', e.message); }}"
        );
        assert_eq!(lines_of(&src), ["true value is nested too deeply to convert to text (more than 128 levels)"], "{method}");
    }
    // Its text is the engine's: the intrinsic join takes the hint, `"string"`, as its separator.
    for method in ["Array.prototype.toString", "Array.prototype.join"] {
        let src = format!("var a = [1, [2, 3]]; a[Symbol.toPrimitive] = {method}; var engine = String(a); alert(a); $.writeln(engine);");
        let seen = answering(&src, &[]);
        assert_eq!(seen.result, Ok(()), "{method}");
        assert_eq!(seen.dialogs[0].args, json!([seen.lines[0]]), "{method}");
    }
}

#[test]
fn cyclic_arrays_stay_text_and_huge_arrays_are_named_errors() {
    // The engine joins an array met again inside itself as '', so cycles stay text.
    assert_eq!(lines_of("var a = [1]; a.push(a); $.writeln(a);"), ["1,"]);
    assert_eq!(lines_of("var a = [1], b = [a]; a.push(b); $.writeln(a);"), ["1,"]);
    // Shared items are joined once per path: 2^21 items from a few dozen arrays.
    let src = "var a = 1; for (var i = 0; i < 21; i++) { a = [a, a]; } try { alert(a); } catch (e) { $.writeln(e instanceof RangeError, ' ', e.message); }";
    assert_eq!(lines_of(src), ["true value is too large to convert to text (more than 1048576 items)"]);
    // A huge length would make the engine reserve memory for every item.
    let src = "var a = []; a.length = 4294967295; try { $.writeln(a); } catch (e) { $.writeln(e instanceof RangeError); }";
    assert_eq!(lines_of(src), ["true"]);
}

#[test]
fn a_non_index_array_property_is_not_a_cycle_or_a_size_error() {
    // A non-index like `a.extra` is not walked: String(a) of an empty array is empty, and the
    // self-reference on `extra` is not a cycle or a size error.
    assert_eq!(first_dialog_text("var a = []; a.extra = a; alert(a);"), "");
    // A shared object already printed on another branch is not a cycle.
    assert_eq!(first_dialog_text("var o = { n: 1 }; alert([o, o]);"), "[object Object],[object Object]");
    // A non-index property is not counted toward the value cap (2^20).
    assert_eq!(first_dialog_text("var a = []; var o = 1; for (var i = 0; i < 21; i++) { o = [o, o]; } a.big = o; alert(a);"), "");
    // An index that refers to the array itself is joined as empty text, not rejected as a cycle.
    assert_eq!(first_dialog_text("var a = []; a[0] = a; alert(a);"), "");
}

#[test]
fn arrays_reached_through_getters_cycles_holes_and_proxies_stay_within_the_bounds() {
    let caught = |src: &str| format!("try {{ {src} }} catch (e) {{ $.writeln(e instanceof RangeError, ' ', e.message); }}");
    let deep = "var d = []; for (var i = 0; i < 100000; i++) { d = [d]; }";
    let too_deep = ["true value is nested too deeply to convert to text (more than 128 levels)"];
    // An index getter returning a deep array: the getter runs once and its value is bounded.
    let src = format!(
        "{deep} var n = 0; var a = []; Object.defineProperty(a, '0', {{ get: function () {{ n++; return d; }} }}); a.length = 1; {} $.writeln(n);",
        caught("alert(a);")
    );
    assert_eq!(lines_of(&src), [too_deep[0], "1"]);
    // A getter on the prototype answering a hole.
    let src = format!(
        "{deep} Object.defineProperty(Array.prototype, '0', {{ get: function () {{ return d; }}, configurable: true }}); var a = [, 1]; {}",
        caught("alert(a);")
    );
    assert_eq!(lines_of(&src), too_deep);
    // A two-array cycle is joined as '' where it closes, like a direct one.
    assert_eq!(first_dialog_text("var a = [], b = [a]; a.push(b); alert(a);"), "");
    assert_eq!(first_dialog_text("var a = [1], b = [2, a]; a.push(b); alert([a, b]);"), "1,2,,2,1,");
    // Holes count as slots along every path: 1024 views of a 2^20-slot array are rejected at
    // once, not after a billion separators.
    let started = std::time::Instant::now();
    let src = caught("var a = Array(1 << 20); alert(Array(1024).fill(a));");
    assert_eq!(lines_of(&src), ["true value is too large to convert to text (more than 1048576 items)"]);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    // A proxy of a deep array converts through its traps, within the same bounds.
    let src = format!("{deep} {}", caught("alert(new Proxy(d, {}));"));
    assert_eq!(lines_of(&src), too_deep);
    // A proxy that hands out deep arrays as elements.
    let src =
        format!("{deep} var p = new Proxy([0], {{ get: function (t, k) {{ return k === 'length' ? 1 : k === '0' ? d : t[k]; }} }}); {}", caught("alert(p);"));
    assert_eq!(lines_of(&src), too_deep);
    // An array-like with the intrinsic conversion is joined the same way.
    assert_eq!(first_dialog_text("alert({ length: 2, 0: 'a', 1: ['b', 'c'], toString: Array.prototype.toString, join: Array.prototype.join });"), "a,b,c");
    // The intrinsic join as the conversion method itself is held to the same bounds.
    assert_eq!(first_dialog_text("alert({ length: 2, 0: 'a', 1: ['b', 'c'], toString: Array.prototype.join });"), "a,b,c");
    let src = format!("{deep} {}", caught("alert({ length: 1, 0: d, toString: Array.prototype.join });"));
    assert_eq!(lines_of(&src), too_deep);
    let src = format!("{deep} d.toString = Array.prototype.join; {}", caught("$.writeln(d);"));
    assert_eq!(lines_of(&src), too_deep);
    // Without a `join`, the intrinsic toString falls back to Object.prototype.toString, per spec.
    assert_eq!(first_dialog_text("alert({ length: 1, 0: 'a', toString: Array.prototype.toString });"), "[object Object]");
    // A replaced Array.prototype.toString or join is the script's own code and runs as such.
    assert_eq!(first_dialog_text("Array.prototype.join = function () { return 'mine'; }; alert([1, [2]]);"), "mine");
    assert_eq!(first_dialog_text("var a = [1, 2]; a.toString = function () { return 'own'; }; alert(a);"), "own");
}

#[test]
fn an_error_whose_name_hides_behind_a_long_prototype_chain_has_a_fixed_text() {
    // `name` sits 33 prototypes up, past the bounded search, and refers back to the error
    // itself: formatting it fully would recurse. The run fails with a bounded text.
    let src = "var e = new Error('x'); var p = { name: e }; for (var i = 0; i < 33; i++) { p = Object.create(p); } Object.setPrototypeOf(e, p); throw e;";
    let f = failure_of(src);
    // The name is left out; the message, safe to show, is kept.
    assert!(f.starts_with("Error: x"), "{f}");
    assert!(f.len() < 4000, "{f}");
}

#[test]
fn the_arguments_of_one_call_are_bounded_together() {
    // Each part is under the cap; together they are over it, and are refused before being joined.
    let src = "var s = 'x'.repeat(1 << 23); var out = []; \
               try { $.writeln(s, s, s); } catch (e) { out.push(e instanceof RangeError); } \
               try { alert.apply(null, [s, s, s]); } catch (e) { out.push(e instanceof RangeError); } \
               $.writeln(out.join(','));";
    assert_eq!(lines_of(src), ["true,true"]);
}

#[test]
fn conversion_methods_run_once_in_the_engines_order() {
    // A proxy's toString trap runs once, and its result is what is shown.
    let src = "var n = 0; var p = new Proxy({}, { get: function (t, k) { if (k === 'toString') { n++; return function () { return 'custom' + n; }; } return t[k]; } }); $.writeln(p); $.writeln(n);";
    assert_eq!(lines_of(src), ["custom1", "1"]);
    // Symbol.toPrimitive comes first, with the 'string' hint, also on an array.
    assert_eq!(first_dialog_text("var a = [1]; a[Symbol.toPrimitive] = function (h) { return 'custom:' + h; }; alert(a);"), "custom:string");
    // valueOf is used when toString gives an object.
    assert_eq!(first_dialog_text("alert({ toString: function () { return {}; }, valueOf: function () { return 7; } });"), "7");
    // An array whose join gives an object falls back to valueOf, as String() does.
    assert_eq!(first_dialog_text("var a = [1]; a.join = function () { return {}; }; a.valueOf = function () { return 'v'; }; alert(a);"), "v");
    // Neither gives a primitive: a TypeError the script can catch.
    assert_eq!(lines_of("try { $.writeln({ toString: null, valueOf: null }); } catch (e) { $.writeln(e instanceof TypeError); }"), ["true"]);
}

/// A host that shows dialogs itself and takes `wait` to answer: `clock` moves on by it.
struct SlowDialogs {
    clock: Arc<ManualClock>,
    wait: Duration,
    lines: Log,
}

impl Host for SlowDialogs {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }

    fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
        self.clock.advance(self.wait);
        Ok(Value::Null)
    }
}

/// A host whose output takes time: each line moves `clock` on by `wait`.
struct SlowWrites {
    clock: Arc<ManualClock>,
    wait: Duration,
    lines: Log,
}

impl Host for SlowWrites {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, line: &str) {
        self.clock.advance(self.wait);
        self.lines.borrow_mut().push(line.to_owned());
    }

    fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
        Ok(Value::Null)
    }
}

/// Time spent writing a line counts, also when the line is the script's last act.
#[test]
fn a_slow_last_output_line_is_charged() {
    let (clock, lines) = (Arc::new(ManualClock::default()), Log::default());
    let host = SlowWrites { clock: clock.clone(), wait: Duration::from_millis(400), lines: lines.clone() };
    let budget = WallClock::with_clock(Duration::from_millis(200), clock);
    let r = Runner::new("$.writeln('x');", Box::new(host), Box::new(budget), RunOptions::default()).unwrap();
    assert_eq!(r.run(), Err("The script was stopped: it ran longer than its time limit.".into()));
    assert_eq!(*lines.borrow(), ["x"], "the line was written once");
}

/// A long thrown text is marked as cut even when it is cut by its UTF-16 length, as text of
/// characters outside the BMP is.
#[test]
fn a_long_thrown_text_is_marked_as_cut() {
    for src in [
        "throw '\u{1F600}'.repeat(2001)",
        "throw new Error('\u{1F600}'.repeat(3000))",
        "throw { message: '\u{1F600}'.repeat(3000) }",
        // A pair the cut would split is left out whole.
        "throw 'x' + '\u{1F600}'.repeat(1001)",
        // Cut again with its prefix: still one marker, whole, at the end.
        "throw new Error('\u{1F600}'.repeat(10) + 'x'.repeat(5000))",
        "throw { message: '\u{1F600}'.repeat(25) + 'x'.repeat(5000) }",
        // A long name and a long message, or a long name alone: one marker, at the end.
        "var e = new Error('m'.repeat(2100)); e.name = 'N'.repeat(150); throw e",
        "var e = new Error(''); e.name = 'N'.repeat(2100); throw e",
    ] {
        let seen = run_probe(src, Probe::default(), Box::new(Unlimited), RunOptions::default());
        let message = seen.result.unwrap_err();
        let tail: String = message.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        assert_eq!(message.matches("(cut)").count(), 1, "{src}: …{tail}");
        assert_eq!(message.matches('…').count(), 1, "{src}: …{tail}");
        assert!(message.ends_with("… (cut)"), "the marker ends the text: {src}: …{tail}");
        assert!(!message.contains("\\u"), "no half of a pair is shown: {src}: …{tail}");
    }
}

#[test]
fn a_top_level_string_is_held_to_the_same_size_cap_as_a_nested_one() {
    let too_big = "var s = 'x'.repeat(20000000); try { alert(s) } catch (e) { $.writeln(e instanceof RangeError, ' ', e.message) }";
    assert_eq!(lines_of(too_big), ["true text is too long to pass to the host (more than 16777216 UTF-16 units)"]);
    let nested = "var s = 'x'.repeat(20000000); try { alert([s]) } catch (e) { $.writeln(e instanceof RangeError, ' ', e.message) }";
    assert_eq!(lines_of(nested), ["true text is too long to pass to the host (more than 16777216 UTF-16 units)"]);
    // Text at the cap leaves no room for the separator: the next element is never read.
    let at_cap = "var a = ['x'.repeat(16777216)]; Object.defineProperty(a, 1, { get: function () { throw new Error('read') } }); \
                  try { alert(a) } catch (e) { $.writeln(e instanceof RangeError, ' ', e.message) }";
    assert_eq!(lines_of(at_cap), ["true text is too long to pass to the host (more than 16777216 UTF-16 units)"]);
    assert_eq!(first_dialog_text("alert('ok');"), "ok");
    // An array holding itself joins it as '' without converting it, as the engine's own join
    // does: its `toString` getter is read once, as `'' + a` reads it.
    let own = "var n = 0, a = [1]; a[1] = a; \
               Object.defineProperty(a, 'toString', { get: function () { return n++ ? function () { return 'Z' } : Array.prototype.toString } }); \
               var engine = '' + a; n = 0; alert(a); $.writeln(engine);";
    assert_eq!(first_dialog_text(own), "1,");
    assert_eq!(lines_of(own), ["1,"]);
}

#[test]
fn promise_jobs_run_before_the_run_is_done() {
    assert_eq!(lines_of("Promise.resolve().then(() => $.writeln('after')); $.writeln('before');"), ["before", "after"]);
}

#[test]
fn a_dialog_inside_a_promise_job_waits_for_its_answer() {
    let src = "(async function () { try { await null; $.writeln('a'); var ok = confirm('Go?'); $.writeln(ok); } catch (e) { $.writeln('caught'); } finally { $.writeln('finally'); } })();";
    let seen = answering(src, &[json!(true)]);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs[0].kind, ModalKind::Confirm);
    assert_eq!(seen.lines, ["a", "true", "finally"]);
}

/// Promise jobs are charged one by one: a chain that schedules one reaction at a time runs
/// until its budget is spent and stops there, before its last reaction. The chain is finite, so
/// jobs that went uncharged would finish it rather than hang the test.
#[test]
fn a_promise_chain_is_charged_job_by_job() {
    let src = "var n = 0; function f() { n++; if (n == 5) $.writeln('early'); if (n < 5000) Promise.resolve().then(f); else $.writeln('last'); } f();";
    let seen = run_probe(src, Probe::default(), Box::new(Countdown(1000)), RunOptions::default());
    assert!(seen.result.as_ref().is_err_and(|m| m.contains("time limit")), "{:?}", seen.result);
    assert_eq!(seen.lines, ["early"], "an early reaction ran, the last did not");
    // With charges to spare, the same chain finishes.
    let seen = run_probe(src, Probe::default(), Box::new(Countdown(100_000)), RunOptions::default());
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["early", "last"]);
}

#[test]
fn an_unsupported_asynchronous_job_fails_the_run() {
    let seen = answering("import('x.js').catch(function () { $.writeln('caught'); });", &[]);
    assert!(seen.result.as_ref().is_err_and(|m| m.contains("not supported")), "{:?}", seen.result);
    assert!(seen.lines.is_empty());
}

#[test]
fn a_cr_only_directive_line_keeps_the_rest_of_the_script() {
    assert_eq!(lines_of("#target photocraft\r$.writeln('ran');\r"), ["ran"]);
}

#[test]
fn a_private_field_named_like_a_directive_is_kept() {
    let src = "class A {\n  #target = 1;\n  get() { return this.#target; }\n}\n$.writeln(new A().get());\n";
    assert_eq!(lines_of(src), ["1"]);
}

#[test]
fn a_bare_private_field_named_like_a_directive_is_kept() {
    let src = "class A {\n  #target\n  get() { return this.#target; }\n}\n$.writeln(new A().get());\n";
    assert_eq!(lines_of(src), ["undefined"]);
}

#[test]
fn a_line_separator_after_a_directive_keeps_the_script() {
    assert_eq!(lines_of("#target photocraft\u{2028}$.writeln('ran');"), ["ran"]);
}

#[test]
fn a_comment_include_is_reported_up_front() {
    let err = Runner::new("//@include \"lib.jsx\"\n$.writeln('ran');", Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err();
    assert_eq!(err.as_deref(), Some("line 1: //@include is not supported yet"));
}

#[test]
fn a_doubled_bom_still_runs() {
    assert_eq!(lines_of("\u{feff}\u{feff}#target photocraft\n$.writeln('ran');"), ["ran"]);
}

/// Run on a 16 MiB thread: debug builds of the parser need far more stack per level than release
/// builds, and the test thread's default is small.
fn on_script_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(16 << 20).spawn(f).unwrap().join().unwrap()
}

fn nested(open: &str, close: &str, depth: usize) -> String {
    format!("var a = {}1{};\n$.writeln('ran');", open.repeat(depth), close.repeat(depth))
}

/// Each of these aborted the process with a native stack overflow inside the parser.
#[test]
fn deeply_nested_source_is_a_named_error_before_parsing() {
    for depth in [700, 20_000, 100_000] {
        for (open, close) in [("[", "]"), ("(", ")"), ("{a:", "}"), ("`${", "}`")] {
            let src = nested(open, close, depth);
            let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
            assert_eq!(err.as_deref(), Some("line 1: SyntaxError: brackets are nested more than 64 levels deep"), "{open} x {depth}");
        }
    }
    // The line is the one where the limit is crossed.
    let src = format!("var a = 1;\nvar b = {}\n{}1{};", "(".repeat(40), "(".repeat(40), ")".repeat(80));
    let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
    assert_eq!(err.as_deref(), Some("line 3: SyntaxError: brackets are nested more than 64 levels deep"));
}

#[test]
fn deeply_nested_eval_and_function_source_is_a_catchable_syntax_error() {
    let src = "var deep = Array(701).join('[') + Array(701).join(']');
               try { eval(deep); } catch (e) { $.writeln(e instanceof SyntaxError, ' ', e.message); }
               try { new Function('return ' + deep); } catch (e) { $.writeln(e instanceof SyntaxError); }
               try { (0, eval)(Array(20001).join('(') + '1' + Array(20001).join(')')); } catch (e) { $.writeln(e instanceof SyntaxError); }";
    let lines = on_script_stack(move || lines_of(src));
    assert_eq!(lines, ["true brackets are nested more than 64 levels deep (line 1)", "true", "true"]);
}

#[test]
fn function_parameters_are_checked_joined_as_boa_parses_them() {
    // Each fragment alone is under the limit; joined with a comma they nest 100 deep.
    let src = "var half = Array(51).join('(');
               try { Function('a=' + half + '0', half + '0' + Array(101).join(')'), ''); $.writeln('parsed'); }
               catch (e) { $.writeln(e instanceof SyntaxError, ' ', e.message); }
               var f = Function('a', 'b = (1)', 'return a + b'); $.writeln(f(1));";
    let lines = on_script_stack(move || lines_of(src));
    assert_eq!(lines, ["true brackets are nested more than 64 levels deep (line 1)", "2"]);
}

#[test]
fn ordinary_nesting_still_runs() {
    let lines = on_script_stack(|| {
        let mut out = Vec::new();
        for (open, close) in [("[", "]"), ("(", ")"), ("{a:", "}")] {
            out.extend(lines_of(&nested(open, close, 50)));
            out.extend(lines_of(&nested(open, close, 64)));
        }
        out.extend(lines_of("var r = eval('[[[[[[1]]]]]]'); $.writeln(r[0][0][0][0][0][0]);"));
        out
    });
    assert_eq!(lines, ["ran", "ran", "ran", "ran", "ran", "ran", "1"]);
    let src = nested("[", "]", 65);
    let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
    assert_eq!(err.as_deref(), Some("line 1: SyntaxError: brackets are nested more than 64 levels deep"));
}

#[test]
fn brackets_in_strings_comments_templates_and_regexes_do_not_count() {
    let many = "(".repeat(1000);
    let src = format!(
        "var s = '{many}', d = \"{many}\", t = `{many}\n{many}`; // {many}\n/* {many}\n{many} */\nvar r = /[(]{{2}}\\(/g, q = /'/; var x = 4 / 2 / 1;\n$.writeln(s.length + d.length, ' ', t.length, ' ', r.test('(('), ' ', x);"
    );
    assert_eq!(on_script_stack(move || lines_of(&src)), ["2000 2001 false 2"]);
}

/// The engine joins an array by its length, holes included, reserving memory for all of it.
#[test]
fn a_sparse_array_with_a_huge_length_is_a_script_error() {
    for len in ["4294967295", "16777216"] {
        let src = format!("var a = []; a.length = {len}; try {{ alert(a); }} catch (e) {{ $.writeln(e instanceof RangeError, ' ', e.message); }}");
        let started = std::time::Instant::now();
        assert_eq!(lines_of(&src), ["true value is too large to convert to text (more than 1048576 items)"], "{len}");
        // Refused by its length, not after walking every hole (4 billion of them would take
        // minutes): fast even in a debug build on a loaded runner.
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{len}: {:?}", started.elapsed());
    }
    let src = "var a = [[]]; a[0].length = 2000000; try { alert(a); } catch (e) { $.writeln(e instanceof RangeError); }";
    assert_eq!(lines_of(src), ["true"]);
    // Within the cap, holes still convert as before.
    assert_eq!(first_dialog_text("var a = []; a.length = 3; alert(a);"), ",,");
}

fn failure_of(src: &str) -> String {
    match answering(src, &[]).result {
        Err(m) => m,
        Ok(()) => panic!("{src}: finished"),
    }
}

#[test]
fn ordinary_uncaught_errors_keep_their_text() {
    assert_eq!(failure_of("throw new Error('boom')"), "Error: boom (script:1:7)\n    at <main> (script:1:7)");
    assert_eq!(failure_of("var e = new Error('m'); e.name = 'Custom'; throw e"), "Custom: m (script:1:9)\n    at <main> (script:1:9)");
    assert_eq!(failure_of("null.x").lines().next(), Some("TypeError: cannot convert 'null' or 'undefined' to object (script:1:6)"));
    assert_eq!(failure_of("throw 'str'"), "\"str\"\n    at <main> (script:1:7)");
    assert_eq!(failure_of("throw 42"), "42\n    at <main> (script:1:7)");
}

/// The engine displayed a thrown object by recursing into it, and panicked on a deep one.
#[test]
fn a_deep_thrown_value_has_a_fixed_text() {
    let deep = "var o = {}; for (var i = 0; i < 100000; i++) { o = { a: o }; }";
    assert_eq!(failure_of(&format!("{deep} throw o;")), "Uncaught exception: the script threw an object that is not an Error.");
    assert_eq!(failure_of(&format!("{deep} throw [o];")), "Uncaught exception: the script threw an object that is not an Error.");
    assert_eq!(failure_of(&format!("{deep} o.message = 'bad'; throw o;")), "Uncaught exception: bad");
    assert_eq!(failure_of(&format!("{deep} var e = new TypeError(); e.message = o; throw e;")), "TypeError");
    assert_eq!(failure_of(&format!("{deep} var e = new Error('m'); e.name = o; throw e;")), "Error: m");
    assert_eq!(failure_of(&format!("{deep} throw Promise.reject(o);")), "Uncaught exception: the script threw an object that is not an Error.");
}

/// The engine formatted the whole message, so a huge one took gigabytes.
#[test]
fn a_huge_error_message_is_cut() {
    for src in
        ["throw new Error('x'.repeat(1 << 20))", "throw 'x'.repeat(1 << 20)", "throw Symbol('x'.repeat(1 << 20))", "throw { message: 'x'.repeat(1 << 20) }"]
    {
        let m = failure_of(src);
        assert!(m.chars().count() < 2100, "{src}: {} chars", m.chars().count());
        assert!(m.ends_with("… (cut)"), "{src}: {}", &m[m.len() - 20..]);
    }
    assert!(failure_of("throw new Error('x'.repeat(1 << 20))").starts_with("Error: xxx"));
    assert!(failure_of("var e = new Error('m'); e.name = 'x'.repeat(1 << 20); throw e").chars().count() < 2100);
}

#[test]
fn an_unhandled_rejection_fails_the_run() {
    assert_eq!(failure_of("Promise.reject(new Error('boom'));"), "Uncaught (in promise) Error: boom");
    assert_eq!(failure_of("(async function main() { await null; throw new TypeError('bad'); })();"), "Uncaught (in promise) TypeError: bad");
    assert_eq!(failure_of("Promise.resolve().then(function () { undefinedFn(); });"), "Uncaught (in promise) ReferenceError: undefinedFn is not defined");
    assert_eq!(failure_of("Promise.reject('plain');"), "Uncaught (in promise) plain");
    assert_eq!(failure_of("Promise.reject(42);"), "Uncaught (in promise) 42");
    // The reason is read from data only: no getter or toString runs after the body.
    let src = "var hits = 0; var o = { toString: function () { hits++; $.writeln('ran'); return 'x'; } };
               Object.defineProperty(o, 'message', { get: function () { $.writeln('ran'); return 'm'; } });
               Promise.reject(o);";
    let seen = answering(src, &[]);
    assert_eq!(seen.result, Err("Uncaught (in promise) the promise was rejected with an object that is not an Error".into()));
    assert!(seen.lines.is_empty());
    let src = "var e = new Error('m'); Object.defineProperty(e, 'name', { get: function () { $.writeln('ran'); return 'N'; } }); Promise.reject(e);";
    let seen = answering(src, &[]);
    assert_eq!(seen.result, Err("Uncaught (in promise) Error: m".into()));
    assert!(seen.lines.is_empty());
}

#[test]
fn handled_rejections_do_not_fail_the_run() {
    let src = "var p = Promise.reject(new Error('x'));
               Promise.resolve().then(function () { p.catch(function (e) { $.writeln('late ', e.message); }); });
               Promise.reject(new Error('y')).catch(function (e) { $.writeln('caught ', e.message); });
               (async function () { try { await Promise.reject(new Error('z')); } catch (e) { $.writeln('awaited ', e.message); } })();";
    assert_eq!(lines_of(src), ["caught y", "awaited z", "late x"]);
}

#[test]
fn an_unhandled_rejection_fails_the_run_after_a_dialog() {
    let seen = answering("alert('a'); $.writeln('after'); Promise.reject(new Error('late'));", &[]);
    assert_eq!(seen.result, Err("Uncaught (in promise) Error: late".into()));
    assert_eq!(seen.lines, ["after"]);
    assert!(seen.calls.is_empty());
}

#[test]
fn a_huge_string_argument_is_a_script_error() {
    for call in ["alert(s)", "$.writeln(s)", "$.writeln([s])"] {
        let src = format!("var s = 'x'.repeat((16 << 20) + 1); try {{ {call}; }} catch (e) {{ $.writeln(e instanceof RangeError, ' ', e.message); }}");
        assert_eq!(lines_of(&src), ["true text is too long to pass to the host (more than 16777216 UTF-16 units)"], "{call}");
    }
}

#[test]
fn a_handle_to_a_closed_document_does_not_follow_a_newer_document_with_its_id() {
    let s = session_with_two_documents();
    let host = SessionHost::new(s.clone()).at_dialog(|session| {
        let closed = session.close(0).unwrap();
        // The same document opened again: free to keep its id, since no open document has it.
        let id = closed.doc.id;
        let index = session.add_document((*closed.doc).clone(), None);
        assert_eq!(session.documents()[index].doc.id, id, "the id was reused");
    });
    let out = host.output();
    let src = "var one = app.documents[0]; alert('wait');
               try { $.writeln(one.name) } catch (e) { $.writeln(e.message) }
               try { app.activeDocument = one; $.writeln('activated') } catch (e) { $.writeln(e.message) }
               $.writeln(one == app.documents[0], ' ', one == app.documents[1], ' ', app.documents[1].name);";
    assert_eq!(Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap().run(), Ok(()));
    assert_eq!(*out.borrow(), ["the document is no longer open", "Object expected", "false false One"]);
}

#[test]
fn deep_brackets_after_a_postfix_operator_or_a_keyword_property_are_refused_before_parsing() {
    for prefix in ["var a = 1; a++ / ", "var o = {return: 1}; o.return / "] {
        let src = format!("{prefix}{}1{} / 1;", "(".repeat(20_000), ")".repeat(20_000));
        let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
        assert_eq!(err.as_deref(), Some("line 1: SyntaxError: brackets are nested more than 64 levels deep"), "{prefix}");
    }
    assert_eq!(lines_of("var a = 4; a++ / 2; var o = {return: 6}; $.writeln(a / 5, ' ', o.return / 3, ' ', /x/.test('x'));"), ["1 2 true"]);
}

#[test]
fn deep_brackets_after_a_number_with_a_trailing_point_are_refused_before_parsing() {
    let src = format!("var n = 1. / {}1{} / 1;", "(".repeat(20_000), ")".repeat(20_000));
    let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
    assert_eq!(err.as_deref(), Some("line 1: SyntaxError: brackets are nested more than 64 levels deep"));
    assert_eq!(lines_of("var n = 1. / 4; var m = 3..toString(); $.writeln(n, ' ', m, ' ', 0.5 / 2);"), ["0.25 3 0.25"]);
}

#[test]
fn a_slash_after_a_statement_that_ends_at_a_line_break_starts_a_regex() {
    // Read as divisions, the backtick would open a template that hides the next line.
    let heads = ["debugger\n/`/;", "a: for (;;) { break a\n/`/; }", "b: for (;;) { continue b\n/`/; }", "for (;;) { break\n/`/; }"];
    for head in heads {
        let src = format!("{head}\nvar deep = {}1{};", "(".repeat(100), ")".repeat(100));
        let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
        assert!(err.as_deref().is_some_and(|e| e.contains("nested more than 64 levels deep")), "{head}: {err:?}");
    }
    // A name after them is still an operand when the line goes on: `break` ends at the break.
    let lines = on_script_stack(|| lines_of("var a = 6, g = 2; for (;;) { break\n}\n$.writeln(a / g / 1);"));
    assert_eq!(lines, ["3"]);
}

#[test]
fn deep_brackets_after_any_slash_are_refused_before_parsing() {
    // The heuristic scan let each of these through to the parser.
    let prefixes = [
        "x = 1.5.return / ",
        "x = 0x1F.in / ",
        "var of = 4; x = of / ",
        "var yield = 4; x = yield / ",
        "class A { #in = 4; m() { return this.#in / ",
        "var a\u{b7}in = 4; x = a\u{b7}in / ",
    ];
    for prefix in prefixes {
        let src = format!("{prefix}{}1{} / 1;", "(".repeat(100), ")".repeat(100));
        let err = on_script_stack(move || Runner::new(&src, Box::new(Probe::default()), Box::new(Unlimited), RunOptions::default()).err());
        assert!(err.as_deref().is_some_and(|e| e.contains("nested more than 64 levels deep")), "{prefix}: {err:?}");
    }
}

/// A host that answers for another document than the one asked about, or names none.
struct Forged(Value);

impl Host for Forged {
    fn execute(&mut self, id: &str, _: Value) -> Result<Value, HostError> {
        match id {
            "session.inspect" => Ok(json!({"active": 0, "documents": [{"index": 0, "id": 1, "serial": 5, "name": "Mine", "dirty": false}]})),
            _ => Ok(self.0.clone()),
        }
    }

    fn write_line(&mut self, _: &str) {}

    fn modal(&mut self, _: ModalKind, args: &Value) -> Result<Value, HostError> {
        Err(HostError(args.to_string()))
    }
}

#[test]
fn a_document_read_answered_for_another_document_is_refused() {
    for answer in [json!({"id": 1, "serial": 6, "name": "Other"}), json!({"id": 2, "serial": 5, "name": "Other"}), json!({"name": "Other"})] {
        let src = "try { alert(app.activeDocument.name) } catch (e) { alert(e.message) }";
        // The forged host refuses the dialog with its arguments: the run ends with them.
        let result = Runner::new(src, Box::new(Forged(answer.clone())), Box::new(Unlimited), RunOptions::default()).unwrap().run();
        assert_eq!(result, Err(json!(["the document is no longer open"]).to_string()), "{answer}");
    }
}
