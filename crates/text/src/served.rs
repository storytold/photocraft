//! Fonts a host serves next to the app, fetched on demand (the web build: no system fonts, and
//! the wasm has no room for more embedded ones, see `build.rs`).
//!
//! The host lists them in `fonts/manifest.txt` next to `index.html`, in craft-fonts' manifest
//! format (`family | style | file | …`, the file relative to the site root), so a craft-fonts
//! checkout's `fonts/` folder works as it is and any other font needs one line. This module is
//! the part that doesn't depend on the platform: reading the manifest, the catalog of families
//! the host can serve, and the queue of families to fetch (picked in a font menu, or needed by a
//! layout). The web shell does the fetching and registers what arrives with
//! [`crate::FontDb::register_font_data`].

use std::sync::{Mutex, PoisonError};

/// One font file listed in a served font manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedFont {
    /// The family the file registers as (its typographic family name, `name` ID 16, else ID 1).
    pub family: String,
    /// Path relative to the site root, e.g. `fonts/open-sans/OpenSans-Regular.ttf`.
    pub file: String,
}

/// Reads a manifest in craft-fonts' format: one font file per line, fields separated by ` | `
/// (`family | style | file | scripts | licence | licence file | sha256 | source`); `#` comments
/// and blank lines are ignored. Only family and file are needed here, so a line needs at least
/// `family | style | file`. Returns the fonts and, for each line that was skipped, why.
pub fn parse_manifest(text: &str) -> (Vec<ServedFont>, Vec<String>) {
    let mut fonts = Vec::new();
    let mut skipped = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        match f.as_slice() {
            [family, _style, file, ..] if !family.is_empty() && is_relative_path(file) => {
                fonts.push(ServedFont { family: (*family).to_string(), file: (*file).to_string() });
            }
            [_, _, file, ..] if !is_relative_path(file) => skipped.push(format!("{line}: the file must be a relative path inside the site")),
            _ => skipped.push(format!("{line}: expected `family | style | file | …`")),
        }
    }
    (fonts, skipped)
}

/// A relative path that stays inside the site: no scheme, no leading `/`, no `..` segment, no
/// query or fragment.
fn is_relative_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.contains(['\\', '?', '#', ':']) && p.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

struct State {
    /// Families the host can serve, installed or not.
    families: Vec<String>,
    /// Families to fetch, not yet taken by [`take_requests`].
    requested: Vec<String>,
    /// Bumped when `families` changes, so font menus refresh.
    generation: u64,
}

static STATE: Mutex<State> = Mutex::new(State { families: Vec::new(), requested: Vec::new(), generation: 0 });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Adds families the host can serve (the web shell, after reading the manifest).
pub fn add_families(families: impl IntoIterator<Item = String>) {
    let mut s = state();
    let before = s.families.len();
    for f in families {
        if !s.families.contains(&f) {
            s.families.push(f);
        }
    }
    if s.families.len() != before {
        s.generation = s.generation.wrapping_add(1);
    }
}

/// Families the host can serve, installed or not (for font menus).
pub fn families() -> Vec<String> {
    state().families.clone()
}

/// Changes whenever [`families`] does.
pub fn generation() -> u64 {
    state().generation
}

/// Asks for a family to be fetched: picked in a font menu, or needed by a layout that doesn't
/// have it. Families the host doesn't serve are ignored; asking again is harmless.
pub fn request(family: &str) {
    let mut s = state();
    if s.families.iter().any(|f| f == family) && !s.requested.iter().any(|f| f == family) {
        s.requested.push(family.to_string());
    }
}

/// The families asked for since the last call (the web shell fetches each one once).
pub fn take_requests() -> Vec<String> {
    std::mem::take(&mut state().requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_craft_fonts_manifest_lines() {
        let text = "# Font manifest for the Crafting Apps.\n\
            \n\
            BIZ UDPGothic | Regular | fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf | Jpan,Latn | OFL-1.1 | fonts/biz-ud-pgothic/OFL.txt | 258d | https://example.org\n\
            Open Sans | Regular | fonts/open-sans/OpenSans[wdth,wght].ttf\n";
        let (fonts, skipped) = parse_manifest(text);
        assert_eq!(
            fonts,
            [
                ServedFont { family: "BIZ UDPGothic".into(), file: "fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf".into() },
                ServedFont { family: "Open Sans".into(), file: "fonts/open-sans/OpenSans[wdth,wght].ttf".into() },
            ]
        );
        assert!(skipped.is_empty(), "{skipped:?}");
    }

    #[test]
    fn skips_malformed_lines_and_paths_outside_the_site() {
        let text = "Lato | Regular\n\
            | Regular | fonts/x.ttf\n\
            A | Regular | /fonts/a.ttf\n\
            B | Regular | ../b.ttf\n\
            C | Regular | fonts/../../c.ttf\n\
            D | Regular | https://example.org/d.ttf\n\
            E | Regular | fonts//e.ttf\n\
            F | Regular | fonts\\f.ttf\n\
            G | Regular | fonts/g.ttf?v=1\n\
            Lato | Bold | fonts/lato/Lato-Bold.ttf\n";
        let (fonts, skipped) = parse_manifest(text);
        assert_eq!(fonts, [ServedFont { family: "Lato".into(), file: "fonts/lato/Lato-Bold.ttf".into() }]);
        assert_eq!(skipped.len(), 9, "{skipped:?}");
    }

    /// One test for the whole queue: the catalog and the queue are process-wide, and parallel
    /// tests taking each other's requests would be flaky.
    #[test]
    fn requests_are_limited_to_the_catalog_and_reach_layout() {
        let served = "Served Test Sans";
        let g = generation();
        add_families([served.to_string(), served.to_string()]);
        assert_eq!(families().iter().filter(|f| *f == served).count(), 1);
        assert_ne!(generation(), g, "the catalog changed");
        let g = generation();
        add_families([served.to_string()]);
        assert_eq!(generation(), g, "nothing new");

        request("Not Served Test Sans");
        request(served);
        request(served);
        let taken = take_requests();
        assert_eq!(taken.iter().filter(|f| *f == served).count(), 1, "{taken:?}");
        assert!(!taken.iter().any(|f| f == "Not Served Test Sans"));

        // A layout that needs a served family the database lacks asks for it (and is drawn with
        // the fallback meanwhile).
        let t = photocraft_doc::TextLayer { text: "Hi".into(), font_family: served.into(), size_pt: 12.0, ..Default::default() };
        let l = crate::TextEngine::new().layout(&t, 72.0);
        assert_eq!(l.glyphs.len(), 2);
        assert!(take_requests().iter().any(|f| f == served));
    }
}
