//! Lazy CJK and non-CJK fallback fonts for the UI.
//!
//! The bundled Inter / JetBrains Mono have no Japanese, Chinese, Korean, Thai, Arabic, Hebrew or
//! Devanagari glyphs, and the OS fonts that do are large (Hiragino ~10 MB, Apple SD Gothic Neo
//! 28 MB, PingFang 78 MB, Noto Sans CJK ~20 MB). Instead of reading them at startup, an egui
//! plugin scans each frame's text for characters no registered font covers, and registers the
//! next system font for them (`ctx.add_font`, active from the next frame, which it requests).
//! Fonts are appended at the lowest priority to every family, so Latin text keeps Inter.
//!
//! Script order follows the UI locale ([`photocraft_text::cjk::script_order`]): Kana prefers a
//! Japanese font, Hangul a Korean one, Bopomofo a Traditional Chinese one, and Han the
//! locale's script (Japanese forms only for a Japanese locale). Once every script has been tried
//! the scan stops. A character of another script the bundled fonts lack (Thai, Arabic, Hebrew,
//! Devanagari) is looked up in the Type tool's font database instead
//! ([`photocraft_text::fonts::FontDb::fallback_face_for`]); it never returns a broad font that
//! also covers CJK, so it cannot shadow the locale-ordered CJK choice above.
//!
//! Builds made with the optional craft-fonts input (`CRAFT_FONTS_DIR`,
//! [`photocraft_text::craft_fonts`]) carry Japanese fonts (BIZ UDPGothic first): they are tried
//! before the system Japanese fonts, in the Japanese slot of the same locale order. Without
//! craft-fonts (and on the web, which never embeds them) nothing changes.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FontData, FontFamily, FontId, Shape};
use photocraft_text::cjk::{self, CjkChar, CjkScript, FontFile};
use photocraft_text::craft_fonts::{self, CraftFont};
use photocraft_text::served;
use std::path::PathBuf;

/// Skip absurdly large files (a corrupt or non-font path must not eat memory).
pub const MAX_FONT_BYTES: u64 = 128 << 20;
/// Name prefix of the registered fallback fonts.
pub const FONT_PREFIX: &str = "system-cjk";
/// Characters whose fallback lookup failed are remembered so they don't stall the scan; a bound
/// keeps odd text from growing it without limit (served fonts arriving reset the loader).
const MAX_GAVE_UP: usize = 256;
/// Distinct classifiable characters collected per frame before coverage is checked; a generous
/// bound so a long run of already-covered characters can't hide a later one.
const MAX_CANDIDATES: usize = 1024;

/// A system face the UI can register, from the Type tool's font database.
pub use photocraft_text::fonts::FallbackFace as SystemFace;

/// The fallback a glyphless UI character needs, or `None` when none applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Missing {
    Cjk(CjkChar),
    Other,
}

/// The fallback a glyphless UI character needs, or `None` when none applies.
fn classify(c: char, order: &[CjkScript; 4]) -> Option<Missing> {
    if let Some(kind) = cjk::classify(c) {
        return Some(Missing::Cjk(kind));
    }
    served::script_of(c, order).map(|_| Missing::Other)
}

/// Where fonts come from; swapped out in tests.
#[derive(Clone)]
pub struct Sources {
    pub locale: fn() -> Option<String>,
    pub files: fn(CjkScript) -> Vec<FontFile>,
    pub last_resort: fn() -> Vec<FontFile>,
    /// Embedded fonts tried before `files` for a script (craft-fonts' Japanese fonts).
    pub embedded: fn(CjkScript) -> Vec<&'static CraftFont>,
    /// A system face covering a non-CJK character the bundled UI fonts lack, from the Type
    /// tool's font database. Empty on the web and in tests.
    pub by_char: fn(char) -> Option<SystemFace>,
}

/// craft-fonts' Japanese fonts for the Japanese script, UI face first (empty without craft-fonts).
pub fn craft_embedded(script: CjkScript) -> Vec<&'static CraftFont> {
    if script == CjkScript::Japanese { craft_fonts::japanese_for_ui() } else { Vec::new() }
}

/// No embedded fonts (tests that pin the system-font behaviour).
pub fn no_embedded(_: CjkScript) -> Vec<&'static CraftFont> {
    Vec::new()
}

/// The Type tool's font database supplies a face for a non-CJK script it can fall back to
/// (system fonts are loaded there; the web has none).
#[cfg(not(target_arch = "wasm32"))]
fn system_by_char(c: char) -> Option<SystemFace> {
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    eng.fonts.fallback_face_for(c)
}

impl Sources {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn system() -> Self {
        Self {
            locale: || match crate::i18n::current().code() {
                code @ ("ja" | "ko" | "zh-hans" | "zh-hant") => Some(code.to_string()),
                _ => cjk::ui_locale().map(str::to_string),
            },
            files: cjk::font_files,
            last_resort: cjk::last_resort_files,
            embedded: craft_embedded,
            by_char: system_by_char,
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn system() -> Self {
        Self { locale: || None, files: |_| Vec::new(), last_resort: Vec::new, embedded: craft_embedded, by_char: |_| None }
    }
}

/// The lazy loader's state: which scripts were tried and which files were read.
pub struct CjkFallback {
    sources: Sources,
    /// set_fonts takes effect next pass; do not collide with a previous loader's font names.
    defer_after_reset: bool,
    order: Option<[CjkScript; 4]>,
    tried: Vec<CjkScript>,
    last_resort_tried: bool,
    /// CJK kinds a loaded script font already covers. Kept independently of the whole egui font
    /// stack so a broad font that happens to cover Han doesn't make us think the locale CJK fonts
    /// aren't needed.
    resolved_kinds: Vec<CjkChar>,
    /// Glyphless non-CJK characters whose fallback lookup found nothing, so the scan moves past
    /// them instead of retrying forever (bounded by [`MAX_GAVE_UP`]).
    gave_up: Vec<char>,
    loaded: Vec<PathBuf>,
    /// `(family, face index)` of the non-CJK system faces already registered, so two characters
    /// of the same script don't register the same face twice.
    loaded_system: Vec<String>,
    /// Fonts registered so far: `(name, path, face index, broad)`. `broad` puts a broad fallback
    /// (Arial Unicode) after the script fonts in every family.
    pub registered: Vec<(String, PathBuf, u32, bool)>,
}

impl CjkFallback {
    pub fn new(sources: Sources) -> Self {
        Self {
            sources,
            defer_after_reset: false,
            order: None,
            tried: Vec::new(),
            last_resort_tried: false,
            resolved_kinds: Vec::new(),
            gave_up: Vec::new(),
            loaded: Vec::new(),
            loaded_system: Vec::new(),
            registered: Vec::new(),
        }
    }

    /// Script order for the UI locale (resolved on first use, not at startup).
    pub fn order(&mut self) -> [CjkScript; 4] {
        *self.order.get_or_insert_with(|| cjk::script_order((self.sources.locale)().as_deref()))
    }

    /// Every CJK script and the last-resort fonts have been tried: no more CJK font can be loaded.
    fn cjk_done(&self) -> bool {
        self.tried.len() >= 4 && self.last_resort_tried
    }

    /// `(name, broad)` of every registered lazy font, for ordering the family stacks.
    fn registered_order(&self) -> Vec<(String, bool)> {
        self.registered.iter().map(|(n, _, _, b)| (n.clone(), *b)).collect()
    }

    /// The next font to register for a glyphless character `c` that no current font covers: a
    /// CJK script font (the character's preferred script, then the locale order, then the
    /// last-resort fonts), or a system face for another script. `None` when no font covers it
    /// (then `c` is remembered so the scan doesn't stall on it).
    pub fn next_font(&mut self, c: char) -> Option<(String, FontData)> {
        let order = self.order();
        match classify(c, &order) {
            Some(Missing::Cjk(kind)) => {
                let font = self.next_cjk_font(kind);
                if let Some((_, data)) = &font {
                    for k in cjk::covered_kinds(data.font.as_ref(), data.index) {
                        if !self.resolved_kinds.contains(&k) {
                            self.resolved_kinds.push(k);
                        }
                    }
                }
                font
            }
            Some(Missing::Other) => {
                let font = self.load_by_char(c);
                if font.is_none() && !self.gave_up.contains(&c) && self.gave_up.len() < MAX_GAVE_UP {
                    self.gave_up.push(c);
                }
                font
            }
            None => None,
        }
    }

    /// The next CJK script font for a character of kind `kind` (see [`Self::next_font`]).
    fn next_cjk_font(&mut self, kind: CjkChar) -> Option<(String, FontData)> {
        let order = self.order();
        let scripts: Vec<CjkScript> = std::iter::once(kind.preferred(&order)).chain(order).collect();
        for s in scripts {
            if self.tried.contains(&s) {
                continue;
            }
            self.tried.push(s);
            if let Some(f) = self.load_embedded(s) {
                return Some(f);
            }
            let files = (self.sources.files)(s);
            if let Some(f) = self.load_first(&files) {
                return Some(f);
            }
        }
        if !self.last_resort_tried {
            self.last_resort_tried = true;
            let files = (self.sources.last_resort)();
            return self.load_first(&files);
        }
        None
    }

    /// A non-CJK system face covering `c`, registered once per face.
    fn load_by_char(&mut self, c: char) -> Option<(String, FontData)> {
        let face = (self.sources.by_char)(c)?;
        let key = format!("{}#{}", face.family, face.index);
        if self.loaded_system.contains(&key) || face.bytes.len() as u64 > MAX_FONT_BYTES {
            return None;
        }
        self.loaded_system.push(key);
        let name = format!("{FONT_PREFIX}-{}", self.registered.len());
        log::info!("UI font fallback: registered system face {} (face {}) as {name}", face.family, face.index);
        self.registered.push((name.clone(), PathBuf::from(&face.family), face.index, face.broad));
        let mut data = FontData::from_owned(face.bytes);
        data.index = face.index;
        Some((name, data))
    }

    /// The first embedded (craft-fonts) font for `s`, registered from its static bytes.
    fn load_embedded(&mut self, s: CjkScript) -> Option<(String, FontData)> {
        let f = (self.sources.embedded)(s).into_iter().find(|f| !f.bytes.is_empty())?;
        let path = PathBuf::from(format!("craft-fonts/{} {}", f.family, f.style));
        if self.loaded.contains(&path) {
            return None;
        }
        self.loaded.push(path.clone());
        let name = format!("{FONT_PREFIX}-{}", self.registered.len());
        log::info!("UI font fallback: registered embedded {} as {name}", path.display());
        self.registered.push((name.clone(), path, 0, false));
        Some((name, FontData::from_static(f.bytes)))
    }

    fn load_first(&mut self, files: &[FontFile]) -> Option<(String, FontData)> {
        for f in files {
            if self.loaded.contains(&f.path) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&f.path) else { continue };
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&f.path) else { continue };
            let index = cjk::face_index_for_family(&bytes, f.family);
            self.loaded.push(f.path.clone());
            let name = format!("{FONT_PREFIX}-{}", self.registered.len());
            log::info!("UI font fallback: registered {} (face {index}) as {name}", f.path.display());
            self.registered.push((name.clone(), f.path.clone(), index, false));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            return Some((name, data));
        }
        None
    }
}

/// Characters in the frame's text that still need a fallback: CJK characters of a kind no loaded
/// font covers (and before the CJK scripts are exhausted), and non-CJK characters no registered
/// font draws. Classifiable characters already covered are dropped during collection, so they
/// can't crowd out a later one, and only [`MAX_CANDIDATES`] are kept.
fn missing_chars(
    ctx: &egui::Context,
    shapes: &[egui::epaint::ClippedShape],
    order: &[CjkScript; 4],
    gave_up: &[char],
    resolved: &[CjkChar],
    cjk_done: bool,
) -> Vec<char> {
    fn collect(shape: &Shape, out: &mut Vec<char>, order: &[CjkScript; 4], resolved: &[CjkChar], cjk_done: bool) {
        match shape {
            Shape::Text(t) => {
                let text = &t.galley.job.text;
                if !text.is_ascii() {
                    for c in text.chars().filter(|c| !c.is_ascii()) {
                        if out.len() >= MAX_CANDIDATES {
                            return;
                        }
                        let wanted = match cjk::classify(c) {
                            Some(kind) => !cjk_done && !resolved.contains(&kind),
                            None => served::script_of(c, order).is_some(),
                        };
                        if wanted && !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out, order, resolved, cjk_done)),
            _ => {}
        }
    }
    let mut chars = Vec::new();
    for s in shapes {
        collect(&s.shape, &mut chars, order, resolved, cjk_done);
    }
    let font = FontId::proportional(12.0);
    let mut missing = Vec::new();
    ctx.fonts_mut(|f| {
        for &c in &chars {
            if missing.len() >= 256 {
                break;
            }
            if gave_up.contains(&c) {
                continue;
            }
            // A CJK character is missing by kind (handled above); a non-CJK one by the stack.
            if cjk::classify(c).is_none() && f.has_glyph(&font, c) {
                continue;
            }
            missing.push(c);
        }
    });
    missing
}

/// (ascent, descent, line gap) of a face in em, from its `hhea` table; `descent` is negative.
fn vertical_metrics(font: &[u8], index: u32) -> Option<(f32, f32, f32)> {
    let u16_at = |o: usize| font.get(o..o.checked_add(2)?).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| font.get(o..o.checked_add(4)?).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let i16_at = |o: usize| u16_at(o).map(|v| v as i16 as f32);
    let dir = if font.get(..4)? == b"ttcf" {
        // Collection: the offset table of face `index` follows a 12-byte header.
        u32_at(12usize.checked_add((index as usize).checked_mul(4)?)?)? as usize
    } else {
        0
    };
    let tables = u16_at(dir.checked_add(4)?)? as usize;
    let (mut hhea, mut head) = (None, None);
    for i in 0..tables {
        let rec = dir.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
        let tag = font.get(rec..rec.checked_add(4)?)?;
        let off = u32_at(rec.checked_add(8)?)? as usize;
        match tag {
            b"hhea" => hhea = Some(off),
            b"head" => head = Some(off),
            _ => {}
        }
    }
    let (hhea, head) = (hhea?, head?);
    let upm = u16_at(head.checked_add(18)?)? as f32;
    if upm <= 0.0 {
        return None;
    }
    Some((i16_at(hhea.checked_add(4)?)? / upm, i16_at(hhea.checked_add(6)?)? / upm, i16_at(hhea.checked_add(8)?)? / upm))
}

/// `y_offset_factor` that puts `face`'s baseline where `primary`'s would be. egui centres a
/// fallback face's row in the primary font's row, so a face with a big line gap (Hiragino: 0.5em)
/// otherwise rides about 0.23em high next to Latin text.
fn baseline_offset(face: (f32, f32, f32), primary: (f32, f32, f32)) -> f32 {
    let height = |m: (f32, f32, f32)| m.0 - m.1 + m.2;
    let off = primary.0 - face.0 - 0.5 * (height(primary) - height(face));
    if off.is_finite() { off.clamp(-0.5, 0.5) } else { 0.0 }
}

/// Registers `name` at the lowest priority in every family. Broad faces (Arial Unicode) must come
/// after the script fonts so they never shadow the locale-ordered CJK fonts. The common case (no
/// broad face, or the new face is itself broad) just appends with `add_font`, which composes with
/// the UI font-size plugin's `set_fonts`. When a narrow face arrives after a broad one the append
/// order is wrong, so the definitions are rebuilt in `set_fonts`.
fn add_to_all_families(ctx: &egui::Context, name: String, mut data: FontData, registered: &[(String, bool)]) {
    // Align to Inter, the primary of every UI family; JetBrains Mono differs by under 0.005em.
    let primary = ctx.fonts(|f| f.definitions().font_data.get("Inter").and_then(|p| vertical_metrics(&p.font, p.index)));
    if let (Some(face), Some(primary)) = (vertical_metrics(&data.font, data.index), primary) {
        data.tweak.y_offset_factor = baseline_offset(face, primary);
    }
    crate::theme::size_ui_font(ctx, &name, &mut data);
    let mut families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
    for f in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&f) {
            families.push(f);
        }
    }
    let new_broad = registered.iter().find(|(k, _)| k == &name).is_some_and(|(_, b)| *b);
    let reorder = !new_broad && registered.iter().any(|(_, b)| *b);
    if !reorder {
        let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
        ctx.add_font(FontInsert { name, data, families });
        return;
    }
    // A broad font is already in the stack and a narrow one is arriving: rebuild the family tails
    // so the lazy fonts stay ordered (primaries, then script fonts, then broad fonts).
    let broad_of = |n: &str| registered.iter().find(|(k, _)| k == n).map(|(_, b)| *b);
    let mut fonts = ctx.fonts(|f| f.definitions().clone());
    fonts.font_data.insert(name.clone(), std::sync::Arc::new(data));
    for family in families {
        let stack = fonts.families.entry(family).or_default();
        let mut primaries = Vec::new();
        let mut narrow = Vec::new();
        let mut broad = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for n in std::mem::take(stack).into_iter().chain(std::iter::once(name.clone())) {
            if !seen.insert(n.clone()) {
                continue;
            }
            match broad_of(&n) {
                None => primaries.push(n),
                Some(false) => narrow.push(n),
                Some(true) => broad.push(n),
            }
        }
        *stack = primaries;
        stack.extend(narrow);
        stack.extend(broad);
    }
    ctx.set_fonts(fonts);
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
        let order = self.0.order();
        let missing = missing_chars(ctx, &output.shapes, &order, &self.0.gave_up, &self.0.resolved_kinds, self.0.cjk_done());
        let Some(&c) = missing.first() else { return };
        let progress = |f: &CjkFallback| f.tried.len() + usize::from(f.last_resort_tried) + f.gave_up.len() + f.registered.len();
        let before = progress(&self.0);
        if let Some((name, data)) = self.0.next_font(c) {
            let order = self.0.registered_order();
            add_to_all_families(ctx, name, data, &order);
            ctx.request_repaint();
        } else if progress(&self.0) > before {
            // No font covered this character; it was given up, so try the next one next frame.
            ctx.request_repaint();
        }
    }
}

/// Installs the lazy fallback (system fonts; nothing on the web).
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
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn lazy_fallbacks_follow_ui_font_size_and_survive_size_changes() {
        use photocraft_engine::prefs::UiFontSize;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        crate::theme::set_ui_font_size(&ctx, UiFontSize::Large);
        let name = "test-lazy-fallback".to_string();
        add_to_all_families(&ctx, name.clone(), FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()), &[(name.clone(), false)]);
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
            assert!(ctx.with_plugin::<CjkFontPlugin, _>(|plugin| plugin.0.cjk_done()).expect("font plugin"), "idle frames must not reload fonts");
            crate::i18n::sync_context(&ctx, "ko");
            ctx.with_plugin::<CjkFontPlugin, _>(|plugin| {
                assert!(!plugin.0.cjk_done());
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
            dir
        })
        .clone()
    }

    fn fake(name: &str) -> FontFile {
        FontFile { path: fake_dir().join(name), family: "" }
    }

    /// No non-CJK system fonts (tests pin the CJK and file-based behaviour).
    fn no_by_char(_: char) -> Option<SystemFace> {
        None
    }

    /// A stand-in system face (Inter bytes, which parse but cover no CJK).
    fn face(family: &str, broad: bool) -> SystemFace {
        SystemFace { family: family.into(), bytes: include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec(), index: 0, broad }
    }

    fn fake_sources(locale: fn() -> Option<String>) -> Sources {
        Sources {
            locale,
            // Korean has a readable "font", Japanese only broken files, Chinese none.
            files: |s| match s {
                CjkScript::Korean => vec![fake("missing.ttc"), fake("empty.ttf"), fake("font.ttf")],
                CjkScript::Japanese => vec![fake("empty.ttf")],
                _ => vec![],
            },
            last_resort: || vec![fake("font.ttf")],
            embedded: no_embedded,
            by_char: no_by_char,
        }
    }

    #[test]
    fn picks_preferred_script_skips_unreadable_and_exhausts() {
        let mut fb = CjkFallback::new(fake_sources(|| Some("en_US".into())));
        assert!(!fb.cjk_done());
        // Hangul: Korean first; missing and empty files are skipped.
        let (name, data) = fb.next_font('카').expect("korean font");
        assert!(name.starts_with(FONT_PREFIX));
        assert_eq!(data.index, 0);
        assert_eq!(fb.tried, vec![CjkScript::Korean]);
        // Han on an English locale: SC, TC, JA have nothing readable, and the last-resort file was
        // already loaded, so nothing new; the CJK scripts are now all tried.
        assert!(fb.next_font('圖').is_none());
        assert_eq!(fb.tried, vec![CjkScript::Korean, CjkScript::SimplifiedChinese, CjkScript::TraditionalChinese, CjkScript::Japanese]);
        assert!(fb.cjk_done());
        assert_eq!(fb.registered.len(), 1, "a file is read at most once");
        assert!(fb.next_font('レ').is_none());
    }

    #[test]
    fn han_follows_locale() {
        for (loc, first) in [
            ("ja_JP.UTF-8", CjkScript::Japanese),
            ("zh-Hans-CN", CjkScript::SimplifiedChinese),
            ("zh-Hant-TW", CjkScript::TraditionalChinese),
            ("ko_KR", CjkScript::Korean),
        ] {
            let mut fb = CjkFallback::new(Sources { locale: || None, files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char: no_by_char });
            fb.order = Some(cjk::script_order(Some(loc)));
            assert!(fb.next_font('圖').is_none());
            assert_eq!(fb.tried[0], first, "{loc}");
        }
    }

    /// The same non-CJK face is registered once, however many characters of its script appear.
    #[test]
    fn a_non_cjk_face_is_not_registered_twice() {
        fn by_char(c: char) -> Option<SystemFace> {
            (0x0E00..=0x0E7F).contains(&(c as u32)).then(|| face("Thai Face", false))
        }
        let mut fb = CjkFallback::new(Sources { locale: || Some("en".into()), files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char });
        let (name, _data) = fb.next_font('ส').expect("thai face");
        assert!(name.starts_with(FONT_PREFIX));
        assert_eq!(fb.registered.len(), 1);
        // A second character: the same face is returned, so nothing new is registered.
        assert!(fb.next_font('า').is_none());
        assert_eq!(fb.registered.len(), 1);
    }

    /// Different characters of one script can need different faces (a face covering one need not
    /// cover another), so the script is not retired after the first character.
    #[test]
    fn different_characters_of_a_script_get_their_own_covering_face() {
        fn by_char(c: char) -> Option<SystemFace> {
            match c {
                'ส' => Some(face("Thai One", false)),
                'า' => Some(face("Thai Two", false)),
                _ => None,
            }
        }
        let mut fb = CjkFallback::new(Sources { locale: || Some("en".into()), files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char });
        assert!(fb.next_font('ส').is_some());
        assert!(fb.next_font('า').is_some(), "the second character must still be looked up");
        assert_eq!(fb.registered.len(), 2);
    }

    /// A character no fallback applies to (emoji, Latin-1) yields nothing.
    #[test]
    fn a_character_with_no_fallback_yields_nothing() {
        let mut fb = CjkFallback::new(fake_sources(|| Some("en".into())));
        assert!(fb.next_font('😀').is_none());
        assert!(fb.next_font('é').is_none(), "the bundled fonts cover Latin-1");
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
    fn lazy_registration_ignores_latin_text() {
        let ctx = egui::Context::default();
        install_with(&ctx, fake_sources(|| Some("ko".into())));
        let _ = render(&ctx, "Layer 1 – café");
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 0, "Latin text loads nothing");
        // Hangul isn't in the stand-in font, but the Korean candidate must have been registered.
        let _ = render(&ctx, "카드 배경");
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let name = format!("{FONT_PREFIX}-0");
        assert!(fonts.font_data.contains_key(&name));
        for (fam, stack) in &fonts.families {
            assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
        }
    }

    /// Thai (a non-CJK script) in a UI label lazily registers a covering system face: the
    /// Layers panel draws the Layer name instead of missing-glyph boxes (#2275).
    #[test]
    fn lazy_registration_loads_a_system_face_for_thai() {
        fn by_char(c: char) -> Option<SystemFace> {
            (0x0E00..=0x0E7F).contains(&(c as u32)).then(|| face("Thai Face", false))
        }
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("th".into()), files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char });
        let _ = render(&ctx, "สวัสดีครับ");
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 1, "one Thai system face is registered");
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let name = format!("{FONT_PREFIX}-0");
        assert!(fonts.font_data.contains_key(&name));
        for (fam, stack) in &fonts.families {
            assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
        }
    }

    /// No system fonts at all: craft-fonts alone draws Japanese.
    fn craft_only(locale: fn() -> Option<String>) -> Sources {
        Sources { locale, files: |_| vec![], last_resort: Vec::new, embedded: craft_embedded, by_char: no_by_char }
    }

    /// A Japanese-only set of files, for tests that need one other script to be loadable.
    fn japanese_files(s: CjkScript) -> Vec<FontFile> {
        if s == CjkScript::Japanese { vec![fake("font.ttf")] } else { Vec::new() }
    }

    /// A character with no available font (Thai here) must not stall the scan: a later character
    /// of another script (Japanese) still gets its fallback.
    #[test]
    fn an_uncoverable_character_does_not_block_another_script() {
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("en".into()), files: japanese_files, last_resort: Vec::new, embedded: no_embedded, by_char: no_by_char });
        let _ = render(&ctx, "กレ"); // Thai (no font), then Katakana
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 1, "the Japanese fallback still registers after the Thai character is given up");
    }

    /// Non-classifiable non-ASCII text (Latin Extended, emoji) must not consume the scan budget
    /// and starve a later CJK character.
    #[test]
    fn non_script_text_does_not_consume_the_scan_budget() {
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("en".into()), files: japanese_files, last_resort: Vec::new, embedded: no_embedded, by_char: no_by_char });
        let prefix: String = (0x0100..=0x01FF).filter_map(char::from_u32).collect();
        let _ = render(&ctx, &format!("{prefix}レ"));
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 1, "the later Japanese character is still collected");
    }

    /// A non-CJK character that appears only after the CJK scripts are exhausted must still load:
    /// the loader keeps scanning, with no stale "nothing to do" state.
    #[test]
    fn a_later_non_cjk_character_still_loads_after_cjk_is_exhausted() {
        fn by_char(c: char) -> Option<SystemFace> {
            (0x0E00..=0x0E7F).contains(&(c as u32)).then(|| face("Thai Face", false))
        }
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("en".into()), files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char });
        let _ = render(&ctx, "レ"); // no CJK font: the scripts are tried and exhausted
        let before = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(before, 0);
        let _ = render(&ctx, "สวัสดี"); // Thai appears afterwards
        let after = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(after, 1, "Thai must still load after CJK exhaustion");
    }

    /// A broad face (one that also covers CJK) is placed after the narrow script faces in every
    /// family, so it never shadows them, even when it was registered first.
    #[test]
    fn a_broad_fallback_is_registered_after_a_narrow_one() {
        fn by_char(c: char) -> Option<SystemFace> {
            match c as u32 {
                0x0900..=0x097F => Some(face("Broad", true)),   // Devanagari: only a broad face
                0x0E00..=0x0E7F => Some(face("Narrow", false)), // Thai: a script face
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("en".into()), files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char });
        let _ = render(&ctx, "अ"); // the broad face registers first
        let _ = render(&ctx, "ก"); // the narrow face arrives later
        let fonts = ctx.fonts(|f| f.definitions().clone());
        let broad = format!("{FONT_PREFIX}-0");
        let narrow = format!("{FONT_PREFIX}-1");
        assert!(fonts.font_data.contains_key(&broad) && fonts.font_data.contains_key(&narrow));
        for (fam, stack) in &fonts.families {
            let pb = stack.iter().position(|n| n == &broad).expect("broad in stack");
            let pn = stack.iter().position(|n| n == &narrow).expect("narrow in stack");
            assert!(pn < pb, "{fam:?}: narrow before broad");
        }
    }

    /// A long run of already-covered CJK characters before a Thai one must not consume the scan
    /// budget: a real CJK font resolves the Han kind, so the Thai face still loads.
    #[test]
    fn covered_cjk_does_not_consume_the_scan_budget() {
        if !cjk::font_files(CjkScript::Japanese).iter().any(|f| f.path.is_file()) {
            eprintln!("skipping: no installed Japanese font");
            return;
        }
        fn by_char(c: char) -> Option<SystemFace> {
            (0x0E00..=0x0E7F).contains(&(c as u32)).then(|| face("Thai Face", false))
        }
        fn files(s: CjkScript) -> Vec<FontFile> {
            if s == CjkScript::Japanese { cjk::font_files(s) } else { Vec::new() }
        }
        let ctx = egui::Context::default();
        install_with(&ctx, Sources { locale: || Some("ja".into()), files, last_resort: Vec::new, embedded: no_embedded, by_char });
        let han: String = (0x4E00..0x4E00 + 256).filter_map(char::from_u32).collect();
        let _ = render(&ctx, &format!("{han}สวัสดี"));
        let n = ctx.fonts(|f| f.definitions().font_data.keys().filter(|k| k.starts_with(FONT_PREFIX)).count());
        assert_eq!(n, 2, "the Japanese and Thai faces are both registered");
    }

    #[test]
    fn craft_fonts_render_japanese_without_system_fonts() {
        let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied() else {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        };
        // An English locale tries Chinese first (none here), then Japanese: the craft font.
        for locale in [(|| Some("en_US".into())) as fn() -> Option<String>, || Some("ja_JP".into()), || None] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts_with(&ctx, craft_only(locale));
            assert!(render(&ctx, "日本語の文字"), "tofu in 日本語の文字");
            assert!(render(&ctx, "レイヤー 1"), "tofu in kana");
            let fonts = ctx.fonts(|f| f.definitions().clone());
            let name = format!("{FONT_PREFIX}-0");
            assert!(fonts.font_data.get(&name).is_some_and(|d| std::ptr::eq(d.font.as_ref().as_ptr(), ui_font.bytes.as_ptr())), "BIZ UDPGothic Regular first");
            for (fam, stack) in &fonts.families {
                assert_eq!(stack.last(), Some(&name), "{fam:?}: appended last so Latin keeps Inter");
                assert!(stack.first().is_some_and(|f| f != &name), "{fam:?}");
            }
        }
    }

    #[test]
    fn ui_works_without_craft_fonts() {
        // The pre-craft-fonts behaviour: no embedded fonts, no system fonts. Latin renders,
        // Japanese is tried (and stays missing) without panicking, and the loader exhausts.
        let ctx = egui::Context::default();
        crate::theme::install_fonts_with(
            &ctx,
            Sources { locale: || None, files: |_| vec![], last_resort: Vec::new, embedded: no_embedded, by_char: no_by_char },
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
