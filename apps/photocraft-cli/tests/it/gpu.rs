//! Tests for photocraft-cli GPU rendering flags, precedence, and fallback behavior.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn bin() -> Command {
    let mut cmd = Command::new(crate::binary());
    cmd.env_remove("PHOTOCRAFT_CONTROL_PORT");
    cmd.env_remove("PHOTOCRAFT_CONTROL_TOKEN");
    cmd.env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE");
    cmd.env_remove("PHOTOCRAFT_GPU");
    cmd
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pc-cli-gpu-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_png(path: &Path, w: u32, h: u32) {
    let data: Vec<u8> = (0..w * h * 4).map(|i| (i as u8).wrapping_mul(7)).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, data).unwrap();
    std::fs::write(path, photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
}

#[test]
fn gpu_and_no_gpu_conflict() {
    let d = tmp("conflict");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    let o = bin().args(["convert", "--gpu", "--no-gpu", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("cannot be used with") || stderr.contains("conflicts with"), "{stderr}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn gpu_env_var_validation() {
    let d = tmp("env_val");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    // Invalid non-empty value rejected with usage error naming the variable
    for bad in ["2", "true", "yes", "false", "on", "off"] {
        let o = bin().env("PHOTOCRAFT_GPU", bad).args(["convert", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "expected exit 2 for PHOTOCRAFT_GPU={bad}");
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(stderr.contains("PHOTOCRAFT_GPU"), "expected stderr to name variable: {stderr}");
        assert!(stderr.contains("expected '1' or '0'"), "expected usage note: {stderr}");
    }

    // Empty or blank value is accepted as unset (CPU default)
    for empty in ["", "   "] {
        let out_sub = d.join(format!("out_{}.png", empty.trim()));
        let o = bin().env("PHOTOCRAFT_GPU", empty).args(["convert", in_png.to_str().unwrap(), out_sub.to_str().unwrap()]).output().unwrap();
        assert!(o.status.success(), "expected success for PHOTOCRAFT_GPU={empty:?}");
        assert!(out_sub.is_file());
    }

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn explicit_flags_override_environment() {
    let d = tmp("override");
    let in_png = d.join("in.png");
    let out1 = d.join("out1.png");
    let out2 = d.join("out2.png");
    write_png(&in_png, 16, 16);

    // --no-gpu overrides PHOTOCRAFT_GPU=1
    let o = bin().env("PHOTOCRAFT_GPU", "1").args(["convert", "--no-gpu", in_png.to_str().unwrap(), out1.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out1.is_file());

    // --gpu overrides PHOTOCRAFT_GPU=0
    let o = bin().env("PHOTOCRAFT_GPU", "0").args(["convert", "--gpu", in_png.to_str().unwrap(), out2.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out2.is_file());

    // Explicit flag overrides even bad environment value
    let out3 = d.join("out3.png");
    let o = bin().env("PHOTOCRAFT_GPU", "invalid").args(["convert", "--no-gpu", in_png.to_str().unwrap(), out3.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out3.is_file());

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn mcp_bridge_rejects_explicit_gpu_flags_and_ignores_env() {
    // Explicit --gpu with --bridge rejected
    let o = bin().args(["mcp", "--bridge", "127.0.0.1:50000", "--gpu"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("configure rendering on the desktop app"), "{stderr}");

    // Explicit --no-gpu with --bridge rejected
    let o = bin().args(["mcp", "--bridge", "127.0.0.1:50000", "--no-gpu"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("configure rendering on the desktop app"), "{stderr}");
}

#[test]
fn convert_and_run_honor_gpu_flag_and_preserve_stdout_protocol() {
    let d = tmp("protocol");
    let out_png = d.join("out.png");

    // run --gpu output on stdout is clean JSON
    let o =
        bin().args(["run", "--gpu", "--new", r#"{"width":32,"height":32}"#, "--cmd", "layer.new.layer", "--out", out_png.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());
    let stdout = String::from_utf8_lossy(&o.stdout);
    for line in stdout.lines() {
        let v: Value = serde_json::from_str(line).expect("each stdout line must be valid JSON");
        assert!(v.get("command").is_some());
    }

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn batch_recovers_renderer_after_failed_item_and_preserves_gpu_policy() {
    let d = tmp("batch_regress");
    let in_dir = d.join("in");
    let out_dir = d.join("out");
    std::fs::create_dir_all(&in_dir).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    // First item is corrupt and fails to open
    let corrupt_png = in_dir.join("01_corrupt.png");
    std::fs::write(&corrupt_png, b"invalid png bytes").unwrap();

    // Second item is valid
    let valid_png = in_dir.join("02_valid.png");
    write_png(&valid_png, 32, 32);

    let o = bin().args(["batch", "--gpu", "--in", in_dir.to_str().unwrap(), "--out", out_dir.to_str().unwrap()]).output().unwrap();

    // Exit code 1 because 1 file failed
    assert_eq!(o.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);

    assert!(stdout.contains("1 succeeded, 1 failed"), "stdout: {stdout}");
    assert!(stderr.contains("FAIL"), "stderr must report failing item: {stderr}");
    assert!(out_dir.join("02_valid.png").is_file(), "second item must have been saved successfully");

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn diagnostics_preserve_clean_stdout_and_emit_to_stderr() {
    let d = tmp("diag_clean");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 32, 32);

    let o = bin().args(["convert", "--gpu", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();

    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());
    // Convert emits no stdout; diagnostics must go to stderr
    assert!(o.stdout.is_empty(), "convert stdout must remain completely empty, got: {:?}", String::from_utf8_lossy(&o.stdout));

    let _ = std::fs::remove_dir_all(&d);
}
