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
//! the 25,165,824-byte gate.
//!
//! The web build embeds no craft fonts: it fetches the faces listed in `web-fonts.txt` (the
//! generated `WEB_FONTS`) from `fonts/<sha16>/` beside the wasm, where
//! `cargo xtask web-fonts` copies them (`apps/photocraft-web/src/fonts.rs` fetches them). Add a line
//! there to serve another face, e.g. BIZ UDPGothic for Japanese. A list line the manifest lacks is
//! left out with a warning, never an error, and never costs the desktop its fonts.
use std::fmt::Write as _;
use std::path::PathBuf;

#[path = "build/web_fonts.rs"]
mod web_fonts;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    println!("cargo::rerun-if-changed=web-fonts.txt");
    println!("cargo::rerun-if-changed=build/web_fonts.rs");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let (mut fonts, mut web) = (String::new(), String::new());
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) {
        // The web build embeds no craft fonts (size gate, see above); it fetches WEB_FONTS instead.
        if !wasm {
            match craft_fonts(&dir) {
                Ok(entries) => fonts = entries,
                Err(e) if required => println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display()),
                Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
            }
        }
        // Separately: a bad web list must not cost the desktop build its craft fonts. A listed face
        // the checkout lacks (an older craft-fonts pin) is left out with a warning, never an error,
        // so the apps don't depend on which craft-fonts commit CI pins: that build just fetches
        // fewer fonts.
        match web_font_entries(&dir) {
            Ok((entries, missing)) => {
                web = entries;
                if !missing.is_empty() {
                    let msg = format!("{} not in CRAFT_FONTS_DIR={} (an older craft-fonts?)", missing.join(", "), dir.display());
                    println!("cargo::warning=web build won't fetch: {msg}");
                }
            }
            Err(e) if required => println!("cargo::error=web-fonts.txt: {e}"),
            Err(e) => println!("cargo::warning=web build fetches no craft fonts: web-fonts.txt: {e}"),
        }
    }
    let src = format!("pub static CRAFT_FONTS: &[CraftFont] = &[\n{fonts}];\npub static WEB_FONTS: &[WebFont] = &[\n{web}];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per manifest line.
fn craft_fonts(dir: &std::path::Path) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
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

/// One `WebFont { .. }` initialiser per `web-fonts.txt` line, from its craft-fonts manifest row, and
/// the listed faces the manifest lacks (an older craft-fonts checkout), which are left out.
fn web_font_entries(dir: &std::path::Path) -> Result<(String, Vec<String>), String> {
    let list_path = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("web-fonts.txt");
    let list = std::fs::read_to_string(&list_path).map_err(|e| format!("{}: {e}", list_path.display()))?;
    let manifest_path = dir.join("fonts/manifest.txt");
    // WEB_FONTS follows the manifest (URLs and hashes), on every target.
    println!("cargo::rerun-if-changed={}", manifest_path.display());
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let rows: Vec<Vec<&str>> =
        manifest.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split(" | ").map(str::trim).collect()).collect();
    let (mut out, mut missing) = (String::new(), Vec::new());
    for e in web_fonts::parse_list(&list)? {
        let Some(row) = rows.iter().find(|r| r.first() == Some(&e.family) && r.get(1) == Some(&e.style)) else {
            missing.push(format!("{} {}", e.family, e.style));
            continue;
        };
        let (Some(file), Some(sha)) = (row.get(2), row.get(6)) else {
            return Err(format!("malformed manifest row for {} {}", e.family, e.style));
        };
        let integrity = web_fonts::sri(sha).ok_or_else(|| format!("bad sha256 for {} {}", e.family, e.style))?;
        let _ = writeln!(
            out,
            "    WebFont {{ family: {:?}, style: {:?}, startup: {}, url: {:?}, integrity: {:?} }},",
            e.family,
            e.style,
            e.startup,
            web_fonts::url(sha, file),
            integrity,
        );
    }
    Ok((out, missing))
}
