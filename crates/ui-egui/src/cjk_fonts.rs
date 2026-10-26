//! Lazy CJK and other-script fallback fonts for the UI.
//!
//! The bundled Inter / JetBrains Mono have no Japanese, Chinese or Korean glyphs, and the OS
//! fonts that do are large (Hiragino ~10 MB, Apple SD Gothic Neo 28 MB, PingFang 78 MB, Noto
//! Sans CJK ~20 MB). Instead of reading them at startup, an egui plugin scans each frame's text
//! for CJK characters no registered font covers, and registers the next system font for that
//! character's script (`ctx.add_font`, active from the next frame, which it requests). Fonts are
//! appended at the lowest priority to every family, so Latin text keeps Inter.
//!
//! Other scripts the bundled fonts lack (Thai, Arabic, Hebrew, Devanagari and the other Indic
//! scripts, ...: [`photocraft_text::scripts`]) work the same way, each with its own attempt state,
//! independent of the CJK order: the first installed candidate file whose character map really
//! has the missing character is registered, once per script. egui shapes each font run with
//! HarfRust, so Thai marks and Arabic joining come from the font; it has no bidi reordering yet.
//!
//! Script order follows the effective UI locale ([`photocraft_text::cjk::script_order`]): Kana
//! prefers a Japanese font, Hangul a Korean one, Bopomofo a Traditional Chinese one, and Han the
//! locale's script. The effective locale is the translated UI language when it names
//! ja/ko/zh-Hans/zh-Hant; otherwise it is the OS locale. An English UI on a Japanese, Korean or
//! Chinese OS therefore follows that OS script, not the English baseline. The craft manifest
//! lists Shippori Mincho and BIZ UDPGothic before Noto Sans CJK SC, so a non-CJK/unknown
//! effective locale keeps the migration baseline's first covering face for shared Han and kana
//! (Shippori Mincho Regular); Hangul intentionally uses the Korean face (NanumGothic) instead,
//! because [`CjkChar::preferred`] always prefers Korean, even though the old linear stack would
//! have picked the broad Simplified Chinese face. An explicit zh-Hans/zh-Hant/ja/ko preference
//! selects its own form and keeps the UI-first face for its script (BIZ UDPGothic for Japanese).
//! The lazy loader reads at most one font per frame, each file at most once, and once every
//! script has been tried the scan stops.
//!
//! Builds made with the optional craft-fonts input (`CRAFT_FONTS_DIR`,
//! [`photocraft_text::craft_fonts`]) carry Japanese, Chinese, Korean and other faces. A
//! per-character [`FontProvider`] serves them, and the system font a locale defers to, in the
//! first frame; the lazy loader remains the later fallback. The provider has no lifetime read
//! cap. It retains at most [`MAX_SYSTEM_CACHE_BYTES`] of unique font-file blobs, charging a
//! collection (TTC) once however many faces it selects; a face that cannot be retained is
//! returned transiently for the current request, so a later Fontconfig candidate is never
//! skipped merely because earlier characters read other fonts.
//! Without craft-fonts (and on the web, which never embeds them) nothing changes.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FallbackRequest, FontData, FontFamily, FontId, FontProvider, Shape};
use photocraft_text::cjk::{self, CjkChar, CjkScript, FontFile};
use photocraft_text::craft_fonts::{self, CraftFont};
use photocraft_text::scripts::{self, Script};
use std::path::PathBuf;

/// Skip absurdly large files (a corrupt or non-font path must not eat memory).
pub const MAX_FONT_BYTES: u64 = 128 << 20;
/// At most this many candidate files are read for one non-CJK script by the lazy loader.
const MAX_SCRIPT_READS: usize = 6;
/// Upper bound on the unique font-file blobs the eager provider retains in its own cache.
/// Structurally invalid candidates are not retained at all. A readable file is charged once,
/// whatever number of faces the provider selects from a collection; it is evicted before the
/// budget is exceeded. A file that cannot be retained is still used for the current request and
/// returned transiently to egui (which may own that installed face independently). There is
/// deliberately no lifetime read cap: a valid Fontconfig match appended after many
/// absent/non-covering static paths (or after earlier characters) must still be reached.
const MAX_SYSTEM_CACHE_BYTES: usize = 128 << 20;
/// Remembered face names are tiny but still bounded independently of the blob budget.
const MAX_SYSTEM_FACE_NAMES: usize = 1024;
/// Name prefix of the registered fallback fonts.
pub const FONT_PREFIX: &str = "system-cjk";

/// Where fonts come from; swapped out in tests.
#[derive(Clone)]
pub struct Sources {
    pub locale: fn() -> Option<String>,
    pub files: fn(CjkScript) -> Vec<FontFile>,
    pub last_resort: fn() -> Vec<FontFile>,
    /// Whether the background Fontconfig CJK lookup is still running. The provider must not fall
    /// through to a lower-priority script (for example Simplified Chinese for a Hant character)
    /// while the preferred script may still gain a system face. Injectable so tests (and the web
    /// build) never depend on timing.
    pub pending: fn() -> bool,
    /// Candidate files for a non-CJK script, most preferred first; read only when a visible
    /// character of that script is missing.
    pub scripts: fn(Script) -> Vec<PathBuf>,
    /// The face of a font file whose character map has the character (`None`: not this file).
    pub covers: fn(&[u8], char) -> Option<u32>,
    /// Embedded craft faces declaring an ISO 15924 script, in the script's own order. The
    /// provider and lazy loader both use this injection (tests can therefore pin craft-font
    /// selection without depending on the build input).
    pub embedded: fn(&'static str) -> Vec<&'static CraftFont>,
    /// Install the per-character craft-font provider, so a headless first frame can shape CJK
    /// and other scripts before the lazy loader gets a second pass (tests panic on a missing
    /// glyph). It selects a face for the character's script instead of putting a broad face into
    /// the family stack up front for every later character.
    pub eager_craft_fonts: bool,
}

/// Craft fonts declaring `iso`, Regular first and (for Japanese) the UI face first; empty
/// without craft-fonts. This is the explicit-locale order; a non-CJK/unknown effective locale
/// uses the raw manifest order through [`craft_face_manifest`] instead.
///
/// The lazy loader tries these before system files for a script, so its fallback order stays
/// `preferred script, then the locale order` even for a character the provider left to it.
pub fn craft_embedded_iso(iso: &'static str) -> Vec<&'static CraftFont> {
    craft_faces(iso).into_iter().map(|(_, f)| f).collect()
}

/// Craft fonts for a CJK script; a convenience wrapper for callers that already hold the enum.
pub fn craft_embedded(script: CjkScript) -> Vec<&'static CraftFont> {
    craft_embedded_iso(cjk_iso(script))
}

/// No embedded fonts (tests that pin the system-font behaviour).
pub fn no_embedded(_: &'static str) -> Vec<&'static CraftFont> {
    Vec::new()
}

/// Craft fonts chosen per character, the way the lazy loader chooses them for one missing glyph.
///
/// A single linear font stack cannot express the per-script preference: a broad Noto Sans CJK SC
/// face covers kana and Hangul too, so putting it in the stack would make it win there over the
/// Japanese or Korean face. A [`FontProvider`] is asked per missing character instead, and this
/// one returns the face for that character's script. When the preferred script has no craft face
/// (e.g. Traditional Chinese), it asks the system font for that script first, and only falls
/// through the locale order (so a Simplified Chinese face comes last) when the system has none.
///
/// egui still resolves a character from an already discovered broad face before asking a
/// provider again; a mixed-script run whose first character discovered SC can therefore serve a
/// later kana/Hangul character from SC. Removing that last ambiguity needs script-aware fallback
/// in epaint (or per-script text runs); this provider fixes the eager-stack case.
struct CraftFontProvider {
    locale: fn() -> Option<String>,
    files: fn(CjkScript) -> Vec<FontFile>,
    last_resort: fn() -> Vec<FontFile>,
    pending: fn() -> bool,
    scripts: fn(Script) -> Vec<PathBuf>,
    covers: fn(&[u8], char) -> Option<u32>,
    embedded: fn(&'static str) -> Vec<&'static CraftFont>,
    /// Interface > UI Font Size multiplier for the discovered faces (see `theme::ui_font_scale`).
    scale: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// System files, faces and failed lookups read for characters no craft face covers.
    system: std::sync::Mutex<SystemCache>,
    /// I/O for a candidate path; `read_font` in production, injectable by tests.
    reader: fn(&std::path::Path) -> Option<Vec<u8>>,
}

/// A readable, structurally valid font file retained as one shared blob, no matter how many
/// faces of it the provider selects. The blob is charged to the byte budget once.
struct CachedFile {
    path: PathBuf,
    data: FontData,
    bytes: usize,
}

struct SystemCache {
    /// Unique retained font blobs, oldest first; evicted when the byte budget is exceeded.
    /// A collection's faces share this one entry, so a TTC is not charged per face.
    files: Vec<CachedFile>,
    /// Paths whose read failed (missing, empty, oversized, unreadable): never retry them for
    /// each missing character.
    failed_reads: std::collections::HashSet<PathBuf>,
    /// Paths that were readable but structurally not a font (truncated download, unrelated
    /// file): negative-cached separately from an I/O failure and from a non-covering face.
    invalid_fonts: std::collections::HashSet<PathBuf>,
    /// Names of faces handed to egui, reusing their name across characters. These carry no font
    /// bytes (the blob stays in `files` while retained) and are capped by
    /// [`MAX_SYSTEM_FACE_NAMES`].
    faces: Vec<SystemFace>,
    /// `face_index_for_family_checked` results, including family misses, so a collection's
    /// `name`/directory tables are not parsed again for every character.
    family_indices: Vec<(PathBuf, String, Option<u32>)>,
    /// Bytes of the unique blobs in `files`. A selected face adds no bytes: egui's installed copy
    /// may outlive the provider's reference, but the provider charges and bounds only its own
    /// retained blob.
    retained_bytes: usize,
    /// Budget for `retained_bytes`; injectable (small) in tests.
    max_bytes: usize,
    /// Next face name; monotonic so names are never reused for a different face.
    next_face_name: usize,
    /// Successful I/O reads. Diagnostics/tests only: there is deliberately no lifetime cap.
    #[cfg_attr(not(test), allow(dead_code))]
    reads: usize,
}

impl Default for SystemCache {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            failed_reads: std::collections::HashSet::new(),
            invalid_fonts: std::collections::HashSet::new(),
            faces: Vec::new(),
            family_indices: Vec::new(),
            retained_bytes: 0,
            max_bytes: MAX_SYSTEM_CACHE_BYTES,
            next_face_name: 0,
            reads: 0,
        }
    }
}

impl SystemCache {
    /// Evict the oldest blobs until `extra` more bytes fit. Returns `false` when even an empty
    /// cache cannot hold `extra`; in that case nothing is evicted, because a request that must be
    /// transient should not discard useful cached candidates. Face names are not pinned blobs:
    /// egui may keep an installed copy, but the provider's own byte cache stays bounded.
    fn make_room(&mut self, extra: usize) -> bool {
        if extra > self.max_bytes {
            return false;
        }
        while self.retained_bytes.saturating_add(extra) > self.max_bytes {
            let Some(file) = self.files.first() else { return false };
            let bytes = file.bytes;
            self.files.remove(0);
            self.retained_bytes = self.retained_bytes.saturating_sub(bytes);
        }
        true
    }

    /// Remember a face name without retaining font bytes. Names are capped; forgetting an old
    /// name only means egui may install a face under a new name later.
    fn remember_face(&mut self, path: PathBuf, index: u32, name: String) {
        if self.faces.len() >= MAX_SYSTEM_FACE_NAMES {
            self.faces.remove(0);
        }
        self.faces.push(SystemFace { path, index, name });
    }
}

struct SystemFace {
    path: PathBuf,
    index: u32,
    name: String,
}

impl CraftFontProvider {
    fn new(sources: &Sources, scale: f32) -> Self {
        Self {
            locale: sources.locale,
            files: sources.files,
            last_resort: sources.last_resort,
            pending: sources.pending,
            scripts: sources.scripts,
            covers: sources.covers,
            embedded: sources.embedded,
            scale: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(scale.to_bits())),
            system: std::sync::Mutex::new(SystemCache::default()),
            reader: read_font,
        }
    }

    fn scale(&self) -> f32 {
        f32::from_bits(self.scale.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn set_scale(&self, scale: f32) {
        self.scale.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// The bytes of `path`, read at most once while its blob is retained and shared even when a
    /// later request wants a different face of the same collection.
    ///
    /// The whole check/read/insert runs under the cache lock, so concurrent layout threads share
    /// one read per path and a success can never be shadowed by another thread's stale failure.
    /// Missing/unreadable paths and readable non-font bytes are negative-cached separately.
    fn system_source(&self, path: &std::path::Path) -> Option<FontData> {
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.failed_reads.contains(path) || cache.invalid_fonts.contains(path) {
            return None;
        }
        if let Some(file) = cache.files.iter().find(|file| file.path.as_path() == path) {
            return Some(FontData::from_blob(file.data.font.clone(), 0));
        }
        let Some(bytes) = (self.reader)(path) else {
            cache.failed_reads.insert(path.to_path_buf());
            return None;
        };
        cache.reads = cache.reads.saturating_add(1);
        if !cjk::font_file_is_readable(&bytes) {
            // A readable non-font is an invalid candidate, not a valid face that merely lacks
            // this character: remember that separately and never retain its bytes.
            cache.invalid_fonts.insert(path.to_path_buf());
            return None;
        }
        let size = bytes.len();
        let data = FontData::from_owned(bytes);
        let source = FontData::from_blob(data.font.clone(), 0);
        if cache.make_room(size) {
            cache.retained_bytes = cache.retained_bytes.saturating_add(size);
            cache.files.push(CachedFile { path: path.to_path_buf(), data, bytes: size });
        }
        // If this file alone exceeds the budget, the bytes are still used for this request but
        // not retained. egui may keep the installed face; the provider's own cache stays bounded.
        Some(source)
    }

    /// The (cached) face index whose family name is `family`; `None` is cached as a family miss.
    fn checked_face_index(&self, path: &std::path::Path, family: &str, bytes: &[u8]) -> Option<u32> {
        {
            let cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((_, _, index)) = cache.family_indices.iter().find(|(cached, name, _)| cached.as_path() == path && name.as_str() == family) {
                return *index;
            }
        }
        let index = cjk::face_index_for_family_checked(bytes, family);
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cache.family_indices.iter().any(|(cached, name, _)| cached.as_path() == path && name.as_str() == family) {
            cache.family_indices.push((path.to_path_buf(), family.to_owned(), index));
        }
        index
    }

    /// Register one face of an already-read system file, reusing its name but applying the
    /// current UI scale on every return (a cache must never bake the scale in).
    ///
    /// A face never adds a second charge for its collection's blob. If the blob was not retained
    /// (it alone exceeds the budget, or another thread evicted it first), this still returns a
    /// complete transient `FontInsert`: egui installs and owns that copy, while the provider's
    /// own cache remains within `max_bytes`.
    fn system_insert_face(&self, path: PathBuf, index: u32, source: FontData) -> FontInsert {
        let mut cache = self.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let name = match cache.faces.iter().find(|f| f.path == path && f.index == index) {
            Some(face) => face.name.clone(),
            None => {
                let name = format!("craft-system-{}", cache.next_face_name);
                cache.next_face_name = cache.next_face_name.saturating_add(1);
                cache.remember_face(path, index, name.clone());
                name
            }
        };
        let mut data = FontData::from_blob(source.font, index);
        apply_font_tweaks(&mut data);
        font_insert(name, data, self.scale())
    }

    /// The first system face for `script` (or any last-resort face) whose requested family really
    /// covers `c`.
    ///
    /// The lazy loader reads these after the first frame; the provider needs the same source for
    /// the first frame, before a headless layout with `MissingGlyphPolicy::Panic` can finish.
    /// A non-empty family is resolved by name first, then checked against that exact face: never
    /// use the first face of a collection that happens to cover `c`.
    fn system_insert_files(&self, files: Vec<FontFile>, c: char) -> Option<FontInsert> {
        for file in files {
            let Some(source) = self.system_source(&file.path) else { continue };
            let index = if file.family.is_empty() {
                0
            } else {
                let Some(index) = self.checked_face_index(&file.path, &file.family, source.bytes()) else { continue };
                index
            };
            // A readable collection can still hold a malformed selected face whose `cmap` and
            // `name` work; do not hand that index to epaint, which panics when it installs a
            // configured face it cannot parse.
            if !cjk::font_face_is_readable(source.bytes(), index) {
                continue;
            }
            if !scripts::face_covers(source.bytes(), index, c) {
                continue;
            }
            return Some(self.system_insert_face(file.path, index, source));
        }
        None
    }

    /// The first non-CJK candidate path that maps `c`; these files have no requested family, so
    /// the same character-map rule as the lazy loader applies.
    fn system_insert_paths(&self, files: Vec<PathBuf>, c: char) -> Option<FontInsert> {
        for path in files {
            let Some(source) = self.system_source(&path) else { continue };
            let Some(index) = (self.covers)(source.bytes(), c) else { continue };
            if !cjk::font_face_is_readable(source.bytes(), index) {
                continue;
            }
            return Some(self.system_insert_face(path, index, source));
        }
        None
    }
}

impl FontProvider for CraftFontProvider {
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let c = request.cluster.chars().next()?;
        let locale = (self.locale)();
        let order = cjk::script_order(locale.as_deref());
        // The migration baseline appended every craft face to the family stack in manifest
        // order, so a non-CJK/unknown effective locale drew shared Han/kana with the first
        // manifest face that covered the character (Shippori Mincho Regular). An explicit
        // ja/ko/zh preference is a new choice and keeps the UI-first order for its script.
        // `locale` already resolves an English UI on a CJK OS to that OS locale, so this
        // baseline does not apply there.
        let manifest_baseline = cjk::locale_script(locale.as_deref()).is_none();
        let craft = |iso: &'static str| {
            if manifest_baseline { craft_face_manifest(self.embedded, iso, c) } else { craft_face_for(self.embedded, iso, c) }
        };
        if let Some(kind) = cjk::classify(c) {
            // Han follows the locale's script. For a non-CJK/unknown effective locale, try the
            // manifest's Japanese faces first (exactly as the baseline stack did) before the
            // SC-first system order.
            if kind == cjk::CjkChar::Han
                && manifest_baseline
                && let Some((index, face)) = craft_face_manifest(self.embedded, "Jpan", c)
            {
                return Some(craft_insert(index, face, self.scale()));
            }
            // The same order as `CjkFallback::next_font`: the character's preferred script first,
            // then the locale order; craft faces and system files per script. A Traditional
            // Chinese locale therefore checks its system Hant font before any Simplified craft
            // face, and only falls through when the system has no Hant face either.
            //
            // While fontconfig is still looking for the preferred script's face, do not cache a
            // lower-priority face (SC for a Hant character): egui keeps that wrong choice for the
            // installed font chain. Returning `None` lets the next layout retry after the scan.
            let mut scripts = vec![kind.preferred(&order)];
            scripts.extend(order);
            let mut tried: Vec<CjkScript> = Vec::new();
            for script in scripts {
                if tried.contains(&script) {
                    continue;
                }
                tried.push(script);
                if let Some((index, face)) = craft(cjk_iso(script)) {
                    return Some(craft_insert(index, face, self.scale()));
                }
                if let Some(insert) = self.system_insert_files((self.files)(script), c) {
                    return Some(insert);
                }
                if (self.pending)() {
                    return None;
                }
            }
            return self.system_insert_files((self.last_resort)(), c);
        }
        let Some(script) = scripts::classify(c) else {
            return craft("Zsym").map(|(index, face)| craft_insert(index, face, self.scale()));
        };
        if let Some((index, face)) = craft(script_iso(script)) {
            return Some(craft_insert(index, face, self.scale()));
        }
        if let Some(insert) = self.system_insert_paths((self.scripts)(script), c) {
            return Some(insert);
        }
        // The lazy loader's last-resort list is broad (Arial Unicode, Droid Sans Fallback): it is
        // the same final source for non-CJK scripts when no script-specific file is installed.
        self.system_insert_files((self.last_resort)(), c)
    }
}

/// Whether `face` really maps `c` in the face [`craft_insert`] would install.
///
/// `craft_insert` builds `FontData::from_static(face.bytes)`, whose default face index is 0, so
/// testing any face of a collection would select a face whose glyphs egui never uses. Test face
/// 0 here, exactly like the lazy loader's `load_embedded_faces`.
fn craft_face_installs_coverage(face: &CraftFont, c: char) -> bool {
    !face.bytes.is_empty() && cjk::font_face_is_readable(face.bytes, 0) && scripts::face_covers(face.bytes, 0, c)
}

/// The best injected craft face declaring `iso` that really covers `c` in the face
/// [`craft_insert`] installs.
fn craft_face_for(embedded: fn(&'static str) -> Vec<&'static CraftFont>, iso: &'static str, c: char) -> Option<(usize, &'static CraftFont)> {
    embedded(iso).into_iter().find_map(|face| {
        let index = craft_index(face)?;
        craft_face_installs_coverage(face, c).then_some((index, face))
    })
}

/// The injected craft face the migration baseline would have used first: the earliest manifest
/// face declaring `iso` that really covers `c` in the face [`craft_insert`] installs. (The
/// baseline appended the whole manifest in order; for Japanese that is Shippori Mincho Regular,
/// not the UI-first BIZ UDPGothic.)
fn craft_face_manifest(embedded: fn(&'static str) -> Vec<&'static CraftFont>, iso: &'static str, c: char) -> Option<(usize, &'static CraftFont)> {
    let mut faces = embedded(iso);
    // Stable: faces outside the global table (test doubles) keep their injected order.
    faces.sort_by_key(|face| craft_index(face).unwrap_or(usize::MAX));
    faces.into_iter().find_map(|face| {
        let index = craft_index(face)?;
        craft_face_installs_coverage(face, c).then_some((index, face))
    })
}

fn cjk_iso(script: CjkScript) -> &'static str {
    match script {
        CjkScript::Japanese => "Jpan",
        CjkScript::SimplifiedChinese => "Hans",
        CjkScript::TraditionalChinese => "Hant",
        CjkScript::Korean => "Kore",
    }
}

fn script_iso(script: Script) -> &'static str {
    use Script::*;
    match script {
        Thai => "Thai",
        Lao => "Laoo",
        Khmer => "Khmr",
        Myanmar => "Mymr",
        Tibetan => "Tibt",
        Arabic => "Arab",
        Hebrew => "Hebr",
        Armenian => "Armn",
        Georgian => "Geor",
        Ethiopic => "Ethi",
        Devanagari => "Deva",
        Bengali => "Beng",
        Gurmukhi => "Guru",
        Gujarati => "Gujr",
        Oriya => "Orya",
        Tamil => "Taml",
        Telugu => "Telu",
        Kannada => "Knda",
        Malayalam => "Mlym",
        Sinhala => "Sinh",
    }
}

/// The manifest index of a craft face (the name suffix of its `FontInsert`).
fn craft_index(face: &CraftFont) -> Option<usize> {
    craft_fonts::CRAFT_FONTS.iter().position(|candidate| std::ptr::eq(candidate, face))
}

/// Craft faces declaring `iso`, Regular first, with the Japanese UI face ahead of the other
/// Japanese faces (the explicit-locale choice; a non-CJK/unknown effective locale keeps the
/// manifest order).
fn craft_faces(iso: &str) -> Vec<(usize, &'static CraftFont)> {
    let mut faces: Vec<(usize, &'static CraftFont)> =
        craft_fonts::CRAFT_FONTS.iter().enumerate().filter(|(_, f)| !f.bytes.is_empty() && f.scripts.contains(&iso)).collect();
    faces.sort_by_key(|(_, f)| (f.family != craft_fonts::UI_JAPANESE_FAMILY, f.style != "Regular"));
    faces
}

/// Baseline and variable-weight tweaks shared by craft and system fallbacks.
fn apply_font_tweaks(data: &mut FontData) {
    let offset = vertical_metrics(data.bytes(), data.index)
        .zip(vertical_metrics(photocraft_text::fonts::INTER_REGULAR.as_slice(), 0))
        .map(|(face, primary)| baseline_offset(face, primary));
    let weight = regular_weight(data.bytes(), data.index);
    if let Some(offset) = offset {
        data.tweak.y_offset_factor = offset;
    }
    if let Some(weight) = weight {
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
    }
}

fn font_insert(name: String, mut data: FontData, scale: f32) -> FontInsert {
    data.tweak.scale = scale;
    let families: Vec<InsertFontFamily> =
        [FontFamily::Proportional, FontFamily::Monospace].into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    FontInsert::new(&name, data, families)
}

/// One craft face as an egui font, with the same baseline/weight tweaks as a lazy fallback and the
/// current UI font-size multiplier. Added to both UI families: a discovered face is appended to
/// each family that asks for it, and a headless frame may use either. `FontData::from_static`
/// leaves the face index at its default 0; the coverage helpers above test that same face.
fn craft_insert(index: usize, face: &'static CraftFont, scale: f32) -> FontInsert {
    let mut data = FontData::from_static(face.bytes);
    apply_font_tweaks(&mut data);
    font_insert(format!("craft-font-{index}"), data, scale)
}

fn craft_provider_id() -> egui::Id {
    egui::Id::unique("photocraft-craft-font-provider")
}

/// The effective locale for CJK choice: an explicitly translated UI language
/// (ja/ko/zh-Hans/zh-Hant) wins; otherwise the OS locale, when known.
///
/// An English UI on a Japanese, Korean or Chinese OS therefore uses that OS script, not the
/// migration baseline's manifest-first face. The baseline applies only when the effective locale
/// is non-CJK or unknown.
fn effective_locale(ui_code: &str, os_locale: Option<&str>) -> Option<String> {
    match ui_code {
        "ja" | "ko" | "zh-hans" | "zh-hant" => Some(ui_code.to_owned()),
        _ => os_locale.map(str::to_owned),
    }
}

impl Sources {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn system() -> Self {
        cjk::start_fontconfig_cjk_scan();
        Self {
            locale: || effective_locale(crate::i18n::current().code(), cjk::ui_locale()),
            files: cjk::font_files,
            last_resort: cjk::last_resort_files,
            pending: cjk::fontconfig_cjk_scan_pending,
            scripts: scripts::system_candidates,
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: true,
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn system() -> Self {
        Self {
            locale: || None,
            files: |_| Vec::new(),
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            // The web build embeds no craft fonts; keep its font stack as it was.
            eager_craft_fonts: false,
        }
    }
}

/// Headless test frame pumps do not wait for `request_repaint_after`, so let the optional
/// Fontconfig worker finish before asserting on installed system fonts. `pub` (rather than
/// `pub(crate)`) so strict-policy integration tests can use the same wait as the unit tests.
///
/// Returns whether the scan finished; it never panics in production. Test callers assert the
/// result (and choose their own message).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn wait_for_fontconfig_cjk_scan() -> bool {
    cjk::start_fontconfig_cjk_scan();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while cjk::fontconfig_cjk_scan_pending() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    !cjk::fontconfig_cjk_scan_pending()
}

/// The lazy loader's state: which scripts were tried and which files were read.
pub struct CjkFallback {
    sources: Sources,
    /// set_fonts takes effect next pass; do not collide with a previous loader's font names.
    defer_after_reset: bool,
    order: Option<[CjkScript; 4]>,
    tried: Vec<CjkScript>,
    last_resort_tried: bool,
    /// Non-CJK scripts whose candidates were tried (each at most once per install).
    scripts_tried: Vec<Script>,
    loaded: Vec<PathBuf>,
    /// Fonts registered so far: (name, path, face index).
    pub registered: Vec<(String, PathBuf, u32)>,
}

impl CjkFallback {
    pub fn new(sources: Sources) -> Self {
        Self {
            sources,
            defer_after_reset: false,
            order: None,
            tried: Vec::new(),
            last_resort_tried: false,
            scripts_tried: Vec::new(),
            loaded: Vec::new(),
            registered: Vec::new(),
        }
    }

    /// Script order for the UI locale (resolved on first use, not at startup).
    pub fn order(&mut self) -> [CjkScript; 4] {
        *self.order.get_or_insert_with(|| cjk::script_order((self.sources.locale)().as_deref()))
    }

    /// Every script and the last-resort fonts have been tried: nothing more to load.
    pub fn exhausted(&self) -> bool {
        self.tried.len() >= 4 && self.last_resort_tried
    }

    /// The next font to register for `c` (kind `kind`) that no current font covers: the first
    /// readable file of the first untried script (the character's preferred script, then the
    /// locale order), then the last-resort fonts. For a non-CJK/unknown effective locale the
    /// craft faces for shared Han and kana follow the manifest's own order (Shippori Mincho
    /// before BIZ UDPGothic), exactly like the migration baseline's family stack; Hangul keeps
    /// the Korean face. `None` when nothing is left or while the preferred script's Fontconfig
    /// lookup is still pending.
    pub fn next_font(&mut self, c: char, kind: CjkChar) -> Option<(String, FontData)> {
        // The craft manifest lists Shippori Mincho first. A non-CJK effective locale keeps that
        // baseline choice for Han and kana; an explicit zh-Hans/zh-Hant/ja/ko preference keeps the
        // script's UI-first order.
        let manifest_baseline = cjk::locale_script((self.sources.locale)().as_deref()).is_none();
        if kind == CjkChar::Han
            && manifest_baseline
            && let Some(f) = self.load_embedded_manifest(CjkScript::Japanese, c)
        {
            return Some(f);
        }
        let order = self.order();
        let scripts: Vec<CjkScript> = std::iter::once(kind.preferred(&order)).chain(order).collect();
        for s in scripts {
            if self.tried.contains(&s) {
                continue;
            }
            let embedded = if manifest_baseline { self.load_embedded_manifest(s, c) } else { self.load_embedded(s, c) };
            if let Some(f) = embedded {
                self.tried.push(s);
                return Some(f);
            }
            let fontconfig_pending = (self.sources.pending)();
            let files = (self.sources.files)(s);
            if let Some(f) = self.load_first(&files, c) {
                self.tried.push(s);
                return Some(f);
            }
            if fontconfig_pending {
                return None;
            }
            self.tried.push(s);
        }
        if !self.last_resort_tried {
            self.last_resort_tried = true;
            let files = (self.sources.last_resort)();
            return self.load_first(&files, c);
        }
        None
    }

    /// Every non-CJK script has been tried.
    pub fn scripts_exhausted(&self) -> bool {
        self.scripts_tried.len() >= Script::ALL.len()
    }

    /// The first candidate font for `c`'s script that really maps `c`; the script is tried once.
    pub fn next_script_font(&mut self, c: char) -> Option<(String, FontData)> {
        let script = scripts::classify(c)?;
        if self.scripts_tried.contains(&script) {
            return None;
        }
        self.scripts_tried.push(script);
        let mut reads = 0;
        for path in (self.sources.scripts)(script) {
            if reads >= MAX_SCRIPT_READS {
                break;
            }
            if self.loaded.contains(&path) {
                continue;
            }
            let Some(bytes) = read_font(&path) else { continue };
            reads += 1;
            if !cjk::font_file_is_readable(&bytes) {
                continue;
            }
            let Some(index) = (self.sources.covers)(&bytes, c) else { continue };
            if !cjk::font_face_is_readable(&bytes, index) {
                continue;
            }
            self.loaded.push(path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) for {script:?} as {name}", path.display());
            self.registered.push((name.clone(), path, index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        log::info!("UI font fallback: no installed font covers {script:?}");
        None
    }

    /// The first embedded (craft-fonts) face for `s` in the UI-first order (explicit locale).
    fn load_embedded(&mut self, s: CjkScript, c: char) -> Option<(String, FontData)> {
        let faces = (self.sources.embedded)(cjk_iso(s));
        self.load_embedded_faces(faces, c)
    }

    /// The first embedded face for `s` in the migration baseline's manifest order
    /// (non-CJK/unknown effective locale): Shippori Mincho Regular before BIZ UDPGothic.
    fn load_embedded_manifest(&mut self, s: CjkScript, c: char) -> Option<(String, FontData)> {
        let mut faces = (self.sources.embedded)(cjk_iso(s));
        faces.sort_by_key(|face| craft_index(face).unwrap_or(usize::MAX));
        self.load_embedded_faces(faces, c)
    }

    /// The first of `faces` that really covers `c`, registered from its static bytes.
    fn load_embedded_faces(&mut self, faces: Vec<&'static CraftFont>, c: char) -> Option<(String, FontData)> {
        let f = faces.into_iter().find(|f| !f.bytes.is_empty() && cjk::font_face_is_readable(f.bytes, 0) && scripts::face_covers(f.bytes, 0, c))?;
        let path = PathBuf::from(format!("craft-fonts/{} {}", f.family, f.style));
        if self.loaded.contains(&path) {
            return None;
        }
        self.loaded.push(path.clone());
        let name = format!("{FONT_PREFIX}-{}", self.registered.len());
        log::info!("UI font fallback: registered embedded {} as {name}", path.display());
        self.registered.push((name.clone(), path, 0));
        Some((name, FontData::from_static(f.bytes)))
    }

    /// The first file whose requested collection face (or face 0 for an empty family) really
    /// covers `c`. A family miss skips the file rather than using whichever face happens to cover
    /// the character.
    fn load_first(&mut self, files: &[FontFile], c: char) -> Option<(String, FontData)> {
        for f in files {
            if self.loaded.contains(&f.path) {
                continue;
            }
            let Some(bytes) = read_font(&f.path) else { continue };
            // Register only what the text engine can actually parse. A readable collection may
            // still have a selected face whose `cmap` is valid while another table points outside
            // the file; `font_face_is_readable` below rejects such a face before registration.
            if !cjk::font_file_is_readable(&bytes) {
                continue;
            }
            let index = if f.family.is_empty() {
                0
            } else {
                let Some(index) = cjk::face_index_for_family_checked(&bytes, &f.family) else { continue };
                index
            };
            // `face_covers` only reads `cmap`; validate the selected face's whole table directory
            // before `ctx.add_font` may install (and panic on) a face another table makes unreadable.
            if !cjk::font_face_is_readable(&bytes, index) {
                continue;
            }
            if !scripts::face_covers(&bytes, index, c) {
                continue;
            }
            self.loaded.push(f.path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) as {name}", f.path.display());
            self.registered.push((name.clone(), f.path.clone(), index));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        None
    }
}

/// A font file's bytes, or `None` for missing, empty, oversized or unreadable files.
fn read_font(path: &std::path::Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

/// First supported missing character for which a fallback has not yet been exhausted.
fn missing_char(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape], fallback: &CjkFallback) -> Option<char> {
    fn collect(shape: &Shape, out: &mut Vec<char>, fallback: &CjkFallback, cjk_pending: bool) {
        let wanted = |c: char| (cjk_pending && cjk::classify(c).is_some()) || scripts::classify(c).is_some_and(|s| !fallback.scripts_tried.contains(&s));
        match shape {
            Shape::Text(t) => {
                let text = &t.galley.job.text;
                if !text.is_ascii() {
                    for c in text.chars().filter(|c| wanted(*c)) {
                        if out.len() >= 256 {
                            return;
                        }
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out, fallback, cjk_pending)),
            _ => {}
        }
    }
    let mut chars = Vec::new();
    for s in shapes {
        collect(&s.shape, &mut chars, fallback, !fallback.exhausted());
    }
    if chars.is_empty() {
        return None;
    }
    let font = FontId::proportional(12.0);
    ctx.fonts_mut(|f| chars.into_iter().find(|c| !f.has_glyph(&font, *c)))
}

/// Big-endian readers over a font's bytes; `None` past the end.
fn be_u16(font: &[u8], o: usize) -> Option<u16> {
    font.get(o..o.checked_add(2)?).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn be_u32(font: &[u8], o: usize) -> Option<u32> {
    font.get(o..o.checked_add(4)?).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The offset of table `tag` in face `index` of `font` (a collection or a single face).
fn table_offset(font: &[u8], index: u32, tag: &[u8; 4]) -> Option<usize> {
    let dir = if font.get(..4)? == b"ttcf" {
        // Collection: the offset table of face `index` follows a 12-byte header.
        be_u32(font, 12usize.checked_add((index as usize).checked_mul(4)?)?)? as usize
    } else {
        0
    };
    let tables = be_u16(font, dir.checked_add(4)?)? as usize;
    (0..tables).find_map(|i| {
        let rec = dir.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        (font.get(rec..rec.checked_add(4)?)? == tag).then_some(())?;
        Some(be_u32(font, rec.checked_add(8)?)? as usize)
    })
}

/// (ascent, descent, line gap) of a face in em, from its `hhea` table; `descent` is negative.
fn vertical_metrics(font: &[u8], index: u32) -> Option<(f32, f32, f32)> {
    let i16_at = |o: usize| be_u16(font, o).map(|v| v as i16 as f32);
    let (hhea, head) = (table_offset(font, index, b"hhea")?, table_offset(font, index, b"head")?);
    let upm = be_u16(font, head.checked_add(18)?)? as f32;
    if upm <= 0.0 {
        return None;
    }
    Some((i16_at(hhea.checked_add(4)?)? / upm, i16_at(hhea.checked_add(6)?)? / upm, i16_at(hhea.checked_add(8)?)? / upm))
}

/// The weight a variable face should be drawn at: Regular (400) within its `wght` axis, or
/// `None` when it has no weight axis or already defaults to Regular. Noto Sans CJK's variable
/// collection defaults to Thin (100), so the UI's CJK text came out hairline (#4820).
fn regular_weight(font: &[u8], index: u32) -> Option<f32> {
    let fvar = table_offset(font, index, b"fvar")?;
    let axes = fvar.checked_add(be_u16(font, fvar.checked_add(4)?)? as usize)?;
    let count = be_u16(font, fvar.checked_add(8)?)? as usize;
    let size = be_u16(font, fvar.checked_add(10)?)? as usize;
    let fixed = |o: usize| be_u32(font, o).map(|v| v as i32 as f32 / 65536.0);
    (0..count.min(64)).find_map(|i| {
        let rec = axes.checked_add(i.checked_mul(size)?)?;
        (font.get(rec..rec.checked_add(4)?)? == b"wght").then_some(())?;
        let (min, default, max) = (fixed(rec.checked_add(4)?)?, fixed(rec.checked_add(8)?)?, fixed(rec.checked_add(12)?)?);
        let regular = 400f32.clamp(min.min(max), max.max(min));
        (regular.is_finite() && (regular - default).abs() > 0.5).then_some(regular)
    })
}

/// `y_offset_factor` that puts `face`'s baseline where `primary`'s would be. egui centres a
/// fallback face's row in the primary font's row, so a face with a big line gap (Hiragino: 0.5em)
/// otherwise rides about 0.23em high next to Latin text.
fn baseline_offset(face: (f32, f32, f32), primary: (f32, f32, f32)) -> f32 {
    let height = |m: (f32, f32, f32)| m.0 - m.1 + m.2;
    let off = primary.0 - face.0 - 0.5 * (height(primary) - height(face));
    if off.is_finite() { off.clamp(-0.5, 0.5) } else { 0.0 }
}

/// Registers `name` at the lowest priority in every font family.
fn add_to_all_families(ctx: &egui::Context, name: String, mut data: FontData) {
    // Align to Inter, the primary of every UI family; JetBrains Mono differs by under 0.005em.
    let primary = ctx.fonts(|f| f.definitions().font_data.get("Inter").and_then(|p| vertical_metrics(p.bytes(), p.index)));
    if let (Some(face), Some(primary)) = (vertical_metrics(data.bytes(), data.index), primary) {
        data.tweak.y_offset_factor = baseline_offset(face, primary);
    }
    if let Some(weight) = regular_weight(data.bytes(), data.index) {
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
    }
    let mut families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
    for f in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&f) {
            families.push(f);
        }
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    crate::theme::size_ui_font(ctx, &name, &mut data);
    ctx.add_font(FontInsert { name, data, families });
}

/// The egui plugin that watches drawn text and loads fallback fonts on demand.
pub struct CjkFontPlugin(pub CjkFallback);

impl egui::Plugin for CjkFontPlugin {
    fn debug_name(&self) -> &'static str {
        "photocraft-cjk-fonts"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if std::mem::take(&mut self.0.defer_after_reset) {
            ctx.request_repaint();
            return;
        }
        if self.0.exhausted() && self.0.scripts_exhausted() {
            return;
        }
        let Some(c) = missing_char(ctx, &output.shapes, &self.0) else { return };
        let font = match cjk::classify(c) {
            Some(kind) => self.0.next_font(c, kind),
            None => self.0.next_script_font(c),
        };
        if let Some((name, data)) = font {
            add_to_all_families(ctx, name, data);
            ctx.request_repaint();
        } else if cjk::classify(c).is_some() && (self.0.sources.pending)() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if !self.0.exhausted() || !self.0.scripts_exhausted() {
            // Nothing readable for this script; try the next one on the next frame.
            ctx.request_repaint();
        }
    }
}

/// Installs the lazy CJK and other-script fallback (system fonts; nothing on the web).
pub fn install(ctx: &egui::Context) {
    install_with(ctx, Sources::system());
}

pub fn install_with(ctx: &egui::Context, sources: Sources) {
    // egui keeps the first plugin of a type. set_fonts removes loaded fallbacks, so reset the
    // existing loader too; otherwise its tried/loaded sets prevent fonts being added again.
    let mut fallback = CjkFallback::new(sources.clone());
    fallback.defer_after_reset = true;
    if ctx
        .with_plugin::<CjkFontPlugin, _>(|plugin| {
            plugin.0 = CjkFallback::new(sources.clone());
            plugin.0.defer_after_reset = true;
        })
        .is_none()
    {
        ctx.add_plugin(CjkFontPlugin(fallback));
    }
    // The provider resolves a craft face for each missing character in the first frame, before the
    // lazy loader's next-pass system fallback. Without it a headless frame panics on CJK text.
    if sources.eager_craft_fonts && !craft_fonts::CRAFT_FONTS.is_empty() {
        let provider = std::sync::Arc::new(CraftFontProvider::new(&sources, crate::theme::ui_font_scale(ctx)));
        let as_provider: std::sync::Arc<dyn FontProvider> = provider.clone();
        ctx.data_mut(|d| d.insert_temp(craft_provider_id(), provider));
        ctx.set_font_providers(vec![as_provider]);
    }
}

/// Keep provider-discovered craft faces in step with Interface > UI Font Size. The size plugin
/// calls this after `ctx.set_fonts` with the rescaled stack. A face discovered while the old stack
/// was still active stays at the old scale for the rest of that pass; recreating `Fonts` for the
/// next pass drops the discovered faces (they are not part of `FontDefinitions`) and the provider
/// rediscovers them at its new scale.
pub(crate) fn set_craft_font_scale(ctx: &egui::Context, scale: f32) {
    let Some(provider) = ctx.data(|d| d.get_temp::<std::sync::Arc<CraftFontProvider>>(craft_provider_id())) else { return };
    provider.set_scale(scale);
    let as_provider: std::sync::Arc<dyn FontProvider> = provider;
    ctx.set_font_providers(vec![as_provider]);
}
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use egui::FontProvider;
    use std::sync::Mutex;

    #[test]
    fn lazy_fallbacks_follow_ui_font_size_and_survive_size_changes() {
        use photocraft_engine::prefs::UiFontSize;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        crate::theme::set_ui_font_size(&ctx, UiFontSize::Large);
        let name = "test-lazy-fallback".to_string();
        add_to_all_families(&ctx, name.clone(), FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()));
        for (size, scale) in [(UiFontSize::Large, 16.0 / 12.0), (UiFontSize::Tiny, 10.0 / 12.0), (UiFontSize::Small, 1.0)] {
            crate::theme::set_ui_font_size(&ctx, size);
            for _ in 0..2 {
                ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
            }
            let fonts = ctx.fonts(|f| f.definitions().clone());
            let fallback = fonts.font_data.get(&name).unwrap();
            assert_eq!(fallback.tweak.scale, scale);
            assert_eq!(fallback.tweak.y_offset_factor, 0.0, "same face as Inter keeps its baseline");
            for stack in fonts.families.values() {
                assert_eq!(stack.iter().filter(|n| *n == &name).count(), 1);
                assert_eq!(stack.last(), Some(&name), "fallback stays at the lowest priority");
            }
        }
    }

    /// A face discovered by [`CraftFontProvider`] during a pass that also changes Interface > UI
    /// Font Size must use the scale of the definitions active in that pass. The size plugin moves
    /// the provider to the new scale only after `ctx.set_fonts` queued the rescaled definitions;
    /// updating the provider in `set_ui_font_size` would put the new scale on a face laid out
    /// with the old stack.
    #[test]
    fn discovered_faces_use_the_scale_of_the_definitions_they_were_laid_out_with() {
        use photocraft_engine::prefs::UiFontSize;
        fn thai_reader(_: &std::path::Path) -> Option<Vec<u8>> {
            Some(synthetic_font(&[(0x0E01, 0x0E5B)]))
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, craft_only(|| None));
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let mut provider = CraftFontProvider::new(
            &Sources {
                locale: || Some("en".into()),
                files: no_files,
                last_resort: Vec::new,
                pending: || false,
                scripts: |script| if script == Script::Thai { vec![PathBuf::from("synthetic-thai.ttf")] } else { Vec::new() },
                covers: scripts::covering_face,
                embedded: no_embedded,
                eager_craft_fonts: false,
            },
            1.0,
        );
        provider.reader = thai_reader;
        let provider = std::sync::Arc::new(provider);
        ctx.data_mut(|d| d.insert_temp(craft_provider_id(), provider.clone()));
        let as_provider: std::sync::Arc<dyn FontProvider> = provider.clone();
        ctx.set_font_providers(vec![as_provider]);

        // The scale of the configured Inter face (part of `FontDefinitions`) and of the face the
        // provider discovered for Thai.
        let inter_scale = |ctx: &egui::Context| -> Option<f32> { ctx.fonts(|f| f.definitions().font_data.get("Inter").map(|data| data.tweak.scale)) };
        let thai_scale = |ctx: &egui::Context| -> Option<f32> {
            ctx.fonts(|f| f.discovered_fonts().iter().find(|insert| insert.name.starts_with("craft-system-")).map(|insert| insert.data.tweak.scale))
        };
        // A frame that checks the state during the change pass, before laying out Thai (the
        // character that makes the provider discover a face).
        let frame = |check: &dyn Fn(&egui::Context)| {
            ctx.run_ui(Default::default(), |ui| {
                check(&ctx);
                ui.label("ก");
            })
            .textures_delta
            .clear();
        };
        // Small (the install default) is active. Walk Small -> Large -> Small, repeat Large
        // (the same size must leave both the definitions and the provider alone, not accumulate)
        // and back to Small.
        assert_eq!(provider.scale(), 1.0);
        for (size, want, same_size) in [
            (UiFontSize::Large, 16.0 / 12.0, false),
            (UiFontSize::Small, 1.0, false),
            (UiFontSize::Large, 16.0 / 12.0, false),
            (UiFontSize::Large, 16.0 / 12.0, true),
            (UiFontSize::Small, 1.0, false),
        ] {
            let old = provider.scale();
            let inter_before = inter_scale(&ctx);
            crate::theme::set_ui_font_size(&ctx, size);
            assert_eq!(provider.scale(), old, "the provider follows the definitions still active in this pass");
            frame(&|ctx: &egui::Context| {
                assert_eq!(inter_scale(ctx), inter_before, "the configured Inter keeps its old scale during the change pass");
                assert_eq!(provider.scale(), old, "the provider does not move before the pass ends");
            });
            assert_eq!(provider.scale(), want, "the provider follows the rescaled definitions");
            if same_size {
                assert_eq!(inter_scale(&ctx), inter_before, "a same-size request leaves the stack alone");
            }
            assert_eq!(thai_scale(&ctx), Some(old), "the pass laid the discovered face out with the old definitions");
            // The queued definitions with the new scale are active now, and a new pass drops the
            // discovered face and rediscovers it at the provider's new scale.
            frame(&|ctx: &egui::Context| {
                assert_eq!(inter_scale(ctx), Some(want), "the rescaled stack is active in the next pass");
                assert_eq!(provider.scale(), want);
            });
            assert_eq!(thai_scale(&ctx), Some(want), "the next pass rediscovers the face at the provider's scale");
        }
    }

    /// A font holding only an `fvar` table with the given axes (tag, min, default, max), as a
    /// single face or as face 1 of a two-face collection.
    fn fvar_font(axes: &[(&[u8; 4], f32, f32, f32)], collection: bool) -> Vec<u8> {
        let fixed = |v: f32| ((v * 65536.0) as i32).to_be_bytes();
        let mut fvar = Vec::new();
        for x in [1u16, 0, 16, 2, axes.len() as u16, 20, 0, 0] {
            fvar.extend(x.to_be_bytes());
        }
        for (tag, min, def, max) in axes {
            fvar.extend(*tag);
            for v in [*min, *def, *max] {
                fvar.extend(fixed(v));
            }
            fvar.extend([0, 0, 1, 0]);
        }
        let face = |at: u32| {
            let mut d = vec![0, 1, 0, 0, 0, 1, 0, 16, 0, 0, 0, 0];
            d.extend(b"fvar");
            d.extend([0; 4]);
            d.extend(at.to_be_bytes());
            d.extend((fvar.len() as u32).to_be_bytes());
            d
        };
        if !collection {
            let mut f = face(28);
            f.extend(&fvar);
            return f;
        }
        // ttcf header (12 + 2×4), two 28-byte directories, then the shared table.
        let mut f = b"ttcf".to_vec();
        f.extend([0, 1, 0, 0, 0, 0, 0, 2]);
        f.extend(20u32.to_be_bytes());
        f.extend(48u32.to_be_bytes());
        f.extend(face(76));
        f.extend(face(76));
        f.extend(&fvar);
        f
    }

    /// #4820: Noto Sans CJK's variable collection defaults to Thin; the fallback asks for Regular.
    #[test]
    fn variable_fallbacks_are_drawn_at_regular_weight() {
        let thin = fvar_font(&[(b"wght", 100.0, 100.0, 900.0)], true);
        assert_eq!(regular_weight(&thin, 1), Some(400.0), "Noto Sans CJK VF, face 1 of a collection");
        assert_eq!(regular_weight(&fvar_font(&[(b"wdth", 75.0, 100.0, 100.0), (b"wght", 100.0, 100.0, 900.0)], false), 0), Some(400.0));
        assert_eq!(regular_weight(&fvar_font(&[(b"wght", 500.0, 600.0, 900.0)], false), 0), Some(500.0), "Regular clamped into the axis");
        assert_eq!(regular_weight(&fvar_font(&[(b"wght", 100.0, 400.0, 900.0)], false), 0), None, "already Regular");
        assert_eq!(regular_weight(&fvar_font(&[(b"wdth", 75.0, 100.0, 100.0)], false), 0), None, "no weight axis");
        assert_eq!(regular_weight(photocraft_text::fonts::INTER_REGULAR.as_slice(), 0), None, "static font");
        for bad in [&thin[..thin.len() - 9], &thin[..30], &b"ttcf"[..], &[][..]] {
            assert_eq!(regular_weight(bad, 1), None);
        }
        // Registered as a fallback, a static face keeps the default (no variation) coordinates.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        add_to_all_families(&ctx, "test-static".into(), FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()));
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let fonts = ctx.fonts(|f| f.definitions().clone());
        assert_eq!(fonts.font_data.get("test-static").map(|d| d.tweak.coords.clone()), Some(Default::default()));
    }

    /// A corrupt or non-font blob has no `hhea`, `head` or `fvar` table: the fallback tweaks
    /// must leave it untouched rather than deriving a baseline or weight from garbage.
    #[test]
    fn font_tweaks_ignore_bytes_without_metrics_or_weight_axes() {
        let mut data = FontData::from_static(b"");
        apply_font_tweaks(&mut data);
        assert_eq!(data.tweak.y_offset_factor, 0.0);
        assert_eq!(data.tweak.coords, Default::default());
    }

    static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

    #[test]
    fn changing_language_resets_the_loader_once_and_reorders_scripts() {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            let ctx = egui::Context::default();
            crate::i18n::sync_context(&ctx, "ja");
            ctx.with_plugin::<CjkFontPlugin, _>(|plugin| {
                assert_eq!(plugin.0.order()[0], CjkScript::Japanese);
                plugin.0.tried = plugin.0.order().to_vec();
                plugin.0.last_resort_tried = true;
            })
            .expect("font plugin");
            crate::i18n::sync_context(&ctx, "ja");
            assert!(ctx.with_plugin::<CjkFontPlugin, _>(|plugin| plugin.0.exhausted()).expect("font plugin"), "idle frames must not reload fonts");
            crate::i18n::sync_context(&ctx, "ko");
            ctx.with_plugin::<CjkFontPlugin, _>(|plugin| {
                assert!(!plugin.0.exhausted());
                assert_eq!(plugin.0.order()[0], CjkScript::Korean);
            })
            .expect("font plugin");
        });
    }

    fn fake_dir() -> PathBuf {
        let mut g = DIR.lock().unwrap();
        g.get_or_insert_with(|| {
            let dir = std::env::temp_dir().join(format!("photocraft-cjk-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("font.ttf"), include_bytes!("../../../assets/fonts/Inter-Regular.ttf")).unwrap();
            std::fs::write(dir.join("empty.ttf"), b"").unwrap();
            std::fs::write(dir.join("thai.ttf"), synthetic_font(&[(0x0E01, 0x0E5B)])).unwrap();
            std::fs::write(dir.join("arabic.ttf"), synthetic_font(&[(0x0600, 0x06FF)])).unwrap();
            std::fs::write(dir.join("devanagari.ttf"), synthetic_font(&[(0x0900, 0x097F)])).unwrap();
            std::fs::write(dir.join("korean.ttf"), synthetic_font(&[(0xCE74, 0xCE74)])).unwrap();
            std::fs::write(dir.join("tamil.ttf"), synthetic_font(&[(0x0B80, 0x0BFF)])).unwrap();
            std::fs::write(dir.join("ethiopic.ttf"), synthetic_font(&[(0x1200, 0x137F)])).unwrap();
            std::fs::write(dir.join("hant.ttf"), synthetic_font(&[(0x9F8D, 0x9F8D)])).unwrap();
            std::fs::write(
                dir.join("families.ttc"),
                two_face_ttc(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"), include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf")),
            )
            .unwrap();
            dir
        })
        .clone()
    }

    fn fake(name: &str) -> FontFile {
        FontFile { path: fake_dir().join(name), family: String::new() }
    }

    #[test]
    fn font_reads_skip_missing_empty_oversized_and_non_file_candidates() {
        let dir = fake_dir();
        assert!(read_font(&dir.join("missing-read-test.ttf")).is_none(), "a missing path is skipped");
        assert!(read_font(&dir).is_none(), "a directory is not a font file");
        assert!(read_font(&dir.join("empty.ttf")).is_none(), "an empty file is skipped");

        let oversized = dir.join("oversized-read-test.ttf");
        let file = std::fs::File::create(&oversized).expect("create a sparse oversized test file");
        file.set_len(MAX_FONT_BYTES + 1).expect("set the sparse file size");
        drop(file);
        assert!(read_font(&oversized).is_none(), "an oversized file is not read");
        std::fs::remove_file(&oversized).expect("remove the sparse test file");

        let valid = dir.join("font.ttf");
        assert_eq!(read_font(&valid).expect("the valid font is read").as_slice(), std::fs::read(&valid).expect("test font bytes").as_slice());
    }

    fn provider_corrupt_thai_candidates(script: Script) -> Vec<PathBuf> {
        if script == Script::Thai {
            vec![
                fake_dir().join("provider-missing-thai.ttf"),
                fake_dir().join("provider-corrupt-thai.ttf"),
                fake_dir().join("provider-corrupt-selected-thai.ttc"),
                fake_dir().join("font.ttf"),
                fake_dir().join("thai.ttf"),
            ]
        } else {
            Vec::new()
        }
    }

    fn provider_missing_thai_candidate(script: Script) -> Vec<PathBuf> {
        if script == Script::Thai { vec![fake_dir().join("provider-only-missing-thai.ttf")] } else { Vec::new() }
    }

    /// The provider's non-CJK script route must reject a structurally invalid file, a readable
    /// collection whose selected covering face is unreadable, and a missing file, then use the
    /// next real candidate. The rejected and missing paths stay negative-cached.
    #[test]
    fn provider_skips_missing_and_corrupt_script_candidates_before_registering() {
        let malformed = fake_dir().join("provider-corrupt-thai.ttf");
        let collection = fake_dir().join("provider-corrupt-selected-thai.ttc");
        std::fs::write(&malformed, font_with_out_of_bounds_table()).expect("write the malformed test font");
        std::fs::write(&collection, two_face_ttc(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"), &font_with_out_of_bounds_table()))
            .expect("write the malformed selected-face collection");

        let sources = Sources {
            locale: || Some("en".into()),
            files: no_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: provider_corrupt_thai_candidates,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        };
        let provider = CraftFontProvider::new(&sources, 1.0);
        let insert = provided_insert(&provider, 'ก').expect("the valid Thai candidate is reached");
        assert_eq!(insert.data.index, 0);
        assert_eq!(insert.data.bytes(), std::fs::read(fake_dir().join("thai.ttf")).expect("Thai test font").as_slice());
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(cache.failed_reads.contains(&fake_dir().join("provider-missing-thai.ttf")), "the missing file is remembered");
        assert!(cache.invalid_fonts.contains(&malformed), "the readable non-font is remembered as invalid");
        assert!(cache.files.iter().any(|file| file.path == collection), "the readable collection is retained for its other faces");
        drop(cache);

        let missing_only = Sources { scripts: provider_missing_thai_candidate, ..sources };
        let provider = CraftFontProvider::new(&missing_only, 1.0);
        assert!(provided(&provider, 'ก').is_none(), "no readable script candidate leaves the character missing");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.failed_reads.len(), 1);
        assert_eq!(cache.reads, 0);
    }

    fn fake_sources(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            // Korean has a readable Hangul font, Japanese only broken files, Chinese none.
            files: |s| match s {
                CjkScript::Korean => vec![fake("missing.ttc"), fake("empty.ttf"), fake("korean.ttf")],
                CjkScript::Japanese => vec![fake("empty.ttf")],
                _ => vec![],
            },
            last_resort: || vec![fake("font.ttf")],
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        }
    }

    #[test]
    fn picks_preferred_script_skips_unreadable_and_exhausts() {
        let mut fb = CjkFallback::new(fake_sources(|| Some("en_US".into())));
        assert!(!fb.exhausted());
        // Hangul: Korean first; missing and empty files are skipped.
        let (name, data) = fb.next_font('카', CjkChar::Hangul).expect("korean font");
        assert!(name.starts_with(FONT_PREFIX));
        assert_eq!(data.index, 0);
        assert_eq!(fb.tried, vec![CjkScript::Korean]);
        // Han on an English locale: system fallback keeps the locale order (SC first); the
        // Japanese-first craft-manifest baseline is a separate provider/lazy-craft path.
        // SC, TC, JA have nothing readable here, and the last-resort file was already loaded,
        // so nothing new; everything is now tried.
        assert!(fb.next_font('日', CjkChar::Han).is_none());
        assert!(fb.exhausted());
        assert_eq!(fb.tried, vec![CjkScript::Korean, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Japanese]);
        assert_eq!(fb.registered.len(), 1, "a file is read at most once");
        assert!(fb.next_font('あ', CjkChar::Kana).is_none());
    }

    #[test]
    fn han_follows_locale() {
        for (loc, first) in [
            ("ja_JP.UTF-8", CjkScript::Japanese),
            ("zh-Hans-CN", CjkScript::SimplifiedChinese),
            ("zh-Hant-TW", CjkScript::TraditionalChinese),
            ("ko_KR", CjkScript::Korean),
        ] {
            let mut fb = CjkFallback::new(Sources {
                locale: || None,
                files: |_| vec![],
                last_resort: Vec::new,
                pending: || false,
                scripts: |_| Vec::new(),
                covers: scripts::covering_face,
                embedded: no_embedded,
                eager_craft_fonts: false,
            });
            fb.order = Some(cjk::script_order(Some(loc)));
            assert!(fb.next_font('日', CjkChar::Han).is_none());
            assert_eq!(fb.tried[0], first, "{loc}");
        }
    }

    /// Runs frames showing `text` until fonts settle; returns whether every glyph is covered.
    fn render(ctx: &egui::Context, text: &str) -> bool {
        for _ in 0..12 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                ui.label(text);
            });
            out.textures_delta.clear();
        }
        ctx.fonts_mut(|f| f.has_glyphs(&FontId::proportional(12.0), &text.replace(' ', "")))
    }

    /// A two-face TrueType collection built at runtime from the repository's OFL-licensed
    /// Inter and JetBrains Mono files. Each face directory is rewritten with offsets relative to
    /// the collection, so no font fixture is committed.
    fn two_face_ttc(first: &[u8], second: &[u8]) -> Vec<u8> {
        let font_count = 2usize;
        let tables = |font: &[u8]| u16::from_be_bytes([font[4], font[5]]) as usize;
        let dir_lengths = [12 + 16 * tables(first), 12 + 16 * tables(second)];
        let mut dir_offsets = Vec::with_capacity(font_count);
        let mut cursor = 12 + 4 * font_count;
        for length in dir_lengths {
            dir_offsets.push(cursor);
            cursor += length;
        }
        let data_start = cursor.next_multiple_of(4);
        let mut bases = Vec::with_capacity(font_count);
        let mut total = data_start;
        for font in [first, second] {
            bases.push(total);
            total += font.len().next_multiple_of(4);
        }
        let mut out = vec![0u8; total];
        out[..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        out[8..12].copy_from_slice(&u32::try_from(font_count).expect("two faces").to_be_bytes());
        for (i, offset) in dir_offsets.iter().enumerate() {
            let at = 12 + i * 4;
            out[at..at + 4].copy_from_slice(&u32::try_from(*offset).expect("collection fits").to_be_bytes());
        }
        for (i, font) in [first, second].into_iter().enumerate() {
            let dir = dir_offsets[i];
            let base = bases[i];
            out[dir..dir + 12].copy_from_slice(&font[..12]);
            for record in 0..tables(font) {
                let src = 12 + 16 * record;
                let dst = dir + 12 + 16 * record;
                out[dst..dst + 8].copy_from_slice(&font[src..src + 8]);
                let table_offset = u32::from_be_bytes(font[src + 8..src + 12].try_into().expect("four bytes"));
                let collection_offset = u32::try_from(base).expect("collection fits") + table_offset;
                out[dst + 8..dst + 12].copy_from_slice(&collection_offset.to_be_bytes());
                out[dst + 12..dst + 16].copy_from_slice(&font[src + 12..src + 16]);
            }
            out[base..base + font.len()].copy_from_slice(font);
        }
        out
    }

    /// A minimal TrueType font mapping the `ranges` (inclusive) to box glyphs, so tests have a
    /// font for a script without committing font files.
    fn synthetic_font(ranges: &[(u32, u32)]) -> Vec<u8> {
        let be16 = |v: &mut Vec<u8>, x: i32| v.extend((x as u16).to_be_bytes());
        let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
        let chars: u32 = ranges.iter().map(|(a, b)| b - a + 1).sum();
        let glyphs = chars as i32 + 1;
        // One 500x700 box (contour of four on-curve points); glyph 0 (.notdef) is empty.
        let mut boxed = Vec::new();
        for x in [1, 50, 0, 550, 700, 3, 0] {
            be16(&mut boxed, x);
        }
        boxed.extend([1u8; 4]);
        for x in [50, 0, 500, 0, 0, 700, 0, -700] {
            be16(&mut boxed, x);
        }
        boxed.extend([0, 0]);
        let glyf: Vec<u8> = boxed.repeat(chars as usize);
        let mut loca = Vec::new();
        be32(&mut loca, 0);
        for i in 0..chars {
            be32(&mut loca, i * boxed.len() as u32);
        }
        be32(&mut loca, chars * boxed.len() as u32);
        let mut head = Vec::new();
        for x in [0x0001_0000, 0x0001_0000, 0, 0x5F0F_3CF5] {
            be32(&mut head, x);
        }
        for x in [0x000B, 1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, -200, 600, 800, 0, 8, 2, 1, 0] {
            be16(&mut head, x);
        }
        let mut hhea = Vec::new();
        be32(&mut hhea, 0x0001_0000);
        for x in [800, -200, 0, 600, 0, 0, 550, 1, 0, 0, 0, 0, 0, 0, 0, glyphs] {
            be16(&mut hhea, x);
        }
        let mut maxp = Vec::new();
        be32(&mut maxp, 0x0001_0000);
        for x in [glyphs, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
            be16(&mut maxp, x);
        }
        let mut hmtx = Vec::new();
        for _ in 0..glyphs {
            be16(&mut hmtx, 600);
            be16(&mut hmtx, 50);
        }
        let mut cmap = Vec::new();
        for x in [0, 1, 3, 10] {
            be16(&mut cmap, x);
        }
        be32(&mut cmap, 12);
        be16(&mut cmap, 12);
        be16(&mut cmap, 0);
        be32(&mut cmap, 16 + 12 * ranges.len() as u32);
        be32(&mut cmap, 0);
        be32(&mut cmap, ranges.len() as u32);
        let mut gid = 1;
        for (a, b) in ranges {
            for x in [*a, *b, gid] {
                be32(&mut cmap, x);
            }
            gid += b - a + 1;
        }
        let tables: [(&[u8; 4], Vec<u8>); 7] =
            [(b"cmap", cmap), (b"glyf", glyf), (b"head", head), (b"hhea", hhea), (b"hmtx", hmtx), (b"loca", loca), (b"maxp", maxp)];
        let mut font = Vec::new();
        be32(&mut font, 0x0001_0000);
        for x in [7, 64, 2, 48] {
            be16(&mut font, x);
        }
        let mut offset = 12 + 16 * tables.len() as u32;
        for (tag, data) in &tables {
            font.extend(*tag);
            be32(&mut font, 0);
            be32(&mut font, offset);
            be32(&mut font, data.len() as u32);
            offset += (data.len() as u32).next_multiple_of(4);
        }
        for (_, data) in &tables {
            font.extend(data);
            font.resize(font.len().next_multiple_of(4), 0);
        }
        font
    }

    /// Thai, Arabic and Devanagari each have a synthetic font; Thai's list starts with unreadable
    /// files and Inter, which doesn't map Thai. No other script has a font.
    fn script_sources() -> Sources {
        Sources {
            locale: || Some("en".into()),
            files: |_| vec![],
            last_resort: Vec::new,
            pending: || false,
            scripts: |s| {
                let f = |name: &str| fake_dir().join(name);
                match s {
                    Script::Thai => vec![f("missing.ttf"), f("empty.ttf"), f("font.ttf"), f("thai.ttf")],
                    Script::Arabic => vec![f("arabic.ttf")],
                    Script::Devanagari => vec![f("devanagari.ttf")],
                    _ => vec![],
                }
            },
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        }
    }

    fn fallback_files(ctx: &egui::Context) -> Vec<String> {
        ctx.with_plugin::<CjkFontPlugin, _>(|p| p.0.registered.iter().map(|(_, path, _)| path.file_name().unwrap().to_string_lossy().into_owned()).collect())
            .unwrap()
    }

    #[test]
    fn layer_names_in_other_scripts_load_a_covering_font_once() {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            crate::theme::install_fonts_with(&ctx, script_sources());
            assert!(render(&ctx, "Layer 1 – café"));
            ctx.with_plugin::<CjkFontPlugin, _>(|p| assert!(p.0.scripts_tried.is_empty(), "Latin text reads no font")).unwrap();
            assert!(fallback_files(&ctx).is_empty());
            // A Thai type layer's name (#2275), Arabic (#1693) and Devanagari: no missing glyphs.
            for text in ["สวัสดี", "ภาพถ่าย/วันหยุดพักผ่อน", "مرحبا بالعالم", "नमस्ते दुनिया"]
            {
                assert!(render(&ctx, text), "tofu in {text}");
            }
            assert_eq!(fallback_files(&ctx), ["thai.ttf", "arabic.ttf", "devanagari.ttf"], "the first file that maps the character, never Inter");
            // Hebrew has no font here: tried once, then left alone (no reload or repaint loop).
            assert!(!render(&ctx, "שלום"));
            assert!(!render(&ctx, "שלום"));
            ctx.with_plugin::<CjkFontPlugin, _>(|p| {
                assert!(p.0.scripts_tried.contains(&Script::Hebrew));
                assert_eq!(p.0.registered.len(), 3);
            })
            .unwrap();
            let fonts = ctx.fonts(|f| f.definitions().clone());
            assert_eq!(fonts.families[&FontFamily::Proportional].first().map(String::as_str), Some("Inter"));
            for stack in fonts.families.values() {
                assert_eq!(stack.iter().filter(|n| n.starts_with(FONT_PREFIX)).count(), 3);
                assert_eq!(stack.last().map(String::as_str), Some("system-cjk-2"), "appended last so Latin keeps Inter");
            }
        }
    }

    #[test]
    fn a_missing_cjk_font_does_not_block_other_scripts_and_vice_versa() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(&ctx, script_sources());
        assert!(!render(&ctx, "日本語/ภาพถ่าย/วันหยุดพักผ่อน.png"));
        assert!(render(&ctx, "ภาพถ่าย/วันหยุดพักผ่อน.png"));
        ctx.with_plugin::<CjkFontPlugin, _>(|p| assert!(p.0.exhausted())).unwrap();
        assert_eq!(fallback_files(&ctx), ["thai.ttf"]);
        // A script with no font leaves CJK available.
        let mut fallback = CjkFallback::new(fake_sources(|| Some("ko".into())));
        assert!(fallback.next_script_font('ส').is_none());
        assert!(fallback.next_script_font('ส').is_none());
        assert!(fallback.next_script_font('A').is_none(), "Latin is never a fallback script");
        assert_eq!(fallback.scripts_tried, [Script::Thai]);
        assert!(fallback.next_font('카', CjkChar::Hangul).is_some());
    }

    #[test]
    fn script_candidates_are_read_a_bounded_number_of_times() {
        static CHECKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let mut fallback = CjkFallback::new(Sources {
            scripts: |_| (0..20).map(|i| fake_dir().join(if i % 2 == 0 { "font.ttf" } else { "thai.ttf" })).collect(),
            covers: |_, _| {
                CHECKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                None
            },
            ..script_sources()
        });
        assert!(fallback.next_script_font('ش').is_none());
        assert!(fallback.next_script_font('ش').is_none());
        assert_eq!(CHECKED.load(std::sync::atomic::Ordering::Relaxed), MAX_SCRIPT_READS, "one bounded pass per script");
        assert!(fallback.registered.is_empty());
        assert!(!fallback.scripts_exhausted());
    }

    #[test]
    fn system_fonts_cover_other_script_samples() {
        // Like the CJK system-font test, run per script when this machine has a font for it.
        // The deterministic tests above do not need installed fonts.
        let samples = [("ภาพถ่าย/วันหยุดพักผ่อน", 'ภ'), ("مرحبا", 'م'), ("שלום", 'ש'), ("नमस्ते", 'न')];
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (text, c) in samples {
            let Some(script) = scripts::classify(c) else { continue };
            if !scripts::system_candidates(script).iter().any(|p| read_font(p).and_then(|b| scripts::covering_face(&b, c)).is_some()) {
                eprintln!("skipping {text}: no {script:?} system font");
                continue;
            }
            assert!(render(&ctx, text), "{text} still has missing glyphs");
        }
    }

    #[test]
    fn vertical_metrics_parse_and_reject_garbage() {
        let (asc, desc, gap) = vertical_metrics(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"), 0).unwrap();
        assert!((asc - 1984.0 / 2048.0).abs() < 1e-4 && (desc + 494.0 / 2048.0).abs() < 1e-4 && gap == 0.0);
        for bad in [&b""[..], b"ttcf", b"ttcf\0\0\0\0\0\0\0\0\xff\xff\xff\xff", b"\0\x01\0\0\0\xff\0\0\0\0\0\0"] {
            assert!(vertical_metrics(bad, 0).is_none());
            assert!(vertical_metrics(bad, u32::MAX).is_none());
        }
    }

    #[test]
    fn a_face_with_a_big_line_gap_is_shifted_down_onto_the_primary_baseline() {
        let inter = (1984.0 / 2048.0, -494.0 / 2048.0, 0.0);
        let hiragino = (0.88, -0.12, 0.5);
        assert!((baseline_offset(hiragino, inter) - 0.2338).abs() < 1e-3);
        assert_eq!(baseline_offset(inter, inter), 0.0);
        assert_eq!(baseline_offset((f32::NAN, 0.0, 0.0), inter), 0.0);
        let jb = (1.02, -0.3, 0.0);
        assert!((baseline_offset(hiragino, jb) - baseline_offset(hiragino, inter)).abs() < 0.005, "one offset serves the monospace family too");
    }

    #[test]
    fn lazy_registration_triggers_only_for_supported_scripts() {
        let ctx = egui::Context::default();
        install_with(&ctx, fake_sources(|| Some("ko".into())));
        let _ = render(&ctx, "Layer 1 – café");
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 0, "Latin text loads nothing");
        // The Korean candidate covers the first missing Hangul character and is registered.
        let _ = render(&ctx, "카드 배경");
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let name = format!("{FONT_PREFIX}-0");
        assert!(fonts.font_data.contains_key(&name));
        for (fam, stack) in &fonts.families {
            assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
        }
    }

    /// Craft fonts available through the first-frame provider, as the desktop shell does.
    fn eager_sources(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            files: |_| vec![],
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: true,
        }
    }

    /// No system fonts at all: craft-fonts alone draws CJK through the lazy loader.
    fn craft_only(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            files: |_| vec![],
            last_resort: Vec::new,
            pending: || false,
            scripts: |_| Vec::new(),
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: false,
        }
    }

    /// The migration baseline's first craft face covering `c`: the first non-empty face in
    /// manifest order (what the old family stack appended first), not the UI-first sort.
    fn baseline_face(c: char) -> Option<&'static CraftFont> {
        craft_fonts::CRAFT_FONTS.iter().find(|f| !f.bytes.is_empty() && scripts::face_covers(f.bytes, 0, c))
    }

    /// A test-only craft face backed by a collection built with [`two_face_ttc`]: face 0 maps
    /// Thai only, face 1 maps 龍. `craft_insert` would install face 0, so the craft coverage
    /// helpers must not accept it for 龍 even though a later face covers the character.
    fn ttc_craft_face() -> &'static CraftFont {
        static FACE: std::sync::OnceLock<CraftFont> = std::sync::OnceLock::new();
        FACE.get_or_init(|| {
            let bytes: &'static [u8] = Box::leak(two_face_ttc(&synthetic_font(&[(0x0E01, 0x0E5B)]), &synthetic_font(&[(0x9F8D, 0x9F8D)])).into_boxed_slice());
            CraftFont { family: "Synthetic Collection", style: "Regular", scripts: &["Hans"], bytes }
        })
    }

    /// The craft helpers must test face 0, the face `FontData::from_static` installs. The old
    /// any-face coverage check returned the face for a character only a later collection face
    /// maps, which then drew tofu in the installed face 0.
    #[test]
    fn craft_face_coverage_matches_the_face_craft_insert_installs() {
        let face = ttc_craft_face();
        // Precondition: an any-face check really accepts 龍 (face 1), so this regression would
        // fail if the helpers went back to `scripts::covering_face`.
        assert_eq!(scripts::covering_face(face.bytes, '龍'), Some(1), "face 1 maps 龍");
        assert!(!scripts::face_covers(face.bytes, 0, '龍'), "the installed face 0 does not map 龍");
        assert!(!craft_face_installs_coverage(face, '龍'), "face 0 lacks the glyph; craft_insert installs face 0");
        assert!(craft_face_installs_coverage(face, 'ก'), "face 0 maps Thai");
    }

    static EMPTY_CRAFT_FACE: CraftFont = CraftFont { family: "Empty", style: "Regular", scripts: &["Zsym"], bytes: &[] };

    fn only_empty_craft_face(_: &'static str) -> Vec<&'static CraftFont> {
        vec![&EMPTY_CRAFT_FACE]
    }

    /// A craft entry without bytes, or a face injected only by a test that is absent from the
    /// global manifest, has no installable face index: selection must return `None`, not a face
    /// name egui cannot resolve.
    #[test]
    fn craft_face_selection_ignores_empty_and_unindexed_faces() {
        assert!(!craft_face_installs_coverage(&EMPTY_CRAFT_FACE, '☃'));
        assert!(craft_face_for(only_empty_craft_face, "Zsym", '☃').is_none());
        assert!(craft_face_manifest(only_empty_craft_face, "Zsym", '☃').is_none());
    }

    /// Every script the lazy fallback can see has a valid ISO 15924 tag, so its injected craft
    /// faces and the CJK scripts are selected by the same tags the manifest declares.
    #[test]
    fn fallback_scripts_map_to_iso_15924_tags() {
        assert_eq!(cjk_iso(CjkScript::Japanese), "Jpan");
        assert_eq!(cjk_iso(CjkScript::SimplifiedChinese), "Hans");
        assert_eq!(cjk_iso(CjkScript::TraditionalChinese), "Hant");
        assert_eq!(cjk_iso(CjkScript::Korean), "Kore");
        let expected = [
            (Script::Thai, "Thai"),
            (Script::Lao, "Laoo"),
            (Script::Khmer, "Khmr"),
            (Script::Myanmar, "Mymr"),
            (Script::Tibetan, "Tibt"),
            (Script::Arabic, "Arab"),
            (Script::Hebrew, "Hebr"),
            (Script::Armenian, "Armn"),
            (Script::Georgian, "Geor"),
            (Script::Ethiopic, "Ethi"),
            (Script::Devanagari, "Deva"),
            (Script::Bengali, "Beng"),
            (Script::Gurmukhi, "Guru"),
            (Script::Gujarati, "Gujr"),
            (Script::Oriya, "Orya"),
            (Script::Tamil, "Taml"),
            (Script::Telugu, "Telu"),
            (Script::Kannada, "Knda"),
            (Script::Malayalam, "Mlym"),
            (Script::Sinhala, "Sinh"),
        ];
        assert_eq!(expected.len(), Script::ALL.len(), "the test lists every fallback script");
        for (script, iso) in expected {
            assert_eq!(script_iso(script), iso, "{script:?}");
        }
        assert_eq!(craft_embedded(CjkScript::Japanese).len(), craft_embedded_iso("Jpan").len());
    }

    /// No system fonts at all: craft-fonts alone draws CJK, in the locale's script order.
    #[test]
    fn craft_fonts_render_cjk_without_system_fonts() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        let Some(chinese) = craft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans CJK SC" && f.style == "Regular") else {
            eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
            return;
        };
        // The craft manifest's Japanese faces come before Noto Sans CJK SC and the baseline
        // appended them in manifest order, so a non-CJK/unknown effective locale keeps Shippori
        // Mincho (the first covering face). An explicit locale keeps its own UI-first script face.
        let Some(baseline) = baseline_face('日') else {
            eprintln!("skipping: the craft-fonts manifest has no face covering 日");
            return;
        };
        for (locale, expected) in [
            ((|| Some("en_US".into())) as fn() -> Option<String>, baseline),
            ((|| Some("ja_JP".into())) as fn() -> Option<String>, ui_font),
            ((|| Some("zh-CN".into())) as fn() -> Option<String>, chinese),
            ((|| None) as fn() -> Option<String>, baseline),
        ] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts_with(&ctx, craft_only(locale));
            assert!(render(&ctx, "日本語の文字"), "tofu in 日本語の文字");
            assert!(render(&ctx, "レイヤー 1"), "tofu in kana");
            let fonts = ctx.fonts(|f| f.definitions().clone());
            let name = format!("{FONT_PREFIX}-0");
            assert!(
                fonts.font_data.get(&name).is_some_and(|d| std::ptr::eq(d.bytes().as_ptr(), expected.bytes.as_ptr())),
                "{} first by locale: {name}",
                expected.family
            );
            for (fam, stack) in &fonts.families {
                assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
                assert!(stack.first().is_some_and(|f| f != &name), "{fam:?}");
            }
        }
    }

    fn provided_insert(provider: &CraftFontProvider, c: char) -> Option<FontInsert> {
        let mut buf = [0u8; 4];
        let cluster = c.encode_utf8(&mut buf);
        provider.font_for(&FallbackRequest { cluster, family: &FontFamily::Proportional })
    }

    fn provided(provider: &CraftFontProvider, c: char) -> Option<String> {
        provided_insert(provider, c).map(|insert| insert.name)
    }

    fn test_sources(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            files: no_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: false,
        }
    }

    fn test_provider(locale: fn() -> Option<String>) -> CraftFontProvider {
        CraftFontProvider::new(&test_sources(locale), 1.0)
    }

    #[test]
    fn effective_locale_prefers_the_translated_ui_then_the_os_locale() {
        assert_eq!(effective_locale("ja", Some("en_US.UTF-8")), Some("ja".into()));
        assert_eq!(effective_locale("zh-hant", Some("ja_JP.UTF-8")), Some("zh-hant".into()));
        assert_eq!(effective_locale("en", Some("ja_JP.UTF-8")), Some("ja_JP.UTF-8".into()), "English UI on a Japanese OS uses the OS locale");
        assert_eq!(effective_locale("de", Some("ko_KR")), Some("ko_KR".into()));
        assert_eq!(effective_locale("en", None), None, "no UI preference and no OS locale stays unknown");
    }

    fn no_script_files(_: Script) -> Vec<PathBuf> {
        Vec::new()
    }

    fn no_files(_: CjkScript) -> Vec<FontFile> {
        Vec::new()
    }

    /// A stand-in system Traditional Chinese file, so the system-before-SC-craft ordering is
    /// testable without depending on the host's fonts. Its content is the SC craft face (which
    /// covers the sample Han character).
    fn hant_system_file() -> Option<PathBuf> {
        static FILE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        let face = craft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans CJK SC" && !f.bytes.is_empty())?;
        FILE.get_or_init(|| {
            let path = std::env::temp_dir().join(format!("photocraft-hant-{}.otf", std::process::id()));
            std::fs::write(&path, face.bytes).ok().map(|()| path)
        })
        .clone()
    }

    fn hant_files(script: CjkScript) -> Vec<FontFile> {
        if script == CjkScript::TraditionalChinese {
            hant_system_file().map(|path| vec![FontFile { path, family: String::new() }]).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    #[test]
    fn craft_font_provider_keeps_the_baseline_face_for_unknown_locales() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        let Some(ui_index) = craft_index(ui_font) else { return };
        let Some((sc_index, sc)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
            eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
            return;
        };
        let Some((korean_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "NanumGothic") else {
            eprintln!("skipping: the craft-fonts manifest has no Korean face");
            return;
        };
        let Some((arabic_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Arabic") else {
            eprintln!("skipping: the craft-fonts manifest has no Arabic face");
            return;
        };
        let Some((devanagari_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Devanagari") else {
            eprintln!("skipping: the craft-fonts manifest has no Devanagari face");
            return;
        };
        let Some(baseline_han) = baseline_face('日') else {
            eprintln!("skipping: the craft-fonts manifest has no face covering 日");
            return;
        };
        let Some(baseline_kana) = baseline_face('あ') else {
            eprintln!("skipping: the craft-fonts manifest has no face covering あ");
            return;
        };
        let baseline_han_index = craft_index(baseline_han).expect("a manifest face has an index");
        let baseline_kana_index = craft_index(baseline_kana).expect("a manifest face has an index");
        let baseline_han_name = format!("craft-font-{baseline_han_index}");
        let baseline_kana_name = format!("craft-font-{baseline_kana_index}");
        let ui_name = format!("craft-font-{ui_index}");
        let sc_name = format!("craft-font-{sc_index}");
        let korean_name = format!("craft-font-{korean_index}");
        let arabic_name = format!("craft-font-{arabic_index}");
        let devanagari_name = format!("craft-font-{devanagari_index}");

        let ja = test_provider((|| Some("ja_JP".into())) as fn() -> Option<String>);
        assert_eq!(provided(&ja, 'あ').as_deref(), Some(ui_name.as_str()), "explicit ja keeps the Japanese UI face");
        assert_eq!(provided(&ja, '日').as_deref(), Some(ui_name.as_str()), "Han follows an explicit Japanese locale");

        let en = test_provider((|| Some("en_US".into())) as fn() -> Option<String>);
        assert_eq!(
            provided(&en, 'あ').as_deref(),
            Some(baseline_kana_name.as_str()),
            "kana in a non-CJK effective locale keeps the manifest's first covering face"
        );
        assert_eq!(provided(&en, '한').as_deref(), Some(korean_name.as_str()), "Hangul in a non-CJK effective locale stays on the Korean face");
        assert_eq!(
            provided(&en, '日').as_deref(),
            Some(baseline_han_name.as_str()),
            "Han in a non-CJK effective locale keeps the manifest's first covering face"
        );
        let unknown = test_provider((|| None) as fn() -> Option<String>);
        assert_eq!(provided(&unknown, '日').as_deref(), Some(baseline_han_name.as_str()), "an unknown effective locale has the same baseline face");
        let english_on_japanese_os = test_provider((|| effective_locale("en", Some("ja_JP"))) as fn() -> Option<String>);
        assert_eq!(
            provided(&english_on_japanese_os, '日').as_deref(),
            Some(ui_name.as_str()),
            "English UI on a Japanese OS follows the OS locale; the manifest baseline is not universal for English UI"
        );
        assert_eq!(provided(&en, 'م').as_deref(), Some(arabic_name.as_str()), "Arabic uses its own craft face");
        assert_eq!(provided(&en, 'न').as_deref(), Some(devanagari_name.as_str()), "Devanagari uses its own craft face");

        let hans = test_provider((|| Some("zh-CN".into())) as fn() -> Option<String>);
        assert_eq!(provided(&hans, '日').as_deref(), Some(sc_name.as_str()));
        assert_eq!(provided(&hans, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Chinese UI keeps the Japanese face");

        let hant = test_provider((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>);
        assert_eq!(provided(&hant, '龍').as_deref(), Some(sc_name.as_str()), "with no system Hant face, the locale order falls through to SC");
        assert_eq!(provided(&hant, '\u{3105}').as_deref(), Some(sc_name.as_str()), "Bopomofo follows the same locale-order fallback");
        assert_eq!(provided(&hant, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Hant UI keeps the Japanese face");

        // A system Hant file, when present, is read before the Simplified Chinese craft fallback.
        if hant_system_file().is_some() {
            let sources = Sources {
                locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
                files: hant_files,
                last_resort: Vec::new,
                pending: || false,
                scripts: no_script_files,
                covers: scripts::covering_face,
                embedded: craft_embedded_iso,
                eager_craft_fonts: false,
            };
            let hant = CraftFontProvider::new(&sources, 1.0);
            let got = provided(&hant, '龍').expect("the system Hant face");
            assert!(got.starts_with("craft-system-"), "the system Hant face comes before the SC craft fallback: {got}");
            assert_ne!(got, sc_name, "the SC craft face must not pre-empt a system Hant face");
        }

        let ko = test_provider((|| Some("ko_KR".into())) as fn() -> Option<String>);
        assert_eq!(provided(&ko, '한').as_deref(), Some(korean_name.as_str()));
        assert_eq!(provided(&ko, 'あ').as_deref(), Some(ui_name.as_str()), "kana in a Korean UI keeps the Japanese face");
        assert_ne!(sc.family, ui_font.family);
    }

    static PENDING_LAST_RESORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static PENDING_CRAFT_SC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static PENDING_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static STRICT_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

    fn pending_last_resort() -> bool {
        PENDING_LAST_RESORT.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn pending_craft_sc() -> bool {
        PENDING_CRAFT_SC.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn pending_ready() -> bool {
        !PENDING_READY.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn strict_pending() -> bool {
        STRICT_PENDING.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn ready_files(script: CjkScript) -> Vec<FontFile> {
        if script == CjkScript::TraditionalChinese && PENDING_READY.load(std::sync::atomic::Ordering::Relaxed) { vec![fake("hant.ttf")] } else { Vec::new() }
    }

    fn ready_hant_files(script: CjkScript) -> Vec<FontFile> {
        if script == CjkScript::TraditionalChinese { vec![fake("hant.ttf")] } else { Vec::new() }
    }

    #[test]
    fn provider_uses_the_requested_collection_family_and_its_exact_coverage() {
        let path = fake_dir().join("families.ttc");
        let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
        let file = |family: &str| vec![FontFile { path: path.clone(), family: family.into() }];

        let insert = provider.system_insert_files(file("JetBrains Mono"), 'A').expect("face 1 covers A");
        assert_eq!(insert.data.index, 1, "the requested family selects face 1, not the first covering face 0");
        assert!(
            provider.system_insert_files(file("JetBrains Mono"), 'ƀ').is_none(),
            "JetBrains Mono does not cover the character; face 0 of Inter must not be used"
        );

        let empty_family = vec![FontFile { path: path.clone(), family: String::new() }];
        let insert = provider.system_insert_files(empty_family, 'ƀ').expect("an empty family is face 0");
        assert_eq!(insert.data.index, 0);
    }

    #[test]
    fn cached_system_face_returns_the_current_scale() {
        let path = fake_dir().join("families.ttc");
        let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
        let file = || vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }];

        let first = provider.system_insert_files(file(), 'A').expect("face 1 covers A");
        assert_eq!(first.data.index, 1);
        assert_eq!(first.data.tweak.scale, 1.0);
        provider.set_scale(4.0 / 3.0);
        let second = provider.system_insert_files(file(), 'A').expect("cached face");
        assert_eq!(second.data.index, 1);
        assert_eq!(second.data.tweak.scale, 4.0 / 3.0, "a cached face must not bake in the old UI scale");
    }

    #[test]
    fn provider_honours_the_injected_embedded_faces() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        if craft_index(ui_font).is_none() {
            return;
        }
        let Some(baseline) = baseline_face('あ') else { return };
        let baseline_name = format!("craft-font-{}", craft_index(baseline).expect("a manifest face has an index"));
        let with_craft = test_provider((|| Some("en_US".into())) as fn() -> Option<String>);
        assert_eq!(provided(&with_craft, 'あ').as_deref(), Some(baseline_name.as_str()));

        let no_craft = Sources { embedded: no_embedded, ..test_sources((|| Some("en_US".into())) as fn() -> Option<String>) };
        let provider = CraftFontProvider::new(&no_craft, 1.0);
        assert!(provided(&provider, 'あ').is_none(), "the provider must use Sources::embedded, not the global craft table");
    }

    #[test]
    fn provider_reaches_a_fontconfig_match_after_six_missing_candidates() {
        let mut files: Vec<FontFile> =
            (0..MAX_SCRIPT_READS).map(|i| FontFile { path: fake_dir().join(format!("missing-{i}.ttf")), family: String::new() }).collect();
        files.push(fake("hant.ttf"));
        let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
        let insert = provider.system_insert_files(files, '龍').expect("the seventh (Fontconfig) candidate must be reachable");
        assert!(insert.name.starts_with("craft-system-"));
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.reads, 1, "only the real file counts as a read; absent paths do not");
        assert_eq!(cache.failed_reads.len(), MAX_SCRIPT_READS);
    }

    #[test]
    fn provider_caches_failed_reads_and_face_lookups_across_characters() {
        let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        let missing = fake("missing-cache.ttf");
        assert!(provider.system_insert_files(vec![missing.clone()], '日').is_none());
        assert!(provider.system_insert_files(vec![missing.clone()], '本').is_none());
        {
            let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(cache.failed_reads.len(), 1, "a missing path is remembered");
            assert_eq!(cache.reads, 0, "a missing path performs no font read");
        }

        // Inter is readable but does not cover either Han character: its bytes are read once.
        let inter = fake("font.ttf");
        assert!(provider.system_insert_files(vec![inter.clone()], '日').is_none());
        assert!(provider.system_insert_files(vec![inter.clone()], '本').is_none());
        {
            let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(cache.reads, 1, "the second character reuses the cached bytes");
            assert_eq!(cache.files.len(), 1);
        }

        // A collection's family lookup is parsed once even when it misses every character.
        let path = fake_dir().join("families.ttc");
        let file = vec![FontFile { path, family: "Noto Sans CJK TC".into() }];
        assert!(provider.system_insert_files(file.clone(), '日').is_none());
        assert!(provider.system_insert_files(file, '本').is_none());
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.family_indices.len(), 1, "a family miss is cached");
        assert_eq!(cache.family_indices[0].2, None);
    }

    static TTC_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counted_ttc_read(path: &std::path::Path) -> Option<Vec<u8>> {
        TTC_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::fs::read(path).ok()
    }

    /// A collection's two faces share one blob: selecting both faces reads the file once and
    /// charges its bytes once, not once per face.
    #[test]
    fn provider_charges_a_ttc_blob_once_for_multiple_selected_faces() {
        let path = fake_dir().join("families.ttc");
        let size = std::fs::metadata(&path).expect("test collection").len() as usize;
        let mut provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        provider.reader = counted_ttc_read;
        TTC_READS.store(0, std::sync::atomic::Ordering::Relaxed);
        let first = provider.system_insert_files(vec![FontFile { path: path.clone(), family: "Inter".into() }], 'A').expect("face 0 covers A");
        let second = provider.system_insert_files(vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }], 'A').expect("face 1 covers A");
        assert_eq!((first.data.index, second.data.index), (0, 1));
        assert!(std::sync::Arc::ptr_eq(&first.data.font, &second.data.font), "both faces share the collection's blob");
        assert_eq!(TTC_READS.load(std::sync::atomic::Ordering::Relaxed), 1, "the collection is read once");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.files.len(), 1, "one retained blob serves both faces");
        assert_eq!(cache.retained_bytes, size, "the TTC blob is charged once");
        assert_eq!(cache.faces.len(), 2, "both selected faces keep their own name");
    }

    /// Selecting a second face under a one-blob budget evicts the earlier blob from the
    /// provider, but the earlier `FontInsert` remains valid because it owns its shared blob.
    #[test]
    fn provider_evicts_an_earlier_selected_blob_under_budget_pressure() {
        let first_path = fake("font.ttf").path;
        let bytes = std::fs::read(&first_path).expect("test font");
        let size = bytes.len();
        let second_path = fake_dir().join("budget-selected-second.ttf");
        std::fs::write(&second_path, &bytes).expect("write a second test font");
        let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        {
            let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.max_bytes = size;
        }
        let first = provider.system_insert_files(vec![FontFile { path: first_path.clone(), family: String::new() }], 'A').expect("the first selected face");
        let second = provider.system_insert_files(vec![FontFile { path: second_path.clone(), family: String::new() }], 'A').expect("the second selected face");
        assert_ne!(first.name, second.name);
        assert_eq!(first.data.bytes(), bytes.as_slice(), "evicting the provider's blob does not invalidate egui's copy");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.files.len(), 1, "only one blob fits");
        assert_eq!(cache.files[0].path, second_path, "the newest selected blob is retained");
        assert_eq!(cache.retained_bytes, size, "the provider cache stays within its one-blob budget");
        assert_eq!(cache.faces.len(), 2, "the earlier face name remains reusable metadata, not bytes");
    }

    /// A selected face whose blob does not fit the budget is still returned complete for this
    /// request. egui may keep it, but the provider retains neither its bytes nor a second copy.
    #[test]
    fn provider_returns_a_transient_valid_face_when_the_blob_cannot_be_retained() {
        let path = fake("font.ttf").path;
        let size = std::fs::metadata(&path).expect("test font").len() as usize;
        let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        {
            let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.max_bytes = size.saturating_sub(1);
        }
        let insert = provider
            .system_insert_files(vec![FontFile { path: path.clone(), family: String::new() }], 'A')
            .expect("the over-budget face is returned transiently");
        assert_eq!(insert.data.index, 0);
        assert_eq!(insert.data.bytes().len(), size, "the transient face carries the whole valid font");
        assert_eq!(insert.data.bytes(), std::fs::read(&path).expect("test font bytes").as_slice());
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.retained_bytes, 0, "an over-budget face retains no provider bytes");
        assert!(cache.files.is_empty(), "the over-budget blob is not retained");
        assert!(cache.retained_bytes <= cache.max_bytes, "the provider cache stays within its budget");
        assert_eq!(cache.reads, 1);
    }

    /// A valid font file whose character map does not include the requested character is still a
    /// valid font: its bytes are cached within the budget and it is not marked invalid.
    #[test]
    fn provider_cache_evicts_retained_bytes_within_the_budget() {
        let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
        let size = bytes.len();
        assert!(size > 0);
        {
            let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.max_bytes = size.saturating_mul(2);
        }
        let mut paths = Vec::new();
        for i in 0..5 {
            let path = fake_dir().join(format!("budget-{i}.ttf"));
            std::fs::write(&path, &bytes).expect("write a synthetic test font");
            paths.push(path);
        }
        for path in &paths {
            let file = vec![FontFile { path: path.clone(), family: String::new() }];
            assert!(provider.system_insert_files(file, '日').is_none(), "the Thai-only face does not cover 日");
        }
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(cache.retained_bytes <= cache.max_bytes, "retained {} > budget {}", cache.retained_bytes, cache.max_bytes);
        assert!(cache.files.len() <= 2, "only two files fit the budget");
        assert!(cache.files.iter().any(|file| file.path == paths[4]), "the newest retained file stays");
        assert!(!cache.files.iter().any(|file| file.path == paths[0]), "the oldest retained file was evicted");
        assert_eq!(cache.reads, 5, "each distinct file was read once");
        assert!(cache.invalid_fonts.is_empty(), "a non-covering face is not an invalid font");
    }

    /// A later covering candidate (such as the Fontconfig match appended last) is reached even
    /// when a tiny budget evicts every retained candidate before it.
    #[test]
    fn provider_reaches_a_later_covering_candidate_under_budget_pressure() {
        let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
        let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
        {
            let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.max_bytes = bytes.len();
        }
        let mut files = Vec::new();
        for i in 0..12 {
            let path = fake_dir().join(format!("budget-later-{i}.ttf"));
            std::fs::write(&path, &bytes).expect("write a synthetic test font");
            files.push(FontFile { path, family: String::new() });
        }
        let covering = fake("hant.ttf").path;
        files.push(FontFile { path: covering.clone(), family: String::new() });
        let insert = provider.system_insert_files(files, '龍').expect("the later covering candidate must be reached");
        assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(cache.faces.iter().any(|face| face.path == covering), "the covering file became a selected face");
        assert!(cache.reads >= 13, "all retained candidates plus the matching file were read");
    }

    /// The old lifetime read cap could exhaust before a Fontconfig match appended after many
    /// earlier characters. Reads are no longer globally capped; only retained bytes are bounded.
    #[test]
    fn provider_has_no_lifetime_read_cap_for_the_final_fontconfig_candidate() {
        let provider = CraftFontProvider::new(&test_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>), 1.0);
        let bytes = synthetic_font(&[(0x0E01, 0x0E5B)]);
        {
            let mut cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.max_bytes = bytes.len();
        }
        for i in 0..130 {
            let path = fake_dir().join(format!("lifetime-{i}.ttf"));
            std::fs::write(&path, &bytes).expect("write a synthetic test font");
            let file = vec![FontFile { path, family: String::new() }];
            assert!(provider.system_insert_files(file, '日').is_none());
        }
        let covering = fake("hant.ttf").path;
        let insert = provider
            .system_insert_files(vec![FontFile { path: covering, family: String::new() }], '龍')
            .expect("the final candidate must still be read after 130 successful reads");
        assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.reads, 131, "130 non-covering reads plus the final candidate");
    }

    static CONCURRENT_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counted_slow_read(path: &std::path::Path) -> Option<Vec<u8>> {
        CONCURRENT_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::read(path).ok()
    }

    /// Layout may ask the provider from several threads. The check/read/insert is serialised, so
    /// the same path is read once and every caller sees the same success.
    #[test]
    fn provider_serialises_concurrent_reads_of_the_same_path() {
        let path = fake("font.ttf").path;
        let mut provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        provider.reader = counted_slow_read;
        CONCURRENT_READS.store(0, std::sync::atomic::Ordering::Relaxed);
        let provider = std::sync::Arc::new(provider);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let provider = std::sync::Arc::clone(&provider);
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let file = vec![FontFile { path, family: String::new() }];
                provider.system_insert_files(file, 'A').is_some()
            }));
        }
        for handle in handles {
            assert!(handle.join().expect("layout thread"), "every caller must get the Inter face");
        }
        assert_eq!(CONCURRENT_READS.load(std::sync::atomic::Ordering::Relaxed), 1, "one read per path even under concurrency");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.faces.len(), 1, "one selected face is shared");
    }

    /// A readable non-font is negative-cached as invalid (read/parsed once) and never conflated
    /// with a valid font that simply does not cover the requested character.
    #[test]
    fn provider_negative_caches_invalid_fonts_separately_from_noncovering_faces() {
        let provider = CraftFontProvider::new(&test_sources((|| Some("en".into())) as fn() -> Option<String>), 1.0);
        let invalid = fake("not-a-font-cache.ttf").path;
        std::fs::write(&invalid, b"this is not a font").expect("write invalid bytes");
        let truncated = fake("truncated-font-cache.ttf").path;
        std::fs::write(&truncated, &include_bytes!("../../../assets/fonts/Inter-Regular.ttf")[..20]).expect("write truncated font");
        for path in [&invalid, &truncated] {
            for c in ['日', '本'] {
                let file = vec![FontFile { path: path.clone(), family: String::new() }];
                assert!(provider.system_insert_files(file, c).is_none());
            }
        }
        {
            let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(cache.invalid_fonts.len(), 2, "both invalid paths are remembered");
            assert_eq!(cache.reads, 2, "invalid bytes are read and examined once each");
            assert!(cache.files.is_empty(), "invalid bytes are never retained as a font");
            assert!(cache.failed_reads.is_empty(), "an invalid font is not an I/O failure");
        }
        let inter = fake("font.ttf").path;
        let file = vec![FontFile { path: inter.clone(), family: String::new() }];
        assert!(provider.system_insert_files(file, '日').is_none(), "Inter does not cover 日");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(cache.files.iter().any(|file| file.path == inter), "a valid non-covering font is cached");
        assert!(!cache.invalid_fonts.contains(&inter), "a non-covering face is not an invalid font");
        drop(cache);
        let file = vec![FontFile { path: inter, family: String::new() }];
        let insert = provider.system_insert_files(file, 'A').expect("the cached valid font still covers A");
        assert!(insert.name.starts_with("craft-system-"), "{insert:?}");
    }

    #[test]
    fn pending_scan_recovers_through_real_layout_and_the_lazy_reload() {
        PENDING_READY.store(false, std::sync::atomic::Ordering::Relaxed);
        let ctx = egui::Context::default();
        let sources = Sources {
            locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
            files: ready_files,
            last_resort: Vec::new,
            pending: pending_ready,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: true,
        };
        crate::theme::install_fonts_with(&ctx, sources);
        let frame = |ctx: &egui::Context| {
            ctx.run_ui(Default::default(), |ui| {
                ui.label("龍");
            })
            .textures_delta
            .clear();
        };
        frame(&ctx);
        assert!(!ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')), "the pending first frame has no Hant face");

        PENDING_READY.store(true, std::sync::atomic::Ordering::Relaxed);
        for _ in 0..4 {
            frame(&ctx);
        }
        assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')), "the lazy loader's add_font reload recovers after the scan");
        let registered = ctx.fonts(|f| f.definitions().font_data.keys().filter(|name| name.starts_with(FONT_PREFIX)).count());
        assert_eq!(registered, 1, "exactly the Hant file was installed");
        PENDING_READY.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn strict_policy_first_frame_after_the_scan_is_accepted() {
        if craft_fonts::CRAFT_FONTS.is_empty() {
            eprintln!("skipping: the eager provider is only installed with craft-fonts");
            return;
        }
        let ctx = egui::Context::default();
        ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
        let sources = Sources {
            locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
            files: ready_hant_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: true,
        };
        crate::theme::install_fonts_with(&ctx, sources);
        ctx.run_ui(Default::default(), |ui| {
            ui.label("龍");
        })
        .textures_delta
        .clear();
        assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')));
    }

    /// A strict first frame while Fontconfig is still pending cannot be recovered by the lazy
    /// loader: epaint panics during layout before `output_hook` can call `add_font`. The real app
    /// uses `MissingGlyphPolicy::Tofu` (brief tofu, then the reload above); strict integration
    /// tests such as `camera_raw_localization` must wait for the scan first. This test records
    /// that limitation instead of claiming a pending strict startup is accepted.
    #[test]
    #[should_panic(expected = "No glyph for")]
    fn pending_strict_policy_startup_is_not_fake_acceptance() {
        let ctx = egui::Context::default();
        ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
        let sources = Sources {
            locale: (|| Some("zh-Hant-TW".into())) as fn() -> Option<String>,
            files: no_files,
            last_resort: Vec::new,
            pending: strict_pending,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: true,
        };
        crate::theme::install_fonts_with(&ctx, sources);
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.label("龍");
        });
    }

    #[test]
    fn language_change_reloads_eager_faces_through_the_app_context() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        let Some(ui_index) = craft_index(ui_font) else { return };
        let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
            eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
            return;
        };
        let ui_name = format!("craft-font-{ui_index}");
        let sc_name = format!("craft-font-{sc_index}");
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            let ctx = egui::Context::default();
            crate::i18n::sync_context(&ctx, "ja");
            for _ in 0..2 {
                ctx.run_ui(Default::default(), |ui| {
                    ui.label("日");
                })
                .textures_delta
                .clear();
            }
            let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
            assert!(discovered.iter().any(|insert| insert.name == ui_name), "ja discovers {ui_name}: {discovered:?}");

            crate::i18n::sync_context(&ctx, "zh-hans");
            for _ in 0..2 {
                ctx.run_ui(Default::default(), |ui| {
                    ui.label("日");
                })
                .textures_delta
                .clear();
            }
            let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
            assert!(discovered.iter().any(|insert| insert.name == sc_name), "zh-hans discovers {sc_name}: {discovered:?}");
            assert!(!discovered.iter().any(|insert| insert.name == ui_name), "the language reload clears the old Japanese face: {discovered:?}");
        });
    }

    /// A valid synthetic font whose `glyf` table record extends past the end of the file. Its
    /// `cmap` still parses and maps Thai, so a coverage-only check accepts it, but the text
    /// engine cannot read every declared table.
    fn font_with_out_of_bounds_table() -> Vec<u8> {
        let mut font = synthetic_font(&[(0x0E01, 0x0E5B)]);
        let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
        for i in 0..tables {
            let rec = 12 + 16 * i;
            if font.get(rec..rec + 4) == Some(&b"glyf"[..]) {
                font[rec + 12..rec + 16].copy_from_slice(&u32::MAX.to_be_bytes());
                return font;
            }
        }
        panic!("synthetic font has no glyf table");
    }

    /// A readable collection whose selected family face has an out-of-bounds `glyf` table. Face 0
    /// stays readable, and face 1's `cmap`/`name` still work, so a file-level check plus coverage
    /// accepts it even though the face cannot be installed safely.
    fn collection_with_out_of_bounds_selected_face() -> Vec<u8> {
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let mut jetbrains = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec();
        let tables = u16::from_be_bytes([jetbrains[4], jetbrains[5]]) as usize;
        let glyf = (0..tables).map(|i| 12 + 16 * i).find(|&rec| jetbrains.get(rec..rec + 4) == Some(&b"glyf"[..])).expect("JetBrains Mono has a glyf table");
        jetbrains[glyf + 12..glyf + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        two_face_ttc(inter, &jetbrains)
    }

    /// A truncated/out-of-bounds table can keep a valid `cmap`, so `load_first` must validate
    /// the file before registering it, and must still reach the next, valid candidate.
    #[test]
    fn lazy_loader_rejects_a_font_with_an_out_of_bounds_table_and_uses_the_next_candidate() {
        let malformed = fake_dir().join("out-of-bounds-thai.ttf");
        std::fs::write(&malformed, font_with_out_of_bounds_table()).unwrap();
        let valid = fake_dir().join("thai.ttf");
        let files = vec![FontFile { path: malformed.clone(), family: String::new() }, FontFile { path: valid.clone(), family: String::new() }];
        let bytes = std::fs::read(&malformed).unwrap();
        assert!(scripts::face_covers(&bytes, 0, 'ก'), "the malformed file's cmap still maps the character");
        assert!(!cjk::font_file_is_readable(&bytes), "an out-of-bounds table makes the file unreadable");

        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        let (_, data) = fallback.load_first(&files, 'ก').expect("the valid fallback is reached");
        assert_eq!(fallback.registered.len(), 1);
        assert_eq!(fallback.registered[0].1, valid, "the malformed file is skipped, not registered");
        assert_eq!(data.index, 0);
        assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());

        // The non-CJK script route must enforce the same validation before registration.
        let mut sources = test_sources(|| Some("en".into()));
        sources.scripts = |_| vec![fake_dir().join("out-of-bounds-thai.ttf"), fake_dir().join("thai.ttf")];
        let mut fallback = CjkFallback::new(sources);
        let (_, data) = fallback.next_script_font('ก').expect("the valid script fallback is reached");
        assert_eq!(fallback.registered.len(), 1);
        assert_eq!(fallback.registered[0].1, valid, "the script route also skips the malformed file");
        assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());

        // A readable collection with an unreadable covering face must also skip that face: the
        // coverage route selects face 1 by its valid cmap, not the readable face 0.
        let collection = fake_dir().join("out-of-bounds-thai.ttc");
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let collection_bytes = two_face_ttc(inter, &font_with_out_of_bounds_table());
        std::fs::write(&collection, &collection_bytes).unwrap();
        assert!(cjk::font_file_is_readable(&collection_bytes), "face 0 keeps the collection readable");
        assert_eq!(scripts::covering_face(&collection_bytes, 'ก'), Some(1), "coverage selects the malformed face");
        assert!(!cjk::font_face_is_readable(&collection_bytes, 1));
        let mut sources = test_sources(|| Some("en".into()));
        sources.scripts = |_| vec![fake_dir().join("out-of-bounds-thai.ttc"), fake_dir().join("thai.ttf")];
        let mut fallback = CjkFallback::new(sources);
        let (_, data) = fallback.next_script_font('ก').expect("the valid script fallback is reached");
        assert_eq!(fallback.registered.len(), 1);
        assert_eq!(fallback.registered[0].1, valid, "the coverage route skips the malformed selected face");
        assert_eq!(data.bytes(), std::fs::read(&valid).unwrap().as_slice());
    }

    /// A readable collection may contain one bad face. The selected face (by family name) can
    /// keep a valid `cmap` and `name` while another table points outside the file, so checking the
    /// file or coverage alone would register a face epaint cannot safely install. Both the lazy
    /// loader and the eager provider must skip that face and continue to the valid candidate.
    #[test]
    fn malformed_selected_collection_face_falls_back_before_lazy_and_provider_registration() {
        let path = fake_dir().join("out-of-bounds-selected.ttc");
        let bytes = collection_with_out_of_bounds_selected_face();
        std::fs::write(&path, &bytes).expect("write the synthetic collection");
        assert!(cjk::font_file_is_readable(&bytes), "face 0 keeps the collection readable");
        assert_eq!(cjk::face_index_for_family_checked(&bytes, "JetBrains Mono"), Some(1), "the selected face's name resolves");
        assert!(scripts::face_covers(&bytes, 1, 'A'), "the selected face's cmap covers the character");
        assert!(!cjk::font_face_is_readable(&bytes, 1), "the selected face's glyf table is out of bounds");

        let valid = fake("font.ttf");
        let candidates = vec![FontFile { path: path.clone(), family: "JetBrains Mono".into() }, FontFile { path: valid.path.clone(), family: String::new() }];
        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        let (name, data) = fallback.load_first(&candidates, 'A').expect("the valid alternative is reached");
        assert_eq!(data.index, 0, "the lazy loader registers the valid face 0, not the malformed selected face");
        assert_eq!(fallback.registered.len(), 1);
        assert_eq!(fallback.registered[0].1, valid.path, "the malformed collection is not registered");

        let provider = CraftFontProvider::new(&test_sources(|| Some("en".into())), 1.0);
        let insert = provider.system_insert_files(candidates, 'A').expect("the provider reaches the valid alternative");
        assert_eq!(insert.data.index, 0, "the provider does not hand the malformed selected face to epaint");
        assert!(insert.name.starts_with("craft-system-"), "{insert:?}");

        // Installing the selected font under the strict policy must not panic: only the valid
        // fallback reaches egui's family builder.
        let ctx = egui::Context::default();
        ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
        crate::theme::install_fonts(&ctx);
        // The lazy loader runs after a frame; initialize the font view before matching that path.
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        add_to_all_families(&ctx, name, data);
        ctx.run_ui(Default::default(), |ui| {
            ui.label("A");
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn lazy_loader_uses_the_requested_collection_family_and_checks_coverage() {
        let path = fake_dir().join("families.ttc");
        let file = |family: &str| vec![FontFile { path: path.clone(), family: family.into() }];

        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        let (_, data) = fallback.load_first(&file("JetBrains Mono"), 'A').expect("face 1 covers A");
        assert_eq!(data.index, 1);

        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        assert!(fallback.load_first(&file("JetBrains Mono"), 'ƀ').is_none(), "coverage is checked on the requested face");

        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        let empty_family = vec![FontFile { path: path.clone(), family: String::new() }];
        let (_, data) = fallback.load_first(&empty_family, 'ƀ').expect("empty family is face 0");
        assert_eq!(data.index, 0);
    }

    fn always_thai_candidate(_: Script) -> Vec<PathBuf> {
        vec![fake_dir().join("thai.ttf")]
    }

    /// A path already registered through either route is not read or registered a second time.
    #[test]
    fn lazy_loader_registers_each_loaded_font_once() {
        let valid = fake("font.ttf");
        let mut fallback = CjkFallback::new(test_sources(|| Some("en".into())));
        let files = vec![valid.clone()];
        fallback.load_first(&files, 'A').expect("the font covers Latin A");
        assert!(fallback.load_first(&files, 'A').is_none(), "a second request for the loaded file registers nothing");
        assert_eq!(fallback.registered.len(), 1);

        let sources = Sources { scripts: always_thai_candidate, ..test_sources(|| Some("en".into())) };
        let mut fallback = CjkFallback::new(sources);
        fallback.loaded.push(fake_dir().join("thai.ttf"));
        assert!(fallback.next_script_font('ก').is_none(), "the script route skips a file the other route already loaded");
        assert!(fallback.registered.is_empty());
        assert_eq!(fallback.scripts_tried, vec![Script::Thai], "the script is still marked tried only once");
    }

    /// A craft face is registered once: a later request for the same embedded file must not add
    /// another font with a new name and a duplicate blob.
    #[test]
    fn lazy_loader_registers_an_embedded_craft_face_once() {
        if baseline_face('あ').is_none() {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        }
        let mut fallback = CjkFallback::new(craft_only(|| Some("en".into())));
        fallback.load_embedded(CjkScript::Japanese, 'あ').expect("the manifest covers kana");
        assert!(fallback.load_embedded(CjkScript::Japanese, 'あ').is_none(), "the same embedded face is not registered twice");
        assert_eq!(fallback.registered.len(), 1);
        assert_eq!(fallback.loaded.len(), 1);
    }

    #[test]
    fn provider_uses_system_script_candidates_and_last_resort_for_non_cjk() {
        let sources = Sources {
            locale: || Some("en".into()),
            files: no_files,
            last_resort: || vec![FontFile { path: fake_dir().join("ethiopic.ttf"), family: String::new() }],
            pending: || false,
            scripts: |s| match s {
                Script::Tamil => vec![fake_dir().join("tamil.ttf")],
                _ => Vec::new(),
            },
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        };
        let provider = CraftFontProvider::new(&sources, 1.0);
        let tamil = provided(&provider, 'த').expect("Tamil system candidate");
        assert!(tamil.starts_with("craft-system-"), "{tamil}");
        let ethiopic = provided(&provider, 'ሀ').expect("last-resort face");
        assert!(ethiopic.starts_with("craft-system-"), "{ethiopic}");
    }

    static PROVIDER_SCRIPT_CALLS: Mutex<Vec<CjkScript>> = Mutex::new(Vec::new());

    fn recorded_provider_files(script: CjkScript) -> Vec<FontFile> {
        PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(script);
        Vec::new()
    }

    /// A Hangul character's preferred script is the locale order's first script, so the provider
    /// builds a list with Korean twice. It must still ask for Korean's files once, then try each
    /// remaining script once, not restart the preferred script after a miss.
    #[test]
    fn provider_tries_each_script_once_when_the_preferred_script_repeats_the_order() {
        PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        let sources = Sources {
            locale: || Some("ko_KR".into()),
            files: recorded_provider_files,
            last_resort: Vec::new,
            pending: || false,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        };
        let provider = CraftFontProvider::new(&sources, 1.0);
        assert!(provided(&provider, '한').is_none(), "the test source has no Hangul face");
        let calls = PROVIDER_SCRIPT_CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        assert_eq!(calls, vec![CjkScript::Korean, CjkScript::TraditionalChinese, CjkScript::SimplifiedChinese, CjkScript::Japanese]);
    }

    /// An unclassified symbol (not CJK, not one of the non-CJK scripts) only has the optional
    /// craft `Zsym` face. Without one the provider must return missing rather than read unrelated
    /// system fonts, and an empty cluster is likewise no request at all.
    #[test]
    fn provider_missing_symbol_and_empty_cluster_do_not_touch_system_fonts() {
        let sources = Sources { embedded: no_embedded, ..test_sources(|| Some("en".into())) };
        let provider = CraftFontProvider::new(&sources, 1.0);
        assert!(provider.font_for(&FallbackRequest { cluster: "", family: &FontFamily::Proportional }).is_none());
        assert!(provided(&provider, '☃').is_none(), "no injected Zsym craft face covers the symbol");
        let cache = provider.system.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.reads, 0, "a symbol is not a CJK or script fallback request");
        assert!(cache.failed_reads.is_empty() && cache.invalid_fonts.is_empty());
    }

    #[test]
    fn provider_applies_the_current_scale_to_craft_faces() {
        if baseline_face('あ').is_none() {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        }
        let provider = test_provider(|| Some("en".into()));
        let first = provided_insert(&provider, 'あ').expect("the craft manifest covers kana");
        assert_eq!(first.data.tweak.scale, 1.0);
        assert!(first.families.iter().any(|f| f.family == FontFamily::Proportional));
        assert!(first.families.iter().any(|f| f.family == FontFamily::Monospace));
        provider.set_scale(4.0 / 3.0);
        let second = provided_insert(&provider, 'あ').expect("the craft face is resolved again at the current scale");
        assert_eq!(second.data.tweak.scale, 4.0 / 3.0, "a craft face must not bake in the scale of an earlier pass");
    }

    /// Face-name metadata is capped independently of the retained-blob budget: forgetting the
    /// oldest name only makes egui install that face under a later name, it never drops bytes
    /// from a file that still fits.
    #[test]
    fn system_face_names_are_bounded_independently_of_font_blobs() {
        let mut cache = SystemCache::default();
        for i in 0..MAX_SYSTEM_FACE_NAMES + 4 {
            cache.remember_face(PathBuf::from(format!("face-{i}.ttf")), i as u32, format!("craft-system-{i}"));
        }
        assert_eq!(cache.faces.len(), MAX_SYSTEM_FACE_NAMES);
        assert_eq!(cache.faces.first().map(|face| face.name.as_str()), Some("craft-system-4"), "the oldest names are evicted first");
        assert_eq!(cache.faces.last().map(|face| face.name.as_str()), Some("craft-system-1027"));
        assert!(cache.files.is_empty() && cache.retained_bytes == 0, "face names do not retain or charge font bytes");
    }

    #[test]
    fn pending_fontconfig_defers_the_last_resort_instead_of_returning_none_forever() {
        PENDING_LAST_RESORT.store(true, std::sync::atomic::Ordering::Relaxed);
        let sources = Sources {
            locale: || Some("zh-Hant-TW".into()),
            files: no_files,
            last_resort: || vec![FontFile { path: fake_dir().join("hant.ttf"), family: String::new() }],
            pending: pending_last_resort,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: no_embedded,
            eager_craft_fonts: false,
        };
        let provider = CraftFontProvider::new(&sources, 1.0);
        assert!(provided(&provider, '龍').is_none(), "pending Hant scan must not fall through to a last resort");
        PENDING_LAST_RESORT.store(false, std::sync::atomic::Ordering::Relaxed);
        let got = provided(&provider, '龍').expect("after the scan the last resort is usable");
        assert!(got.starts_with("craft-system-"), "{got}");
    }

    #[test]
    fn pending_fontconfig_does_not_cache_the_lower_priority_sc_craft_face_for_hant() {
        let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
            eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
            return;
        };
        PENDING_CRAFT_SC.store(true, std::sync::atomic::Ordering::Relaxed);
        let sources = Sources {
            locale: || Some("zh-Hant-TW".into()),
            files: no_files,
            last_resort: Vec::new,
            pending: pending_craft_sc,
            scripts: no_script_files,
            covers: scripts::covering_face,
            embedded: craft_embedded_iso,
            eager_craft_fonts: false,
        };
        let provider = CraftFontProvider::new(&sources, 1.0);
        assert!(provided(&provider, '龍').is_none(), "a pending Hant scan must not return (and cache) the lower-priority SC craft face");
        PENDING_CRAFT_SC.store(false, std::sync::atomic::Ordering::Relaxed);
        let expected = format!("craft-font-{sc_index}");
        assert_eq!(provided(&provider, '龍').as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn every_craft_face_is_available_to_both_ui_families() {
        let mut checked = 0;
        for (i, face) in craft_fonts::CRAFT_FONTS.iter().enumerate() {
            if face.bytes.is_empty() {
                continue;
            }
            let insert = craft_insert(i, face, 1.0);
            assert_eq!(insert.name, format!("craft-font-{i}"));
            assert!(insert.families.iter().any(|f| f.family == FontFamily::Proportional), "{} is missing from Proportional", face.family);
            assert!(insert.families.iter().any(|f| f.family == FontFamily::Monospace), "{} is missing from Monospace", face.family);
            checked += 1;
        }
        if checked == 0 {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        }
    }

    #[test]
    fn the_craft_provider_shapes_the_first_headless_frame_in_both_families() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        let Some(ui_index) = craft_index(ui_font) else { return };
        let Some((sc_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans CJK SC") else {
            eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
            return;
        };
        let Some((korean_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "NanumGothic") else {
            eprintln!("skipping: the craft-fonts manifest has no Korean face");
            return;
        };
        let Some((arabic_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Arabic") else {
            eprintln!("skipping: the craft-fonts manifest has no Arabic face");
            return;
        };
        let Some((devanagari_index, _)) = craft_fonts::CRAFT_FONTS.iter().enumerate().find(|(_, f)| f.family == "Noto Sans Devanagari") else {
            eprintln!("skipping: the craft-fonts manifest has no Devanagari face");
            return;
        };
        let Some(baseline_kana) = baseline_face('か') else { return };
        let Some(baseline_han) = baseline_face('日') else { return };
        let baseline_kana_index = craft_index(baseline_kana).expect("a manifest face has an index");
        let baseline_han_index = craft_index(baseline_han).expect("a manifest face has an index");
        for (locale, text, expected) in [
            ((|| Some("ja_JP".into())) as fn() -> Option<String>, "かな", ui_index),
            ((|| Some("en_US".into())) as fn() -> Option<String>, "かな", baseline_kana_index),
            ((|| Some("en_US".into())) as fn() -> Option<String>, "日本", baseline_han_index),
            ((|| None) as fn() -> Option<String>, "日本", baseline_han_index),
            ((|| Some("en_US".into())) as fn() -> Option<String>, "한", korean_index),
            ((|| Some("en_US".into())) as fn() -> Option<String>, "مرحبا", arabic_index),
            ((|| Some("en_US".into())) as fn() -> Option<String>, "नमस्ते", devanagari_index),
            ((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>, "かな", ui_index),
            ((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>, "龍", sc_index),
            ((|| Some("zh-CN".into())) as fn() -> Option<String>, "中文", sc_index),
        ] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts_with(&ctx, eager_sources(locale));
            ctx.run_ui(Default::default(), |ui| {
                ui.label(text);
                ui.label(egui::RichText::new(text).monospace());
            })
            .textures_delta
            .clear();
            assert!(ctx.fonts_mut(|f| f.has_glyphs(&FontId::monospace(12.0), text)), "{text} is missing from Monospace");
            let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
            assert!(
                discovered.iter().any(|insert| insert.name == format!("craft-font-{expected}")),
                "{text}: expected craft-font-{expected}, discovered {discovered:?}"
            );
            for insert in &discovered {
                assert!(insert.families.iter().any(|f| f.family == FontFamily::Proportional), "{insert:?}");
                assert!(insert.families.iter().any(|f| f.family == FontFamily::Monospace), "{insert:?}");
            }
        }
        // A Traditional Chinese locale has no Hant craft face. A system Hant file, when present,
        // must be read before the Simplified Chinese craft fallback.
        if hant_system_file().is_some() {
            let mut sources = eager_sources((|| Some("zh-Hant-TW".into())) as fn() -> Option<String>);
            sources.files = hant_files;
            let ctx = egui::Context::default();
            crate::theme::install_fonts_with(&ctx, sources);
            ctx.run_ui(Default::default(), |ui| {
                ui.label("龍");
            })
            .textures_delta
            .clear();
            let discovered = ctx.fonts(|f| f.discovered_fonts().to_vec());
            assert!(discovered.iter().any(|insert| insert.name.starts_with("craft-system-")), "the system Hant face draws the first frame: {discovered:?}");
            assert!(
                !discovered.iter().any(|insert| insert.name == format!("craft-font-{sc_index}")),
                "the Simplified Chinese face must not pre-empt the Hant system font: {discovered:?}"
            );
        }
    }

    #[test]
    fn ui_works_without_craft_fonts() {
        // The pre-craft-fonts behaviour: no embedded fonts, no system fonts. Latin renders,
        // Japanese is tried (and stays missing) without panicking, and the loader exhausts.
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(
            &ctx,
            Sources {
                locale: || None,
                files: |_| vec![],
                last_resort: Vec::new,
                pending: || false,
                scripts: |_| Vec::new(),
                covers: scripts::covering_face,
                embedded: no_embedded,
                eager_craft_fonts: false,
            },
        );
        assert!(render(&ctx, "Layer 1 – café"));
        assert!(!render(&ctx, "日本語の文字"));
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 0);
        if craft_fonts::CRAFT_FONTS.is_empty() {
            assert!(craft_embedded(CjkScript::Japanese).is_empty());
        }
    }

    /// With the real system fonts, every CJK sample renders (skipped per script when the
    /// machine has no font for it, e.g. CI Linux without Noto CJK).
    #[test]
    fn system_fonts_cover_cjk_samples() {
        #[cfg(target_os = "linux")]
        assert!(wait_for_fontconfig_cjk_scan(), "Fontconfig CJK lookup did not finish within 10 seconds");
        let available = |s: CjkScript| cjk::font_files(s).iter().any(|f| f.path.is_file());
        let samples = [
            ("图层", CjkScript::SimplifiedChinese),
            ("圖層", CjkScript::TraditionalChinese),
            ("카드 배경", CjkScript::Korean),
            ("レイヤー", CjkScript::Japanese),
        ];
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (text, script) in samples {
            if !available(script) {
                eprintln!("skipping {text}: no {script:?} system font");
                continue;
            }
            assert!(render(&ctx, text), "{text} still has missing glyphs");
        }
        // All at once in a fresh context too (several scripts in one frame).
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let all: String = samples.iter().filter(|(_, s)| available(*s)).map(|(t, _)| *t).collect::<Vec<_>>().join(" ");
        assert!(render(&ctx, &all), "{all}");
    }
}
