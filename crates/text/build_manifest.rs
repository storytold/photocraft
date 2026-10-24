//! Read the optional craft-fonts build input, shared by build.rs and its regression tests.
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One `CraftFont { .. }` initialiser per manifest line, or an error for the entire checkout.
pub fn craft_fonts(dir: &Path) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let checkout = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let fonts = checkout.join("fonts");
    println!("cargo::rerun-if-changed={}", fonts.display());
    let root = fonts.canonicalize().map_err(|e| format!("fonts/: {e}"))?;
    // The caller chooses the checkout (which may itself be a symlink), not the manifest. A
    // redirected fonts/ directory must not be able to redefine the trusted root.
    if root != fonts || !root.is_dir() {
        return Err("fonts/ must be a directory inside the configured checkout, not a redirected link".into());
    }
    let manifest_path = font_file(&checkout, &root, "fonts/manifest.txt")?;
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        // The pinned craft-fonts format has eight fields, with lower-case SHA-256 at column 7.
        let [family, style, file, scripts, _licence, _licence_file, hash, _source] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        let path = font_file(&checkout, &root, file)?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(format!("{file}: SHA-256 must be 64 lower-case hex digits"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
        if format!("{:x}", Sha256::digest(&bytes)) != *hash {
            return Err(format!("{file}: SHA-256 mismatch"));
        }
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(out)
}

/// Accept portable, checkout-relative font paths only, then resolve links and check containment.
fn font_file(checkout: &Path, root: &Path, file: &str) -> Result<PathBuf, String> {
    // The manifest uses '/' on every host. Reject Windows prefixes/separators on Unix too, so
    // one checkout cannot have different trust boundaries depending on where it is built.
    let mut parts = file.split('/');
    if parts.next() != Some("fonts") || file.contains(['\\', ':']) || parts.any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(format!("{file}: expected a relative path inside fonts/ without traversal"));
    }
    let candidate = checkout.join(file);
    // Watch the lexical name as well as its target: retargeting a link or fixing a missing font
    // must invalidate Cargo's cached optional-font result too.
    println!("cargo::rerun-if-changed={}", candidate.display());
    let path = candidate.canonicalize().map_err(|e| format!("{file}: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("{file}: not a regular file inside fonts/ (outside paths are forbidden)"));
    }
    println!("cargo::rerun-if-changed={}", path.display());
    Ok(path)
}
