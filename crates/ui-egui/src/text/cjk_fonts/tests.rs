use super::*;
use craft::ui::fonts::{FontRef, I18nScripts, SystemFonts, UiFonts};
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FontData, FontFamily, FontId};
use photocraft_text::scripts;

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

/// The craft manifest's face of `family` in `style`, as the stack names it.
fn craft_name(family: &str, style: &str) -> Option<String> {
    craft_faces().iter().find(|face| face.family == family && face.style == style).map(EmbeddedFace::name)
}

/// The first face of the proportional chain with a Han script: what draws shared Han and kana.
fn first_han_face(ctx: &egui::Context) -> Option<String> {
    let han: Vec<String> =
        craft_faces().iter().filter(|face| ["Jpan", "Hans", "Hant", "Kore"].iter().any(|tag| face.has_script(tag))).map(EmbeddedFace::name).collect();
    ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].iter().find(|name| han.contains(name)).cloned())
}

/// A face added after the install, as the on-demand loader and the served-face loader add one: at the lowest
/// priority of both UI families.
fn add_late_face(ctx: &egui::Context, name: &str) {
    let families =
        [FontFamily::Proportional, FontFamily::Monospace].into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    ctx.add_font(FontInsert::new(name, FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()), families));
}

#[test]
fn lazy_fallbacks_follow_ui_font_size_and_survive_size_changes() {
    use photocraft_engine::prefs::UiFontSize;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    crate::theme::set_ui_font_size(&ctx, UiFontSize::Large);
    add_late_face(&ctx, "test-lazy-fallback");
    for (size, scale) in [(UiFontSize::Large, 16.0 / 12.0), (UiFontSize::Tiny, 10.0 / 12.0), (UiFontSize::Small, 1.0)] {
        crate::theme::set_ui_font_size(&ctx, size);
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        }
        let fonts = ctx.fonts(|f| f.definitions().clone());
        assert_eq!(fonts.font_data.get("test-lazy-fallback").unwrap().tweak.scale, scale);
        if let Some(face) = craft_faces().first() {
            assert_eq!(fonts.font_data.get(&face.name()).map(|data| data.tweak.scale), Some(scale), "a craft face is sized like the rest");
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let stack = &fonts.families[&family];
            assert_eq!(stack.iter().filter(|n| *n == "test-lazy-fallback").count(), 1);
            assert_eq!(stack.last().map(String::as_str), Some("test-lazy-fallback"), "fallback stays at the lowest priority");
        }
    }
    // A face that arrives while the size is already set is sized without a size change.
    crate::theme::set_ui_font_size(&ctx, UiFontSize::Medium);
    for _ in 0..2 {
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    }
    add_late_face(&ctx, "test-later-fallback");
    for _ in 0..3 {
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    }
    let fonts = ctx.fonts(|f| f.definitions().clone());
    assert_eq!(fonts.font_data.get("test-later-fallback").unwrap().tweak.scale, 14.0 / 12.0);
    assert_eq!(fonts.font_data.get("test-lazy-fallback").unwrap().tweak.scale, 14.0 / 12.0, "one rescale, not one per pass");
}

#[test]
fn system_fonts_cover_other_script_samples() {
    // Like the CJK system-font test, run per script when this machine has a font for it.
    let samples = [("ภาพถ่าย/วันหยุดพักผ่อน", 'ภ'), ("مرحبا", 'م'), ("שלום", 'ש'), ("नमस्ते", 'न')];
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for (text, c) in samples {
        let Some(script) = scripts::classify(c) else { continue };
        if !scripts::system_candidates(script).iter().any(|p| std::fs::read(p).ok().and_then(|b| scripts::covering_face(&b, c)).is_some()) {
            eprintln!("skipping {text}: no {script:?} system font");
            continue;
        }
        assert!(render(&ctx, text), "{text} still has missing glyphs");
    }
}

/// A font collection of the two faces `first` and `second`, each table directory rewritten for offsets from the start of
/// the collection.
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

/// A minimal TrueType font mapping the `ranges` (inclusive) to box glyphs, so tests have a font for a script without
/// committing font files.
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

/// A craft face installs as face 0, the one `FontData::from_static` uses, so a character only a later face of a
/// collection maps is not drawn from it: here face 0 maps Thai only and face 1 maps 龍.
#[test]
fn craft_face_coverage_matches_the_face_craft_insert_installs() {
    let bytes: &'static [u8] = Box::leak(two_face_ttc(&synthetic_font(&[(0x0E01, 0x0E5B)]), &synthetic_font(&[(0x9F8D, 0x9F8D)])).into_boxed_slice());
    let face = CraftFont { family: "Synthetic Collection", style: "Regular", scripts: &["Hans"], bytes };
    let listed = embedded(&[face]);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].index, 0, "the stack installs the collection's first face");

    let ctx = egui::Context::default();
    UiFonts::new(&I18nScripts)
        .app_face("inter", FontData::from_static(photocraft_text::fonts::INTER_REGULAR.as_slice()), &[FontRef::proportional(), FontRef::monospace()])
        .craft_faces(&listed)
        .ui_language("zh-hans")
        .system_fonts(SystemFonts::Off)
        .install(&ctx);
    render(&ctx, "ก龍");
    assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), 'ก')), "face 0 maps Thai");
    assert!(!ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(12.0), '龍')), "face 1 is not installed, so 龍 has no glyph");
}

/// A craft entry without bytes has no face to install: the stack must not name one egui cannot resolve. The faces
/// that remain keep the manifest's order and install as face 0, the one `FontData::from_static` uses.
#[test]
fn craft_face_selection_ignores_empty_and_unindexed_faces() {
    let faces = [
        CraftFont { family: "First", style: "Regular", scripts: &["Jpan"], bytes: b"first" },
        CraftFont { family: "Empty", style: "Regular", scripts: &["Zsym"], bytes: &[] },
        CraftFont { family: "Second", style: "Bold", scripts: &["Hans", "Latn"], bytes: b"second" },
    ];
    let listed = embedded(&faces);
    assert_eq!(listed.iter().map(EmbeddedFace::name).collect::<Vec<_>>(), ["First Regular", "Second Bold"]);
    assert!(listed.iter().all(|face| face.index == 0));
    assert!(embedded(&[]).is_empty(), "a build without craft-fonts has no faces");
}

/// With no Han preference the stack keeps the manifest order `craft_faces` passes, so the manifest's first Japanese face
/// with bytes leads shared Han and kana, ahead of the Japanese interface face.
#[test]
fn craft_font_provider_keeps_the_baseline_face_for_unknown_locales() {
    let manifest_first = craft_fonts::CRAFT_FONTS.iter().find(|face| !face.bytes.is_empty() && face.scripts.contains(&"Jpan")).map(|face| face.family);
    let (Some(ui_face), Some(baseline)) = (craft_fonts::japanese_for_ui().first().copied(), manifest_first) else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    if ui_face.family == baseline {
        return;
    }
    let stack = UiFonts::new(&I18nScripts).craft_faces(&craft_faces()).ui_language("en").stack();
    let chain = stack.chain(&FontRef::proportional());
    let at = |family: &str| chain.iter().position(|name| name.starts_with(family));
    let (kept, ui) = (at(baseline), at(ui_face.family));
    assert!(kept.is_some() && kept < ui, "the baseline face leads shared Han: {kept:?} {ui:?}");
}

/// CJK text draws from the craft faces of a UI language that names a Han script, and the first Han face is
/// that script's own.
#[test]
fn craft_fonts_render_cjk_without_system_fonts() {
    let Some(ui_font) = craft_fonts::japanese_for_ui().first().copied().and_then(|f| craft_name(f.family, f.style)) else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let Some(chinese) = craft_name("Noto Sans CJK SC", "Regular") else {
        eprintln!("skipping: the craft-fonts manifest has no Simplified Chinese face");
        return;
    };
    for (code, expected) in [("ja", ui_font), ("zh-hans", chinese)] {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            let ctx = egui::Context::default();
            crate::i18n::sync_context(&ctx, code);
            assert!(render(&ctx, "日本語の文字"), "tofu in 日本語の文字");
            assert!(render(&ctx, "レイヤー 1"), "tofu in kana");
            assert_eq!(first_han_face(&ctx), Some(expected), "{code}");
        });
    }
}

/// The craft faces are in the stack the install builds, so the first pass after it draws their text in both UI families
/// even under the strict missing-glyph policy: nothing waits for a loader.
#[test]
fn craft_faces_draw_the_first_frame_in_both_families() {
    if craft_name("BIZ UDPGothic", "Regular").is_none() || craft_name("Noto Sans CJK SC", "Regular").is_none() {
        eprintln!("skipping: the craft-fonts manifest lacks a Japanese or a Simplified Chinese face");
        return;
    }
    for (code, text) in [("ja", "かな"), ("zh-hans", "中文")] {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            let ctx = egui::Context::default();
            ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
            crate::i18n::sync_context(&ctx, code);
            ctx.run_ui(Default::default(), |ui| {
                ui.label(text);
                ui.label(egui::RichText::new(text).monospace());
            })
            .textures_delta
            .clear();
            for family in [FontId::proportional(12.0), FontId::monospace(12.0)] {
                assert!(ctx.fonts_mut(|f| f.has_glyphs(&family, text)), "{text} is missing from {:?}", family.family);
            }
        });
    }
}

/// Changing the UI language installs the stack again in the new language's order.
#[test]
fn language_change_reloads_eager_faces_through_the_app_context() {
    let (Some(ui_font), Some(chinese)) =
        (craft_fonts::japanese_for_ui().first().copied().and_then(|f| craft_name(f.family, f.style)), craft_name("Noto Sans CJK SC", "Regular"))
    else {
        eprintln!("skipping: the craft-fonts manifest lacks a Japanese or a Simplified Chinese face");
        return;
    };
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let ctx = egui::Context::default();
        crate::i18n::sync_context(&ctx, "ja");
        render(&ctx, "日");
        assert_eq!(first_han_face(&ctx), Some(ui_font));
        crate::i18n::sync_context(&ctx, "zh-hans");
        render(&ctx, "日");
        assert_eq!(first_han_face(&ctx), Some(chinese), "the language reload puts the Chinese face first");
    });
}

/// A craft face is in a family's stack once.
#[test]
fn lazy_loader_registers_an_embedded_craft_face_once() {
    if craft_faces().is_empty() {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::install_fonts(&ctx);
    render(&ctx, "日本語");
    let fonts = ctx.fonts(|f| f.definitions().clone());
    for (family, stack) in &fonts.families {
        for face in craft_faces() {
            assert!(stack.iter().filter(|name| **name == face.name()).count() <= 1, "{} twice in {family:?}", face.name());
        }
    }
}

/// Every script the build embeds a face for draws in both UI families (a headless frame may use either).
#[test]
fn every_craft_face_is_available_to_both_ui_families() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut checked = 0;
    for (script, text) in [("Jpan", "日本語"), ("Kore", "한"), ("Arab", "مرحبا"), ("Deva", "नमस्ते")] {
        if !craft_faces().iter().any(|face| face.has_script(script)) {
            continue;
        }
        render(&ctx, text);
        for family in [FontId::proportional(12.0), FontId::monospace(12.0)] {
            assert!(ctx.fonts_mut(|f| f.has_glyphs(&family, text)), "{text} is missing from {:?}", family.family);
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
    }
}

#[test]
fn ui_works_without_craft_fonts() {
    // The pre-craft-fonts behaviour: Latin renders, other scripts draw only where the machine has a font, and
    // nothing panics under the default policy.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    assert!(render(&ctx, "Layer 1 – café"));
    let _ = render(&ctx, "日本語の文字");
    if craft_fonts::CRAFT_FONTS.is_empty() {
        assert!(craft_faces().is_empty());
    }
}

/// With the real system fonts, every CJK sample renders (skipped per script when the
/// machine has no font for it, e.g. CI Linux without Noto CJK).
#[test]
fn system_fonts_cover_cjk_samples() {
    use photocraft_text::cjk::{self, CjkScript};
    #[cfg(target_os = "linux")]
    assert!(wait_for_fontconfig_cjk_scan(), "Fontconfig CJK lookup did not finish within 10 seconds");
    let available = |s: CjkScript| cjk::font_files(s).iter().any(|f| f.path.is_file());
    let samples =
        [("图层", CjkScript::SimplifiedChinese), ("圖層", CjkScript::TraditionalChinese), ("카드 배경", CjkScript::Korean), ("レイヤー", CjkScript::Japanese)];
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
