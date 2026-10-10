//! Process-level AVIF export through convert, run and batch, including builds without AVIF.

use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(feature = "avif")]
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_codecs::{ChannelLayout, Format, Image};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("pc-cli-avif-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_photocraft-cli"))
}

fn write_png(path: &Path) {
    let mut pixels = Vec::new();
    for y in 0..23 {
        for x in 0..37 {
            pixels.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, (x * 3 + y * 5) as u8, if x < 18 { 255 } else { 128 }]);
        }
    }
    let image = Image::from_u8(37, 23, ChannelLayout::Rgba, pixels).unwrap();
    std::fs::write(path, photocraft_codecs::encode(&image, Format::Png, &Default::default()).unwrap()).unwrap();
}

#[cfg(feature = "avif")]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "avif")]
fn assert_avif(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(photocraft_codecs::detect(&bytes), Some(Format::Avif));
    assert!(bytes.len() > 32, "only a container header was written");
    bytes
}

#[cfg(not(feature = "avif"))]
#[test]
fn unsupported_avif_export_preserves_the_existing_output() {
    let directory = TestDirectory::new();
    let source = directory.join("source.png");
    let target = directory.join("existing.avif");
    write_png(&source);
    std::fs::write(&target, b"keep existing output").unwrap();
    let output = bin().arg("convert").arg(&source).arg(&target).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("AVIF") && error.contains("cannot be written in this build"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep existing output");
}

#[cfg(feature = "avif")]
#[test]
fn convert_exports_avif_and_passes_quality_to_the_encoder() {
    let directory = TestDirectory::new();
    let source = directory.join("source.png");
    let low_path = directory.join("low.AVIF");
    let high_path = directory.join("high.avif");
    write_png(&source);
    let imported = photocraft_io::import("source.png", &std::fs::read(&source).unwrap()).unwrap().document;
    let mut results = Vec::new();
    for (q, target) in [(10, &low_path), (95, &high_path)] {
        let output = bin().arg("convert").arg(&source).arg(target).args(["--quality", &q.to_string()]).output().unwrap();
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("warning: lossy compression"));
        let bytes = assert_avif(target);
        let mut options = photocraft_io::ExportOptions::default();
        options.encode.jpeg_quality = q;
        // The process command must carry its quality through automation/file-save to IO.
        assert_eq!(bytes, photocraft_io::export(&imported, "avif", &options).unwrap().bytes);
        results.push(bytes);
    }
    assert_ne!(results[0], results[1], "CLI ignored --quality for AVIF");

    let target = directory.join("override.bin");
    let output = bin().arg("convert").arg(&source).arg(&target).args(["--format", "avif", "--quality=95"]).output().unwrap();
    assert_success(&output);
    assert_eq!(assert_avif(&target), results[1], "explicit --format differs from the .avif extension path");

    // The export-only feature must fail honestly when the user asks to reopen its output.
    let output = bin().arg("info").arg(&high_path).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("decoding is not available"));
}

#[cfg(feature = "avif")]
#[test]
fn run_and_batch_can_save_avif() {
    let directory = TestDirectory::new();
    let new_output = directory.join("new.avif");
    let output = bin()
        .args(["run", "--new", r#"{"width":32,"height":16,"background":"transparent"}"#, "--out"])
        .arg(&new_output)
        .args(["--quality", "80"])
        .output()
        .unwrap();
    assert_success(&output);
    assert_avif(&new_output);

    let input_directory = directory.join("input");
    let output_directory = directory.join("output");
    std::fs::create_dir(&input_directory).unwrap();
    write_png(&input_directory.join("first.png"));
    write_png(&input_directory.join("second.png"));
    let actions = directory.join("actions.json");
    std::fs::write(&actions, "[]").unwrap();
    let output = bin()
        .arg("batch")
        .arg("--actions")
        .arg(&actions)
        .arg("--in")
        .arg(&input_directory)
        .arg("--out")
        .arg(&output_directory)
        .args(["--format", ".avif", "--quality", "80"])
        .output()
        .unwrap();
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 succeeded, 0 failed"));
    assert_avif(&output_directory.join("first.avif"));
    assert_avif(&output_directory.join("second.avif"));
}

#[test]
fn bad_avif_quality_does_not_replace_an_existing_file() {
    let directory = TestDirectory::new();
    let source = directory.join("source.png");
    let target = directory.join("existing.avif");
    write_png(&source);
    std::fs::write(&target, b"keep existing output").unwrap();
    for invalid in ["0", "101", "NaN", "80.5"] {
        let output = bin().arg("convert").arg(&source).arg(&target).args(["--quality", invalid]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("bad --quality"));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep existing output");
    }
}
