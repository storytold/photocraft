//! Runs in a real script host process: every way a script can go wrong ends in a named failure
//! within its limit, and this (the app's) process carries on.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use photocraft_engine::Session;
#[cfg(unix)]
use photocraft_script_js::isolate::Crash;
use photocraft_script_js::isolate::{CrashCause, Event, Failure, FailureKind, HelperCommand, IsolateOptions, IsolatedJob, Reply};
use photocraft_script_js::{ModalKind, RunOptions};
use serde_json::{Value, json};

const LIMIT: Duration = Duration::from_secs(3);
/// For a test whose script or host does a lot of work before its point (in a debug build on a
/// loaded CI runner): the point is not the time limit.
const SLOW_LIMIT: Duration = Duration::from_secs(30);
/// How far past its limit a stopped run may report.
/// How far past a limit a run may still be stopping: generous, for loaded CI runners on every
/// platform; what is checked is that it stops, not how fast.
const GRACE: Duration = Duration::from_secs(5);

/// The full name the test harness knows test `f` of this module by (`isolate::f`), for running
/// it alone in a copy of this test binary.
#[cfg(unix)]
fn test_name(f: &str) -> String {
    match module_path!().split_once("::") {
        Some((_, module)) => format!("{module}::{f}"),
        None => f.to_owned(),
    }
}

/// The host executable: one that serves `--script-host` (such as a release build of this one)
/// named by `PHOTOCRAFT_TEST_SCRIPT_HOST`, or else this crate's own. A test run from a nextest
/// archive finds it where nextest unpacked it (`NEXTEST_BIN_EXE_*`), not at the path it was built
/// at, which is on another machine.
fn host_program() -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_TEST_SCRIPT_HOST")
        .or_else(|| std::env::var_os("NEXTEST_BIN_EXE_photocraft_script_host"))
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_photocraft-script-host").into())
        .into()
}

fn options(key: &str) -> IsolateOptions {
    IsolateOptions {
        run: RunOptions::default(),
        time_limit: LIMIT,
        memory_limit: photocraft_script_js::isolate::DEFAULT_MEMORY_LIMIT,
        coalesce: key.into(),
        helper: HelperCommand::new(host_program()),
        reply_limits: (photocraft_script_js::isolate::COMMAND_REPLY_LIMIT, photocraft_script_js::isolate::DIALOG_REPLY_LIMIT),
        queue_grace: photocraft_script_js::isolate::QUEUE_GRACE,
    }
}

/// What the interface saw of a run.
struct Seen {
    result: Result<(), Failure>,
    lines: Vec<String>,
    dialogs: Vec<ModalKind>,
    elapsed: Duration,
}

/// Drive a run like an interface: commands go to `session` (or get an empty session's answer),
/// dialogs take `answers` in order, `each` sees every event first.
fn drive(job: &mut IsolatedJob, mut session: Option<&mut Session>, answers: &[Value], mut each: impl FnMut(&mut IsolatedJob, &Event)) -> Seen {
    let start = Instant::now();
    let mut answers: VecDeque<Value> = answers.iter().cloned().collect();
    let (mut lines, mut dialogs) = (Vec::new(), Vec::new());
    loop {
        assert!(start.elapsed() < Duration::from_secs(60), "the run never finished");
        let Some(event) = job.poll(Duration::from_millis(20)) else { continue };
        each(job, &event);
        match event {
            Event::Execute { id, params } => {
                let result = match session.as_deref_mut() {
                    Some(s) => s.execute(&id, params).map_err(|e| e.to_string()),
                    None => Ok(json!({"active": null, "documents": []})),
                };
                let _ = job.reply(Reply::Executed(result));
            }
            Event::Output(line) => lines.push(line),
            Event::Dialog(m) => {
                dialogs.push(m.kind);
                let _ = job.reply(Reply::Dialog(answers.pop_front().unwrap_or(Value::Null)));
            }
            Event::Finished(result) => return Seen { result, lines, dialogs, elapsed: start.elapsed() },
        }
    }
}

fn run(src: &str) -> Seen {
    let mut job = IsolatedJob::spawn(src.into(), options("run")).unwrap();
    drive(&mut job, None, &[], |_, _| {})
}

fn failure(seen: &Seen) -> &Failure {
    match &seen.result {
        Err(f) => f,
        Ok(()) => panic!("the run succeeded; output {:?}", seen.lines),
    }
}

fn report(name: &str, seen: &Seen) {
    eprintln!("{name}: {:?} in {:?}", seen.result.as_ref().map_err(|f| (&f.kind, &f.message)), seen.elapsed);
}

#[test]
fn a_script_with_dialogs_runs_in_one_host_and_writes_its_output_once() {
    let src = "$.writeln('before'); var ok = confirm('Go?'); var n = prompt('Name', 'x'); alert('hi'); $.writeln(ok, ' ', n, ' ', app.documents.length);";
    let mut job = IsolatedJob::spawn(src.into(), options("dialogs")).unwrap();
    let host = job.host_id();
    let mut hosts = Vec::new();
    let seen = drive(&mut job, None, &[json!(true), json!("Ann")], |j, _| hosts.push(j.host_id()));
    report("dialogs", &seen);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["before", "true Ann 0"]);
    assert_eq!(seen.dialogs, [ModalKind::Confirm, ModalKind::Prompt, ModalKind::Alert]);
    assert!(hosts.iter().all(|h| *h == host), "one host for the whole run");
}

/// A dialog whose text passes the per-string bound but not, encoded, the frame's (control
/// characters take six bytes each in JSON) never reaches the app, and nothing waits for an
/// answer: the run is stopped by name.
#[test]
fn a_dialog_too_large_to_send_stops_the_run_before_any_wait() {
    let src = r#"
        try { alert(new Array(12 * 1024 * 1024 + 1).join("\u0001")); $.writeln("shown"); } catch (e) { $.writeln("caught"); }
    "#;
    let mut opts = options("bigdialog");
    opts.time_limit = SLOW_LIMIT;
    let mut job = IsolatedJob::spawn(src.into(), opts).unwrap();
    let seen = drive(&mut job, None, &[], |_, _| {});
    report("big dialog", &seen);
    assert!(matches!(&seen.result, Err(f) if f.message.contains("The dialog could not be shown") && f.message.contains("over the limit")), "{:?}", seen.result);
    assert!(seen.lines.is_empty(), "{:?}", seen.lines);
    assert!(seen.dialogs.is_empty(), "no dialog reached the app");
}

#[test]
fn time_at_an_open_dialog_is_not_charged() {
    let mut opts = options("slow-answer");
    opts.time_limit = Duration::from_millis(800);
    let mut job = IsolatedJob::spawn("alert('wait'); $.writeln('after');".into(), opts).unwrap();
    let seen = drive(&mut job, None, &[], |_, e| {
        if matches!(e, Event::Dialog(_)) {
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.lines, ["after"]);
}

#[test]
fn an_endless_loop_inside_a_callee_frame_times_out() {
    // Each call gets a fresh loop counter, so only the app's clock can stop this.
    let seen = run("function inner() { for (var i = 0; i < 99999990; i++) {} } for (;;) inner();");
    report("callee loop", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Timeout);
    assert!(seen.elapsed <= LIMIT + GRACE, "{:?}", seen.elapsed);
}

/// Boa 0.22 drops a loop-limit error raised after an `await`, so the host runs without that
/// limit and the app's clock stops these loops with the time-limit error, never a silent end.
#[test]
fn an_endless_loop_after_an_await_times_out_by_name() {
    for src in ["async function f(){ await 0; while(true){} } f();", "while(true){}"] {
        let seen = run(src);
        report(src, &seen);
        let f = failure(&seen);
        assert_eq!(f.kind, FailureKind::Timeout, "{src}");
        assert!(f.message.contains("time limit"), "{src}: {}", f.message);
        assert!(seen.elapsed <= LIMIT + GRACE, "{src}: {:?}", seen.elapsed);
    }
}

#[test]
fn catastrophic_regex_backtracking_times_out() {
    let seen = run("/^(a+)+$/.test('a'.repeat(40) + '!'); $.writeln('returned');");
    report("redos", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Timeout);
    assert!(seen.lines.is_empty());
    assert!(seen.elapsed <= LIMIT + GRACE, "{:?}", seen.elapsed);
}

#[test]
fn a_gigantic_string_hits_the_memory_limit() {
    let seen = run("var s = 'x'.repeat(Math.pow(2, 30)); var t = s + s; $.writeln(t.length);");
    report("repeat 2^30", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::MemoryLimit, "{}", failure(&seen));
    assert!(seen.elapsed <= LIMIT + GRACE, "{:?}", seen.elapsed);
}

/// The heap cap holds by itself, as it must on systems without a memory rlimit: a buffer over
/// the heap's limit but under the run's (a zeroed one, whose pages the system fills lazily)
/// is refused by the cap alone; the resident size never passes the limit.
#[test]
fn the_heap_cap_refuses_memory_the_host_never_touches() {
    let mut opts = options("cap");
    opts.memory_limit = photocraft_script_js::isolate::MIN_MEMORY_LIMIT;
    opts.time_limit = SLOW_LIMIT;
    opts.helper.envs.push(("PHOTOCRAFT_TEST_NO_RLIMIT".into(), "1".into()));
    let heap = photocraft_script_js::isolate::MIN_MEMORY_LIMIT - photocraft_script_js::isolate::STACK as u64;
    let src = format!("var b = new ArrayBuffer({}); $.writeln('kept ' + b.byteLength);", heap + (64 << 20));
    let mut job = IsolatedJob::spawn(src, opts).expect("the host starts");
    let seen = drive(&mut job, None, &[], |_, _| {});
    report("untouched buffer", &seen);
    // The engine either throws (its allocation is fallible) or the host ends at the failed
    // allocation; either way the buffer is never made.
    let f = failure(&seen);
    assert!(f.kind == FailureKind::MemoryLimit || f.message.contains("while allocating"), "{f}");
    assert!(seen.lines.is_empty(), "{:?}", seen.lines);
}

#[test]
fn source_nested_a_hundred_thousand_deep_fails_cleanly() {
    // Unclosed brackets: either the parser reports a syntax error or it overflows the host's stack.
    let seen = run(&"[".repeat(100_000));
    report("100k brackets", &seen);
    let f = failure(&seen);
    let named = match f.kind {
        FailureKind::Script => true,
        FailureKind::Crashed(c) => c.cause == Some(CrashCause::StackOverflow),
        _ => false,
    };
    assert!(named, "{f:?}");
    assert!(seen.elapsed <= LIMIT + GRACE, "{:?}", seen.elapsed);
}

#[test]
fn a_deep_proxy_chain_ends_cleanly() {
    let seen = run("var p = {}; for (var i = 0; i < 1000000; i++) p = new Proxy(p, {}); $.writeln('built'); p.x; $.writeln('returned');");
    report("proxy chain", &seen);
    // Success is fine too; what matters is a named end and a living app.
    if let Err(f) = &seen.result {
        assert!(matches!(f.kind, FailureKind::Script | FailureKind::Crashed(_) | FailureKind::MemoryLimit | FailureKind::Timeout), "{f:?}");
    }
    assert!(seen.elapsed <= LIMIT + GRACE, "{:?}", seen.elapsed);
}

/// SIGKILL from outside, as the system's out-of-memory killer would.
#[cfg(unix)]
fn kill_host(job: &IsolatedJob) {
    let status = std::process::Command::new("kill").args(["-9", &job.host_id().to_string()]).status().unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn a_host_killed_mid_run_fails_the_run_as_killed() {
    let key = "killed-run";
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut job = IsolatedJob::spawn("for (;;) { app.documents.length; }".into(), options(key)).unwrap();
    let mut commands = 0;
    let seen = drive(&mut job, Some(&mut session), &[], |j, e| {
        if let Event::Execute { params, .. } = e {
            assert_eq!(params["coalesce"], json!(key), "every command carries the run's key");
            commands += 1;
            if commands == 20 {
                kill_host(j);
            }
        }
    });
    report("killed mid-run", &seen);
    let f = failure(&seen);
    let FailureKind::Crashed(crash) = f.kind else { panic!("{f:?}") };
    assert_eq!((crash.signal, crash.cause), (Some(9), Some(CrashCause::Killed)), "{}", f.message);
    assert!(commands >= 20);
}

#[cfg(unix)]
#[test]
fn a_dialog_open_when_the_host_dies_is_dropped() {
    let mut job = IsolatedJob::spawn("confirm('still there?'); $.writeln('after');".into(), options("dialog-death")).unwrap();
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(20));
        if let Some(Event::Dialog(_)) = job.poll(Duration::from_millis(20)) {
            break;
        }
    }
    kill_host(&job);
    let finished = loop {
        assert!(start.elapsed() < Duration::from_secs(20));
        match job.poll(Duration::from_millis(20)) {
            Some(Event::Finished(r)) => break r,
            Some(e) => panic!("nothing but the end arrives: {e:?}"),
            None => {}
        }
    };
    assert!(matches!(finished, Err(Failure { kind: FailureKind::Crashed(_), .. })), "{finished:?}");
    assert!(job.reply(Reply::Dialog(json!(true))).is_err(), "the dialog's answer has no taker");
    assert!(job.is_finished());
    assert!(job.poll(Duration::from_millis(10)).is_none());
}

#[test]
fn cancel_stops_a_busy_host() {
    let mut job = IsolatedJob::spawn("for (;;) {}".into(), options("cancel")).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    job.cancel();
    let seen = drive(&mut job, None, &[], |_, _| {});
    assert_eq!(failure(&seen).kind, FailureKind::Cancelled);
    assert!(seen.elapsed < Duration::from_secs(10), "{:?}", seen.elapsed);
}

#[test]
fn script_errors_come_back_as_script_failures() {
    let seen = run("$.writeln('a'); null.x;");
    assert_eq!(failure(&seen).kind, FailureKind::Script);
    assert_eq!(seen.lines, ["a"]);
    let seen = run("#include \"a.jsx\"");
    assert_eq!(failure(&seen).kind, FailureKind::Script);
}

#[test]
fn script_errors_from_the_host_name_the_script_file() {
    let mut opts = options("named");
    opts.run.script_name = Some("named-probe.jsx".into());
    let mut job = IsolatedJob::spawn("$.writeln('a');\nthrow new Error('boom');".into(), opts).unwrap();
    let seen = drive(&mut job, None, &[], |_, _| {});
    let f = failure(&seen);
    assert_eq!(f.kind, FailureKind::Script);
    assert!(f.message.contains("(named-probe.jsx:2:7)"), "{}", f.message);
    assert!(!f.message.contains("script:"), "{}", f.message);
    assert!(failure(&run("throw new Error('boom');")).message.contains("(script:1:7)"));
}

#[test]
fn a_missing_host_executable_is_reported() {
    let mut opts = options("missing");
    opts.helper = HelperCommand::new("/nonexistent/photocraft-script-host");
    let Err(f) = IsolatedJob::spawn("1".into(), opts) else { panic!("started") };
    assert_eq!(f.kind, FailureKind::Unavailable);
}

/// `sh -c script`, with `paths` as the script's positional parameters (`"$1"`, `"$2"`, …):
/// passed as arguments, never put into the script's text, so no path can change what it runs.
#[cfg(unix)]
fn sh(script: &str, paths: &[&std::path::Path]) -> HelperCommand {
    let mut args: Vec<std::ffi::OsString> = vec!["-c".into(), script.into(), "stand-in".into()];
    args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
    HelperCommand { program: "sh".into(), args, envs: Vec::new() }
}

/// What a [`stand_in`] run waits for before the interface polls it.
#[cfg(unix)]
#[derive(Clone, Copy)]
enum Settle {
    /// Nothing: the interface polls at once.
    No,
    /// Until the app holds the host's requests void (its output ended, its result came, it
    /// exited), so a request it queued before can't be told from one still live.
    Done,
}

/// Write a fixture file the test made the name of: a new file, never through what was already
/// at the path (a link from an earlier run is removed, not followed).
#[cfg(unix)]
fn fixture(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write as _;
    drop(std::fs::remove_file(path));
    std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap().write_all(bytes).unwrap();
}

/// A stand-in host that writes `frames`, then runs the shell commands `then` (which see `more`
/// as `"$2"`, `"$3"`, …); the interface starts polling as `settle` says. Only the run's end may
/// reach the interface.
#[cfg(unix)]
fn stand_in(name: &str, frames: &[u8], then: &str, more: &[&std::path::Path], settle: Settle) -> Seen {
    // Absolute: the host starts in the temporary folder, so a relative `TMPDIR` would not reach it.
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("script-host-{name}-{}-{}", std::process::id(), frames.len()));
    fixture(&dir, frames);
    let mut opts = options(name);
    let mut paths = vec![dir.as_path()];
    paths.extend_from_slice(more);
    opts.helper = sh(&format!("cat \"$1\"; {then}"), &paths);
    let mut job = IsolatedJob::spawn("1".into(), opts).unwrap();
    if let Settle::Done = settle {
        let start = Instant::now();
        while !job.requests_void() {
            assert!(start.elapsed() < Duration::from_secs(60), "{name}: the host's requests never became void");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let seen = drive(&mut job, None, &[], |_, e| assert!(matches!(e, Event::Finished(_)), "{name}: a request reached the interface: {e:?}"));
    let _ = std::fs::remove_file(&dir);
    seen
}

/// A stand-in host that writes `frames` and then waits: the app must refuse what it sends.
#[cfg(unix)]
fn rogue(frames: &[u8]) -> Seen {
    stand_in("rogue", frames, "sleep 30", &[], Settle::No)
}

#[cfg(unix)]
fn frame(v: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap();
    let mut out = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    out.extend(body);
    out
}

#[cfg(unix)]
#[test]
fn an_untrusted_host_cannot_reach_other_commands_or_oversize_the_app() {
    for (name, bytes, why) in [
        // Not in the script command table: refused by its id, whatever its params.
        ("other command", frame(&json!({"type": "execute", "id": "file.revert", "params": {}})), Some("is not available to scripts")),
        // In the table, with params it doesn't take.
        ("bad params", frame(&json!({"type": "execute", "id": "document.activate", "params": {"offset": 1}})), Some("is not one it takes")),
        // Neither is a message the protocol has: a file picker, or a window.
        ("file picker", frame(&json!({"type": "dialog", "kind": "save_file", "args": {}})), None),
        ("window", frame(&json!({"type": "window", "id": 0, "window": null})), None),
        ("own history key", frame(&json!({"type": "execute", "id": "session.inspect", "params": {"coalesce": "someone-else"}})), None),
        ("oversize", vec![0xFF, 0xFF, 0xFF, 0x7F], None),
        ("malformed", frame(&json!({"type": "format_disk"})), None),
    ] {
        let seen = rogue(&bytes);
        report(name, &seen);
        let f = failure(&seen);
        assert_eq!(f.kind, FailureKind::HostViolation, "{name}");
        if let Some(why) = why {
            assert!(f.message.contains(why), "{name}: {}", f.message);
        }
        assert!(seen.elapsed < Duration::from_secs(8), "{name}: {:?}", seen.elapsed);
    }
}

/// Set for `late_marker_writer` when the late-stderr test runs it as a host's grandchild: the
/// file the writer creates once it runs. `LATE_MARKER_HOST` is the host's pid.
#[cfg(unix)]
const LATE_MARKER: &str = "PHOTOCRAFT_TEST_LATE_MARKER";
#[cfg(unix)]
const LATE_MARKER_HOST: &str = "PHOTOCRAFT_TEST_LATE_MARKER_HOST";

/// Not a test of its own (it does nothing in a normal run): run by
/// `a_memory_marker_read_after_the_host_exited_still_counts` as a grandchild of a stand-in host,
/// it starts a writer in a new process group that says it is ready, waits until the host is
/// gone and only then writes the memory marker to the host's standard error.
#[cfg(unix)]
#[test]
fn late_marker_writer() {
    use std::os::unix::process::CommandExt;
    let (Some(ready), Some(host)) = (std::env::var_os(LATE_MARKER), std::env::var_os(LATE_MARKER_HOST)) else { return };
    // The writer must outlive this process (it is the point): the `sh` started here puts it in
    // the background and exits at once, so it is reaped here and init adopts the writer.
    // Bounded waits (10 s): a host that never goes leaves no writer behind for long.
    let script = r#"( touch "$1"; i=0; while kill -0 "$2" 2>/dev/null && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; echo 'memory allocation of 5 bytes failed' >&2 ) &"#;
    let started = std::process::Command::new("sh")
        .args([std::ffi::OsStr::new("-c"), std::ffi::OsStr::new(script), std::ffi::OsStr::new("writer"), &ready, &host])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .process_group(0)
        .status()
        .unwrap();
    assert!(started.success());
}

/// A host that dies of a failed allocation writes the marker to its standard error and aborts.
/// The watchdog can reap it before the marker has been read; the crash is still classified
/// as the memory limit, not as an abort. Here the marker arrives late on purpose, from a
/// grandchild that keeps the pipe open after the host itself is gone.
#[cfg(unix)]
#[test]
fn a_memory_marker_read_after_the_host_exited_still_counts() {
    // The grandchild lets go of standard output, so the app sees the host's output end at once;
    // only standard error stays open, as a host's last write can still be in the pipe. It runs
    // in a process group of its own (started by this test binary, `late_marker_writer`, as
    // nothing but `sh` is on every Unix runner), so stopping the host's group on exit leaves it
    // to deliver the marker late, as a pipe being drained would. The host aborts once the writer
    // says it runs, and the writer writes once the host is gone. That order is fixed; what is
    // timed is the supervisor's grace for a last write after the host exits (`STDERR_GRACE`,
    // 500 ms), which the writer has to meet once it sees the host gone.
    let exe = std::env::current_exe().unwrap();
    let ready = std::env::temp_dir().canonicalize().unwrap().join(format!("script-host-late-marker-ready-{}", std::process::id()));
    drop(std::fs::remove_file(&ready));
    let script = format!(
        "{LATE_MARKER}=\"$3\" {LATE_MARKER_HOST}=$$ \"$2\" --exact {} --quiet >/dev/null & i=0; while [ ! -e \"$3\" ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; kill -ABRT $$",
        test_name("late_marker_writer")
    );
    let seen = stand_in("late-stderr", &[], &script, &[&exe, &ready], Settle::No);
    drop(std::fs::remove_file(&ready));
    report("late stderr", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::MemoryLimit);
}

/// The script host waits at each dialog for its answer: a run with many dialogs finishes in one
/// pass, its lines run once.
#[test]
fn many_dialogs_are_answered_in_one_pass() {
    let mut job =
        IsolatedJob::spawn("var n = 0; for (var i = 0; i < 1200; i++) { if (confirm('q' + i)) { n++; } } $.writeln('n=' + n);".into(), options("dialogs"))
            .unwrap();
    let answers = vec![Value::Bool(true); 1200];
    let seen = drive(&mut job, None, &answers, |_, _| {});
    report("1200 dialogs", &seen);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs.len(), 1200);
    assert_eq!(seen.lines, ["n=1200"]);
}

/// Every output line is sent to and handled by the app, so a run's total output is bounded: 300
/// lines of 1 MiB are stopped with a named error at 32 MiB, well inside the time limit.
#[test]
fn a_flood_of_long_output_lines_is_stopped_by_size() {
    let seen = run("var s = 'x'.repeat(1 << 20); for (var i = 0; i < 300; i++) { $.writeln(s); }");
    report("output flood", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Script);
    assert_eq!(failure(&seen).message, "The script was stopped: it wrote more than 32 MiB of output.");
    assert_eq!(seen.lines.len(), 32);
}

/// A stand-in host: `sh -c` running `script`, with `files` written to the temp dir first and
/// `{name}` in the script standing for its path (a quoted positional parameter).
#[cfg(unix)]
fn shell_host(name: &str, files: &[(&str, Vec<u8>)], script: &str) -> (IsolatedJob, Vec<std::path::PathBuf>) {
    shell_host_within(name, files, script, LIMIT)
}

#[cfg(unix)]
fn shell_host_within(name: &str, files: &[(&str, Vec<u8>)], script: &str, limit: Duration) -> (IsolatedJob, Vec<std::path::PathBuf>) {
    let mut script = script.to_owned();
    let mut paths = Vec::new();
    for (i, (key, bytes)) in files.iter().enumerate() {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!("script-host-{name}-{key}-{}", std::process::id()));
        fixture(&path, bytes);
        script = script.replace(&format!("{{{key}}}"), &format!("\"${}\"", i + 1));
        paths.push(path);
    }
    let mut opts = options(name);
    opts.time_limit = limit;
    opts.helper = sh(&script, &paths.iter().map(std::path::PathBuf::as_path).collect::<Vec<_>>());
    (IsolatedJob::spawn("1".into(), opts).unwrap(), paths)
}

/// The host gets none of the app's environment (cargo sets `CARGO_MANIFEST_DIR` for this test)
/// and starts in the temporary folder, not the app's working directory.
#[cfg(unix)]
#[test]
fn the_host_starts_without_the_apps_environment_or_working_directory() {
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some(), "the test process has the variable");
    let said = |line: &str| frame(&json!({"type": "output", "line": line}));
    let done = frame(&json!({"type": "finished", "error": null}));
    let tmp = std::env::temp_dir().canonicalize().unwrap();
    // The folder the host should start in is a fixture file, compared as text: no character of
    // it is read as shell code.
    let want = tmp.as_os_str().as_encoded_bytes().to_vec();
    let files = [("leaked", [said("leaked"), done.clone()].concat()), ("clean", [said("clean"), done.clone()].concat()), ("want", want)];
    let check = r#"if [ -z "$CARGO_MANIFEST_DIR" ] && [ "$(pwd -P)" = "$(cat {want})" ]; then cat {clean}; else cat {leaked}; fi"#;
    let (mut job, paths) = shell_host("environment", &files, check);
    let seen = drive(&mut job, None, &[], |_, _| {});
    paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
    assert_eq!(seen.lines, ["clean"], "{:?}", seen.result);
    assert_eq!(seen.result, Ok(()));
}

/// The app handles every output line, so it holds the total to the run's limit itself: a host
/// that streams more is stopped, whatever it claims to have counted.
#[cfg(unix)]
#[test]
fn a_host_that_streams_too_much_output_is_stopped() {
    let line = frame(&json!({"type": "output", "line": "x".repeat(1 << 20)}));
    let (mut job, paths) = shell_host("flood", &[("line", line)], "for i in $(seq 300); do cat {line}; done; sleep 30");
    let seen = drive(&mut job, None, &[], |_, _| {});
    paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
    report("output flood host", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::HostViolation);
    assert!(failure(&seen).message.contains("more than 32 MiB of output"), "{}", failure(&seen).message);
    assert_eq!(seen.lines.len(), 32);
}

/// Empty lines cost no bytes: the app also holds the number of lines a host may send.
#[cfg(unix)]
#[test]
fn a_host_that_streams_too_many_empty_lines_is_stopped() {
    let one = frame(&json!({"type": "output", "line": ""}));
    let count = photocraft_script_js::bounds::MAX_CALLS + 10;
    let lines: Vec<u8> = one.iter().copied().cycle().take(one.len() * count).collect();
    let (mut job, paths) = shell_host_within("emptylines", &[("lines", lines)], "cat {lines}; sleep 60", SLOW_LIMIT);
    let seen = drive(&mut job, None, &[], |_, _| {});
    paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
    report("empty line flood host", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::HostViolation);
    assert!(failure(&seen).message.contains("dialogs and output lines"), "{}", failure(&seen).message);
    assert_eq!(seen.lines.len(), photocraft_script_js::bounds::MAX_CALLS);
}

/// A cancel outranks a result the host had already queued, unread behind a busy interface: the
/// run reports it was cancelled.
#[cfg(unix)]
#[test]
fn a_cancel_outranks_a_queued_result() {
    let mut frames = Vec::new();
    for _ in 0..200 {
        frames.extend(frame(&json!({"type": "output", "line": "queued"})));
    }
    frames.extend(frame(&json!({"type": "finished", "error": null})));
    let (mut job, paths) = shell_host("queued", &[("frames", frames)], "cat {frames}; sleep 30");
    // Nothing is polled before the cancel, and the interface's queue holds far fewer than the
    // 200 lines before the result, so the result can't have been read yet however this is
    // scheduled; the wait only lets the queue fill, so the result waits behind it.
    std::thread::sleep(Duration::from_millis(300));
    job.cancel();
    let seen = drive(&mut job, None, &[], |_, _| {});
    paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
    report("queued then cancelled", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Cancelled);
}

/// A result the app has read stands: a cancel after it (the result waits for the interface to
/// poll) does not turn a finished run into a cancelled one.
#[cfg(unix)]
#[test]
fn a_cancel_after_the_result_was_read_leaves_the_result() {
    let finished = frame(&json!({"type": "finished", "error": null}));
    let (mut job, paths) = shell_host("read-result", &[("frames", finished)], "cat {frames}; sleep 30");
    // Evidence the result was read: the app stops the host as soon as it has it.
    let host = job.host_id().to_string();
    assert!(gone(&host), "the host was not stopped after its result");
    job.cancel();
    let seen = drive(&mut job, None, &[], |_, _| {});
    paths.iter().for_each(|p| drop(std::fs::remove_file(p)));
    report("result read, then cancelled", &seen);
    assert_eq!(seen.result, Ok(()));
}

/// Something the host started keeps its output open. Cancelling still ends the run at once,
/// and stops that process too.
#[cfg(unix)]
#[test]
fn a_cancel_is_reported_while_a_descendant_holds_the_output() {
    let (mut job, paths) = shell_host("descendant", &[("pid", Vec::new())], "sleep 30 & echo $! > {pid}; wait");
    // Cancelled once the descendant is there to be stopped, not after a guess at how long that takes.
    let pid = live_descendant(&paths[0]);
    job.cancel();
    let seen = drive(&mut job, None, &[], |_, _| {});
    drop(std::fs::remove_file(&paths[0]));
    report("descendant, cancelled", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Cancelled);
    assert!(seen.elapsed < Duration::from_secs(10), "{:?}", seen.elapsed);
    assert!(gone(&pid), "the host's descendant {pid} outlived the run");
}

/// So does the watchdog's stop at the time limit: the run ends as timed out, and what the host
/// started is stopped with it.
#[cfg(unix)]
#[test]
fn a_timeout_stops_a_descendant_that_holds_the_output() {
    let (mut job, paths) = shell_host_within("descendant-timeout", &[("pid", Vec::new())], "sleep 30 & echo $! > {pid}; wait", Duration::from_secs(2));
    let pid = live_descendant(&paths[0]);
    let seen = drive(&mut job, None, &[], |_, _| {});
    drop(std::fs::remove_file(&paths[0]));
    report("descendant, timed out", &seen);
    assert_eq!(failure(&seen).kind, FailureKind::Timeout);
    assert!(seen.elapsed < Duration::from_secs(12), "{:?}", seen.elapsed);
    assert!(gone(&pid), "the host's descendant {pid} outlived the run");
}

/// The pid a stand-in host wrote to `file` once that process is running, waiting at most 10 s.
#[cfg(unix)]
fn live_descendant(file: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pid = std::fs::read_to_string(file).unwrap_or_default().trim().to_owned();
        let running =
            !pid.is_empty() && std::process::Command::new("kill").args(["-0", &pid]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if running {
            return pid;
        }
        assert!(Instant::now() < deadline, "no live descendant appeared in {}", file.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The host exits without a result while something it started keeps its output open: the run
/// is reported after a short grace instead of when that process ends.
#[cfg(unix)]
#[test]
fn a_host_that_exits_while_a_descendant_holds_the_output_is_reported() {
    let (mut job, paths) = shell_host("orphan", &[("pid", Vec::new())], "(sleep 30 & echo $! > {pid}); sleep 0.2; exit 3");
    let pid_file = paths[0].clone();
    let seen = drive(&mut job, None, &[], |_, _| {});
    report("orphan", &seen);
    assert!(matches!(failure(&seen).kind, FailureKind::Crashed(Crash { code: Some(3), .. })), "{:?}", failure(&seen));
    assert!(seen.elapsed < Duration::from_secs(10), "{:?}", seen.elapsed);
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    drop(std::fs::remove_file(&pid_file));
    // OpenBSD sees a host that exits on its own only once it is reaped, so its group is not
    // signalled then (`isolate::limits`): what it started may outlive it there.
    if cfg!(not(target_os = "openbsd")) {
        assert!(gone(pid.trim()), "the host's descendant {} outlived the host", pid.trim());
    }
}

/// Whether process `pid` ends within a few seconds.
#[cfg(unix)]
fn gone(pid: &str) -> bool {
    // `kill` must run: a failure to start it says nothing about the process.
    let signalable = || std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().expect("kill runs").success();
    // On Linux a zombie still takes a signal, though it has stopped and closed its pipes; a
    // container that is slow to reap orphans leaves one behind.
    #[cfg(target_os = "linux")]
    let alive = || signalable() && !pid.parse().is_ok_and(exited);
    #[cfg(not(target_os = "linux"))]
    let alive = signalable;
    let start = Instant::now();
    while alive() && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
    }
    !alive()
}

#[test]
fn a_run_started_from_a_thread_that_ends_still_completes() {
    let spawner = std::thread::spawn(|| {
        let job = IsolatedJob::spawn("confirm('still there?'); $.writeln('after');".into(), options("short-lived-spawner")).unwrap();
        // The host is up and settled before the starting thread ends.
        std::thread::sleep(Duration::from_millis(300));
        job
    });
    let mut job = spawner.join().unwrap();
    // Long enough for anything tied to the starting thread to have ended the host.
    std::thread::sleep(Duration::from_millis(700));
    let seen = drive(&mut job, None, &[json!(true)], |_, _| {});
    report("short-lived spawner", &seen);
    assert_eq!(seen.result, Ok(()));
    assert_eq!(seen.dialogs, [ModalKind::Confirm]);
    assert_eq!(seen.lines, ["after"]);
}

/// Set in the copy of this test binary that stands in for the app in
/// `the_host_exits_when_the_app_dies`.
#[cfg(target_os = "linux")]
const STAND_IN_APP: &str = "PHOTOCRAFT_TEST_STAND_IN_APP";

/// Whether process `pid` has exited (gone, or a zombie nobody reaped yet).
#[cfg(target_os = "linux")]
fn exited(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        // The state follows the parenthesised command name.
        Ok(stat) => stat.rsplit_once(')').is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_host_exits_when_the_app_dies() {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    use photocraft_script_js::isolate::protocol::{MAX_TO_HOST, ToHost, write_frame};

    if std::env::var_os(STAND_IN_APP).is_some() {
        // The stand-in app: start a host on a busy script, report its id, and wait to be killed.
        let mut host = Command::new(host_program())
            .arg(photocraft_script_js::isolate::SCRIPT_HOST_FLAG)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = ToHost::Start {
            source: "for (;;) {}".into(),
            app_name: None,
            script_name: None,
            memory_limit: photocraft_script_js::isolate::DEFAULT_MEMORY_LIMIT,
        };
        write_frame(host.stdin.as_mut().unwrap(), &start, MAX_TO_HOST).unwrap();
        println!("host-pid {}", host.id());
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(60));
        // Only reached when nobody killed this stand-in.
        let _ = host.kill();
        let _ = host.wait();
        return;
    }
    let mut app = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test_name("the_host_exits_when_the_app_dies"), "--nocapture", "--test-threads=1"])
        .env(STAND_IN_APP, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(app.stdout.take().unwrap()).lines();
    let pid: u32 = loop {
        let line = lines.next().expect("the stand-in app reported no host").unwrap();
        // The test harness may print the test's name on the same line first.
        if let Some((_, pid)) = line.split_once("host-pid ") {
            break pid.trim().parse().unwrap();
        }
    };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!exited(pid), "the host is running before the app dies");
    // The app dies without closing anything itself; the system closes its end of the pipe.
    app.kill().unwrap();
    app.wait().unwrap();
    let start = Instant::now();
    while !exited(pid) {
        assert!(start.elapsed() < Duration::from_secs(10), "the host outlived the app by {:?}", start.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("host gone {:?} after the app died", start.elapsed());
}

#[cfg(unix)]
#[test]
fn a_request_still_queued_when_the_host_is_done_is_dropped() {
    let command = frame(&json!({"type": "execute", "id": "document.activate", "params": {"document": 0}}));
    let finished = frame(&json!({"type": "finished", "error": null}));
    for (name, frames, then) in [
        // Its output ended while the process lingers on: only the end of its output makes
        // the request void, not the process ending.
        ("output closed", command.clone(), "exec 1>&-; sleep 30"),
        // It exited without waiting for the answer.
        ("exited", command.clone(), "exit 0"),
        // It reported its result without waiting for the answer.
        ("finished", [command.clone(), finished].concat(), "sleep 30"),
    ] {
        let seen = stand_in("done-host", &frames, then, &[], Settle::Done);
        report(name, &seen);
        if name == "finished" {
            assert_eq!(seen.result, Ok(()), "{name}");
        } else {
            assert!(seen.result.is_err(), "{name}");
        }
    }
}

#[cfg(unix)]
#[test]
fn a_host_that_lingers_after_closing_its_output_is_stopped_by_the_app() {
    let seen = stand_in("lingering", &[], "exec 1>&-; exec sleep 30", &[], Settle::No);
    report("lingering", &seen);
    let f = failure(&seen);
    assert_eq!(f.kind, FailureKind::HostViolation, "{}", f.message);
    assert!(f.message.contains("kept running after it closed its output"), "{}", f.message);
    assert!(seen.elapsed < Duration::from_secs(10), "{:?}", seen.elapsed);
}

#[test]
fn a_script_over_the_message_limit_fails_by_name() {
    let limit = photocraft_script_js::isolate::protocol::MAX_TO_HOST;
    let src = format!("//{}", "x".repeat(limit));
    // That it is refused before it is encoded is checked in the supervisor's own tests.
    let mut job = IsolatedJob::spawn(src, options("oversize script")).unwrap();
    let seen = drive(&mut job, None, &[], |_, _| {});
    report("oversize script", &seen);
    let f = failure(&seen);
    assert_eq!(f.kind, FailureKind::Script, "{}", f.message);
    assert!(f.message.starts_with("The script is too large to run: 257 MiB"), "{}", f.message);
    assert!(seen.elapsed < Duration::from_secs(10), "{:?}", seen.elapsed);
}

#[cfg(unix)]
#[test]
fn an_untrusted_host_can_only_show_dialogs_with_text_arguments() {
    let nested = format!("{}\"deep\"{}", "[".repeat(100), "]".repeat(100));
    let dialog = |args: &str| {
        let body = format!(r#"{{"type":"dialog","kind":"alert","args":{args}}}"#);
        let mut out = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
        out.extend(body.into_bytes());
        out
    };
    for (name, args) in
        [("not a list", "\"hi\"".to_owned()), ("a number", "[1]".to_owned()), ("an object", r#"["a",{"b":"c"}]"#.to_owned()), ("nested lists", nested)]
    {
        let seen = rogue(&dialog(&args));
        report(name, &seen);
        assert_eq!(failure(&seen).kind, FailureKind::HostViolation, "{name}");
        assert!(seen.elapsed < Duration::from_secs(8), "{name}: {:?}", seen.elapsed);
    }
}

/// The script host is its own process: a parser stack overflow there is a named crash, so it
/// runs deeply nested scripts the in-process guard would refuse.
#[test]
fn the_script_host_runs_nesting_the_in_process_guard_refuses() {
    let src = format!("var x = {}1{}; $.writeln(x);", "(".repeat(200), ")".repeat(200));
    let mut job = IsolatedJob::spawn(src, options("deep")).unwrap();
    let seen = drive(&mut job, None, &[], |_, _| {});
    assert_eq!(seen.result, Ok(()), "{:?}", seen.lines);
    assert_eq!(seen.lines, ["1"]);
}

/// The time the app takes to answer a command (a busy or minimized window) is not the script's.
#[test]
fn time_the_app_takes_to_answer_does_not_count_against_the_limit() {
    let mut opts = options("slow-app");
    opts.time_limit = Duration::from_millis(800);
    let mut job = IsolatedJob::spawn("for (var i = 0; i < 6; i++) { app.documents.length; } $.writeln('done');".into(), opts).unwrap();
    let mut delayed = 0;
    let seen = drive(&mut job, None, &[], |_, e| {
        if matches!(e, Event::Execute { .. }) {
            delayed += 1;
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    assert_eq!(delayed, 6, "every command was answered slowly");
    assert_eq!(seen.result, Ok(()), "1.8 s of answering, 0.8 s limit: {:?}", seen.lines);
    assert_eq!(seen.lines, ["done"]);
}

/// A command the app never answers, and a dialog nobody answers, end the run once their own limit
/// passes: the run's clock is stopped meanwhile, so its time limit never would.
#[test]
fn an_unanswered_command_or_dialog_ends_the_run() {
    for (src, dialog) in [("app.documents.length;", false), ("alert('nobody answers');", true)] {
        let mut opts = options("no reply");
        opts.reply_limits = (Duration::from_millis(300), Duration::from_millis(300));
        let mut job = IsolatedJob::spawn(src.into(), opts).unwrap();
        let start = Instant::now();
        let result = loop {
            assert!(start.elapsed() < Duration::from_secs(30), "{src}: the run never ended");
            match job.poll(Duration::from_millis(20)) {
                Some(Event::Finished(result)) => break result,
                // Neither is ever answered.
                Some(Event::Execute { .. }) => assert!(!dialog, "{src}"),
                Some(Event::Dialog(_)) => assert!(dialog, "{src}"),
                _ => {}
            }
        };
        let f = result.expect_err(src);
        assert_eq!(f.kind, FailureKind::Timeout, "{src}: {f}");
        assert!(f.message.contains("got no answer"), "{src}: {f}");
    }
}

/// When the interface falls behind and the queue of events is full, the host waits for room;
/// that wait is the app's, not the script's time.
#[test]
fn time_the_interface_takes_to_take_output_does_not_count_against_the_limit() {
    let mut opts = options("slow interface");
    opts.time_limit = Duration::from_secs(1);
    let mut job = IsolatedJob::spawn("for (var i = 0; i < 300; i++) $.writeln(i);".into(), opts).unwrap();
    // The interface is busy elsewhere for twice the run's limit.
    std::thread::sleep(Duration::from_secs(2));
    let seen = drive(&mut job, None, &[], |_, _| {});
    assert!(seen.result.is_ok(), "{:?}", seen.result);
    assert_eq!(seen.lines.len(), 300);
}

/// That wait counts again past its grace even while it lasts: an interface that stops taking
/// events altogether does not hold the host forever.
#[cfg(target_os = "linux")]
#[test]
fn an_interface_that_stops_taking_output_does_not_hold_the_run_forever() {
    let mut opts = options("stalled interface");
    // Long enough that the queue fills well before the limit, so the block is what ends it.
    opts.time_limit = Duration::from_secs(2);
    opts.queue_grace = Duration::from_millis(200);
    let mut job = IsolatedJob::spawn("for (var i = 0; ; i++) $.writeln(i);".into(), opts).unwrap();
    let host = format!("/proc/{}", job.host_id());
    // Nobody polls: the queue fills and stays full.
    let start = Instant::now();
    while std::path::Path::new(&host).exists() {
        assert!(start.elapsed() < Duration::from_secs(20), "the host still runs while the interface takes nothing");
        std::thread::sleep(Duration::from_millis(20));
    }
    let seen = drive(&mut job, None, &[], |_, _| {});
    assert_eq!(failure(&seen).kind, FailureKind::Timeout, "{:?}", seen.result);
}

/// An answer too large to send ends the run with that reason, not as a host that stopped
/// without a result.
#[test]
fn an_answer_too_large_to_send_ends_the_run_with_the_reason() {
    // Refused before it is encoded: encoding 257 MiB would outlast this limit.
    let mut opts = options("huge answer");
    opts.time_limit = Duration::from_millis(400);
    let mut job = IsolatedJob::spawn("var s = prompt('Name?'); $.writeln(s.length);".into(), opts).unwrap();
    let huge = "x".repeat(photocraft_script_js::isolate::protocol::MAX_TO_HOST + 1);
    let seen = drive(&mut job, None, &[json!(huge)], |_, _| {});
    let f = failure(&seen);
    assert_eq!(f.kind, FailureKind::Script, "{f}");
    assert!(f.message.contains("a dialog's answer could not be sent"), "{f}");
}

/// A command's result within the byte limit but with more values than the host reads is
/// refused to the script as a command error, not sent to break the host's input.
#[test]
fn a_result_with_too_many_values_is_a_command_error() {
    let src = "try { app.documents; $.writeln('sent') } catch (e) { $.writeln('refused ', e.message.indexOf('too large') >= 0) }";
    let mut job = IsolatedJob::spawn(src.into(), options("many values")).unwrap();
    let many = Value::Array(vec![json!(0); photocraft_script_js::isolate::protocol::MAX_FRAME_VALUES + 1]);
    let (start, mut lines) = (Instant::now(), Vec::new());
    let seen = loop {
        assert!(start.elapsed() < Duration::from_secs(60), "the run never finished");
        match job.poll(Duration::from_millis(20)) {
            Some(Event::Execute { .. }) => job.reply(Reply::Executed(Ok(many.clone()))).unwrap(),
            Some(Event::Finished(result)) => break result,
            Some(Event::Output(line)) => lines.push(line),
            _ => {}
        }
    };
    assert_eq!(seen, Ok(()), "{lines:?}");
    assert_eq!(lines, ["refused true"]);
}

/// Runaway recursion after an `await` ends the run by name: it must not finish as a success with
/// the rest of the script silently skipped.
#[test]
fn runaway_recursion_after_an_await_does_not_finish_as_a_success() {
    let src = "function r(n) { return r(n + 1) + 1; } (async function () { await 0; r(0); $.writeln('finished'); })();";
    let mut opts = options("async recursion");
    opts.time_limit = SLOW_LIMIT;
    let mut job = IsolatedJob::spawn(src.into(), opts).unwrap();
    let seen = drive(&mut job, None, &[], |_, _| {});
    report("async recursion", &seen);
    assert!(seen.result.is_err(), "it ended as a success: {:?}", seen.lines);
    assert!(seen.lines.is_empty(), "{:?}", seen.lines);
}
