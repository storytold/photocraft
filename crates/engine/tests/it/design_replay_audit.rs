//! Every journaled command, recorded into a design timeline and replayed, must give the same
//! document. A command that can't belongs in `photocraft_engine::design_cmds::BAKED`, whose steps
//! replay their stored result instead. Opt-in (slow):
//! `cargo test -p photocraft-engine --test it design_replay_audit -- --ignored --nocapture`.
use std::sync::mpsc;
use std::time::Duration;

use photocraft_engine::{Session, command_specs};
use serde_json::{Value, json};

/// A filled pixel layer A under a selection, then (by `kind`) an active text or shape layer, no
/// selection, or a filled clipboard, Fade source, last filter and last transform.
fn fixture(kind: usize) -> Session {
    photocraft_engine::print_cmds::disable_printing();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 8})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    match kind {
        1 => drop(s.execute("type.create", json!({"x": 2, "y": 10, "text": "Hi", "size": 8, "color": "#000000", "name": "T"})).unwrap()),
        2 => drop(s.execute("shape.create", json!({"kind": "rect", "rect": [4, 4, 10, 6], "fill": "#0000ff"})).unwrap()),
        3 => drop(s.execute("select.deselect", json!({})).unwrap()),
        4 => {
            s.execute("edit.copy", json!({})).unwrap();
            s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
            s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 1, 0], "interpolation": "nearest"})).unwrap();
        }
        _ => {}
    }
    s.execute("design.start", json!({})).unwrap();
    s
}

const FIXTURES: usize = 5;

/// The active document without its timeline, to compare content.
fn content(s: &Session) -> photocraft_doc::Document {
    let mut d = (*s.active().unwrap().doc).clone();
    d.design_timeline = None;
    d
}

/// Commands that leave the process (files, printers, plug-ins, the clipboard of the OS) or are
/// the timeline's own.
const SKIP: &[&str] = &["file.", "design.", "plugin.", "session.", "edit.undo", "edit.redo", "history.", "test."];

enum Outcome {
    NotRun,
    Same,
    Differs(String),
}

/// Run command `id` on fixture `kind`, replay the timeline and compare the results.
fn audit(id: &'static str, kind: usize, p: Value) -> Outcome {
    let mut s = fixture(kind);
    let n = |s: &mut Session| s.execute("design.list", json!({})).map(|v| v["steps"].as_array().map_or(0, Vec::len)).unwrap_or(0);
    if s.execute(id, p).is_err() || n(&mut s) == 0 {
        return Outcome::NotRun;
    }
    let recorded = content(&s);
    match s.execute("design.replay", json!({})) {
        Err(e) => Outcome::Differs(format!("replay failed: {e}")),
        Ok(v) => {
            let failed: Vec<&Value> = v["steps"].as_array().into_iter().flatten().filter(|x| x["state"]["state"] != "ok").collect();
            if !failed.is_empty() {
                Outcome::Differs(format!("{failed:?}"))
            } else if content(&s) != recorded {
                Outcome::Differs("different document".into())
            } else {
                Outcome::Same
            }
        }
    }
}

#[test]
#[ignore = "slow full-registry replay audit; run with --ignored --nocapture"]
fn every_command_replays_identically() {
    let params = [
        json!({}),
        json!({"radius": 2, "amount": 30, "angle": 15, "opacity": 0.5, "x": 3, "y": 3, "width": 6, "height": 5}),
        json!({"name": "N", "text": "Yo", "value": 1, "size": 4, "color": "#ff0000", "mode": "multiply", "points": [[2, 2], [10, 8]], "kind": "rect", "rect": [2, 2, 6, 4]}),
    ];
    let (mut same, mut not_run, mut differs) = (0, 0, Vec::new());
    for spec in command_specs().iter().filter(|c| c.journal && !SKIP.iter().any(|p| c.id.starts_with(p))) {
        let mut outcome = Outcome::NotRun;
        'tries: for kind in 0..FIXTURES {
            for p in &params {
                let (tx, rx) = mpsc::channel();
                let (id, p) = (spec.id, p.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(audit(id, kind, p));
                });
                outcome = rx.recv_timeout(Duration::from_secs(20)).unwrap_or(Outcome::Differs("timed out".into()));
                if !matches!(outcome, Outcome::NotRun) {
                    break 'tries;
                }
            }
        }
        match outcome {
            Outcome::NotRun => not_run += 1,
            Outcome::Same => same += 1,
            Outcome::Differs(why) => differs.push(format!("{} — {why}", spec.id)),
        }
    }
    println!("replay audit: {same} identical, {} differ, {not_run} not run on the fixture", differs.len());
    for d in &differs {
        println!("  {d}");
    }
    assert!(differs.is_empty(), "{} commands don't replay identically; bake them (design_cmds::BAKED) or fix them", differs.len());
}
