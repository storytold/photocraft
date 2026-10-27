//! Exercise the native 8BF host/worker IPC against the original local conformance DLLs.
//!
//! Usage: cargo run -p photocraft-plugins --example native_8bf_smoke -- \
//!   <workers-dir> <fixtures-dir> [vendor-plugin-path]
//! Worker files are named photocraft-8bf-worker32.exe and photocraft-8bf-worker64.exe.
//! Fixture subdirectories are i686/ and x86_64/.

use photocraft_plugins::native_8bf::{
    client::{self, Launch},
    pe::{self, Architecture},
    wire::Request,
};
use serde_json::{Value, json};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Target {
    label: &'static str,
    architecture: Architecture,
    fixture_dir: &'static str,
    worker_file: &'static str,
}

const TARGETS: [Target; 2] = [
    Target { label: "i686", architecture: Architecture::X86, fixture_dir: "i686", worker_file: "photocraft-8bf-worker32.exe" },
    Target { label: "x86_64", architecture: Architecture::X64, fixture_dir: "x86_64", worker_file: "photocraft-8bf-worker64.exe" },
];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn launch(worker: &Path, architecture: Architecture) -> io::Result<Launch> {
    let mut launch = Launch::from_environment(architecture)?;
    #[cfg(windows)]
    {
        launch.program = worker.to_path_buf();
        launch.args.clear();
    }
    #[cfg(not(windows))]
    {
        launch.args.clear();
        launch.args.push(worker.as_os_str().to_owned());
    }
    #[cfg(windows)]
    {
        launch.timeout = Duration::from_secs(15);
    }
    #[cfg(not(windows))]
    {
        launch.timeout = Duration::from_secs(60);
    }
    Ok(launch)
}

fn request(path: &Path, entry: &str, width: u32, height: u32, channels: u8, depth: u8) -> io::Result<Request> {
    Ok(Request { path: client::windows_path(path)?, entry: entry.to_owned(), width, height, channels, depth, show_ui: false })
}

fn sample_pixels(request: &Request) -> io::Result<Vec<u8>> {
    let length = request.pixel_len()?;
    if request.depth == 8 {
        return Ok((0u8..=255).cycle().take(length).collect());
    }
    let count = length / 2;
    let edges = [0u16, 1, 2, 32767, 32768, 32769, 65534, 65535];
    let mut bytes = Vec::with_capacity(length);
    for value in edges.into_iter().cycle().take(count) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

fn expected_pixels(pixels: &[u8], channels: u8, depth: u8, invert: bool) -> io::Result<Vec<u8>> {
    if !invert {
        return Ok(pixels.to_vec());
    }
    if depth == 8 {
        return Ok(pixels.iter().map(|value| 255 - value).collect());
    }
    if depth != 16 || !pixels.len().is_multiple_of(2) {
        return Err(invalid("unsupported or malformed expected pixel data"));
    }
    let maximum = 65535u32;
    let native_maximum = 32768u32;
    let mut output = Vec::with_capacity(pixels.len());
    for pair in pixels.as_chunks::<2>().0 {
        let sample = u16::from_le_bytes(*pair);
        let native = (u32::from(sample) * native_maximum + 32767) / maximum;
        let inverted = native_maximum - native;
        let result = if inverted == native { sample } else { ((inverted * maximum + 16384) / native_maximum) as u16 };
        output.extend_from_slice(&result.to_le_bytes());
    }
    let sample_count = output.len() / 2;
    if sample_count % usize::from(channels) != 0 {
        return Err(invalid("sample count is not a whole number of pixels"));
    }
    Ok(output)
}

fn fixture_path(fixtures: &Path, target: &Target, file: &str) -> PathBuf {
    fixtures.join(target.fixture_dir).join(file)
}

struct PixelSpec {
    width: u32,
    height: u32,
    channels: u8,
    depth: u8,
    invert: bool,
}

fn run_pixels(worker: &Path, architecture: Architecture, plugin: &Path, mode: Option<&str>, handle_case: Option<&str>, spec: PixelSpec) -> io::Result<()> {
    let mut launch = launch(worker, architecture)?;
    if let Some(mode) = mode {
        launch.environment.insert("PHOTOCRAFT_8BF_FIXTURE_MODE".into(), OsString::from(mode));
    }
    if let Some(handle_case) = handle_case {
        launch.environment.insert("PHOTOCRAFT_8BF_HANDLE_CASE".into(), OsString::from(handle_case));
    }
    let request = request(plugin, "PluginMain", spec.width, spec.height, spec.channels, spec.depth)?;
    let pixels = sample_pixels(&request)?;
    let expected = expected_pixels(&pixels, spec.channels, spec.depth, spec.invert)?;
    let actual = client::exchange(&launch, request, pixels, || false)?;
    if actual != expected {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "host IPC returned unexpected pixels"));
    }
    Ok(())
}

fn run_error_case(worker: &Path, architecture: Architecture, plugin: &Path, mode: &str) -> io::Result<()> {
    let mut launch = launch(worker, architecture)?;
    launch.environment.insert("PHOTOCRAFT_8BF_FIXTURE_MODE".into(), OsString::from(mode));
    let request = request(plugin, "PluginMain", 4, 3, 3, 8)?;
    let pixels = sample_pixels(&request)?;
    match client::exchange(&launch, request, pixels, || false) {
        Err(error) => {
            let detail = error.to_string();
            let lower = detail.to_ascii_lowercase();
            let valid = match mode {
                "cancel" => error.kind() == io::ErrorKind::Interrupted,
                "crash" | "exit_error" => {
                    error.kind() != io::ErrorKind::TimedOut
                        && error.kind() != io::ErrorKind::NotFound
                        && detail.contains("8BF worker exited")
                        && !lower.contains("timed out")
                        && !lower.contains("could not start")
                }
                "bad_rect" => detail.contains("filter requested pixels outside image bounds"),
                "start_error" => detail.contains("8BF selector 3 returned -32125"),
                _ => false,
            };
            if valid { Ok(()) } else { Err(io::Error::new(error.kind(), format!("unexpected {mode} error: {error}"))) }
        }
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, format!("{mode} unexpectedly returned pixels"))),
    }
}

fn record<F>(results: &mut Vec<Value>, architecture: &str, name: &str, run: F)
where
    F: FnOnce() -> io::Result<()>,
{
    let started = Instant::now();
    let result = run();
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(()) => results.push(json!({
            "architecture": architecture,
            "case": name,
            "passed": true,
            "elapsed_ms": elapsed_ms,
        })),
        Err(error) => {
            eprintln!("{architecture} {name}: {error}");
            results.push(json!({
                "architecture": architecture,
                "case": name,
                "passed": false,
                "elapsed_ms": elapsed_ms,
                "error": error.to_string(),
            }));
        }
    }
}

fn run_target(target: &Target, workers: &Path, fixtures: &Path, results: &mut Vec<Value>) {
    let worker = workers.join(target.worker_file);
    let conformance = fixture_path(fixtures, target, "photocraft-conformance.8bf");
    let host_services = fixture_path(fixtures, target, "photocraft-host-services.8bf");
    let handle_services = fixture_path(fixtures, target, "photocraft-handle-services.8bf");

    for (channels, depth, label) in [(3, 8, "rgb8"), (1, 8, "gray8"), (3, 16, "rgb16"), (1, 16, "gray16")] {
        for (mode, invert) in [("identity", false), ("invert", true)] {
            let name = format!("{label}_{mode}");
            record(results, target.label, &name, || {
                run_pixels(&worker, target.architecture, &conformance, Some(mode), None, PixelSpec { width: 8, height: 4, channels, depth, invert })
            });
        }
    }

    record(results, target.label, "logging_noise", || {
        run_pixels(&worker, target.architecture, &conformance, Some("noise"), None, PixelSpec { width: 8, height: 4, channels: 3, depth: 8, invert: true })
    });
    record(results, target.label, "threaded_alloc", || {
        run_pixels(
            &worker,
            target.architecture,
            &conformance,
            Some("threaded_alloc"),
            None,
            PixelSpec { width: 8, height: 4, channels: 3, depth: 8, invert: true },
        )
    });
    for mode in ["cancel", "crash", "exit_error", "bad_rect", "start_error"] {
        let name = mode.to_owned();
        record(results, target.label, &name, || run_error_case(&worker, target.architecture, &conformance, mode));
    }
    record(results, target.label, "property_services", || {
        run_pixels(
            &worker,
            target.architecture,
            &host_services,
            Some("identity"),
            None,
            PixelSpec { width: 2, height: 1, channels: 3, depth: 8, invert: false },
        )
    });
    for case in ["owned", "direct", "indirect"] {
        let name = format!("handle_services_{case}");
        record(results, target.label, &name, || {
            run_pixels(
                &worker,
                target.architecture,
                &handle_services,
                Some("identity"),
                Some(case),
                PixelSpec { width: 2, height: 1, channels: 3, depth: 8, invert: false },
            )
        });
    }
}

fn run_vendor(worker_dir: &Path, plugin_path: &Path, results: &mut Vec<Value>) {
    let canonical = match fs::canonicalize(plugin_path) {
        Ok(path) => path,
        Err(error) => {
            record(results, "unknown", "vendor_no_ui", || Err(error));
            return;
        }
    };
    let metadata = match pe::inspect_file(&canonical) {
        Ok(metadata) => metadata,
        Err(error) => {
            record(results, "unknown", "vendor_no_ui", || Err(error));
            return;
        }
    };
    let (architecture, worker_file, label) = match metadata.architecture {
        Architecture::X86 => (Architecture::X86, "photocraft-8bf-worker32.exe", "i686"),
        Architecture::X64 => (Architecture::X64, "photocraft-8bf-worker64.exe", "x86_64"),
    };
    record(results, label, "vendor_no_ui", || {
        let worker = worker_dir.join(worker_file);
        let mut launch = launch(&worker, architecture)?;
        launch.timeout = launch.timeout.min(Duration::from_secs(60));
        let request = request(&canonical, &metadata.entry, 2, 2, 3, 8)?;
        let pixels = sample_pixels(&request)?;
        let output = client::exchange(&launch, request, pixels.clone(), || false)?;
        if output.len() != pixels.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "vendor smoke returned an unexpected pixel length"));
        }
        Ok(())
    });
}

fn main() -> io::Result<()> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err(invalid("usage: native_8bf_smoke <workers-dir> <fixtures-dir> [vendor-plugin-path]"));
    }
    let workers = fs::canonicalize(PathBuf::from(args.first().cloned().ok_or_else(|| invalid("missing workers directory"))?))?;
    let fixtures = fs::canonicalize(PathBuf::from(args.get(1).cloned().ok_or_else(|| invalid("missing fixtures directory"))?))?;
    if !workers.is_dir() || !fixtures.is_dir() {
        return Err(invalid("workers and fixtures arguments must be directories"));
    }
    let mut results = Vec::new();
    for target in &TARGETS {
        run_target(target, &workers, &fixtures, &mut results);
    }
    if let Some(plugin) = args.get(2) {
        run_vendor(&workers, Path::new(plugin), &mut results);
    }

    let failed = results.iter().filter(|result| result.get("passed").and_then(Value::as_bool) != Some(true)).count();
    let report = json!({
        "suite": "PhotoCraft native 8BF host IPC smoke",
        "passed": failed == 0,
        "case_count": results.len(),
        "failure_count": failed,
        "results": results,
    });
    let serialized = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    let report_path = env::current_dir()?.join("target/8bf-validation/native-ipc-smoke.json");
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&report_path, &serialized)?;
    println!("{}", String::from_utf8_lossy(&serialized));
    eprintln!("report: {}", report_path.display());
    if failed == 0 { Ok(()) } else { Err(io::Error::other(format!("{failed} native 8BF smoke case(s) failed"))) }
}
