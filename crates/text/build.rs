//! Optional craft-fonts build input (https://github.com/storytold/craft-fonts, recipe from its
//! `docs/integration.md`; rules in `craftrules/standards/fonts.md`).
//!
//! With `CRAFT_FONTS_DIR=<craft-fonts checkout>`, every font in its `fonts/manifest.txt` is
//! embedded and exposed as `photocraft_text::CRAFT_FONTS`; unset, `CRAFT_FONTS` is empty and the
//! build is exactly as before (an empty value counts as unset). Use an absolute path: build
//! scripts run in the crate's directory, so a relative one resolves from `crates/text`. A bad
//! checkout is a warning, or an error with `CRAFT_FONTS_REQUIRED=1` (release builds).
//!
//! PhotoCraft difference from the recipe: the web build (wasm32) embeds nothing. The wasm must
//! stay under the 24 MiB gate in `packaging/web/package.sh` (#237; Cloudflare's per-file cap is
//! 25 MiB). Measured 2026-10-06 (`trunk build --release`): 24,190,076 bytes
//! without craft-fonts, 28,867,488 with only the UI face (BIZ UDPGothic Regular) embedded, over
//! the 25,165,824-byte gate. The web build keeps no Japanese font until craft-fonts can be
//! served next to the wasm instead of inside it.
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let mut entries = String::new();
    let mut font_count = 0;
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty() && !wasm).map(PathBuf::from) {
        match craft_fonts(&dir) {
            Ok((data, count)) => {
                entries = data;
                font_count = count;
            }
            Err(e) if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() => {
                println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display());
            }
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }
    // OnceLock has interior mutability, so reference a named static rather than
    // borrowing an array temporary (Rust correctly rejects that promotion).
    let src = format!("static CRAFT_FONT_STORAGE: [CraftFont; {font_count}] = [\n{entries}];\npub static CRAFT_FONTS: &[CraftFont] = &CRAFT_FONT_STORAGE;\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per manifest line.
fn craft_fonts(dir: &std::path::Path) -> Result<(String, usize), String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = String::new();
    let mut count = 0;
    for (index, line) in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).enumerate() {
        count = index.saturating_add(1);
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        // Cap the input and output, then verify the lossless compressed payload before embedding.
        const MAX_FONT_BYTES: u64 = 32 << 20;
        let mut original = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| format!("{file}: {e}"))?
            .take(MAX_FONT_BYTES + 1)
            .read_to_end(&mut original)
            .map_err(|e| format!("{file}: {e}"))?;
        if original.is_empty() || original.len() as u64 > MAX_FONT_BYTES {
            return Err(format!("{file}: font must be nonempty and at most 32 MiB"));
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&original).map_err(|e| format!("{file}: compression: {e}"))?;
        let compressed = encoder.finish().map_err(|e| format!("{file}: compression: {e}"))?;
        let mut verified = Vec::new();
        flate2::read::MultiGzDecoder::new(compressed.as_slice())
            .take(MAX_FONT_BYTES + 1)
            .read_to_end(&mut verified)
            .map_err(|e| format!("{file}: compression verification: {e}"))?;
        if verified != original {
            return Err(format!("{file}: compression changed font bytes"));
        }
        let compressed_path = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?).join(format!("craft-font-{index}.gz"));
        std::fs::write(&compressed_path, &compressed).map_err(|e| format!("writing {}: {e}", compressed_path.display()))?;
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], compressed: include_bytes!({:?}), original_len: {}, decoded: OnceLock::new(), #[cfg(test)] original: include_bytes!({:?}) }},",
            scripts.join(", "),
            compressed_path.display().to_string(),
            original.len(),
            path.display().to_string(),
        );
    }
    Ok((out, count))
}
