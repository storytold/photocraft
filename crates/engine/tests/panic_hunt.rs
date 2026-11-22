//! Fuzz every command with adversarial params; a command must return `Err`, never panic or hang.
//! (Rule 9 in AGENTS.md.) Panics are counted by a hook and reported with the command; a hang is
//! ended by nextest's slow-timeout and the last `panic_hunt:` line on stderr names it. Opt-in (slow):
//! `cargo test -p photocraft-engine --test panic_hunt -- --ignored`.
use photocraft_engine::{Session, command_specs};
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once, PoisonError};

fn fresh() -> Session {
    // Belt and braces with CI's PHOTOCRAFT_NO_PRINT: nothing in this process may reach a printer
    // or its driver (file.print.page refuses to ask one, so a hung driver can't hold SPOOLER).
    photocraft_engine::print_cmds::disable_printing();
    let mut s = Session::new();
    // `file.printers` and `file.print.printers` list no printers and do not ask the spooler.
    s.printer_source = Some(std::sync::Arc::new(Vec::new));
    s.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 1, "y": 1, "width": 6, "height": 5})).unwrap();
    s
}

/// Panics anywhere in the process, counted by a hook. `Session::execute` contains a command's
/// panic and returns an ordinary `Err`, so the unwind never reaches `catch_unwind` here; the hook
/// still sees it, on whichever thread (rayon workers included) it happened (#4982).
static PANICS: AtomicUsize = AtomicUsize::new(0);
/// One run at a time, so a panic is charged to the run that caused it.
static RUN: Mutex<()> = Mutex::new(());

/// Run `f` and say whether it finished without panicking, contained or not (returning Ok or a
/// graceful Err both count). A hang has no deadline here: nextest's slow-timeout ends it, and the
/// last `panic_hunt:` line on stderr names the command.
fn run_counted(f: impl FnOnce()) -> bool {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANICS.fetch_add(1, Ordering::SeqCst);
            default(info);
        }));
    });
    let _one = RUN.lock().unwrap_or_else(PoisonError::into_inner);
    let before = PANICS.load(Ordering::SeqCst);
    let ok = catch_unwind(AssertUnwindSafe(f)).is_ok();
    ok && PANICS.load(Ordering::SeqCst) == before
}

/// `Err` with a reason on panic, `Ok` otherwise. Each command runs on a fresh session.
fn run_guarded(id: &str, p: &Value) -> Result<(), String> {
    eprintln!("panic_hunt: {id} <- {p}");
    let (id, p) = (id.to_string(), p.clone());
    if run_counted(move || {
        let mut s = fresh();
        let _ = s.execute(&id, p);
    }) {
        Ok(())
    } else {
        Err("panicked".into())
    }
}

#[test]
#[ignore = "slow full-registry fuzz; run with --ignored"]
fn no_command_panics_or_hangs_on_adversarial_params() {
    let params: Vec<Value> = vec![
        json!({}),
        json!({"x": -9, "y": -9, "radius": 0, "amount": 0, "angle": 0, "opacity": 0, "scale": 0, "levels": 0, "tolerance": 0, "gamma": 0, "layer": 999999, "channel": 999, "index": 999999, "count": 0}),
        json!({"x": 1e5, "y": -1e5, "radius": 300, "amount": 300, "angle": 1e5, "opacity": -100, "scale": -5, "gamma": -1}),
        json!({"points": [], "from": [0, 0], "to": [0, 0], "stops": [], "matrix": [0, 0, 0, 0, 0, 0], "colors": [], "name": "", "mode": "", "style": ""}),
        // Color Range and Replace Color samples (#6280): far off the canvas, malformed, and a
        // click order that doesn't match them.
        json!({"points": [[1e9, -1e9], [0.5, 0.5]], "subtractPoints": [[-1e9, 1e9], [2, 2]], "order": ["+", "-", "x", 5], "localized": true, "range": 0}),
        json!({"points": [[3, 3]], "subtractPoints": [[3]], "order": {}, "fuzziness": -1}),
        json!({"subtractPoints": "none", "order": ["-"], "labMinimum": [0, 0, 0], "labMaximum": [100, 0, 0]}),
    ];
    let mut bad: Vec<String> = Vec::new();
    for spec in command_specs() {
        for p in &params {
            // Print and Print One Copy reach real printers (lp, or the Windows driver): fuzz them
            // as dry runs so the hunt never prints a page.
            let mut p = p.clone();
            if spec.id.starts_with("file.print")
                && let Some(o) = p.as_object_mut()
            {
                o.insert("dryRun".into(), json!(true));
            }
            let p = &p;
            if let Err(why) = run_guarded(spec.id, p) {
                bad.push(format!("{} [{why}] <- {p}", spec.id));
                break;
            }
        }
    }
    assert!(bad.is_empty(), "{} commands panicked:\n{}", bad.len(), bad.join("\n"));
}

/// #4982: a panic the code under test contains still fails the run; a graceful `Err` does not.
#[test]
fn contained_panics_count_and_graceful_errors_do_not() {
    let contained = run_counted(|| {
        let _ = catch_unwind(|| panic!("contained, as Session::execute does"));
    });
    assert!(!contained);
    let graceful = run_counted(|| {
        let mut s = fresh();
        assert!(s.execute("layer.delete", json!({"layer": 999999})).is_err());
    });
    assert!(graceful);
}
