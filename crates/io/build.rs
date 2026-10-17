//! Download PDFium with a bounded timeout and copy it beside build outputs.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const PDFIUM_VERSION: &str = "8100";
const DOWNLOAD_TIMEOUT_SECONDS: &str = "30";
const MAX_ARCHIVE_BYTES: &str = "100000000";

fn main() {
    println!("cargo:rerun-if-env-changed=PDFIUM_DYNAMIC_LIB_PATH");

    if env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "wasm32" {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let binary_name = match (target_os.as_str(), target_arch.as_str()) {
        ("windows", "x86_64" | "x86" | "aarch64") => "pdfium.dll",
        ("linux", "x86_64" | "aarch64") => "libpdfium.so",
        ("macos", "x86_64" | "aarch64") => "libpdfium.dylib",
        _ => {
            info(format!("unsupported platform {}-{}, skipping", target_os, target_arch));
            return;
        }
    };

    let Some(source) = env::var_os("PDFIUM_DYNAMIC_LIB_PATH").map(PathBuf::from) else {
        let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
            info(format!("OUT_DIR is not set; PDF import requires {} at runtime", binary_name));
            return;
        };

        let library_path = out_dir.join(binary_name);
        let license_path = out_dir.join("PDFium-LICENSE.txt");
        let third_party_licenses_path = out_dir.join("PDFium-Third-Party-Licenses.txt");
        if !library_path.is_file() || !license_path.is_file() || !third_party_licenses_path.is_file() {
            let asset_name = match (target_os.as_str(), target_arch.as_str()) {
                ("windows", "x86_64") => "pdfium-win-x64.tgz",
                ("windows", "x86") => "pdfium-win-x86.tgz",
                ("windows", "aarch64") => "pdfium-win-arm64.tgz",
                ("linux", "x86_64") => "pdfium-linux-x64.tgz",
                ("linux", "aarch64") => "pdfium-linux-arm64.tgz",
                ("macos", "x86_64") => "pdfium-mac-x64.tgz",
                ("macos", "aarch64") => "pdfium-mac-arm64.tgz",
                _ => return,
            };
            let url = format!("https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F{PDFIUM_VERSION}/{asset_name}");
            let archive_path = out_dir.join(asset_name);

            info(format!("downloading {} (maximum {} seconds)", asset_name, DOWNLOAD_TIMEOUT_SECONDS));
            match download_file(&url, &archive_path).and_then(|()| extract_pdfium(&archive_path, &library_path, binary_name)) {
                Ok(()) => info(format!("downloaded {}", binary_name)),
                Err(error) => {
                    let _ = fs::remove_file(&library_path);
                    let _ = fs::remove_file(&license_path);
                    let _ = fs::remove_file(&third_party_licenses_path);
                    info(format!("download unavailable: {error}"));
                    info(format!("PDF import requires {} at runtime", binary_name));
                }
            }
            let _ = fs::remove_file(archive_path);
        }

        let Some(profile_dir) = profile_dir() else {
            info("could not find Cargo profile directory; PDFium was not copied");
            return;
        };

        copy_to_profile(&library_path, &profile_dir, binary_name);
        copy_to_profile(&license_path, &profile_dir, "PDFium-LICENSE.txt");
        copy_to_profile(&third_party_licenses_path, &profile_dir, "PDFium-Third-Party-Licenses.txt");
        return;
    };

    if !source.is_file() {
        info(format!("PDFIUM_DYNAMIC_LIB_PATH does not point to a file: {}", source.display()));
        return;
    }

    let Some(profile_dir) = profile_dir() else {
        info("could not find Cargo profile directory; PDFium was not copied");
        return;
    };

    copy_to_profile(&source, &profile_dir, binary_name);
}

fn profile_dir() -> Option<PathBuf> {
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let out_dir = env::var_os("OUT_DIR").map(PathBuf::from)?;
    out_dir.ancestors().find(|ancestor| ancestor.file_name().is_some_and(|name| name == profile.as_str())).map(PathBuf::from)
}

fn info(message: impl std::fmt::Display) {
    eprintln!("info: pdfium: {message}");
}

fn copy_to_profile(source: &std::path::Path, profile_dir: &std::path::Path, binary_name: &str) {
    let destination = profile_dir.join(binary_name);
    if let Err(error) = fs::copy(source, &destination) {
        info(format!("failed to copy {} to {}: {}", source.display(), destination.display(), error));
    } else {
        info(format!("copied to {}", destination.display()));
    }
}

fn download_file(url: &str, destination: &std::path::Path) -> Result<(), String> {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let output = Command::new(curl)
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            DOWNLOAD_TIMEOUT_SECONDS,
            "--max-filesize",
            MAX_ARCHIVE_BYTES,
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .output()
        .map_err(|error| format!("could not start {curl}: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let _ = fs::remove_file(destination);
        Err(format!("curl exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn extract_pdfium(archive_path: &std::path::Path, destination: &std::path::Path, binary_name: &str) -> Result<(), String> {
    use std::io::Read;

    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);

    let output_dir = destination.parent().ok_or_else(|| "PDFium output directory is unavailable".to_string())?;
    let mut library_found = false;
    let mut license = None;
    let mut third_party_licenses = Vec::new();
    let entries = archive.entries().map_err(|error| error.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().map_err(|error| error.to_string())?.into_owned();
        if path.file_name().and_then(|name| name.to_str()) == Some(binary_name) {
            entry.unpack(destination).map_err(|error| error.to_string())?;
            library_found = true;
        } else if path == std::path::Path::new("LICENSE") {
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).map_err(|error| error.to_string())?;
            license = Some(contents);
        } else if let Ok(relative_path) = path.strip_prefix("licenses")
            && !relative_path.as_os_str().is_empty()
            && entry.header().entry_type().is_file()
        {
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).map_err(|error| error.to_string())?;
            third_party_licenses.push((relative_path.to_string_lossy().into_owned(), contents));
        }
    }

    if !library_found {
        return Err(format!("{binary_name} not found in the downloaded archive"));
    }
    let license = license.ok_or_else(|| "PDFium license not found in the downloaded archive".to_string())?;
    if third_party_licenses.is_empty() {
        return Err("PDFium third-party licenses not found in the downloaded archive".to_string());
    }

    fs::write(output_dir.join("PDFium-LICENSE.txt"), license).map_err(|error| error.to_string())?;
    third_party_licenses.sort_by(|left, right| left.0.cmp(&right.0));
    let mut notices = Vec::new();
    for (name, contents) in third_party_licenses {
        notices.extend_from_slice(format!("===== {name} =====\n").as_bytes());
        notices.extend_from_slice(&contents);
        if !contents.ends_with(b"\n") {
            notices.push(b'\n');
        }
        notices.push(b'\n');
    }
    fs::write(output_dir.join("PDFium-Third-Party-Licenses.txt"), notices).map_err(|error| error.to_string())
}
