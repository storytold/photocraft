//! ScriptUI windows: what the UI is sent, and how its events reach the script.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use photocraft_script_js::{Host, HostError, ModalKind, RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

type Answer = Box<dyn FnMut(ModalKind, &Value) -> Value>;
/// Window updates in order: the window id and its controls, or `None` when hidden.
type Updates = Rc<RefCell<Vec<(u32, Option<Value>)>>>;

/// Records output, window updates and the dialogs shown; answers dialogs from `answers` in
/// order. `live` answers inside the call, as the script host process does; otherwise the run
/// pauses at each dialog and the test answers through [`Runner::answer`].
#[derive(Default, Clone)]
struct Ui {
    lines: Rc<RefCell<Vec<String>>>,
    windows: Updates,
    shown: Rc<RefCell<Vec<(ModalKind, Value)>>>,
    answers: Rc<RefCell<VecDeque<Answer>>>,
    live: bool,
    /// The next this many window updates fail, as a frame too large to send would.
    refuse_windows: Rc<std::cell::Cell<u32>>,
    /// Window messages (counted from 0) that fail.
    refuse_at: Rc<RefCell<Vec<usize>>>,
    window_calls: Rc<std::cell::Cell<usize>>,
    /// How long each window update takes, on `clock` when there is one (else really).
    window_delay: Rc<std::cell::Cell<std::time::Duration>>,
    clock: Option<std::sync::Arc<crate::ManualClock>>,
    /// Window updates that took their time.
    delayed: Rc<std::cell::Cell<usize>>,
    /// Output lines and window updates in the order they came: `out …`, `shown 0`, `hidden 0`.
    order: Rc<RefCell<Vec<String>>>,
}

impl Ui {
    fn answer(&self, kind: ModalKind, args: &Value) -> Value {
        self.shown.borrow_mut().push((kind, args.clone()));
        let next = self.answers.borrow_mut().pop_front();
        match next {
            Some(mut f) => f(kind, args),
            None => panic!("no answer left for {kind:?} {args}"),
        }
    }
}

impl Host for Ui {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
        self.order.borrow_mut().push(format!("out {line}"));
    }

    fn modal(&mut self, kind: ModalKind, args: &Value) -> Option<Result<Value, HostError>> {
        self.live.then(|| Ok(self.answer(kind, args)))
    }

    fn window(&mut self, id: u32, window: Option<&Value>) -> Result<(), HostError> {
        let call = self.window_calls.get();
        self.window_calls.set(call + 1);
        if !self.window_delay.get().is_zero() {
            match &self.clock {
                Some(c) => c.advance(self.window_delay.get()),
                None => std::thread::sleep(self.window_delay.get()),
            }
            self.delayed.set(self.delayed.get() + 1);
        }
        if self.refuse_at.borrow().contains(&call) {
            return Err(HostError("the window could not be shown: too large".into()));
        }
        if self.refuse_windows.get() > 0 {
            self.refuse_windows.set(self.refuse_windows.get() - 1);
            return Err(HostError("the window could not be shown: too large".into()));
        }
        self.windows.borrow_mut().push((id, window.cloned()));
        self.order.borrow_mut().push(format!("{} {id}", if window.is_some() { "shown" } else { "hidden" }));
        Ok(())
    }
}

fn answers(list: Vec<Value>) -> VecDeque<Answer> {
    list.into_iter().map(|v| Box::new(move |_: ModalKind, _: &Value| v.clone()) as Answer).collect()
}

/// Run `src` to the end, answering dialogs with `list`, live or by replay.
fn run(src: &str, list: Vec<Value>, live: bool) -> (Step, Ui) {
    let ui = Ui { live, ..Ui::default() };
    *ui.answers.borrow_mut() = answers(list);
    let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
    loop {
        match r.step() {
            Step::Paused(m) => {
                let a = ui.answer(m.kind, &m.args);
                r.answer(a).unwrap();
            }
            other => return (other, ui),
        }
    }
}

fn both(src: &str, list: Vec<Value>) -> Vec<String> {
    let (live, a) = run(src, list.clone(), true);
    let (replayed, b) = run(src, list, false);
    assert_eq!(live, Step::Done, "live: {:?}", a.lines.borrow());
    assert_eq!(replayed, Step::Done, "replayed: {:?}", b.lines.borrow());
    assert_eq!(*a.lines.borrow(), *b.lines.borrow(), "a replay writes what a live run writes, once");
    assert_eq!(*a.windows.borrow(), *b.windows.borrow(), "and sends the same window updates, once");
    a.lines.borrow().clone()
}

/// The first control of `tree` with this `type`, depth first.
fn find<'a>(tree: &'a Value, ty: &str) -> Option<&'a Value> {
    if tree["type"] == ty {
        return Some(tree);
    }
    tree["children"].as_array()?.iter().find_map(|c| find(c, ty))
}

#[test]
fn a_dialog_sends_its_controls_and_returns_the_ok_result_with_the_typed_text() {
    let src = r#"
        var w = new Window("dialog", "Rename");
        var g = w.add("group");
        g.add("statictext", undefined, "Name:");
        var e = g.add("edittext", undefined, "old");
        e.characters = 20;
        var ok = w.add("button", undefined, "OK");
        w.add("button", undefined, "Cancel", {name: "cancel"});
        var r = w.show();
        $.writeln(r, " ", e.text);
    "#;
    // window 0, group 1, statictext 2, edittext 3, OK 4, Cancel 5.
    let (step, ui) = run(src, vec![json!({"event": "click", "id": 4, "values": [{"id": 3, "value": "new"}]})], true);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["1 new"]);
    let shown = ui.shown.borrow();
    let (kind, args) = &shown[0];
    assert_eq!(*kind, ModalKind::Window);
    assert_eq!(args["id"], 0);
    let tree = &args["window"];
    assert_eq!(tree["type"], "window");
    assert_eq!(tree["style"], "dialog");
    assert_eq!(tree["text"], "Rename");
    let edit = find(tree, "edittext").unwrap();
    assert_eq!((edit["id"].as_u64(), edit["text"].as_str(), edit["characters"].as_f64()), (Some(3), Some("old"), Some(20.0)));
    let buttons: Vec<&Value> = tree["children"].as_array().unwrap().iter().filter(|c| c["type"] == "button").collect();
    assert_eq!(buttons[0]["default"], true, "a button labelled OK is the default button");
    assert_eq!(buttons[1]["cancel"], true, "a button named cancel is the cancel button");
    assert_eq!(find(tree, "group").unwrap()["orientation"], "row");
    assert_eq!(*ui.windows.borrow(), [(0, None)], "the dialog is hidden when show() returns");
}

#[test]
fn handlers_run_between_events_and_the_next_event_sees_their_changes() {
    let src = r#"
        var w = new Window("dialog", "Count");
        var label = w.add("statictext", undefined, "0");
        var more = w.add("button", undefined, "More");
        var n = 0;
        more.onClick = function () { n++; label.text = String(n); $.writeln("clicked ", n); };
        var check = w.add("checkbox", undefined, "Twice");
        check.onClick = function () { $.writeln("check ", this.value); };
        w.add("button", undefined, "Cancel");
        $.writeln("result ", w.show(), " ", label.text, " ", check.value);
    "#;
    // window 0, label 1, More 2, checkbox 3, Cancel 4.
    let events = vec![
        json!({"event": "click", "id": 2}),
        json!({"event": "click", "id": 3, "values": [{"id": 3, "value": true}]}),
        json!({"event": "click", "id": 2}),
        json!({"event": "click", "id": 4}),
    ];
    let lines = both(src, events.clone());
    assert_eq!(lines, ["clicked 1", "check true", "clicked 2", "result 2 2 true"]);
    let (_, ui) = run(src, events, true);
    let labels: Vec<Value> = ui.shown.borrow().iter().map(|(_, a)| find(&a["window"], "statictext").unwrap()["text"].clone()).collect();
    assert_eq!(labels, [json!("0"), json!("1"), json!("1"), json!("2")]);
}

#[test]
fn close_values_on_close_and_the_close_box() {
    let src = r#"
        var w = new Window("dialog");
        var go = w.add("button", undefined, "Run");
        go.onClick = function () { w.close(7); };
        var asked = 0;
        w.onClose = function () { asked++; return asked > 1; };
        $.writeln(w.show(), " ", asked);
        var v = new Window("dialog");
        v.add("button", undefined, "Close").onClick = function () { v.close(); };
        $.writeln(v.show());
    "#;
    // w: window 0, Run 1; v: window 2, Close 3.
    let events = vec![
        // The close box: onClose says no, so the window stays.
        json!({"event": "close"}),
        // Again: now it closes, as a cancel.
        json!({"event": "close"}),
        json!({"event": "click", "id": 3}),
    ];
    assert_eq!(both(src, events), ["2 2", "1"]);
    let src2 = r#"var w = new Window("dialog"); var b = w.add("button", undefined, "Run"); b.onClick = function () { w.close(7); }; $.writeln(w.show());"#;
    assert_eq!(both(src2, vec![json!({"event": "click", "id": 1})]), ["7"]);
}

#[test]
fn lists_take_items_and_report_the_chosen_one() {
    let src = r#"
        var w = new Window("dialog");
        var d = w.add("dropdownlist", undefined, ["Red", "Green", "Blue"]);
        d.selection = 0;
        var l = w.add("listbox");
        l.add("item", "one"); l.add("separator"); l.add("item", "two");
        d.onChange = function () { $.writeln("changed to ", d.selection.text, " at ", d.selection.index); };
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(d.selection, " ", d.items.length, " ", l.selection, " ", l.find("two").index);
        l.remove(0);
        $.writeln(l.items.length, " ", l.items[0].type, " ", l.find("two").index);
    "#;
    // window 0, dropdown 1, listbox 2, OK 3.
    let events =
        vec![json!({"event": "change", "id": 1, "values": [{"id": 1, "value": 2}]}), json!({"event": "click", "id": 3, "values": [{"id": 2, "value": 2}]})];
    assert_eq!(both(src, events.clone()), ["changed to Blue at 2", "Blue 3 two 2", "2 separator 1"]);
    let (_, ui) = run(src, events, true);
    let first = &ui.shown.borrow()[0].1["window"];
    assert_eq!(find(first, "dropdownlist").unwrap()["items"], json!(["Red", "Green", "Blue"]));
    assert_eq!(find(first, "dropdownlist").unwrap()["selection"], 0);
    assert_eq!(find(first, "listbox").unwrap()["items"], json!(["one", null, "two"]), "a separator is sent as null");
}

#[test]
fn a_palette_does_not_wait_and_sends_its_updates() {
    let src = r#"
        var p = new Window("palette", "Working");
        var bar = p.add("progressbar", undefined, 0, 4);
        p.show();
        for (var i = 1; i <= 4; i++) { bar.value = i; if (i % 2 == 0) p.update(); }
        app.refresh();
        p.close();
        $.writeln("done");
    "#;
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["done"]);
    let windows = ui.windows.borrow();
    let values: Vec<Option<f64>> = windows.iter().map(|(_, w)| w.as_ref().map(|w| find(w, "progressbar").unwrap()["value"].as_f64().unwrap())).collect();
    assert_eq!(values, [Some(0.0), Some(2.0), Some(4.0), Some(4.0), None]);
    assert_eq!(windows[0].1.as_ref().unwrap()["style"], "palette");
    assert_eq!(find(windows[0].1.as_ref().unwrap(), "progressbar").unwrap()["max"], 4.0);
    assert!(ui.shown.borrow().is_empty(), "a palette opens no dialog");
}

#[test]
fn values_and_events_for_other_windows_or_disabled_controls_are_ignored() {
    let src = r#"
        var other = new Window("dialog");
        var foreign = other.add("edittext", undefined, "untouched");
        var w = new Window("dialog");
        var b = w.add("button", undefined, "Go");
        b.enabled = false;
        b.onClick = function () { $.writeln("disabled ran"); };
        w.add("button", undefined, "OK");
        $.writeln(w.show(), " ", foreign.text);
    "#;
    // other 0, foreign 1, w 2, Go 3, OK 4.
    let events = vec![
        json!({"event": "click", "id": 3, "values": [{"id": 1, "value": "changed"}]}),
        json!({"event": "click", "id": 1}),
        json!({"event": "click", "id": 99, "values": [{"id": 99, "value": 1}, {"id": "x"}, 5]}),
        json!("not an event"),
        json!({"event": "click", "id": 4}),
    ];
    assert_eq!(both(src, events), ["1 untouched"]);
}

#[test]
fn hostile_windows_end_in_named_errors() {
    let cases = [
        (r#"var w = new Window("dialog"); var g = w; for (var i = 0; i < 100; i++) g = g.add("group");"#, "nested at most 16 deep"),
        (r#"var w = new Window("dialog"); for (var i = 0; i < 5000; i++) w.add("button");"#, "at most 4096"),
        (r#"new Window("dialog { b: Button { text: 'x' } }")"#, "resource strings are not supported"),
        (r#"var w = new Window("dialog"); w.add("button").add("button");"#, "cannot hold other controls"),
        (r#"var w = new Window("dialog"); w.add("treeview");"#, "\"treeview\" is not supported"),
        (r#"new Window("floating")"#, "unknown window type"),
        (r#"var w = new Window("dialog"); w.add("dropdownlist", undefined, {length: 1e9});"#, "at most 10000 items"),
        (r#"Window.prototype; var b = {}; b.show = new Window("dialog").show; b.show();"#, "not a ScriptUI window"),
        (r#"var w = new Window("dialog"); w.add("button").show();"#, "show() is a method of windows"),
    ];
    for (src, expected) in cases {
        let wrapped = format!("try {{ {src} }} catch (e) {{ $.writeln(String(e)); }}");
        let (step, ui) = run(&wrapped, vec![], true);
        assert_eq!(step, Step::Done, "{src}");
        let lines = ui.lines.borrow();
        assert!(lines.first().is_some_and(|l| l.contains(expected)), "{src}: {lines:?}");
    }
}

#[test]
fn a_window_with_too_much_text_is_refused_before_it_is_sent() {
    let src = r#"
        var w = new Window("dialog");
        var big = new Array(1 << 21).join("x");
        for (var i = 0; i < 10; i++) w.add("statictext", undefined, big);
        try { w.show(); } catch (e) { $.writeln(e.name, ": ", e.message); }
    "#;
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    assert!(ui.lines.borrow()[0].starts_with("RangeError: a ScriptUI window can hold at most"), "{:?}", ui.lines.borrow());
    assert!(ui.shown.borrow().is_empty());
}

#[test]
fn a_handler_can_open_another_dialog_and_an_alert() {
    let src = r#"
        var w = new Window("dialog", "Outer");
        var more = w.add("button", undefined, "More…");
        more.onClick = function () {
            alert("inner");
            var inner = new Window("dialog", "Inner");
            inner.add("button", undefined, "OK");
            $.writeln("inner ", inner.show());
        };
        w.add("button", undefined, "Cancel");
        $.writeln("outer ", w.show());
    "#;
    // w 0, More 1, Cancel 2, inner 3, its OK 4.
    let events = vec![json!({"event": "click", "id": 1}), Value::Null, json!({"event": "click", "id": 4}), json!({"event": "click", "id": 2})];
    assert_eq!(both(src, events.clone()), ["inner 1", "outer 2"]);
    let (_, ui) = run(src, events, true);
    let kinds: Vec<ModalKind> = ui.shown.borrow().iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, [ModalKind::Window, ModalKind::Alert, ModalKind::Window, ModalKind::Window]);
}

#[test]
fn pickers_send_their_prompt_and_filter_and_return_paths() {
    let src = r#"
        var f = File.openDialog("Pick an image", "Images:*.png;*.jpg");
        $.writeln(f.name, "|", f.path, "|", f.fsName, "|", f);
        var many = File.openDialog("Pick several", function (x) { return true; }, true);
        $.writeln(many.length, " ", many[1].name);
        var s = new File("/tmp/out.png").saveDlg("Save as");
        $.writeln(s === null);
        var d = Folder.selectDialog("Where?");
        $.writeln(d.name, " ", d instanceof Object, " ", typeof Folder.fs);
        try { f.open("r"); } catch (e) { $.writeln(e.message); }
    "#;
    let answers = vec![json!("C:\\Users\\me\\My Pictures\\cat.png"), json!(["/a/one.png", "/a/two.png", 5, ""]), Value::Null, json!("/home/me/out/")];
    let lines = both(src, answers.clone());
    assert_eq!(
        lines,
        [
            "cat.png|C:/Users/me/My Pictures|C:\\Users\\me\\My Pictures\\cat.png|C:/Users/me/My Pictures/cat.png",
            "2 two.png",
            "true",
            "out true string",
            "scripts cannot read or write files in PhotoCraft yet",
        ]
    );
    let (_, ui) = run(src, answers, true);
    let shown = ui.shown.borrow();
    let kinds: Vec<ModalKind> = shown.iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, [ModalKind::OpenFile, ModalKind::OpenFile, ModalKind::SaveFile, ModalKind::SelectFolder]);
    assert_eq!(shown[0].1, json!({"prompt": "Pick an image", "filter": "Images:*.png;*.jpg", "multiple": false}));
    assert_eq!(shown[1].1, json!({"prompt": "Pick several", "filter": null, "multiple": true}), "a filter function stays in the script");
    assert_eq!(shown[3].1, json!({"prompt": "Where?"}));
}

#[test]
fn a_cancelled_or_malformed_pick_is_null() {
    let src = r#"
        $.writeln(File.openDialog("a"), " ", File.openDialog("b", null, true), " ", Folder.selectDialog(), " ", File.openDialog("c", "", true));
    "#;
    let answers = vec![json!(42), json!([]), json!({"path": "/x"}), json!("/only.png")];
    assert_eq!(both(src, answers), ["null null null /only.png"]);
}

#[test]
fn constructors_have_prototypes_so_instanceof_works() {
    let src = r#"
        var f = File("/x/a.txt"), d = new Folder("/x"), w = new Window("dialog");
        $.writeln(f instanceof File, " ", d instanceof Folder, " ", w instanceof Window, " ", f instanceof Folder);
        $.writeln(f.constructor === File, " ", w.add("button").constructor === Window, " ", typeof File.prototype.openDlg);
    "#;
    assert_eq!(both(src, vec![]), ["true true true false", "true true function"]);
}

#[test]
fn a_window_update_that_fails_is_the_scripts_error_live_and_replayed() {
    let src = r#"
        var p = new Window("palette");
        try { p.show(); $.writeln("shown"); } catch (e) { $.writeln("caught: ", e.message); }
        $.writeln(confirm("again?"));
    "#;
    for live in [true, false] {
        // Only the first attempt's update fails: a replay must fail from the tape, not ask again.
        let ui = Ui { live, refuse_windows: Rc::new(std::cell::Cell::new(1)), ..Ui::default() };
        *ui.answers.borrow_mut() = answers(vec![json!(true)]);
        let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
        let step = loop {
            match r.step() {
                Step::Paused(m) => {
                    let a = ui.answer(m.kind, &m.args);
                    r.answer(a).unwrap();
                }
                other => break other,
            }
        };
        assert_eq!(step, Step::Done);
        assert_eq!(*ui.lines.borrow(), ["caught: the window could not be shown: too large", "true"], "live: {live}");
        assert!(ui.windows.borrow().is_empty(), "the window was not sent again on a replay (live: {live})");
    }
}

#[test]
fn values_for_disabled_or_hidden_controls_are_not_applied() {
    let src = r#"
        var w = new Window("dialog");
        var e = w.add("edittext", undefined, "locked"); e.enabled = false;
        var g = w.add("group"); g.enabled = false;
        var inner = g.add("checkbox", undefined, "inner");
        var h = w.add("edittext", undefined, "hidden"); h.visible = false;
        var free = w.add("edittext", undefined, "free");
        inner.onClick = function () { $.writeln("inner ran"); };
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(e.text, " ", inner.value, " ", h.text, " ", free.text);
    "#;
    // window 0, e 1, g 2, inner 3, h 4, free 5, OK 6.
    let values = json!([{"id": 1, "value": "x"}, {"id": 3, "value": true}, {"id": 4, "value": "y"}, {"id": 5, "value": "z"}]);
    let events = vec![json!({"event": "click", "id": 3, "values": values}), json!({"event": "click", "id": 6})];
    assert_eq!(both(src, events), ["locked false hidden z"]);
}

#[test]
fn an_items_text_cannot_push_a_list_past_its_limit() {
    let src = r#"
        var w = new Window("dialog");
        var l = w.add("listbox");
        for (var i = 0; i < 9999; i++) l.add("item", "x");
        var sneaky = { toString: function () { l.add("item", "inner"); return "outer"; } };
        try { l.add("item", sneaky); } catch (e) { $.writeln(e.name); }
        $.writeln(l.items.length);
    "#;
    assert_eq!(both(src, vec![]), ["RangeError", "10000"]);
}

#[test]
fn a_list_stays_within_its_limit_whatever_setters_the_script_adds() {
    let src = r#"
        var w = new Window("dialog");
        var l = w.add("listbox");
        for (var i = 0; i < 9999; i++) l.add("item", "x");
        var d = 0;
        Object.defineProperty(Object.prototype, "selected", {configurable: true, set: function () { if (d < 50) { d++; try { l.add("item", "inner"); } catch (e) {} } }});
        try { l.add("item", "outer"); } catch (e) { $.writeln(e.name); }
        $.writeln(l.items.length);
    "#;
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["10000"]);
}

#[test]
fn a_removed_control_takes_no_values_and_runs_no_handlers() {
    let src = r#"
        var w = new Window("dialog");
        var e = w.add("edittext", undefined, "kept");
        var b = w.add("button", undefined, "Gone");
        b.onClick = function () { $.writeln("removed ran"); };
        w.remove(e); w.remove(b);
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(e.text);
    "#;
    // window 0, e 1, b 2, OK 3.
    let events = vec![json!({"event": "click", "id": 2, "values": [{"id": 1, "value": "changed"}]}), json!({"event": "click", "id": 3})];
    assert_eq!(both(src, events), ["kept"]);
}

#[test]
fn an_item_whose_setup_throws_is_not_left_in_the_list() {
    let src = r#"
        var w = new Window("dialog");
        var l = w.add("listbox");
        l.add("item", "first");
        Object.defineProperty(Object.getPrototypeOf(l.items[0]), "selected", {configurable: true, set: function () { throw new Error("no"); }});
        try { l.add("item", "second"); } catch (e) { $.writeln(e.message); }
        delete Object.getPrototypeOf(l.items[0]).selected;
        l.add("item", "third");
        $.writeln(l.items.length, " ", l.items[1].text, " ", l.items[1].index);
    "#;
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["no", "2 third 1"]);
}

#[test]
fn a_palette_that_fails_to_update_stays_shown_and_is_hidden_on_close() {
    let src = r#"
        var p = new Window("palette");
        p.show();
        try { p.show(); } catch (e) { $.writeln("caught"); }
        p.close();
        var q = new Window("palette");
        try { q.show(); } catch (e) { $.writeln("caught again"); }
        q.close();
    "#;
    // Window messages in order: p shown (sent), p shown again (refused), p hidden (sent), q shown
    // (refused); q was never shown, so its close sends nothing.
    let ui = Ui { live: true, refuse_at: Rc::new(RefCell::new(vec![1, 3])), ..Ui::default() };
    let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.step(), Step::Done);
    assert_eq!(*ui.lines.borrow(), ["caught", "caught again"]);
    let sent: Vec<(u32, bool)> = ui.windows.borrow().iter().map(|(id, w)| (*id, w.is_some())).collect();
    assert_eq!(sent, [(0, true), (0, false)]);
}

#[test]
fn the_time_a_window_update_takes_counts_against_the_time_limit() {
    // The clock moves only in the window update, so nothing else can spend the limit first.
    let clock = std::sync::Arc::new(crate::ManualClock::default());
    let ui = Ui { live: true, clock: Some(clock.clone()), ..Ui::default() };
    ui.window_delay.set(std::time::Duration::from_millis(150));
    let budget = photocraft_script_js::WallClock::with_clock(std::time::Duration::from_millis(50), clock);
    let mut r = Runner::new("var w = new Window('palette'); w.show();", Box::new(ui.clone()), Box::new(budget), RunOptions::default()).unwrap();
    let Step::Failed(m) = r.step() else { panic!("the run went past its time limit") };
    assert_eq!(ui.delayed.get(), 1, "the update ran, and it is what spent the time");
    assert!(m.contains("time limit"), "{m}");
}

#[test]
fn a_palette_whose_on_show_throws_is_not_shown() {
    let src = r#"
        var p = new Window("palette");
        p.onShow = function () { throw new Error("no"); };
        try { p.show(); } catch (e) { $.writeln("caught ", e.message); }
        p.onShow = null;
        app.refresh();
        p.close();
    "#;
    let ui = Ui { live: true, ..Ui::default() };
    let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.step(), Step::Done, "{:?}", ui.lines.borrow());
    assert_eq!(*ui.lines.borrow(), ["caught no"]);
    assert!(ui.windows.borrow().is_empty(), "nothing was sent: {:?}", ui.windows.borrow());
}

#[test]
fn a_palette_closed_by_its_failing_on_show_stays_closed() {
    let src = r#"
        var p = new Window("palette");
        p.show();
        p.onShow = function () { this.close(); throw new Error("no"); };
        try { p.show(); } catch (e) { $.writeln("caught ", e.message); }
        p.onShow = null;
        app.refresh();
    "#;
    let ui = Ui { live: true, ..Ui::default() };
    let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.step(), Step::Done, "{:?}", ui.lines.borrow());
    assert_eq!(*ui.lines.borrow(), ["caught no"]);
    let sent: Vec<(u32, bool)> = ui.windows.borrow().iter().map(|(id, w)| (*id, w.is_some())).collect();
    assert_eq!(sent, [(0, true), (0, false)], "shown, hidden by the handler, and not shown again by refresh()");
}

#[test]
fn a_palette_its_failing_on_show_sent_stays_shown_and_is_hidden_on_close() {
    let src = r#"
        var p = new Window("palette");
        p.onShow = function () { this.update(); throw new Error("no"); };
        try { p.show(); } catch (e) { $.writeln("caught ", e.message); }
        p.onShow = null;
        p.close();
    "#;
    let ui = Ui { live: true, ..Ui::default() };
    let mut r = Runner::new(src, Box::new(ui.clone()), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.step(), Step::Done, "{:?}", ui.lines.borrow());
    assert_eq!(*ui.lines.borrow(), ["caught no"]);
    let sent: Vec<(u32, bool)> = ui.windows.borrow().iter().map(|(id, w)| (*id, w.is_some())).collect();
    assert_eq!(sent, [(0, true), (0, false)], "sent by update(), so close() hides it");
}

/// All of a run's lists together hold a bounded number of items, and adding them takes time in
/// proportion to their number (25 full lists in seconds, not hours).
#[test]
fn lists_hold_a_bounded_number_of_items_in_all() {
    let src = r#"
        var w = new Window('dialog', 'Lists'), n = 0, why = '';
        try {
            for (var l = 0; l < 26; l++) {
                var list = w.add('listbox');
                for (var i = 0; i < 10000; i++) { list.add('item', 'x'); n++; }
            }
        } catch (e) { why = (e instanceof RangeError) + ' ' + e.message; }
        $.writeln(n, ' ', why);
    "#;
    let started = std::time::Instant::now();
    let (step, ui) = run(src, vec![], false);
    // The script catches the limit and finishes.
    assert_eq!(step, Step::Done);
    let lines = ui.lines.borrow().clone();
    assert_eq!(lines, ["250000 true a script's lists can hold at most 250000 items in all"]);
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "{:?}", started.elapsed());
}

/// An item whose `text` setter removes it and then throws gives its count back once, not twice:
/// the lists' bound still holds exactly afterwards.
#[test]
fn an_item_removed_by_its_own_failing_setter_is_counted_once() {
    let src = r#"
        var w = new Window('dialog', 'Lists'), first = w.add('listbox'), n = 0, why = '';
        first.add('item', 'kept');
        var victim = first;
        Object.defineProperty(Object.prototype, 'text', {
            configurable: true,
            set: function (v) { victim.remove(this); throw new Error('no'); }
        });
        try { first.add('item', 'gone'); } catch (e) {}
        delete Object.prototype.text;
        try {
            for (var l = 0; l < 26; l++) {
                var list = w.add('listbox');
                for (var i = 0; i < 10000; i++) { list.add('item', 'x'); n++; }
            }
        } catch (e) { why = e.message; }
        $.writeln(first.items.length, ' ', n, ' ', why);
    "#;
    let (step, ui) = run(src, vec![], false);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["1 249999 a script's lists can hold at most 250000 items in all"]);
}

#[test]
fn bounds_given_to_the_constructor_or_add_size_the_control() {
    let src = r#"
        var b = [0, 0, 300, 200];
        var w = new Window("palette", "Sized", b);
        b[2] = 900; // the window keeps its own copy
        w.add("edittext", [10, 10, 130, 40], "x");
        w.add("panel", {x: 0, y: 0, width: 90, height: 60});
        w.add("group", {left: 5, top: 5, right: 55, bottom: 25});
        // A bounds whose getter throws leaves no control behind.
        try { w.add("button", {get x() { throw new Error("no"); }}, "never"); } catch (e) {}
        w.show();
    "#;
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    let windows = ui.windows.borrow();
    let tree = windows[0].1.as_ref().expect("the palette is sent");
    assert_eq!((tree["width"].as_f64(), tree["height"].as_f64()), (Some(300.0), Some(200.0)));
    let edit = find(tree, "edittext").unwrap();
    assert_eq!((edit["width"].as_f64(), edit["height"].as_f64()), (Some(120.0), Some(30.0)));
    let panel = find(tree, "panel").unwrap();
    assert_eq!((panel["width"].as_f64(), panel["height"].as_f64()), (Some(90.0), Some(60.0)));
    let group = find(tree, "group").unwrap();
    assert_eq!((group["width"].as_f64(), group["height"].as_f64()), (Some(50.0), Some(20.0)));
    assert!(find(tree, "button").is_none(), "{tree}");
}

/// Sending a palette reads its controls, which runs the script's getters; one that closes the
/// palette meanwhile leaves it closed, not sent again.
#[test]
fn a_palette_closed_while_it_is_read_is_not_sent_again() {
    let getter = r#"Object.defineProperty(t, "text", { get: function () { if (arm) { arm = false; w.close(); } return "x"; } });"#;
    for (name, tail, want) in [("show", "arm = true; w.show();", vec![None]), ("update", "w.show(); arm = true; w.update();", vec![Some(()), None])] {
        let src = format!(r#"var arm = false; var w = new Window("palette", "P"); var t = w.add("statictext", undefined, "x"); {getter} {tail}"#);
        let (step, ui) = run(&src, vec![], true);
        assert_eq!(step, Step::Done, "{name}: {:?}", ui.lines.borrow());
        let sent: Vec<Option<()>> = ui.windows.borrow().iter().map(|(_, w)| w.as_ref().map(|_| ())).collect();
        assert_eq!(sent, want, "{name}");
    }
}

/// A list's `selection` is an item, as in ScriptUI: assigning an index selects that item, and
/// reading it gives the item (`.text`, `.index`), not the number. A bad index selects nothing.
#[test]
fn a_list_selection_set_by_index_reads_back_as_the_item() {
    let src = "var w = new Window('dialog'), d = w.add('dropdownlist', undefined, ['A', 'B', 'C']);
               $.writeln(d.selection);
               d.selection = 1;
               $.writeln(typeof d.selection, ' ', d.selection.text, ' ', d.selection.index, ' ', d.items[1].selected, ' ', d.items[0].selected);
               d.selection = d.items[2]; $.writeln(d.selection.text, ' ', d.items[1].selected);
               d.selection = 9; $.writeln(d.selection);
               d.selection = 0; d.remove(0); $.writeln(d.selection);";
    let (step, ui) = run(src, vec![], true);
    assert_eq!(step, Step::Done);
    assert_eq!(*ui.lines.borrow(), ["null", "object B 1 true false", "C false", "null", "null"]);
}

/// A button handler that throws ends `show()` with its error, and the dialog is hidden first:
/// a script that catches the error and goes on does not leave it on screen holding the app.
#[test]
fn a_handler_error_hides_the_dialog_before_show_throws() {
    let src = r#"
        var w = new Window("dialog");
        var b = w.add("button", undefined, "Go");
        b.onClick = function () { throw new Error("bad click"); };
        try { w.show(); } catch (e) { $.writeln("caught ", e.message); }
        $.writeln("after");
    "#;
    for live in [true, false] {
        let (step, ui) = run(src, vec![json!({"event": "click", "id": 1})], live);
        assert_eq!(step, Step::Done);
        assert_eq!(*ui.lines.borrow(), ["caught bad click", "after"]);
        assert_eq!(ui.windows.borrow().last(), Some(&(0, None)), "hidden: {:?}", ui.windows.borrow());
        // Hidden before the script's `catch` ran, not after.
        let order = ui.order.borrow();
        let at = |what: &str| order.iter().position(|e| e == what);
        assert!(at("hidden 0").zip(at("out caught bad click")).is_some_and(|(hide, caught)| hide < caught), "{live}: {order:?}");
    }
}

/// `show()` of a dialog from its own `onShow` is refused: it would reset the `closed` and
/// `result` the outer `show()` waits on, and the outer one would return the inner's result
/// without the user closing it.
#[test]
fn a_dialog_shown_again_from_its_own_handler_is_refused() {
    let src = r#"
        var w = new Window("dialog"), inner = "";
        var b = w.add("button", undefined, "Done");
        b.onClick = function () { w.close(5); };
        w.onShow = function () { try { w.show(); } catch (e) { inner = e.message; } };
        var r = w.show();
        $.writeln(inner, " ", r);
        // Shown again once closed: fine.
        w.onShow = null;
        $.writeln(w.show());
    "#;
    let out = both(src, vec![json!({"event": "click", "id": 1}), json!({"event": "click", "id": 1})]);
    assert_eq!(out, ["the dialog is already showing 5", "5"]);
}

/// Another dialog shown from a handler still works, nested.
#[test]
fn a_second_dialog_shown_from_a_handler_still_works() {
    let src = r#"
        var a = new Window("dialog"), b = new Window("dialog");
        var go = a.add("button", undefined, "Open");
        b.add("button", undefined, "Close").onClick = function () { b.close(3); };
        go.onClick = function () { $.writeln("inner ", b.show()); a.close(4); };
        $.writeln("outer ", a.show());
    "#;
    let out = both(src, vec![json!({"event": "click", "id": 2}), json!({"event": "click", "id": 3})]);
    assert_eq!(out, ["inner 3", "outer 4"]);
}

/// A getter that closes the dialog while its controls are read ends `show()` there, without
/// asking the app to wait on a dialog that is already closed.
#[test]
fn a_dialog_closed_while_its_controls_are_read_is_not_waited_on() {
    let src = r#"
        var w = new Window("dialog");
        var t = w.add("statictext", undefined, "x");
        var reads = 0;
        Object.defineProperty(t, "text", { get: function () { if (++reads == 1) w.close(9); return "x"; }, configurable: true });
        $.writeln(w.show());
    "#;
    let out = both(src, vec![]);
    assert_eq!(out, ["9"]);
}

/// A choice that an item's `selected` setter takes out of the list is not left as the list's
/// selection.
#[test]
fn a_selection_removed_by_an_items_setter_is_not_kept() {
    let src = r#"
        var w = new Window("dialog"), list = w.add("listbox", undefined, ["a", "b"]);
        var a = list.items[0], b = list.items[1];
        Object.defineProperty(a, "selected", { set: function (v) { if (list.items.length == 2) list.remove(b); }, configurable: true });
        list.selection = b;
        $.writeln(list.items.length, " ", list.selection);
    "#;
    assert_eq!(both(src, vec![]), ["1 null"]);
}

/// A list whose items can't all be added is not left in the dialog: the error comes before it
/// is attached (too many) or the half-made list is taken out again (an item that throws), and
/// its items are given back.
#[test]
fn a_list_that_fails_to_initialize_is_not_left_in_the_dialog() {
    let src = r#"
        var w = new Window("dialog"), why = [];
        try { w.add("listbox", undefined, new Array(10001)); } catch (e) { why.push(e.message); }
        var bad = ["x", { toString: function () { throw new Error("no text"); } }];
        try { w.add("dropdownlist", undefined, bad); } catch (e) { why.push(e.message); }
        $.writeln(w.children.length, " ", why.join(" | "));
        // The items it held are counted no longer: 250000 more still fit.
        var n = 0;
        try { for (var l = 0; l < 25; l++) { var k = w.add("listbox"); for (var i = 0; i < 10000; i++) { k.add("item", "y"); n++; } } } catch (e) { why.push(e.message); }
        $.writeln(n);
    "#;
    assert_eq!(both(src, vec![]), ["0 a list can hold at most 10000 items | no text", "250000"]);
}

/// An item whose list's `items` can't be published (a setter of the script's throws) is not
/// kept in the list.
#[test]
fn an_item_the_list_could_not_publish_is_not_kept() {
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox");
        l.add("item", "a");
        Object.defineProperty(l, "items", { configurable: true, get: function () { return []; }, set: function (v) { throw new Error("no"); } });
        try { l.add("item", "b"); } catch (e) { $.writeln("caught ", e.message); }
        $.writeln(l.find("b") === null, " ", l.find("a") !== null);
    "#;
    assert_eq!(both(src, vec![]), ["caught no", "true true"]);
}

/// A control that fails to set up is taken out of its parent even when the parent's `children`
/// can't be published, and the script sees the setup's own error.
#[test]
fn a_failed_control_is_taken_out_when_its_parent_cannot_publish_its_children() {
    let src = r#"
        var w = new Window("dialog"), refuse = false;
        var real = w.children;
        Object.defineProperty(w, "children", { configurable: true, get: function () { return real; }, set: function (v) { if (refuse) throw new Error("children"); real = v; } });
        // Publishing fails only once the list's second item has failed.
        var bad = ["x", { toString: function () { refuse = true; throw new Error("no text"); } }];
        try { w.add("dropdownlist", undefined, bad); } catch (e) { $.writeln(e.message); }
        refuse = false;
        var ok = w.add("button", undefined, "OK");
        $.writeln(ok.parent === w, " ", w.children.length, " ", w.children[0] === ok);
    "#;
    assert_eq!(both(src, vec![]), ["no text", "true 1 true"]);
}

/// An item's `selected` setter that assigns `selection` again wins, and the flags agree with it:
/// the assignment it interrupted neither keeps its own choice nor undoes the flags.
#[test]
fn a_selection_made_by_an_items_setter_agrees_with_the_flags() {
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox", undefined, ["A", "B"]);
        var a = l.items[0], b = l.items[1], once = true, flag = { a: false, b: false };
        Object.defineProperty(a, "selected", { configurable: true, get: function () { return flag.a; }, set: function (v) { flag.a = v; } });
        Object.defineProperty(b, "selected", { configurable: true, get: function () { return flag.b; }, set: function (v) { flag.b = v; if (once) { once = false; l.selection = b; } } });
        l.selection = a;
        $.writeln(l.selection === b, " ", a.selected, " ", b.selected);
    "#;
    assert_eq!(both(src, vec![]), ["true false true"]);
}

/// The index the user chose names the item they saw: a getter of the script's that removes an
/// item while the window is read does not turn the choice into the item after it.
#[test]
fn a_list_choice_names_the_item_the_user_saw() {
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox", undefined, ["A", "B"]), e = w.add("edittext"), once = true;
        Object.defineProperty(e, "text", { configurable: true, get: function () { if (once) { once = false; l.remove(0); } return ""; }, set: function (v) {} });
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(l.selection === null ? "none" : l.selection.text);
    "#;
    // window 0, listbox 1, edittext 2, OK 3; the user picks the first item they saw, A.
    let events = vec![json!({"event": "click", "id": 3, "values": [{"id": 1, "value": 0}]})];
    assert_eq!(both(src, events), ["none"]);
}

/// An item whose list could not publish its `items` is out of the array the script holds too,
/// and no longer the selection a setter gave it meanwhile.
#[test]
fn an_unpublished_item_leaves_the_kept_array_and_the_selection() {
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox");
        l.add("item", "a");
        var arr = l.items;
        Object.defineProperty(l, "items", { configurable: true, get: function () { return arr; }, set: function (v) { l.selection = v[v.length - 1]; throw new Error("no"); } });
        try { l.add("item", "b"); } catch (e) {}
        $.writeln(arr.length, " ", arr[0].text, " ", l.selection === null);
    "#;
    assert_eq!(both(src, vec![]), ["1 a true"]);
}

/// A list's reply is checked against the items as they are once `enabled` was read (its getter
/// may change them), and applied in one assignment, so an item's `selected` setter that chooses
/// another item keeps that choice.
#[test]
fn a_list_reply_is_applied_after_the_controls_getters_and_once() {
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox", undefined, ["A", "B", "C"]), reads = 0;
        l.selection = 1;
        var on = true;
        // Read once as the window is sent, then as the reply is applied: A goes at that one.
        Object.defineProperty(l, "enabled", { configurable: true, get: function () { if (++reads == 2) l.remove(0); return on; }, set: function (v) { on = v; } });
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(l.selection.text);
    "#;
    // window 0, listbox 1, OK 2: the user picks A, which the getter took out before the reply.
    assert_eq!(both(src, vec![json!({"event": "click", "id": 2, "values": [{"id": 1, "value": 0}]})]), ["B"]);
    let src = r#"
        var w = new Window("dialog"), l = w.add("listbox", undefined, ["A", "B"]), once = true, flag = false;
        var a = l.items[0], b = l.items[1];
        Object.defineProperty(a, "selected", { configurable: true, get: function () { return flag; }, set: function (v) { flag = v; if (v && once) { once = false; l.selection = b; } } });
        w.add("button", undefined, "OK");
        w.show();
        $.writeln(l.selection.text);
    "#;
    assert_eq!(both(src, vec![json!({"event": "click", "id": 2, "values": [{"id": 1, "value": 0}]})]), ["B"]);
}
