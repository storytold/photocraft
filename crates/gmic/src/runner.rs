//! Finding and running the `gmic` program.
//!
//! A run happens in a fresh temporary folder: the image list goes in as `in.cimg`, the pipeline
//! as the body of a one-command `.gmic` file (so G'MIC parses it exactly once, as script), and
//! `out.cimg` comes back. The child gets no stdin; both output pipes are drained (capped) while
//! the run is polled for its deadline and for cancellation, and a stopped run is killed and
//! reaped. Native only: on `wasm32` everything returns [`Error::Unsupported`].

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use crate::catalog::Catalog;
#[cfg(target_arch = "wasm32")]
use crate::cimg::Image;
#[cfg(target_arch = "wasm32")]
use crate::Error;
use crate::Result;

/// The environment variable that names the `gmic` program, when no path is configured.
pub const ENV: &str = "PHOTOCRAFT_GMIC";
/// The most samples a run reads back (1 Gi samples, 4 GiB as float32).
pub const MAX_SAMPLES: usize = 1 << 30;
/// Default time limit of a filter run.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(300);
/// Time limit of a catalogue rebuild (a full `parse_gui` takes about a minute).
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(600);
#[cfg(target_arch = "wasm32")]
const UNSUPPORTED: &str = "G'MIC filters need the desktop app: the web version can't start other programs";

/// How a run may be stopped early.
#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub timeout: Duration,
    /// Polled while the program runs; `true` stops it with [`Error::Cancelled`].
    pub cancelled: Option<&'a (dyn Fn() -> bool + Sync)>,
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        RunOptions { timeout: RUN_TIMEOUT, cancelled: None }
    }
}

impl<'a> RunOptions<'a> {
    pub fn with_timeout(self, timeout: Duration) -> Self {
        RunOptions { timeout, ..self }
    }
}

/// A located, working `gmic` program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gmic {
    path: PathBuf,
    version: String,
}

impl Gmic {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// G'MIC's version string (`"4.0.5"`).
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Removes ANSI colour sequences (G'MIC colours its messages).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The message of G'MIC's last `*** Error ***` line in `stderr`, else its last non-empty line.
pub fn error_message(stderr: &str) -> Option<String> {
    let text = strip_ansi(stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = lines.iter().rev().find(|l| l.contains("*** Error")).or(lines.last())?;
    // "[gmic]./run/ *** Error in ./ *** Unknown command…" → "Unknown command…"
    let msg = pick.rsplit("***").next().unwrap_or(pick).trim();
    let msg = if msg.is_empty() { pick } else { msg };
    Some(msg.chars().take(500).collect())
}

/// The platform's per-user cache folder for PhotoCraft's G'MIC files (the catalogue cache).
pub fn default_cache_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    }?;
    Some(base.join("photocraft").join("gmic"))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::{CATALOG_TIMEOUT, ENV, Gmic, MAX_SAMPLES, RunOptions, error_message, strip_ansi};
    use crate::catalog::Catalog;
    use crate::cimg::{self, Image};
    use crate::{Error, Result};

    const PROGRAM: &str = if cfg!(windows) { "gmic.exe" } else { "gmic" };
    /// Where package managers put `gmic` when the app's PATH doesn't include it (apps started
    /// from the macOS Finder get a minimal PATH).
    const WELL_KNOWN: &[&str] = &["/opt/homebrew/bin/gmic", "/usr/local/bin/gmic", "/usr/bin/gmic", "/opt/local/bin/gmic", "/snap/bin/gmic"];
    /// Output kept from each pipe (the tail).
    const PIPE_CAP: usize = 64 * 1024;
    /// The name of the one command in a run's `.gmic` file.
    const RUN_COMMAND: &str = "photocraft_run";

    fn io(what: &str, e: std::io::Error) -> Error {
        Error::Io(format!("{what}: {e}"))
    }

    /// The `gmic` program: `configured` (a preference) when set, else `$PHOTOCRAFT_GMIC`, else
    /// the first `gmic` on `PATH` or in a well-known install location.
    pub fn locate(configured: Option<&str>) -> Result<PathBuf> {
        if let Some(c) = configured.map(str::trim).filter(|c| !c.is_empty()) {
            let p = PathBuf::from(c);
            return if p.is_file() { Ok(p) } else { Err(Error::NotFound(format!("{c:?} is not a file (Preferences › Plug-ins › G'MIC executable)"))) };
        }
        if let Some(e) = std::env::var_os(ENV).filter(|v| !v.is_empty()) {
            let p = PathBuf::from(&e);
            return if p.is_file() { Ok(p) } else { Err(Error::NotFound(format!("{ENV}={} is not a file", p.display()))) };
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let p = dir.join(PROGRAM);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
        if !cfg!(windows) {
            for w in WELL_KNOWN {
                let p = PathBuf::from(w);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
        Err(Error::NotFound(format!("no `{PROGRAM}` on PATH; install G'MIC's command-line program, or set its path in Preferences › Plug-ins")))
    }

    /// A temporary folder, removed with everything in it when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Result<TempDir> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("photocraft-gmic-{}-{}-{nanos}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&dir).map_err(|e| io("couldn't create a temporary folder for G'MIC", e))?;
            Ok(TempDir(dir))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The child's exit and the tails of its output.
    struct Output {
        success: bool,
        status: String,
        stderr: String,
    }

    /// Runs `program args` in `dir` until it exits, its deadline passes or it is cancelled.
    fn exec(program: &Path, args: &[&str], dir: &Path, opts: &RunOptions<'_>) -> Result<Output> {
        let mut cmd = Command::new(program);
        cmd.args(args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: no console window flashes up for each run.
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|e| Error::NotFound(format!("couldn't start {}: {e}", program.display())))?;
        let drain = |p: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut tail = Vec::new();
                let mut buf = [0u8; 8192];
                if let Some(mut p) = p {
                    while let Ok(n) = p.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        tail.extend_from_slice(buf.get(..n).unwrap_or_default());
                        if tail.len() > PIPE_CAP {
                            tail.drain(..tail.len() - PIPE_CAP);
                        }
                    }
                }
                tail
            })
        };
        let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let deadline = Instant::now() + opts.timeout;
        let stop = |child: &mut std::process::Child, e: Error| {
            let _ = child.kill();
            let _ = child.wait();
            Err(e)
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if opts.cancelled.is_some_and(|c| c()) => return stop(&mut child, Error::Cancelled),
                Ok(None) if Instant::now() >= deadline => return stop(&mut child, Error::Timeout(opts.timeout.as_secs())),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return stop(&mut child, io("waiting for G'MIC failed", e)),
            }
        };
        let _ = out.join();
        let stderr = err.join().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        Ok(Output { success: status.success(), status: status.to_string(), stderr })
    }

    fn failed(o: &Output) -> Error {
        Error::Failed(error_message(&o.stderr).unwrap_or_else(|| format!("gmic exited abnormally ({})", o.status)))
    }

    impl Gmic {
        /// Checks that `path` is a working G'MIC and reads its version.
        pub fn probe(path: &Path, opts: &RunOptions<'_>) -> Result<Gmic> {
            let dir = TempDir::new()?;
            let o = exec(path, &["-v", "-", "echo", "${-strver}"], &dir.0, opts)?;
            if !o.success {
                return Err(Error::NotFound(format!("{} is not a working G'MIC: {}", path.display(), failed(&o))));
            }
            let text = strip_ansi(&o.stderr);
            let version = text.lines().map(str::trim).rev().find(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit() || c == '.'));
            let version = version.ok_or_else(|| Error::NotFound(format!("{} did not report a G'MIC version", path.display())))?;
            Ok(Gmic { path: path.to_path_buf(), version: version.to_string() })
        }

        /// Runs `pipeline` (G'MIC script) on `input` and returns the resulting image list.
        pub fn run(&self, input: &[Image], pipeline: &str, opts: &RunOptions<'_>) -> Result<Vec<Image>> {
            if pipeline.trim().is_empty() {
                return Err(Error::Params("the G'MIC command is empty".into()));
            }
            if pipeline.len() > 1 << 20 || pipeline.contains('\0') {
                return Err(Error::Params("the G'MIC command is too long or contains a NUL character".into()));
            }
            let dir = TempDir::new()?;
            let mut script = format!("{RUN_COMMAND} :\n");
            for l in pipeline.lines() {
                script.push_str("  ");
                script.push_str(l);
                script.push('\n');
            }
            std::fs::write(dir.0.join("run.gmic"), script).map_err(|e| io("couldn't write the G'MIC command file", e))?;
            let mut args = vec!["-v", "-", "run.gmic"];
            if !input.is_empty() {
                std::fs::write(dir.0.join("in.cimg"), cimg::encode(input)).map_err(|e| io("couldn't write the image for G'MIC", e))?;
                args.push("in.cimg");
            }
            args.extend([RUN_COMMAND, "output", "out.cimg,float32"]);
            let o = exec(&self.path, &args, &dir.0, opts)?;
            if !o.success {
                return Err(failed(&o));
            }
            let out = dir.0.join("out.cimg");
            let len = std::fs::metadata(&out).map(|m| m.len()).map_err(|_| Error::Failed("the command produced no image".into()))?;
            if len > (MAX_SAMPLES as u64) * 8 + 1024 * 1024 {
                return Err(Error::Image("the G'MIC result is too large".into()));
            }
            let bytes = std::fs::read(&out).map_err(|e| io("couldn't read G'MIC's result", e))?;
            cimg::decode(&bytes, MAX_SAMPLES)
        }

        /// Downloads G'MIC's latest filter definitions (`gmic update`, from gmic.eu) into G'MIC's
        /// own configuration folder. The catalogue reads them.
        pub fn update(&self, opts: &RunOptions<'_>) -> Result<()> {
            let dir = TempDir::new()?;
            let o = exec(&self.path, &["-v", "-", "update"], &dir.0, opts)?;
            if o.success { Ok(()) } else { Err(failed(&o)) }
        }

        /// The raw catalogue JSON (`gmic parse_gui json`).
        pub fn catalog_json(&self, opts: &RunOptions<'_>) -> Result<Vec<u8>> {
            let dir = TempDir::new()?;
            let o = exec(&self.path, &["-v", "-", "parse_gui", "json", "output_text", "catalog.json"], &dir.0, opts)?;
            if !o.success {
                return Err(failed(&o));
            }
            let file = dir.0.join("catalog.json");
            let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            if len == 0 {
                return Err(Error::Catalog(NO_FILTERS.into()));
            }
            if len > crate::catalog::MAX_BYTES as u64 {
                return Err(Error::Catalog("the filter list is larger than the limit".into()));
            }
            std::fs::read(&file).map_err(|e| io("couldn't read G'MIC's filter list", e))
        }

        /// The catalogue: from `cache_dir` when cached for this G'MIC version (and not
        /// `refresh`), else rebuilt (update, then `parse_gui`, about a minute) and cached.
        pub fn load_catalog(&self, cache_dir: Option<&Path>, refresh: bool, opts: &RunOptions<'_>) -> Result<Catalog> {
            let safe: String = self.version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
            let cache = cache_dir.map(|d| d.join(format!("catalog-{safe}.json")));
            if !refresh && let Some(c) = &cache {
                let cached = std::fs::read(c).ok().filter(|b| b.len() <= crate::catalog::MAX_BYTES).and_then(|b| Catalog::parse(&b).ok());
                if let Some(cat) = cached.filter(|c| !c.filters.is_empty()) {
                    return Ok(cat);
                }
            }
            // Offline is fine when the definitions were downloaded before.
            if let Err(Error::Cancelled) = self.update(&opts.with_timeout(Duration::from_secs(60))) {
                return Err(Error::Cancelled);
            }
            let json = self.catalog_json(&opts.with_timeout(opts.timeout.max(CATALOG_TIMEOUT)))?;
            let cat = Catalog::parse(&json)?;
            if cat.filters.is_empty() {
                return Err(Error::Catalog(NO_FILTERS.into()));
            }
            if let (Some(c), Some(d)) = (&cache, cache_dir) {
                // A cache that can't be written only means rebuilding next time.
                if std::fs::create_dir_all(d).is_ok() {
                    let tmp = c.with_extension("json.tmp");
                    if std::fs::write(&tmp, &json).is_ok() {
                        let _ = std::fs::rename(&tmp, c);
                    }
                }
            }
            Ok(cat)
        }
    }

    pub(super) const NO_FILTERS: &str = "G'MIC returned no filters. G'MIC downloads its filter definitions from gmic.eu the first time it is used; check the internet connection, then choose Refresh Filters";
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::locate;

#[cfg(target_arch = "wasm32")]
pub fn locate(_configured: Option<&str>) -> Result<PathBuf> {
    Err(Error::Unsupported(UNSUPPORTED.into()))
}

#[cfg(target_arch = "wasm32")]
impl Gmic {
    pub fn probe(_path: &Path, _opts: &RunOptions<'_>) -> Result<Gmic> {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
    pub fn run(&self, _input: &[Image], _pipeline: &str, _opts: &RunOptions<'_>) -> Result<Vec<Image>> {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
    pub fn update(&self, _opts: &RunOptions<'_>) -> Result<()> {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
    pub fn catalog_json(&self, _opts: &RunOptions<'_>) -> Result<Vec<u8>> {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
    pub fn load_catalog(&self, _cache_dir: Option<&Path>, _refresh: bool, _opts: &RunOptions<'_>) -> Result<Catalog> {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
}

/// Finds and probes G'MIC in one step ([`locate`], then [`Gmic::probe`]).
pub fn find(configured: Option<&str>) -> Result<Gmic> {
    let path = locate(configured)?;
    Gmic::probe(&path, &RunOptions::default().with_timeout(Duration::from_secs(20)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let err = "[gmic]./ Start\n\u{1b}[0;31;59m\u{1b}[1m[gmic]./run/ *** Error in ./ *** Unknown command 'x'.\u{1b}[0;0;0m\n\n";
        assert_eq!(error_message(err).as_deref(), Some("Unknown command 'x'."));
        assert_eq!(error_message("just text\n").as_deref(), Some("just text"));
        assert_eq!(error_message("\n \n"), None);
        assert_eq!(strip_ansi("\u{1b}[1mbold\u{1b}[0m"), "bold");
        assert_eq!(strip_ansi("trailing \u{1b}"), "trailing ");
    }
}
