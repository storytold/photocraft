//! Launches an isolated PhotoCraft, resizes its window the way a person does (real mouse input through the
//! OS's modal sizing loop) or the way a program does (`SetWindowPos`), watches it from outside, and judges
//! what it saw with [`plan::judge`].
//!
//! The app is only ever reached through its own control channel and its window handle, found by process id,
//! so a PhotoCraft you have open is never touched; its settings live in a throwaway directory under
//! `target/tmp`.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::control::Control;
use crate::plan::{self, Axes, CursorCheck, Fixed, Kind, Limits, Observations, Probe, Segment, Sweep};
use crate::win::{self, Window};

const OPT_IN: &str = "PHOTOCRAFT_RESIZE_STRESS";

/// What the test reads from the environment (see docs/development.md › Windows resize stress test).
struct Config {
    exe: PathBuf,
    extra_args: Vec<String>,
    drag_seconds: f64,
    soak_seconds: f64,
    seed: u64,
    limits: Limits,
    out_dir: Option<PathBuf>,
    /// The primary display's size in px as the plans see it: the real one, or a smaller one (see [`display_size`]).
    screen: (i32, i32),
}

/// The primary display's size in px. `PHOTOCRAFT_STRESS_SCREEN=WxH` makes the plans act as if the display were
/// that size, so the plans for a small display (1366x768, say) can be run on a big one: the windows and sweeps
/// stay inside that corner of the real display. A size that is malformed, or bigger than the real display, stops
/// the test with a message: a typo here must not quietly run the full-size plans.
fn display_size() -> (i32, i32) {
    let real = win::screen_size();
    let Some(text) = std::env::var("PHOTOCRAFT_STRESS_SCREEN").ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else {
        return real;
    };
    let asked = plan::parse_screen(&text).unwrap_or_else(|| panic!("PHOTOCRAFT_STRESS_SCREEN={text:?} is not a size like 1366x768 (at least 640x480)"));
    assert!(
        asked.0 <= real.0 && asked.1 <= real.1,
        "PHOTOCRAFT_STRESS_SCREEN={text} is bigger than this {}x{} display: it can only pretend to be smaller",
        real.0,
        real.1
    );
    asked
}

impl Config {
    /// Read the `PHOTOCRAFT_STRESS_*` variables. A malformed number falls back to its default, but a malformed or
    /// too large `PHOTOCRAFT_STRESS_SCREEN` stops the test (see [`display_size`]): it would change what is tested.
    fn from_env() -> Config {
        let text = |name: &str| std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let number = |name: &str, default: f64| text(name).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(default);
        let defaults = Limits::default();
        Config {
            exe: text("PHOTOCRAFT_STRESS_EXE").map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_photocraft")), PathBuf::from),
            extra_args: text("PHOTOCRAFT_STRESS_ARGS").map(|v| v.split_whitespace().map(str::to_owned).collect()).unwrap_or_default(),
            // A day is far more than a run needs; the cap keeps a typo from planning (or sleeping) forever.
            drag_seconds: number("PHOTOCRAFT_STRESS_SECONDS", 15.0).min(86_400.0),
            soak_seconds: number("PHOTOCRAFT_STRESS_SOAK_SECONDS", 60.0).min(86_400.0),
            seed: text("PHOTOCRAFT_STRESS_SEED").and_then(|v| v.parse().ok()).unwrap_or(7252),
            limits: Limits {
                max_stall_ms: number("PHOTOCRAFT_STRESS_MAX_STALL_MS", defaults.max_stall_ms),
                memory_growth_mb: number("PHOTOCRAFT_STRESS_MEMORY_GROWTH_MB", defaults.memory_growth_mb),
                ..defaults
            },
            out_dir: text("PHOTOCRAFT_STRESS_OUT").map(PathBuf::from),
            screen: display_size(),
        }
    }
}

/// Holds the right to use the real mouse and desktop.
pub struct Session {
    _serial: MutexGuard<'static, ()>,
}

/// The tests move the real mouse and open a window, so they refuse to run unless asked in so many words (the
/// variable set to exactly `1`: see [`plan::opted_in`]), and run one at a time even when the test harness uses
/// several threads.
pub fn begin() -> Session {
    let value = std::env::var_os(OPT_IN);
    assert!(
        plan::opted_in(value.as_deref()),
        "this test takes over the real mouse and opens a PhotoCraft window: set {OPT_IN}=1 to run it (only the exact value 1 counts, and it is {}) \
         (docs/development.md › Windows resize stress test)",
        value.as_deref().map_or_else(|| "not set".to_owned(), |v| format!("{v:?}"))
    );
    static SERIAL: Mutex<()> = Mutex::new(());
    Session { _serial: SERIAL.lock().unwrap_or_else(PoisonError::into_inner) }
}

/// The app's standard error, collected line by line by a thread that reads its pipe (so the app never blocks on it).
type Log = Arc<Mutex<Vec<String>>>;

/// Whether any line the app has logged so far contains `needle`.
fn log_has(log: &Log, needle: &str) -> bool {
    log.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|l| l.contains(needle))
}

/// The last `n` lines the app logged, for a failure message.
fn log_tail(log: &Log, n: usize) -> String {
    let lines = log.lock().unwrap_or_else(PoisonError::into_inner);
    lines.iter().skip(lines.len().saturating_sub(n)).cloned().collect::<Vec<_>>().join("\n")
}

/// Read `stderr` line by line into `log` in a thread, so that the app never blocks on a full pipe.
fn capture_stderr(stderr: impl Read + Send + 'static, log: &Log) -> JoinHandle<()> {
    let sink = Arc::clone(log);
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            sink.lock().unwrap_or_else(PoisonError::into_inner).push(line);
        }
    })
}

/// How long a failed start waits for the thread reading the app's standard error to take in the last of it.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// Wait for `reader` to reach the end of the app's standard error, for at most `grace`; whether it did. The app has
/// ended by then, so this is quick, unless something it started still holds the pipe open: that must not stall a
/// failure, so the thread is left running when it is not done.
fn reader_finished(reader: JoinHandle<()>, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    while !reader.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let _ = reader.join();
    true
}

/// A loopback port that was free a moment ago. The app binds it later, so [`App::launch`] tries again with a new
/// one if something else got there first.
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| format!("no free port: {e}"))
}

/// How many times a press is tried before a drag that will not start is a failure.
const GRAB_ATTEMPTS: u32 = 3;

/// Why a drag ended before its sweep.
enum DragError {
    /// The press did not start an OS resize.
    NotStarted,
    /// The test stopped itself, for the reason given: a refused press, or something else moving the mouse.
    Stopped(String),
}

impl DragError {
    fn stopped(why: impl Into<String>) -> DragError {
        DragError::Stopped(why.into())
    }
}

/// One isolated PhotoCraft process and its main window.
struct App {
    child: Child,
    window: Window,
    port: u16,
    token: String,
    dir: PathBuf,
    log: Log,
    keep_dir: bool,
    /// Presses that did not start a resize and were tried again (see [`App::drag_border`]).
    missed_grabs: u32,
    /// The display the plans and sweeps are fitted to (see [`Config::screen`]).
    screen: (i32, i32),
    /// What the run did differently from what was asked, for the report: sweeps cut down to fit the display.
    notes: BTreeSet<String>,
}

/// What one attempt to start the app is given: its own settings folder, the folder it may write to, the file its
/// control token goes in, and the control port to bind.
struct Start<'a> {
    port: u16,
    settings: &'a Path,
    out: &'a Path,
    token_file: &'a Path,
}

impl App {
    /// Start an isolated PhotoCraft (its own settings folder, control port and token) and wait until its control
    /// server answers and its main window exists. On failure the error carries the end of the app's log, and the
    /// settings folder, with the app's own log file, is kept and named in it, as for a run that fails later.
    fn launch(cfg: &Config, name: &str) -> Result<App, String> {
        App::launch_in(Path::new(env!("CARGO_TARGET_TMPDIR")), name, cfg.screen, capture_stderr, |start| {
            let mut command = Command::new(&cfg.exe);
            command
                .args(["--new-instance", "--control"])
                .arg(start.port.to_string())
                .arg("--control-token-file")
                .arg(start.token_file)
                .arg("--automation-write-root")
                .arg(start.out)
                .args(&cfg.extra_args)
                .env("PHOTOCRAFT_CONFIG_DIR", start.settings)
                .env("RUST_BACKTRACE", "1")
                // A control port, token or automation root from the caller's shell must not leak into the test.
                .env_remove("PHOTOCRAFT_CONTROL_PORT")
                .env_remove("PHOTOCRAFT_CONTROL_TOKEN")
                .env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE")
                .env_remove("PHOTOCRAFT_AUTOMATION_READ_ROOT")
                .env_remove("PHOTOCRAFT_AUTOMATION_WRITE_ROOT");
            command
        })
    }

    /// [`App::launch`] with the folder the throwaway settings go under, the reader of the app's standard error and
    /// the command that starts the app left to the caller, so that a start that fails can be simulated.
    fn launch_in(
        root: &Path,
        name: &str,
        screen: (i32, i32),
        capture: impl Fn(ChildStderr, &Log) -> JoinHandle<()>,
        mut command: impl FnMut(&Start) -> Command,
    ) -> Result<App, String> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = root.join(format!("resize-stress-{name}-{}-{stamp}", std::process::id()));
        // Absolute, because failures name it.
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        let (settings, out) = (dir.join("settings"), dir.join("out"));
        for d in [&settings, &out] {
            std::fs::create_dir_all(d).map_err(|e| format!("create {}: {e}", d.display()))?;
        }
        let token_file = dir.join("control.token");
        let mut last = String::new();
        // Another process can take the port between choosing it and the app binding it: try again.
        for _ in 0..3 {
            let port = free_port()?;
            let mut cmd = command(&Start { port, settings: &settings, out: &out, token_file: &token_file });
            let mut child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("start {}: {e}", cmd.get_program().to_string_lossy()))?;
            let log: Log = Arc::default();
            let reader = child.stderr.take().map(|stderr| capture(stderr, &log));
            match wait_until_ready(&mut child, &log, port, &token_file) {
                Ok((token, window)) => {
                    return Ok(App { child, window, port, token, dir, log, keep_dir: false, missed_grabs: 0, screen, notes: BTreeSet::new() });
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // The app is gone, so all it wrote is in the pipe: let the reader take it in before it is read.
                    let note = if reader.is_none_or(|reader| reader_finished(reader, STDERR_GRACE)) {
                        String::new()
                    } else {
                        format!("\n(the app's standard error may be incomplete: it was still being read {} ms after the app ended)", STDERR_GRACE.as_millis())
                    };
                    last = format!("{error}\n--- the app's log ---\n{}{note}", log_tail(&log, 40));
                    if !log_has(&log, "failed to bind") {
                        break;
                    }
                }
            }
        }
        // The folder stays: the app's own log file is in it, and an app that never came up is when it is wanted most.
        Err(format!("{last}\n(kept in {})", dir.display()))
    }

    /// How the app ended, if it has.
    fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(format!("the process cannot be queried: {e}")),
        }
    }

    /// A new control connection. The app drops one that sits idle for 30 s, so callers connect when they need one
    /// instead of keeping one across a drag.
    fn connect(&self, timeout: Duration) -> Result<Control, String> {
        Control::connect(self.port, &self.token, timeout)
    }

    /// Press at `grab`, nudge until the OS enters its modal sizing loop, sweep the cursor back and forth, release.
    fn drag(&mut self, grab: (i32, i32), axes: Axes, sweep: Sweep, seconds: f64, step_s: f64) -> Result<(), DragError> {
        if !win::move_cursor(grab.0, grab.1) {
            return Err(DragError::stopped("SendInput refused to move the cursor"));
        }
        thread::sleep(Duration::from_millis(150));
        // Never press unless the test window is exactly what a click here would hit, and the cursor is where
        // the test put it: otherwise something else (you, or another window) is in the way.
        match win::cursor() {
            Some(at) if (at.0 - grab.0).abs() > 3 || (at.1 - grab.1).abs() > 3 => {
                return Err(DragError::stopped(format!("the cursor is at {at:?}, not at {grab:?}: something else is moving the mouse; not pressing")));
            }
            None => return Err(DragError::stopped("cannot read the cursor position")),
            Some(_) => {}
        }
        if !self.window.is_under(grab.0, grab.1) {
            let rect = self.window.rect();
            return Err(DragError::stopped(format!(
                "the test window (at {rect:?}) is not under the cursor at {grab:?}: another window is in front, or the window moved; not pressing"
            )));
        }
        let Some(_held) = win::HeldButton::press() else {
            return Err(DragError::stopped("SendInput refused the mouse press (a window of a more privileged process may have the foreground)"));
        };
        // egui starts the resize after a few px of movement.
        let direction = if sweep.amplitude < 0 { -1 } else { 1 };
        for i in 1..=plan::NUDGE_PX {
            let (x, y) = plan::point_at(grab, axes, direction * i);
            win::move_cursor(x, y);
            thread::sleep(Duration::from_millis(16));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !self.window.in_modal_loop() {
            thread::sleep(Duration::from_millis(10));
        }
        if !self.window.in_modal_loop() {
            return Err(DragError::NotStarted);
        }
        let started = Instant::now();
        let mut moves = 0u32;
        let mut next = started;
        while started.elapsed().as_secs_f64() < seconds {
            if self.exited().is_some() {
                break;
            }
            let (x, y) = plan::point_at(grab, axes, sweep.offset(started.elapsed().as_secs_f64()));
            win::move_cursor(x, y);
            moves += 1;
            if moves.is_multiple_of(25)
                && let Some(at) = win::cursor()
            {
                match plan::check_cursor(at, grab, axes, self.screen) {
                    CursorCheck::OnPath => {}
                    // `fit_sweep` keeps the sweep on the display, so this means the plan or the window was not what was
                    // expected. It is not put down to the user, who may well not have touched the mouse.
                    CursorCheck::AtScreenEdge => {
                        return Err(DragError::stopped(format!(
                            "the OS stopped the cursor at the edge of the {}x{} display, at {at:?}: the sweep ran out of screen (or something moved the mouse there)",
                            self.screen.0, self.screen.1
                        )));
                    }
                    CursorCheck::Moved => {
                        return Err(DragError::stopped(format!(
                            "the cursor left its path (at {at:?}): something else moved the mouse; stopped so as not to fight it"
                        )));
                    }
                }
            }
            next += Duration::from_secs_f64(step_s);
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        let (x, y) = plan::point_at(grab, axes, plan::NUDGE_PX);
        win::move_cursor(x, y);
        Ok(())
    }

    /// Wait for the window to come to rest: the OS's sizing loop over and the window's rectangle unchanged for
    /// 150 ms. A slow renderer lags behind the mouse, so the OS can still be working through a drag long after
    /// the test let go of the button; a rectangle read then is out of date by the time it is used.
    fn wait_until_at_rest(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut last, mut since) = (None, Instant::now());
        while Instant::now() < deadline {
            let now = self.window.rect();
            if now != last || self.window.in_modal_loop() {
                (last, since) = (now, Instant::now());
            } else if since.elapsed() >= Duration::from_millis(150) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Drag the border or corner `kind` names, from where the window is now (once it is at rest). When the
    /// press does not start a resize the border is grabbed again, as a person would: very slow rendering
    /// (the software adapter) loses about one grab in forty. The retries are counted in `missed_grabs` and
    /// reported; only a drag that never starts is a failure.
    fn drag_border(&mut self, kind: Kind, sweep: Sweep, seconds: f64, step_s: f64) -> Result<(), String> {
        for attempt in 1..=GRAB_ATTEMPTS {
            self.wait_until_at_rest();
            let rect = self.window.rect().ok_or("cannot read the window's rectangle")?;
            let (grab, axes) = plan::grab_point(kind, rect);
            // Fit the sweep to the room the window's real rectangle leaves on the display (the OS may have made it
            // bigger than planned), and say so in the report when it had to be cut down. A display with no room
            // for a meaningful sweep fails the run here, with the numbers, rather than running a token one.
            let fitted = plan::fit_sweep(sweep, grab, axes, self.screen).map_err(|why| format!("{kind:?} with the window at {rect:?}: {why}"))?;
            if fitted.reduced {
                self.notes.insert(format!(
                    "{kind:?}: the sweep was cut from {} to {} px (period {:.2} to {:.2} s, so the border moves as fast) to fit the {}x{} display",
                    sweep.amplitude, fitted.sweep.amplitude, sweep.period_s, fitted.sweep.period_s, self.screen.0, self.screen.1
                ));
            }
            let result = self.drag(grab, axes, fitted.sweep, seconds, step_s);
            self.wait_until_at_rest();
            match result {
                Ok(()) => return Ok(()),
                Err(DragError::NotStarted) if attempt < GRAB_ATTEMPTS => self.missed_grabs += 1,
                Err(DragError::NotStarted) => {
                    self.missed_grabs += 1;
                    return Err(format!("the OS never entered its modal sizing loop in {GRAB_ATTEMPTS} tries: the press did not start a resize"));
                }
                Err(DragError::Stopped(why)) => return Err(why),
            }
        }
        Ok(())
    }

    /// Maximize and restore the window over and over for `seconds`, pausing a little differently each time.
    fn maximize_restore(&mut self, seconds: f64) {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        let mut i = 0u64;
        while Instant::now() < end && self.exited().is_none() {
            let dwell = Duration::from_millis(100 + 50 * (i % 7));
            self.window.maximize();
            thread::sleep(dwell);
            self.window.restore();
            thread::sleep(dwell);
            i += 1;
        }
    }

    /// Resize the window from the program as fast as it will take it. Returns how many sizes were set.
    fn api_storm(&mut self, seconds: f64, seed: u64, screen: (i32, i32)) -> usize {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        let mut set = 0;
        for size in plan::storm_sizes(seed, 2000, screen).into_iter().cycle() {
            if Instant::now() >= end || self.exited().is_some() || !self.window.resize(size) {
                break;
            }
            set += 1;
            thread::sleep(Duration::from_millis(2));
        }
        set
    }

    /// One soak segment: park the window, then do what the plan says.
    fn run_segment(&mut self, seg: &Segment, screen: (i32, i32)) -> Result<(), String> {
        if !self.window.park(seg.park, seg.size) {
            return Err("cannot place the window".into());
        }
        thread::sleep(Duration::from_millis(400));
        match seg.kind {
            Kind::RightEdge | Kind::LeftEdge | Kind::Corner => self.drag_border(seg.kind, seg.sweep, seg.seconds, seg.step_s),
            Kind::ShortGrabs => {
                let end = Instant::now() + Duration::from_secs_f64(seg.seconds);
                let mut i = 0u32;
                while Instant::now() < end && self.exited().is_none() {
                    // Each short drag can leave the window a little wider: keep room for the next one.
                    if self.window.rect().is_some_and(|(_, _, right, _)| right + seg.sweep.amplitude + plan::MARGIN > screen.0) {
                        self.window.park(seg.park, seg.size);
                        thread::sleep(Duration::from_millis(300));
                    }
                    let sweep = Sweep { period_s: if i.is_multiple_of(2) { 0.1 } else { 0.2 }, ..seg.sweep };
                    self.drag_border(Kind::RightEdge, sweep, 0.3 + 0.1 * f64::from(i % 8), seg.step_s)?;
                    thread::sleep(Duration::from_millis(u64::from(20 + 30 * (i % 7))));
                    i += 1;
                }
                Ok(())
            }
            Kind::MaximizeRestore => {
                self.maximize_restore(seg.seconds);
                Ok(())
            }
        }
    }

    /// The size, in px, of the frame the app renders now (what `ui.screenshot` captures), next to the window's.
    fn rendered_size(&self) -> Result<(u32, u32), String> {
        let mut control = self.connect(Duration::from_secs(30))?;
        let shot = control.call("ui.screenshot", json!({"path": "final.png", "focus": false}))?;
        let side = |key: &str| shot[key].as_u64().and_then(|v| u32::try_from(v).ok()).ok_or_else(|| format!("ui.screenshot reported no {key}: {shot}"));
        Ok((side("width")?, side("height")?))
    }

    /// Whether the app's rendered frame is the window's size; the problem if it is not.
    fn surface_problem(&self, tolerance: f64) -> Option<String> {
        let client = self.window.client()?;
        match self.rendered_size() {
            Ok((w, h)) if (f64::from(w) - f64::from(client.0)).abs() > tolerance || (f64::from(h) - f64::from(client.1)).abs() > tolerance => {
                Some(format!("the render surface is {w}x{h} px but the window is {}x{} px (a stale swap chain)", client.0, client.1))
            }
            Ok(_) => None,
            Err(e) => Some(format!("no frame could be rendered: {e}")),
        }
    }
}

impl Drop for App {
    /// Kill the app and remove its throwaway settings folder, unless the run failed: that folder (with the app's own
    /// log file) is then kept, and its path is in the failure message.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !self.keep_dir {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// The app's private memory in MB once start-up work (the system font scan, the first document) has stopped
/// allocating: two readings a second apart within 4 MB of each other, or what it is after 20 s. Growth is
/// then measured from a settled start, not from a moment when the app was still warming up.
fn settled_memory_mb(pid: u32) -> Option<f64> {
    let mut last = win::private_mb(pid)?;
    let mut steady = 0;
    for _ in 0..20 {
        thread::sleep(Duration::from_secs(1));
        let now = win::private_mb(pid)?;
        steady = if (now - last).abs() < 4.0 { steady + 1 } else { 0 };
        last = now;
        if steady >= 2 {
            break;
        }
    }
    Some(last)
}

/// Wait for the control server and the main window; returns the control token and the window.
fn wait_until_ready(child: &mut Child, log: &Log, port: u16, token_file: &Path) -> Result<(String, Window), String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the app exited while starting ({status})"));
        }
        if log_has(log, "failed to bind") {
            return Err("the control port was taken".into());
        }
        let token = std::fs::read_to_string(token_file).ok().map(|t| t.trim().to_owned()).filter(|t| t.len() == 64);
        if let Some(token) = token
            && let Some(window) = win::find_window(child.id())
            && let Ok(mut control) = Control::connect(port, &token, Duration::from_secs(5))
            && control.call("ui.inspect", json!({})).is_ok()
        {
            return Ok((token, window));
        }
        if Instant::now() > deadline {
            return Err("the app did not come up within 180 s".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// What the background prober saw.
#[derive(Default)]
struct ProbeLog {
    probes: Vec<Probe>,
    failures: Vec<String>,
    span_s: f64,
}

/// Asks the app `ui.inspect` four times a second from another thread. An answer proves that the UI thread
/// ran a frame, so a window that stops drawing while it is being resized shows up as late or missing answers.
struct Prober {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<ProbeLog>,
}

impl Prober {
    /// Begin probing in a thread: connect (and reconnect after any error), ask `ui.inspect` every
    /// [`plan::PROBE_INTERVAL_S`], and record each answer's latency and each failure. `timeout` is how long one
    /// request may take before it counts as a failure.
    fn start(port: u16, token: String, timeout: Duration) -> Prober {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let began = Instant::now();
            let mut log = ProbeLog::default();
            let mut connection: Option<Control> = None;
            while !flag.load(Ordering::Relaxed) {
                let tick = Instant::now();
                if connection.is_none() {
                    match Control::connect(port, &token, timeout) {
                        Ok(c) => connection = Some(c),
                        Err(e) => log.failures.push(format!("t={:.1}s: {e}", began.elapsed().as_secs_f64())),
                    }
                }
                if let Some(c) = connection.as_mut() {
                    match c.call("ui.inspect", json!({})) {
                        Ok(v) => {
                            let latency_ms = tick.elapsed().as_secs_f64() * 1000.0;
                            log.probes.push(Probe { at_s: began.elapsed().as_secs_f64(), latency_ms, frame: v["frame"].as_u64().unwrap_or(0) });
                        }
                        Err(e) => {
                            log.failures.push(format!("t={:.1}s: {e}", began.elapsed().as_secs_f64()));
                            connection = None;
                        }
                    }
                }
                thread::sleep(Duration::from_secs_f64(plan::PROBE_INTERVAL_S).saturating_sub(tick.elapsed()));
            }
            log.span_s = began.elapsed().as_secs_f64();
            log
        });
        Prober { stop, handle }
    }

    /// Stop probing and hand over what was seen.
    fn finish(self) -> ProbeLog {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap_or_default()
    }
}

/// The window states the tests drive, one test each.
#[derive(Clone, Copy, Debug)]
pub enum Scenario {
    RightBorder,
    LeftBorder,
    Corner,
    BelowMinimum,
    MaximizeRestore,
    ApiStorm,
    Soak,
}

impl Scenario {
    /// The name in reports and in the output directory's layout (`PHOTOCRAFT_STRESS_OUT/<name>/`).
    fn name(self) -> &'static str {
        match self {
            Scenario::RightBorder => "right-border",
            Scenario::LeftBorder => "left-border",
            Scenario::Corner => "corner-large-document",
            Scenario::BelowMinimum => "below-minimum-size",
            Scenario::MaximizeRestore => "maximize-restore",
            Scenario::ApiStorm => "api-storm",
            Scenario::Soak => "soak",
        }
    }

    /// The document opened first. #7252 reports an empty window, so the border tests that mirror it open none.
    fn document(self) -> Option<(u32, u32)> {
        match self {
            Scenario::RightBorder | Scenario::BelowMinimum => None,
            Scenario::Corner => Some((6000, 4000)),
            _ => Some((640, 480)),
        }
    }

    /// The drag this scenario makes, if it is a fixed one (the others park the window at the default size).
    fn fixed(self) -> Option<Fixed> {
        match self {
            Scenario::RightBorder => Some(Fixed::RightBorder),
            Scenario::LeftBorder => Some(Fixed::LeftBorder),
            Scenario::Corner => Some(Fixed::Corner),
            Scenario::BelowMinimum => Some(Fixed::BelowMinimum),
            Scenario::MaximizeRestore | Scenario::ApiStorm | Scenario::Soak => None,
        }
    }
}

/// Run one scenario against a fresh PhotoCraft and panic with the problems found, if any.
pub fn run(scenario: Scenario) {
    let _session = begin();
    win::init_dpi();
    let cfg = Config::from_env();
    let mut app = App::launch(&cfg, scenario.name()).unwrap_or_else(|e| panic!("{e}"));
    let (obs, details) = match drive(&mut app, &cfg, scenario) {
        Ok(done) => done,
        Err(error) => {
            app.keep_dir = true;
            panic!("{}: {error}\n--- the app's log ---\n{}\n(kept in {})", scenario.name(), log_tail(&app.log, 40), app.dir.display());
        }
    };
    let problems = plan::judge(&obs, &cfg.limits);
    let summary = summarize(&cfg, scenario, &obs, &problems, details);
    eprintln!(
        "[resize-stress] {}: {}\n{}",
        scenario.name(),
        if problems.is_empty() { "passed" } else { "FAILED" },
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    );
    if let Some(out) = &cfg.out_dir {
        let here = out.join(scenario.name());
        let saved = std::fs::create_dir_all(&here)
            .and_then(|()| std::fs::write(here.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap_or_default()))
            .and_then(|()| std::fs::write(here.join("app-stderr.log"), log_tail(&app.log, usize::MAX)));
        if let Err(e) = saved {
            eprintln!("[resize-stress] could not write to {}: {e}", here.display());
        }
    }
    if !problems.is_empty() {
        app.keep_dir = true;
        panic!(
            "{} found {} problem(s):\n - {}\n--- the app's log ---\n{}\n(kept in {})",
            scenario.name(),
            problems.len(),
            problems.join("\n - "),
            log_tail(&app.log, 30),
            app.dir.display()
        );
    }
}

/// Run the scenario and collect what to judge, plus details for the report.
fn drive(app: &mut App, cfg: &Config, scenario: Scenario) -> Result<(Observations, Value), String> {
    let screen = cfg.screen;
    let mut control = app.connect(Duration::from_secs(30))?;
    let gpu = control.call("ui.inspect", json!({}))?.pointer("/perf/timings/gpuInfo").cloned().unwrap_or(Value::Null);
    if let Some((width, height)) = scenario.document() {
        control.call("engine.execute", json!({"command": "file.new", "params": {"width": width, "height": height, "background": "white"}}))?;
    }
    drop(control);
    // The first screenshot allocates the capture buffers for good: take one now, so that the memory growth
    // measured below is the resizing's and not that one-time cost.
    let _ = app.rendered_size();
    let memory_before = settled_memory_mb(app.child.id());

    // A fixed scenario's window and sweep come from `plan::fixed_plan`, fitted to the display; the others start from
    // the default size, likewise fitted. (`fit_sweep` then fits each sweep again, to where the window really is.)
    let fixed = scenario.fixed().map(|which| plan::fixed_plan(which, screen));
    let (park, size) = fixed.map_or((plan::PARK, plan::default_size(screen)), |p| (p.park, p.size));
    if !app.window.park(park, size) {
        return Err("cannot place the window".into());
    }
    thread::sleep(Duration::from_millis(1500));

    let mut obs = Observations::default();
    let mut details = json!({});
    let prober = Prober::start(app.port, app.token.clone(), Duration::from_millis(cfg.limits.max_stall_ms as u64));
    let drag = |app: &mut App, obs: &mut Observations| {
        if let Some(p) = fixed
            && let Err(e) = app.drag_border(p.kind, p.sweep, cfg.drag_seconds, 0.004)
        {
            obs.step_failures.push(format!("{}: {e}", scenario.name()));
        }
    };
    match scenario {
        Scenario::RightBorder | Scenario::BelowMinimum | Scenario::LeftBorder | Scenario::Corner => drag(app, &mut obs),
        Scenario::MaximizeRestore => app.maximize_restore(cfg.drag_seconds),
        Scenario::ApiStorm => {
            let sizes = app.api_storm(cfg.drag_seconds, cfg.seed, screen);
            details["sizes_set"] = json!(sizes);
        }
        Scenario::Soak => {
            let segments = plan::soak_plan(cfg.seed, cfg.soak_seconds, screen);
            details["segments"] = json!(segments.len());
            // The app's memory after each segment: the shape of any growth (steady, or a step) tells a leak from a one-off.
            let mut memory_after_segment = Vec::new();
            for (i, seg) in segments.iter().enumerate() {
                if app.exited().is_some() {
                    break;
                }
                if let Err(e) = app.run_segment(seg, screen) {
                    obs.step_failures.push(format!("segment {} ({:?}): {e}", i + 1, seg.kind));
                }
                thread::sleep(Duration::from_secs(1));
                // Every segment must end with the app drawing at the window's size.
                if let Some(problem) = app.surface_problem(cfg.limits.size_tolerance_px) {
                    obs.step_failures.push(format!("segment {} ({:?}): {problem}", i + 1, seg.kind));
                }
                memory_after_segment.push(win::private_mb(app.child.id()).map(|mb| mb.round()));
            }
            details["memory_mb_after_each_segment"] = json!(memory_after_segment);
        }
    }
    thread::sleep(Duration::from_secs(1));
    let log = prober.finish();
    (obs.probes, obs.probe_failures, obs.probe_span_s) = (log.probes, log.failures, log.span_s);
    obs.died = app.exited();
    // Before the checks below, which allocate on their own (a screenshot, a document).
    obs.memory_mb = memory_before.zip(win::private_mb(app.child.id()));

    // Recovery: the window is at rest. The app must draw at its size, lay out at it, and still take commands.
    obs.client_px = app.window.client().unwrap_or((0, 0));
    obs.surface_px = app.rendered_size().ok();
    match app.connect(Duration::from_secs(30)) {
        Ok(mut control) => {
            if let Ok(inspect) = control.call("ui.inspect", json!({})) {
                let ppp = inspect.pointer("/window/pixelsPerPoint").and_then(Value::as_f64).unwrap_or(1.0);
                let (w, h) = (inspect.pointer("/window/width").and_then(Value::as_f64), inspect.pointer("/window/height").and_then(Value::as_f64));
                obs.laid_out_px = w.zip(h).map(|(w, h)| (w * ppp, h * ppp));
            }
            let edit = control
                .call("engine.execute", json!({"command": "file.new", "params": {"width": 64, "height": 64, "background": "white"}}))
                .and_then(|_| control.call("engine.execute", json!({"command": "edit.fill", "params": {"color": "#2266dd"}})));
            obs.edit_error = edit.err();
        }
        Err(e) => obs.edit_error = Some(format!("cannot connect: {e}")),
    }
    details["gpu"] = gpu;
    details["missed_grabs"] = json!(app.missed_grabs);
    // Anything the run did differently from what was asked (a sweep cut down to the display) is said here, not hidden.
    details["notes"] = json!(app.notes);
    Ok((obs, details))
}

/// The JSON report of a run: the verdict and the problems behind it, the numbers it was judged on, and `details`
/// (the GPU, and whatever the run did differently from what was asked: see `details.notes`).
fn summarize(cfg: &Config, scenario: Scenario, obs: &Observations, problems: &[String], details: Value) -> Value {
    let latencies = obs.probes.iter().map(|p| p.latency_ms);
    let frames_per_s = match (obs.probes.first(), obs.probes.last()) {
        (Some(first), Some(last)) if last.at_s > first.at_s => Some(last.frame.saturating_sub(first.frame) as f64 / (last.at_s - first.at_s)),
        _ => None,
    };
    json!({
        "scenario": scenario.name(),
        "verdict": if problems.is_empty() { "pass" } else { "fail" },
        "problems": problems,
        "exe": cfg.exe.display().to_string(),
        "args": cfg.extra_args,
        "seed": cfg.seed,
        "drag_seconds": cfg.drag_seconds,
        "screen_px": cfg.screen,
        "real_screen_px": win::screen_size(),
        "details": details,
        "probes": {
            "answered": obs.probes.len(),
            "failed": obs.probe_failures.len(),
            "max_latency_ms": latencies.fold(0.0, f64::max),
            "max_gap_ms": plan::max_gap_ms(&obs.probes, obs.probe_span_s),
            "app_frames_per_s": frames_per_s,
        },
        "final": {"client_px": obs.client_px, "surface_px": obs.surface_px, "laid_out_px": obs.laid_out_px},
        "memory_mb": obs.memory_mb,
        "limits": {"max_stall_ms": cfg.limits.max_stall_ms, "memory_growth_mb": cfg.limits.memory_growth_mb, "size_tolerance_px": cfg.limits.size_tolerance_px},
    })
}

/// Starts that fail, simulated with a stand-in for the app: no window, no mouse, and no wait for the real 180 s
/// timeout. What matters is what a failed start leaves behind to explain itself.
#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::os::windows::process::CommandExt;
    use std::sync::atomic::AtomicBool;

    use super::*;

    /// A folder for one test's starts, removed when the test ends however it ends.
    struct Root(PathBuf);

    impl Root {
        fn new(test: &str) -> Root {
            Root(Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("launch-{test}-{}", std::process::id())))
        }

        /// The folders the starts left behind.
        fn kept(&self) -> Vec<PathBuf> {
            std::fs::read_dir(&self.0).map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default()
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `cmd /C script`: a stand-in for the app that writes to its standard error and then ends, or stays up.
    fn stand_in(script: &str) -> Command {
        let mut command = Command::new("cmd");
        command.arg("/C").raw_arg(format!("\"{script}\""));
        command
    }

    /// The error of a start that must fail.
    fn failed(started: Result<App, String>) -> String {
        match started {
            Ok(_) => panic!("the stand-in for the app cannot come up"),
            Err(error) => error,
        }
    }

    /// A reader of the app's standard error that is slow to hand over the first of it, as one that is not scheduled
    /// promptly would be.
    struct Slow<R>(R, Option<Duration>);

    impl<R: Read> Read for Slow<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(delay) = self.1.take() {
                thread::sleep(delay);
            }
            self.0.read(buf)
        }
    }

    /// A reader that hands over one line and then does not reach the end of the pipe until `release` is set (or 15 s
    /// have passed, so that a start that waits for it without a limit fails its test instead of hanging), as one
    /// would if something the app started still held the pipe open.
    struct Stalls {
        line: Option<&'static str>,
        release: Arc<AtomicBool>,
    }

    impl Read for Stalls {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(line) = self.line.take() {
                buf[..line.len()].copy_from_slice(line.as_bytes());
                return Ok(line.len());
            }
            for _ in 0..1500 {
                if self.release.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Ok(0)
        }
    }

    #[test]
    fn a_start_that_fails_keeps_its_settings_folder_and_names_it() {
        let root = Root::new("keeps");
        let token = "0123456789abcdef".repeat(4);
        let starts = Cell::new(0);
        let error = failed(App::launch_in(&root.0, "fake", (1920, 1080), capture_stderr, |start| {
            starts.set(starts.get() + 1);
            // What the app writes before it dies: its own log file, and its control token.
            let logs = start.settings.join("logs");
            std::fs::create_dir_all(&logs).unwrap();
            std::fs::write(logs.join("photocraft.log"), "startup diagnostics\n").unwrap();
            std::fs::write(start.token_file, &token).unwrap();
            stand_in("echo the app is gone 1>&2 & exit /b 3")
        }));
        assert_eq!(starts.get(), 1, "only a taken control port is tried again: {error}");
        let kept = root.kept();
        assert_eq!(kept.len(), 1, "{kept:?}");
        let dir = &kept[0];
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(error.contains(&format!("(kept in {})", dir.display())), "{error}");
        assert!(error.contains("exited while starting") && error.contains("the app is gone"), "{error}");
        assert_eq!(std::fs::read_to_string(dir.join("settings").join("logs").join("photocraft.log")).unwrap(), "startup diagnostics\n");
        assert!(!error.contains(&token), "the control token must not be in the message: {error}");
    }

    #[test]
    fn a_taken_control_port_is_tried_again_and_a_start_that_never_works_is_still_kept() {
        let root = Root::new("port");
        let starts = Cell::new(0);
        let error = failed(App::launch_in(&root.0, "fake", (1920, 1080), capture_stderr, |_| {
            starts.set(starts.get() + 1);
            // Reports the port as taken and stays up, as the app does (`ping` is only a way to wait 2 s).
            stand_in("echo photocraft: control server failed to bind 127.0.0.1:1 1>&2 & ping -n 3 127.0.0.1 >nul")
        }));
        assert_eq!(starts.get(), 3, "{error}");
        assert!(error.contains("the control port was taken"), "{error}");
        let kept = root.kept();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(error.contains(&format!("(kept in {})", kept[0].display())), "{error}");
    }

    #[test]
    fn the_last_lines_of_a_failed_start_are_in_the_error_even_when_the_reader_is_slow() {
        let root = Root::new("slow");
        // The stand-in is gone almost at once; its last words are still in the pipe when the reader is asked for them.
        let error = failed(App::launch_in(
            &root.0,
            "fake",
            (1920, 1080),
            |stderr, log| capture_stderr(Slow(stderr, Some(Duration::from_millis(400))), log),
            |_| stand_in("echo the last words 1>&2 & exit /b 3"),
        ));
        assert!(error.contains("exited while starting"), "{error}");
        assert!(error.contains("the last words"), "{error}");
        assert!(!error.contains("incomplete"), "the reader finished in time: {error}");
    }

    #[test]
    fn a_reader_that_never_finishes_does_not_stall_a_failed_start_and_the_error_says_so() {
        let root = Root::new("stalled");
        let release = Arc::new(AtomicBool::new(false));
        let began = Instant::now();
        let error = failed(App::launch_in(
            &root.0,
            "fake",
            (1920, 1080),
            |_, log| capture_stderr(Stalls { line: Some("what the app said first\n"), release: Arc::clone(&release) }, log),
            |_| stand_in("exit /b 3"),
        ));
        let waited = began.elapsed();
        release.store(true, Ordering::Relaxed);
        assert!(waited < Duration::from_secs(10), "{waited:?}");
        assert!(error.contains("exited while starting"), "{error}");
        assert!(error.contains("what the app said first"), "what was read is kept: {error}");
        assert!(error.contains("standard error may be incomplete"), "{error}");
        let kept = root.kept();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(error.contains(&format!("(kept in {})", kept[0].display())), "{error}");
    }
}
