//! Font database: bundled fonts (always available, also on the web), the optional craft-fonts
//! Japanese fonts ([`crate::craft_fonts`], when built with `CRAFT_FONTS_DIR`), optional system
//! fonts found by scanning platform directories (plus Fontconfig's file list on Linux),
//! user-registered font data and font files or folders ([`FontDb::register_font_path`]), and
//! PostScript-name lookup for PSD import.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use parley::FontContext;
use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, FontStyle, FontWeight, FontWidth, GenericFamily, SourceCache};
use skrifa::raw::{FileRef, TableProvider, types::Tag};
use skrifa::{MetadataProvider, string::StringId};

/// Family used when a style names no family or an unknown one.
pub const DEFAULT_FAMILY: &str = "Inter";
/// Bundled monospace family.
pub const MONO_FAMILY: &str = "JetBrains Mono";

/// An `assets/fonts` file as deflated by build.rs.
macro_rules! bundled_font {
    ($file:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/fonts/", $file, ".deflate"))
    };
}

// The font files are embedded once, here. The UI (`ui-egui` theme) uses these same statics:
// a second `include_bytes!` of the same file elsewhere put a second 1.5 MB copy into the wasm.
// They are stored deflated by build.rs (0.73 MB instead of 1.5 MB: the web build's 24 MiB size
// gate, packaging/web/package.sh) and inflated on first use, once per process.
pub static INTER_REGULAR: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(bundled_font!("Inter-Regular.ttf")));
pub static INTER_MEDIUM: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(bundled_font!("Inter-Medium.ttf")));
pub static INTER_SEMIBOLD: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(bundled_font!("Inter-SemiBold.ttf")));
pub static JETBRAINS_MONO_REGULAR: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(bundled_font!("JetBrainsMono-Regular.ttf")));

/// Fonts shipped with PhotoCraft (OFL; licences in `assets/fonts`).
pub static BUNDLED: [(&str, &LazyLock<Vec<u8>>); 4] = [
    ("Inter-Regular.ttf", &INTER_REGULAR),
    ("Inter-Medium.ttf", &INTER_MEDIUM),
    ("Inter-SemiBold.ttf", &INTER_SEMIBOLD),
    ("JetBrainsMono-Regular.ttf", &JETBRAINS_MONO_REGULAR),
];

/// Inflated fonts are bounded (the largest is 0.42 MB), so corrupt data can't exhaust memory.
const MAX_BUNDLED_FONT_BYTES: u64 = 16 << 20;

/// Inflate a bundled font. Data that doesn't inflate gives no font (empty bytes, which every
/// consumer skips), never a panic; `bundled_fonts_inflate_to_their_files` keeps it from happening.
fn inflate_font(deflated: &[u8]) -> Vec<u8> {
    use std::io::Read as _;
    let mut out = Vec::new();
    match flate2::read::DeflateDecoder::new(deflated).take(MAX_BUNDLED_FONT_BYTES + 1).read_to_end(&mut out) {
        Ok(_) if out.len() as u64 <= MAX_BUNDLED_FONT_BYTES => out,
        _ => Vec::new(),
    }
}

/// Families tried (if installed) after the requested one, for missing glyphs. The Southeast Asian
/// and CJK families ([`THAI_FAMILIES`], [`KHMER_FAMILIES`], [`LAO_FAMILIES`], [`MYANMAR_FAMILIES`],
/// [`INDIC_FAMILIES`], [`crate::cjk::families`]) go between these two lists, in the UI locale's script order.
const FALLBACK_CANDIDATES: &[&str] = &["Noto Sans", "Segoe UI", "DejaVu Sans", "Geeza Pro", "Arial Hebrew", "Noto Sans Arabic", "Noto Sans Hebrew"];
/// Thai-capable families: Windows (Leelawadee UI since Windows 8; Tahoma and Leelawadee on
/// older systems), macOS (Thonburi, Sukhumvit Set) and Linux (Noto). Text in a font without Thai
/// glyphs falls back to these, as with Photoshop's missing glyph protection (#1909).
pub const THAI_FAMILIES: &[&str] =
    &["Leelawadee UI", "Leelawadee", "Tahoma", "Thonburi", "Sukhumvit Set", "Noto Sans Thai", "Noto Sans Thai Looped", "Noto Sans Thai UI"];
/// Khmer-capable families: macOS (Khmer Sangam MN, Khmer MN), Windows (`Khmer UI` on 8; from 8.1
/// `Leelawadee UI` in [`THAI_FAMILIES`] covers Khmer too) and Linux (Noto). Khmer in a font without
/// Khmer glyphs had no candidate to fall back to and drew .notdef boxes (#218).
pub const KHMER_FAMILIES: &[&str] = &["Khmer Sangam MN", "Khmer MN", "Khmer UI", "Noto Sans Khmer"];
/// Lao-capable families: macOS (Lao Sangam MN, Lao MN), Windows (`Lao UI` on 8; `Leelawadee UI` in
/// [`THAI_FAMILIES`] from 8.1) and Linux (Noto, static and looped forms).
pub const LAO_FAMILIES: &[&str] = &["Lao Sangam MN", "Lao MN", "Lao UI", "Noto Sans Lao", "Noto Sans Lao Looped"];
/// Myanmar/Burmese-capable families: macOS (Myanmar Sangam MN, Myanmar MN), Windows (Myanmar Text)
/// and Linux (Noto).
pub const MYANMAR_FAMILIES: &[&str] = &["Myanmar Sangam MN", "Myanmar MN", "Myanmar Text", "Noto Sans Myanmar"];
/// Indic-script families for document text. Windows has `Nirmala UI` for all nine scripts (from
/// Windows 8); macOS has the Kohinoor and Sangam MN families; Linux has Noto Sans. Indic text in a
/// font without these glyphs had no candidate to fall back to and drew .notdef boxes (#218).
pub const DEVANAGARI_FAMILIES: &[&str] = &["Nirmala UI", "Kohinoor Devanagari", "Devanagari Sangam MN", "Noto Sans Devanagari", "Noto Sans Devanagari UI"];
/// Bengali-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const BENGALI_FAMILIES: &[&str] = &["Nirmala UI", "Kohinoor Bangla", "Bangla Sangam MN", "Noto Sans Bengali", "Noto Sans Bengali UI"];
/// Gurmukhi-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const GURMUKHI_FAMILIES: &[&str] = &["Nirmala UI", "Gurmukhi Sangam MN", "Noto Sans Gurmukhi", "Noto Sans Gurmukhi UI"];
/// Gujarati-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const GUJARATI_FAMILIES: &[&str] = &["Nirmala UI", "Kohinoor Gujarati", "Gujarati Sangam MN", "Noto Sans Gujarati", "Noto Sans Gujarati UI"];
/// Tamil-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const TAMIL_FAMILIES: &[&str] = &["Nirmala UI", "Tamil Sangam MN", "Noto Sans Tamil", "Noto Sans Tamil UI"];
/// Telugu-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const TELUGU_FAMILIES: &[&str] = &["Nirmala UI", "Kohinoor Telugu", "Telugu Sangam MN", "Noto Sans Telugu", "Noto Sans Telugu UI"];
/// Kannada-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const KANNADA_FAMILIES: &[&str] = &["Nirmala UI", "Kannada Sangam MN", "Noto Sans Kannada", "Noto Sans Kannada UI"];
/// Malayalam-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const MALAYALAM_FAMILIES: &[&str] = &["Nirmala UI", "Malayalam Sangam MN", "Noto Sans Malayalam", "Noto Sans Malayalam UI"];
/// Sinhala-capable families (see [`DEVANAGARI_FAMILIES`]).
pub const SINHALA_FAMILIES: &[&str] = &["Nirmala UI", "Sinhala Sangam MN", "Noto Sans Sinhala", "Noto Sans Sinhala UI"];
/// All Indic-script families, in the order they join the fallback candidates.
pub const INDIC_FAMILIES: &[&[&str]] = &[
    DEVANAGARI_FAMILIES,
    BENGALI_FAMILIES,
    GURMUKHI_FAMILIES,
    GUJARATI_FAMILIES,
    TAMIL_FAMILIES,
    TELUGU_FAMILIES,
    KANNADA_FAMILIES,
    MALAYALAM_FAMILIES,
    SINHALA_FAMILIES,
];
/// Broad-coverage and emoji fonts, tried after the CJK script fonts so shared Han characters get
/// the locale's forms instead of Arial Unicode's.
const FALLBACK_LAST: &[&str] = &["Arial Unicode MS", "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji"];

/// Every fallback family candidate, CJK ordered for `order` (see [`crate::cjk::script_order`]).
pub fn fallback_candidates(order: &[crate::cjk::CjkScript; 4]) -> Vec<&'static str> {
    let mut v = FALLBACK_CANDIDATES.to_vec();
    v.extend_from_slice(THAI_FAMILIES);
    v.extend_from_slice(KHMER_FAMILIES);
    v.extend_from_slice(LAO_FAMILIES);
    v.extend_from_slice(MYANMAR_FAMILIES);
    for fams in INDIC_FAMILIES {
        for f in *fams {
            // Nirmala UI covers all nine scripts and is listed by each; probe it once.
            if !v.contains(f) {
                v.push(f);
            }
        }
    }
    for s in order {
        // craft-fonts' Japanese fonts (if built in) go ahead of the installed Japanese fonts, in
        // the Japanese slot of the locale order, so shared Han keeps the locale's forms.
        if *s == crate::cjk::CjkScript::Japanese {
            v.extend(crate::craft_fonts::japanese_families());
        }
        v.extend_from_slice(crate::cjk::families(*s));
    }
    v.extend_from_slice(FALLBACK_LAST);
    v
}

/// Bumped whenever font data is registered: fonts can arrive after start (the web build fetches
/// served fonts on demand, [`crate::served`]), so lists derived from the database, such as font
/// menus, compare it to know when to refresh.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever font data is registered in any [`FontDb`].
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Bumped only by the [`FontDb`] of [`crate::shared`], so caches of the shared engine's fonts
/// aren't invalidated by unrelated databases (a document's own engine, a test's throwaway one).
static SHARED_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever font data is registered in the shared engine's [`FontDb`] ([`crate::shared`]).
/// Font menus key their caches on this, not [`generation`].
pub fn shared_generation() -> u64 {
    SHARED_GENERATION.load(Ordering::Relaxed)
}

/// One face in the database (for font menus).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    pub family: String,
    /// OpenType typographic subfamily (name 17), or legacy subfamily (name 2).
    pub style: String,
    /// Stable face identity, including faces that share the same CSS attributes.
    pub postscript_name: Option<String>,
    /// CSS weight (100–900); variable fonts report their default.
    pub weight: f32,
    pub italic: bool,
    /// Variation axes: (tag, min, default, max).
    pub axes: Vec<(String, f32, f32, f32)>,
}

/// Result of a PostScript-name lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFont {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// True when the exact face was found; false for a heuristic family guess.
    pub exact: bool,
}

/// How deep [`FontDb::register_font_path`] (and the system font scan) looks into subfolders.
pub const FONT_DIR_DEPTH: u32 = 8;

/// Bounds on one registration of font files, so a path can't exhaust memory: the font data is
/// held in memory once registered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontFileLimits {
    /// A larger file is skipped.
    pub max_file_bytes: u64,
    /// More font files is an error.
    pub max_files: usize,
    /// Files adding up to more is an error.
    pub max_total_bytes: u64,
}

impl FontFileLimits {
    /// The largest CJK collections are about 100 MB.
    pub const DEFAULT: Self = Self { max_file_bytes: 256 << 20, max_files: 4096, max_total_bytes: 1 << 30 };
}

/// What registering font files added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegisteredFonts {
    /// The families the registered files provide (private `.` families left out), sorted. A file
    /// registered before still reports its families.
    pub families: Vec<String>,
    /// Files that registered.
    pub files: usize,
    /// Files that didn't, with the reason (names as given, or relative to the folder).
    pub skipped: Vec<(String, String)>,
}

/// Why registering font files failed.
#[derive(Clone, Debug, PartialEq)]
pub enum FontFileError {
    /// Nothing exists at the path.
    NotFound(String),
    /// The path can't be read, or is neither a file nor a folder.
    Io { path: String, error: String },
    /// A folder with no `.ttf`, `.otf`, `.ttc` or `.otc` file in it or its subfolders.
    NoFontFiles(String),
    /// More font files than [`FontFileLimits::max_files`].
    TooManyFiles { path: String, max: usize },
    /// Font files adding up to more than [`FontFileLimits::max_total_bytes`].
    TooLarge { path: String, max_bytes: u64 },
    /// No file registered a font: each file with the reason.
    NoFonts { path: String, failures: Vec<(String, String)> },
}

impl std::fmt::Display for FontFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "no font file or folder at `{path}`"),
            Self::Io { path, error } => write!(f, "cannot read `{path}`: {error}"),
            Self::NoFontFiles(path) => write!(f, "no font files (.ttf, .otf, .ttc, .otc) in `{path}`"),
            Self::TooManyFiles { path, max } => write!(f, "`{path}` holds more than {max} font files; register a smaller folder"),
            Self::TooLarge { path, max_bytes } => {
                write!(f, "the font files in `{path}` add up to more than {}; register a smaller folder", size_text(*max_bytes))
            }
            Self::NoFonts { path, failures } => {
                write!(f, "no fonts could be read from `{path}`")?;
                for (i, (name, why)) in failures.iter().take(3).enumerate() {
                    write!(f, "{} {name}: {why}", if i == 0 { ":" } else { ";" })?;
                }
                if failures.len() > 3 {
                    write!(f, "; and {} more", failures.len() - 3)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FontFileError {}

const NOT_A_FONT: &str = "not a font file: expected TrueType or OpenType data (.ttf, .otf, .ttc, .otc)";
const WOFF: &str = "WOFF and WOFF2 web fonts are not supported: use the .ttf or .otf file";
const NO_FACES: &str = "no font faces could be read from it";

fn too_large(max_bytes: u64) -> String {
    format!("larger than {}", size_text(max_bytes))
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 && b.is_multiple_of(1 << 30) => format!("{} GiB", b >> 30),
        b if b >= 1 << 20 && b.is_multiple_of(1 << 20) => format!("{} MiB", b >> 20),
        b => format!("{b} bytes"),
    }
}

/// A font file extension (`ttf`, `otf`, `ttc` or `otc`, any case): what folder scans pick up.
pub fn is_font_extension(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc")
}

pub struct FontDb {
    pub(crate) fcx: FontContext,
    system_loaded: bool,
    /// PostScript name → (family, weight, italic), filled lazily.
    ps_cache: HashMap<String, Option<ResolvedFont>>,
    fallbacks: Vec<String>,
    /// Preferences ▸ Type ▸ Missing Glyph Protection: route glyphs the chosen face lacks through
    /// [`Self::fallback_stack`] (#1909). Off, only the default family shapes the text.
    missing_glyph_protection: bool,
    face_cache: HashMap<String, Vec<FaceInfo>>,
    face_aliases: HashMap<(String, usize), String>,
    /// Hash of each file registered by [`Self::register_font_files`] → its families, so the same
    /// file registered again adds no duplicate faces.
    registered_files: HashMap<u64, Vec<String>>,
    /// This is the shared engine's database: registrations also move [`shared_generation`].
    shared: bool,
}

impl Default for FontDb {
    fn default() -> Self {
        Self::new()
    }
}

impl FontDb {
    /// Bundled fonts only (deterministic: used by tests and the web build).
    pub fn new() -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        let mut db = FontDb {
            fcx: FontContext { collection, source_cache: SourceCache::default() },
            system_loaded: false,
            ps_cache: HashMap::new(),
            fallbacks: Vec::new(),
            missing_glyph_protection: true,
            face_cache: HashMap::new(),
            face_aliases: HashMap::new(),
            registered_files: HashMap::new(),
            shared: false,
        };
        for (_, bytes) in BUNDLED {
            db.register_static_font(bytes.as_slice());
        }
        // The optional craft-fonts (empty unless built with CRAFT_FONTS_DIR; always empty on
        // wasm32), before any system font.
        for f in crate::craft_fonts::CRAFT_FONTS.iter().filter(|f| f.is_japanese()) {
            db.register_static_font(f.bytes);
        }
        db.refresh_generics();
        db
    }

    /// Bundled fonts plus the fonts installed on this machine (no-op on the web).
    pub fn with_system_fonts() -> Self {
        let mut db = Self::new();
        db.load_system_fonts();
        db
    }

    /// Scans the platform font directories once, plus Fontconfig's file list on Linux. Font files
    /// are memory-mapped lazily by fontique when a face is first used.
    pub fn load_system_fonts(&mut self) {
        if self.system_loaded {
            return;
        }
        self.system_loaded = true;
        #[cfg(not(target_arch = "wasm32"))]
        {
            // One call per file: fontique's directory scan is much slower on large folders.
            let mut files = Vec::new();
            for d in system_font_dirs() {
                collect_font_files(&d, 0, usize::MAX, &mut files);
            }
            #[cfg(target_os = "linux")]
            files.extend(fontconfig_font_paths());
            // PingFang (the macOS Chinese UI font) lives outside the font folders.
            if cfg!(target_os = "macos") {
                files.extend(crate::cjk::mac_pingfang());
            }
            let mut seen = HashSet::new();
            for path in files {
                let key = path.canonicalize().unwrap_or_else(|_| path.clone());
                if seen.insert(key) {
                    self.fcx.collection.load_fonts_from_paths([path]);
                }
            }
            self.ps_cache.clear();
            self.face_cache.clear();
            self.refresh_generics();
        }
    }

    /// Registers font data (TTF/OTF, or every face of a TTC/OTC). Returns the family names added.
    pub fn register_font_data(&mut self, bytes: Vec<u8>) -> Vec<String> {
        self.register_blob(Blob::new(Arc::new(bytes)))
    }

    /// Registers a font file (TTF/OTF/TTC/OTC, judged by its data, not its name) or every font
    /// file in a folder and its subfolders ([`FONT_DIR_DEPTH`] deep, by extension). Files that
    /// fail are reported in [`RegisteredFonts::skipped`]; it is an error only when nothing could
    /// be registered or `limits` are exceeded.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn register_font_path(&mut self, path: &std::path::Path, limits: FontFileLimits) -> Result<RegisteredFonts, FontFileError> {
        let shown = path.display().to_string();
        let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => FontFileError::NotFound(shown.clone()),
            _ => FontFileError::Io { path: shown.clone(), error: e.to_string() },
        })?;
        if meta.is_file() {
            let name = path.file_name().map_or_else(|| shown.clone(), |n| n.to_string_lossy().into_owned());
            let bytes = read_font_file(path, limits.max_file_bytes);
            return self.register_font_files(&shown, vec![(name, bytes)], limits);
        }
        if !meta.is_dir() {
            return Err(FontFileError::Io { path: shown, error: "not a file or folder".into() });
        }
        let mut paths = Vec::new();
        collect_font_files(path, 0, limits.max_files.saturating_add(1), &mut paths);
        if paths.len() > limits.max_files {
            return Err(FontFileError::TooManyFiles { path: shown, max: limits.max_files });
        }
        // Deterministic registration order: a directory listing's isn't.
        paths.sort();
        let mut total = 0u64;
        let mut files = Vec::with_capacity(paths.len());
        for p in &paths {
            // Stop before reading past the total; an oversized file is skipped unread.
            let size = std::fs::metadata(p).map_or(0, |m| m.len());
            if size <= limits.max_file_bytes {
                total = total.saturating_add(size);
                if total > limits.max_total_bytes {
                    return Err(FontFileError::TooLarge { path: shown, max_bytes: limits.max_total_bytes });
                }
            }
            files.push((relative_name(path, p), read_font_file(p, limits.max_file_bytes)));
        }
        self.register_font_files(&shown, files, limits)
    }

    /// Registers font files already read: `(name, bytes or why they couldn't be read)`, `source`
    /// naming them in errors. A file registered before (same bytes) adds no faces again but
    /// still reports its families. Errors as for [`Self::register_font_path`].
    pub fn register_font_files(
        &mut self,
        source: &str,
        files: Vec<(String, Result<Vec<u8>, String>)>,
        limits: FontFileLimits,
    ) -> Result<RegisteredFonts, FontFileError> {
        if files.is_empty() {
            return Err(FontFileError::NoFontFiles(source.into()));
        }
        if files.len() > limits.max_files {
            return Err(FontFileError::TooManyFiles { path: source.into(), max: limits.max_files });
        }
        let total = files.iter().filter_map(|(_, b)| b.as_ref().ok()).fold(0u64, |t, b| t.saturating_add(b.len() as u64));
        if total > limits.max_total_bytes {
            return Err(FontFileError::TooLarge { path: source.into(), max_bytes: limits.max_total_bytes });
        }
        let mut out = RegisteredFonts::default();
        for (name, bytes) in files {
            match bytes.and_then(|b| self.register_font_file(b, limits.max_file_bytes)) {
                Ok(families) => {
                    out.files += 1;
                    for f in families {
                        if !is_hidden_family(&f) && !out.families.contains(&f) {
                            out.families.push(f);
                        }
                    }
                }
                Err(why) => out.skipped.push((name, why)),
            }
        }
        if out.files == 0 {
            return Err(FontFileError::NoFonts { path: source.into(), failures: out.skipped });
        }
        out.families.sort_by_key(|f| f.to_lowercase());
        Ok(out)
    }

    /// One file of [`Self::register_font_files`]: its families, or why it isn't a usable font.
    fn register_font_file(&mut self, bytes: Vec<u8>, max_bytes: u64) -> Result<Vec<String>, String> {
        if bytes.len() as u64 > max_bytes {
            return Err(too_large(max_bytes));
        }
        if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
            return Err(WOFF.into());
        }
        if face_count(&bytes) == 0 {
            return Err(NOT_A_FONT.into());
        }
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let key = hasher.finish();
        if let Some(families) = self.registered_files.get(&key) {
            return Ok(families.clone());
        }
        let families = self.register_font_data(bytes);
        if families.is_empty() {
            return Err(NO_FACES.into());
        }
        self.registered_files.insert(key, families.clone());
        Ok(families)
    }

    /// Registers embedded font data without copying it.
    fn register_static_font(&mut self, bytes: &'static [u8]) -> Vec<String> {
        self.register_blob(Blob::new(Arc::new(bytes)))
    }

    fn register_blob(&mut self, blob: Blob<u8>) -> Vec<String> {
        let added = self.fcx.collection.register_fonts(blob, None);
        let mut names = Vec::new();
        for (id, _) in added {
            if let Some(n) = self.fcx.collection.family_name(id)
                && !names.iter().any(|x: &String| x == n)
            {
                names.push(n.to_string());
            }
        }
        // Deterministic order, user-facing families first: a collection's registration order isn't
        // stable, and macOS ships private UI faces (".Hiragino Kaku Gothic Interface") whose
        // metrics differ from the public family's.
        names.sort_by_key(|n| (is_hidden_family(n), n.to_lowercase()));
        self.ps_cache.clear();
        self.face_cache.clear();
        self.refresh_generics();
        GENERATION.fetch_add(1, Ordering::Relaxed);
        if self.shared {
            SHARED_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        names
    }

    /// Marks this as the shared engine's database ([`crate::shared`]) and moves
    /// [`shared_generation`], since its fonts replace whatever caches saw before.
    pub(crate) fn mark_shared(&mut self) {
        self.shared = true;
        SHARED_GENERATION.fetch_add(1, Ordering::Relaxed);
    }

    fn refresh_generics(&mut self) {
        let c = &mut self.fcx.collection;
        if let Some(inter) = c.family_id(DEFAULT_FAMILY) {
            for g in [GenericFamily::SansSerif, GenericFamily::Serif, GenericFamily::SystemUi, GenericFamily::UiSansSerif] {
                if c.generic_families(g).next().is_none() {
                    c.set_generic_families(g, std::iter::once(inter));
                }
            }
        }
        if let Some(mono) = c.family_id(MONO_FAMILY) {
            for g in [GenericFamily::Monospace, GenericFamily::UiMonospace] {
                if c.generic_families(g).next().is_none() {
                    c.set_generic_families(g, std::iter::once(mono));
                }
            }
        }
        let order = crate::cjk::ui_script_order();
        self.fallbacks = fallback_candidates(&order).into_iter().filter(|f| c.family_id(f).is_some()).map(str::to_string).collect();
        // Served fonts that are a script's fallback (the manifest's `scripts`), once they arrived.
        for f in crate::served::script_fallback_families() {
            if c.family_id(&f).is_some() && !self.fallbacks.contains(&f) {
                self.fallbacks.push(f);
            }
        }
    }

    /// Families available after the requested one (bundled default + installed coverage fonts).
    /// With missing glyph protection off (Preferences ▸ Type), only the default family shapes.
    pub(crate) fn fallback_stack(&self) -> impl Iterator<Item = &str> {
        let fallbacks = self.missing_glyph_protection.then_some(self.fallbacks.iter().map(String::as_str));
        std::iter::once(DEFAULT_FAMILY).chain(fallbacks.into_iter().flatten())
    }

    /// Preferences ▸ Type ▸ Missing Glyph Protection, applied by the engine's `apply_prefs`.
    pub fn set_missing_glyph_protection(&mut self, on: bool) {
        self.missing_glyph_protection = on;
    }

    /// Is missing glyph protection on?
    pub fn missing_glyph_protection(&self) -> bool {
        self.missing_glyph_protection
    }

    /// All family names, sorted.
    pub fn families(&mut self) -> Vec<String> {
        // Private system faces (a leading '.', e.g. macOS ".SF NS") are hidden, as in Photoshop.
        let mut v: Vec<String> = self.fcx.collection.family_names().filter(|n| !is_hidden_family(n)).map(str::to_string).collect();
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup();
        v
    }

    /// Each listed family's localized (non-English) name, for font menus with Preferences ▸ Type ▸
    /// Show Font Names in English off (Photoshop then lists e.g. "MS Gothic" as "ＭＳ ゴシック").
    /// Families without one are absent. Read from the same name ID the family name comes from
    /// (typographic family, else family), so the pair always names the same family. Documents keep
    /// the English family: this is a display label only.
    pub fn localized_family_names(&mut self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for family in self.families() {
            let Some(info) = self.fcx.collection.family_by_name(&family) else {
                continue;
            };
            let Some(face) = info.fonts().first() else {
                continue;
            };
            let Some(blob) = face.load(Some(&mut self.fcx.source_cache)) else {
                continue;
            };
            let Ok(font) = skrifa::FontRef::from_index(blob.as_ref(), face.index()) else {
                continue;
            };
            let names =
                |id| -> Vec<(String, Option<String>)> { font.localized_strings(id).map(|s| (s.chars().collect(), s.language().map(str::to_string))).collect() };
            let typographic = names(StringId::TYPOGRAPHIC_FAMILY_NAME);
            let names = if typographic.is_empty() { names(StringId::FAMILY_NAME) } else { typographic };
            if let Some(local) = localized_name(&family, &names) {
                out.insert(family, local);
            }
        }
        out
    }

    pub fn has_family(&mut self, name: &str) -> bool {
        self.fcx.collection.family_id(name).is_some()
    }

    /// Whether the face selected for a character style exposes an OpenType GSUB feature.
    ///
    /// This deliberately checks the matched face, rather than every face in the family: a
    /// regular face can lack a feature that its italic or bold sibling has (or vice versa).
    pub(crate) fn selected_face_has_feature(&mut self, family: &str, weight: u16, italic: bool, feature: Tag) -> bool {
        let family = if family.is_empty() || !self.has_family(family) { DEFAULT_FAMILY } else { family };
        let Some(info) = self.fcx.collection.family_by_name(family) else {
            return false;
        };
        let style = if italic { FontStyle::Italic } else { FontStyle::Normal };
        let Some(font) = info.match_font(FontWidth::default(), style, FontWeight::new(weight.clamp(1, 1000) as f32), true) else {
            return false;
        };
        let Some(blob) = font.load(Some(&mut self.fcx.source_cache)) else {
            return false;
        };
        let Ok(font) = skrifa::FontRef::from_index(blob.as_ref(), font.index()) else {
            return false;
        };
        font.gsub()
            .ok()
            .and_then(|gsub| gsub.feature_list().ok())
            .is_some_and(|features| features.feature_records().iter().any(|record| record.feature_tag() == feature))
    }

    /// Faces of a family, preserving names even when multiple faces share CSS attributes.
    pub fn faces(&mut self, family: &str) -> Vec<FaceInfo> {
        if let Some(faces) = self.face_cache.get(family) {
            return faces.clone();
        }
        let Some(info) = self.fcx.collection.family_by_name(family) else {
            return Vec::new();
        };
        let faces: Vec<_> = info
            .fonts()
            .iter()
            .map(|f| {
                let blob = f.load(Some(&mut self.fcx.source_cache));
                let font = blob.as_ref().and_then(|b| skrifa::FontRef::from_index(b.as_ref(), f.index()).ok());
                let name = |id| font.as_ref().and_then(|f| f.localized_strings(id).english_or_first()).map(|s| s.to_string()).filter(|s| !s.is_empty());
                let italic = !matches!(f.style(), FontStyle::Normal);
                FaceInfo {
                    family: info.name().to_string(),
                    style: name(StringId::TYPOGRAPHIC_SUBFAMILY_NAME)
                        .or_else(|| name(StringId::SUBFAMILY_NAME))
                        .unwrap_or_else(|| fallback_style(f.weight().value(), italic)),
                    postscript_name: name(StringId::POSTSCRIPT_NAME),
                    weight: f.weight().value(),
                    italic,
                    axes: f.axes().iter().map(|a| (a.tag.to_string(), a.min, a.default, a.max)).collect(),
                }
            })
            .collect();
        self.face_cache.insert(family.to_string(), faces.clone());
        faces
    }

    /// Resolve a menu style to its actual metadata instead of guessing from its spelling.
    pub fn named_face(&mut self, family: &str, style: &str) -> Option<FaceInfo> {
        self.faces(family).into_iter().find(|f| f.style.eq_ignore_ascii_case(style))
    }

    /// Select an exact face for shaping. Private aliases are runtime-only: the document keeps
    /// its public family and PostScript name. CSS matching alone cannot distinguish e.g. the
    /// numeric subfamilies of Yoon fonts, all of which declare weight 400 / normal.
    pub(crate) fn select_named_face(&mut self, style: &mut photocraft_doc::text::CharStyle) {
        let family = style.font_family.clone();
        let faces = self.faces(&family);
        let index = style.postscript_name.as_ref().and_then(|ps| faces.iter().position(|f| f.postscript_name.as_ref() == Some(ps))).or_else(|| {
            faces.iter().position(|f| {
                !style.font_style.is_empty()
                    && f.style.eq_ignore_ascii_case(&style.font_style)
                    && f.weight.round() as u16 == style.weight
                    && f.italic == style.italic
            })
        });
        let Some(index) = index else { return };
        let Some(face) = faces.get(index) else { return };
        let key = (family.clone(), index);
        let alias = if let Some(alias) = self.face_aliases.get(&key) {
            alias.clone()
        } else {
            let Some(info) = self.fcx.collection.family_by_name(&family) else { return };
            let Some(font) = info.fonts().get(index) else { return };
            let Some(blob) = font.load(Some(&mut self.fcx.source_cache)) else { return };
            let Some(blob) = isolated_face(blob, font.index()) else { return };
            let alias = format!(".PhotoCraft-face-{}-{index}", info.id().to_u64());
            self.fcx.collection.register_fonts(blob, Some(FontInfoOverride { family_name: Some(&alias), ..Default::default() }));
            self.face_aliases.insert(key, alias.clone());
            alias
        };
        style.font_family = alias;
        style.weight = face.weight.round() as u16;
        style.italic = face.italic;
    }

    /// Finds a face by PostScript name (as stored in PSD files): exact match by reading the
    /// `name` table of candidate faces, else a heuristic split (`Arial-BoldMT` → Arial, 700).
    pub fn resolve_postscript(&mut self, ps: &str) -> ResolvedFont {
        if let Some(Some(r)) = self.ps_cache.get(ps) {
            return r.clone();
        }
        let guess = guess_from_postscript(ps);
        let exact = self.find_exact(ps, &guess.family);
        let r = exact.unwrap_or_else(|| {
            // Keep the guessed family only if we have it; otherwise let fallback pick.
            let mut g = guess.clone();
            if !self.has_family(&g.family)
                && let Some(f) = self.families().into_iter().find(|f| f.replace(' ', "").eq_ignore_ascii_case(&g.family.replace(' ', "")))
            {
                g.family = f;
            }
            g
        });
        self.ps_cache.insert(ps.to_string(), Some(r.clone()));
        r
    }

    fn find_exact(&mut self, ps: &str, family_guess: &str) -> Option<ResolvedFont> {
        let first_word = family_guess.split(' ').next().unwrap_or(family_guess).to_lowercase();
        let mut candidates = self.families();
        candidates.sort_by_key(|f| !f.to_lowercase().replace(' ', "").starts_with(&first_word));
        for fam in candidates {
            for face in self.faces(&fam) {
                let Some(name) = face.postscript_name else { continue };
                let resolved = ResolvedFont { family: face.family, weight: face.weight.round() as u16, italic: face.italic, exact: true };
                self.ps_cache.insert(name.clone(), Some(resolved.clone()));
                if name == ps {
                    return Some(resolved);
                }
            }
        }
        None
    }
}

/// Make a collection expose only the requested face without rewriting any font tables.
/// TTC table offsets are absolute; changing its face directory preserves them verbatim.
fn isolated_face(blob: Blob<u8>, index: u32) -> Option<Blob<u8>> {
    match FileRef::new(blob.as_ref()).ok()? {
        FileRef::Font(_) => (index == 0).then_some(blob),
        FileRef::Collection(collection) => {
            collection.get(index).ok()?;
            // Bound a copy of untrusted font data. Ordinary single-face fonts need no copy.
            if blob.as_ref().len() > 256 * 1024 * 1024 {
                return None;
            }
            let offset = 12usize.checked_add(usize::try_from(index).ok()?.checked_mul(4)?)?;
            let face_offset: [u8; 4] = blob.as_ref().get(offset..offset.checked_add(4)?)?.try_into().ok()?;
            let mut bytes = blob.as_ref().to_vec();
            bytes.get_mut(8..12)?.copy_from_slice(&1u32.to_be_bytes());
            bytes.get_mut(12..16)?.copy_from_slice(&face_offset);
            Some(Blob::new(Arc::new(bytes)))
        }
    }
}

fn fallback_style(weight: f32, italic: bool) -> String {
    let name = match weight.round() as i32 {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    };
    match (name, italic) {
        ("Regular", true) => "Italic".into(),
        (_, true) => format!("{name} Italic"),
        (_, false) => name.into(),
    }
}

/// Number of faces in a font file (1 for TTF/OTF, n for TTC/OTC, 0 if unreadable).
pub fn face_count(bytes: &[u8]) -> usize {
    match FileRef::new(bytes) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len() as usize,
        Err(_) => 0,
    }
}

// CamelCase style suffix of a name without a hyphen (`GmarketSansBold`). Only a capitalised word
// counts ("Daylight" keeps its "light"), and one after a vendor tag stays in a legacy family name
// (`ArialRoundedMTBold` is the family "Arial Rounded MT Bold").
fn split_trailing_style(ps: &str) -> (&str, &str) {
    let base = ps.trim_end_matches("MT").trim_end_matches("PS");
    let mut family = base;
    for suffix in ["Italic", "Oblique"] {
        if let Some(rest) = family.strip_suffix(suffix).filter(|s| !s.is_empty()) {
            family = rest;
            break;
        }
    }
    for suffix in [
        "ExtraLight",
        "UltraLight",
        "ExtraBold",
        "UltraBold",
        "SemiBold",
        "DemiBold",
        "Hairline",
        "Regular",
        "Medium",
        "Normal",
        "Heavy",
        "Black",
        "Light",
        "Thin",
        "Bold",
        "Demi",
    ] {
        if let Some(rest) = family.strip_suffix(suffix).filter(|s| !s.is_empty()) {
            family = rest;
            break;
        }
    }
    if family.len() < base.len() && !family.ends_with("MT") { (family, ps.get(family.len()..).unwrap_or("")) } else { (ps, "") }
}

/// Heuristic PostScript-name split: `MyriadPro-BoldIt` → ("Myriad Pro", 700, italic).
pub fn guess_from_postscript(ps: &str) -> ResolvedFont {
    // A vendor or family name may contain hyphens; start with the final face suffix.
    let (mut pro, mut style) = ps.rsplit_once('-').unwrap_or_else(|| split_trailing_style(ps));
    // Keep compound style suffixes the old heuristic understood, such as Bold-It.
    // Regular alone must not consume a family component named Bold or Black.
    if matches!(style.to_ascii_lowercase().as_str(), "it" | "italic" | "oblique")
        && let Some((family, weight)) = pro.rsplit_once('-')
        && matches!(
            weight.to_ascii_lowercase().as_str(),
            "thin"
                | "hairline"
                | "extralight"
                | "ultralight"
                | "light"
                | "regular"
                | "roman"
                | "book"
                | "medium"
                | "semibold"
                | "demibold"
                | "demi"
                | "bold"
                | "extrabold"
                | "ultrabold"
                | "heavy"
                | "black"
        )
    {
        pro = family;
        style = ps.strip_prefix(family).and_then(|s| s.strip_prefix('-')).unwrap_or(style);
    }
    let fam = pro.trim_end_matches("MT").trim_end_matches("PS").trim_end_matches("Std").trim_end_matches("Pro");
    let suffix = if pro.ends_with("Pro") {
        " Pro"
    } else if pro.ends_with("Std") {
        " Std"
    } else {
        ""
    };
    // Split camel case: "TimesNewRoman" → "Times New Roman".
    let mut family = String::new();
    let chars: Vec<char> = fam.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()) && chars[i - 1].is_uppercase()) {
            family.push(' ');
        }
        family.push(c);
    }
    family.push_str(suffix);
    let s = style.to_lowercase();
    let weight = if s.contains("thin") || s.contains("hairline") {
        100
    } else if s.contains("extralight") || s.contains("ultralight") {
        200
    } else if s.contains("light") {
        300
    } else if s.contains("medium") {
        500
    } else if s.contains("semibold") || s.contains("demibold") || s.contains("demi") {
        600
    } else if s.contains("extrabold") || s.contains("ultrabold") || s.contains("heavy") {
        800
    } else if s.contains("black") {
        900
    } else if s.contains("bold") {
        700
    } else {
        400
    };
    let italic = s.contains("italic") || s.ends_with("it") || s.contains("oblique");
    ResolvedFont { family: family.trim().to_string(), weight, italic, exact: false }
}

/// Font files below `dir` ([`FONT_DIR_DEPTH`] deep), stopping once `out` holds `limit` paths.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn collect_font_files(dir: &std::path::Path, depth: u32, limit: usize, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if out.len() >= limit {
            return;
        }
        let p = e.path();
        if p.is_dir() {
            if depth < FONT_DIR_DEPTH {
                collect_font_files(&p, depth + 1, limit, out);
            }
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(is_font_extension) {
            out.push(p);
        }
    }
}

/// A file's bytes, unless it is larger than `max_bytes` (then not read).
#[cfg(not(target_arch = "wasm32"))]
fn read_font_file(path: &std::path::Path, max_bytes: u64) -> Result<Vec<u8>, String> {
    read_font_data(std::fs::File::open(path).map_err(|e| e.to_string())?, max_bytes)
}

/// An open font file's bytes, unless it is larger than `max_bytes` (then not read), or why not:
/// the reason [`FontDb::register_font_files`] reports for it. For files opened another way, such
/// as through a directory capability.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_font_data(file: std::fs::File, max_bytes: u64) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len > max_bytes {
        return Err(too_large(max_bytes));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    // Bounded again: the file can grow after the size check.
    file.take(max_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large(max_bytes));
    }
    Ok(bytes)
}

/// `file`'s path below `root`, with `/` separators (as reported in [`RegisteredFonts::skipped`]).
#[cfg(not(target_arch = "wasm32"))]
fn relative_name(root: &std::path::Path, file: &std::path::Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Fontconfig knows about configured font directories that are not in our portable path list.
/// Keep this optional and shell-free: directory scans remain the fallback when `fc-list` is not
/// installed or Fontconfig cannot enumerate fonts.
#[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
fn fontconfig_font_paths() -> Vec<std::path::PathBuf> {
    let Ok(output) = std::process::Command::new("fc-list").arg("--format=%{file}\\n").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    fontconfig_paths_from_output(&output.stdout).into_iter().filter(|path| path.is_file()).collect()
}

#[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
fn fontconfig_paths_from_output(output: &[u8]) -> Vec<std::path::PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut seen = HashSet::new();
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| std::path::PathBuf::from(OsString::from_vec(line.to_vec())))
        .filter(|path| {
            path.is_absolute()
                && path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        })
        .map(|path| path.canonicalize().unwrap_or(path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(all(test, target_os = "linux", not(target_arch = "wasm32")))]
mod fontconfig_tests {
    use super::fontconfig_paths_from_output;
    use std::path::PathBuf;

    #[test]
    fn parses_unique_absolute_font_paths_from_fontconfig() {
        let paths = fontconfig_paths_from_output(
            b"/usr/share/fonts/example.ttf\nrelative.otf\n/usr/share/fonts/README.txt\n/usr/local/fonts/collection.OTC\n/usr/share/fonts/example.ttf\n\n",
        );

        assert_eq!(paths, vec![PathBuf::from("/usr/share/fonts/example.ttf"), PathBuf::from("/usr/local/fonts/collection.OTC")]);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn system_font_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/System/Library/Fonts"));
        v.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = &home {
            v.push(h.join("Library/Fonts"));
        }
    } else if cfg!(target_os = "windows") {
        let windir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        v.push(windir.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        v.push(PathBuf::from("/usr/share/fonts"));
        v.push(PathBuf::from("/usr/local/share/fonts"));
        // Inside a Flatpak sandbox the host's system and per-user fonts are mounted here
        // (`/usr/share/fonts` is the runtime's own small set). Missing dirs are skipped.
        v.push(PathBuf::from("/run/host/fonts"));
        v.push(PathBuf::from("/run/host/user-fonts"));
        if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
            v.push(PathBuf::from(d).join("fonts"));
        }
        if let Some(h) = &home {
            v.push(h.join(".local/share/fonts"));
            v.push(h.join(".fonts"));
        }
    }
    v
}

/// A platform-private family (macOS names its UI faces with a leading '.'): never listed or
/// picked by default.
/// The localized family name among a font's `(name, language)` family-name records: the first
/// one in a language other than English that differs from `english` (the name the font menus
/// store). `None` when the font only names itself in English (most fonts).
pub fn localized_name(english: &str, names: &[(String, Option<String>)]) -> Option<String> {
    let is_english = |lang: &str| {
        let primary = lang.split(['-', '_']).next().unwrap_or(lang);
        primary.eq_ignore_ascii_case("en")
    };
    names
        .iter()
        .filter(|(name, lang)| lang.as_deref().is_some_and(|l| !l.is_empty() && !is_english(l)) && !name.trim().is_empty() && name != english)
        .map(|(name, _)| name.clone())
        .next()
}

pub fn is_hidden_family(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn hidden_families_sort_last_and_are_not_listed() {
        assert!(super::is_hidden_family(".Hiragino Kaku Gothic Interface"));
        assert!(!super::is_hidden_family("Hiragino Sans"));
        let mut v = [".Hiragino Kaku Gothic Interface".to_string(), "Hiragino Kaku Gothic ProN".to_string()];
        v.sort_by_key(|n| (super::is_hidden_family(n), n.to_lowercase()));
        assert_eq!(v[0], "Hiragino Kaku Gothic ProN");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn scans_flatpak_host_fonts() {
        let dirs = super::system_font_dirs();
        assert!(dirs.iter().any(|d| d.ends_with("run/host/fonts")));
        assert!(dirs.iter().any(|d| d.ends_with("run/host/user-fonts")));
    }

    /// The bundled UI fonts cover Czech (the `cs` UI language) on their own: every letter with a
    /// diacritic and the Czech quotation marks, so no system fallback font is needed.
    #[test]
    fn bundled_fonts_cover_czech() {
        use skrifa::MetadataProvider;
        let czech = "aábcčdďeéěfghiíjklmnňoópqrřsštťuúůvwxyýzž„“";
        let letters: String = czech.chars().chain(czech.chars().flat_map(char::to_uppercase)).collect();
        for (name, bytes) in super::BUNDLED {
            let font = skrifa::FontRef::new(bytes.as_slice()).expect("bundled font parses");
            let cmap = font.charmap();
            let missing: String = letters.chars().filter(|c| cmap.map(*c).is_none_or(|g| g.to_u32() == 0)).collect();
            assert!(missing.is_empty(), "{name} lacks Czech glyphs: {missing}");
        }
    }

    /// The bundled UI fonts cover French (the `fr` UI language) on their own: every accented
    /// letter, the ligatures, the guillemets and the no-break space used before `: ; ? !`.
    #[test]
    fn bundled_fonts_cover_french() {
        use skrifa::MetadataProvider;
        let french = "àâæçéèêëîïôœùûüÿ";
        let letters: String = french.chars().chain(french.chars().flat_map(char::to_uppercase)).chain("«»\u{a0}’".chars()).collect();
        for (name, bytes) in super::BUNDLED {
            let font = skrifa::FontRef::new(bytes.as_slice()).expect("bundled font parses");
            let cmap = font.charmap();
            let missing: String = letters.chars().filter(|c| cmap.map(*c).is_none_or(|g| g.to_u32() == 0)).collect();
            assert!(missing.is_empty(), "{name} lacks French glyphs: {missing:?}");
        }
    }

    /// The deflated fonts in the binary inflate to exactly the files in `assets/fonts`.
    #[test]
    fn bundled_fonts_inflate_to_their_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        for (name, bytes) in super::BUNDLED {
            assert_eq!(bytes.as_slice(), std::fs::read(dir.join(name)).unwrap(), "{name}");
        }
    }

    /// Corrupt or oversized font data gives no font, not a panic.
    #[test]
    fn corrupt_bundled_font_data_gives_no_font() {
        assert!(super::inflate_font(b"\xff\xff not deflate").is_empty());
        assert!(super::inflate_font(b"").is_empty());
        let mut db = super::FontDb::new();
        assert!(db.register_font_data(super::inflate_font(b"\x00garbage")).is_empty());
    }

    fn names(v: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
        v.iter().map(|(n, l)| (n.to_string(), l.map(str::to_string))).collect()
    }

    /// Show Font Names in English off: a CJK font's native family name is its menu label.
    #[test]
    fn localized_name_picks_the_first_non_english_family_name() {
        let ms_gothic = names(&[("MS Gothic", Some("en-US")), ("ＭＳ ゴシック", Some("ja-JP"))]);
        assert_eq!(super::localized_name("MS Gothic", &ms_gothic).as_deref(), Some("ＭＳ ゴシック"));
        let yahei = names(&[("Microsoft YaHei", Some("en")), ("微软雅黑", Some("zh-CN")), ("微軟雅黑", Some("zh-TW"))]);
        assert_eq!(super::localized_name("Microsoft YaHei", &yahei).as_deref(), Some("微软雅黑"));
        // English-only fonts, other English locales, unknown languages and copies of the English
        // name give no localized name.
        assert_eq!(super::localized_name("Arial", &names(&[("Arial", Some("en-US"))])), None);
        assert_eq!(super::localized_name("Arial", &names(&[("Arial", Some("en-GB")), ("Arial", Some("de-DE"))])), None);
        assert_eq!(super::localized_name("X", &names(&[("Y", None), ("  ", Some("ja-JP"))])), None);
        assert_eq!(super::localized_name("X", &[]), None);
    }

    #[test]
    fn missing_font_dirs_are_skipped() {
        let mut files = Vec::new();
        super::collect_font_files(std::path::Path::new("/nonexistent/photocraft/fonts"), 0, usize::MAX, &mut files);
        assert!(files.is_empty());
    }
}

#[cfg(test)]
#[path = "font_names_tests.rs"]
mod font_names_tests;

// Two attributes: clippy's unwrap-in-tests allowance looks for a plain `cfg(test)`.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
#[path = "font_files_tests.rs"]
mod font_files_tests;
