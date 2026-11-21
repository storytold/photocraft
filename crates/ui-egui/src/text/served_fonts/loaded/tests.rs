use super::*;
use craft::ui::fonts::{I18nScripts, SystemFonts};

fn inter() -> &'static [u8] {
    photocraft_text::fonts::INTER_REGULAR.as_slice()
}

#[test]
fn loaded_faces_are_owned_deduplicated_and_bounded() {
    let mut loaded = Loaded::default();
    let families = vec!["Inter".into()];
    assert!(!loaded.add(b"not a font", &families));
    assert!(!loaded.add(inter(), &["different manifest family".into()]));
    assert!(loaded.add(inter(), &families));
    assert!(!loaded.add(inter(), &families));
    assert_eq!((loaded.files.len(), loaded.faces, loaded.bytes), (1, 1, inter().len()));
    let clone = loaded.clone();
    assert!(Arc::ptr_eq(&loaded.files[0].bytes, &clone.files[0].bytes));
    assert_eq!(loaded.selected(&families)[0].1.index, 0);
    let mut full = Loaded { bytes: MAX_BYTES, ..Loaded::default() };
    assert!(!full.add(inter(), &families));
    let mut full = Loaded { faces: MAX_FACES, ..Loaded::default() };
    assert!(!full.add(inter(), &families));
}

// Build a collection from attributed bundled fonts, relocating its table offsets.
// No font fixture is added to the repository.
fn collection(fonts: &[&[u8]]) -> Vec<u8> {
    let mut out = b"ttcf\0\x01\0\0".to_vec();
    out.extend_from_slice(&(fonts.len() as u32).to_be_bytes());
    out.resize(12 + 4 * fonts.len(), 0);
    for (i, font) in fonts.iter().enumerate() {
        let start = out.len();
        out[12 + 4 * i..16 + 4 * i].copy_from_slice(&(start as u32).to_be_bytes());
        out.extend_from_slice(font);
        let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
        for table in 0..tables {
            let offset = start + 12 + table * 16 + 8;
            let value = u32::from_be_bytes(out[offset..offset + 4].try_into().unwrap());
            out[offset..offset + 4].copy_from_slice(&(value + start as u32).to_be_bytes());
        }
    }
    out
}

#[test]
fn collection_uses_actual_regular_face_index_not_the_first_face() {
    let medium = photocraft_text::fonts::INTER_MEDIUM.as_slice();
    let ttc = collection(&[medium, inter()]);
    let mut loaded = Loaded::default();
    let families = vec!["Inter".into()];
    assert!(loaded.add(&ttc, &families));
    assert_eq!(loaded.faces, 2);
    let selected = loaded.selected(&families);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].1.index, 1, "regular, not the collection's first (medium) face");
    assert!(selected[0].0.ends_with("-1"));
}

#[test]
fn runtime_delivery_reaches_all_ui_families_and_survives_reinstalls() {
    let _catalog = super::super::changing_catalog();
    let ctx = egui::Context::default();
    // A real attributed font posing as the host's script fallback, independent of optional
    // platform Arabic/Japanese font installations.
    photocraft_text::served::add_fonts(&super::super::parse_manifest("Inter | Regular | fonts/Inter.ttf | Arab").0);
    let mut app = crate::PhotocraftApp::with_context(ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
    super::super::install(&mut app, &ctx, vec![("Inter".into(), inter().to_vec())]);
    for _ in 0..2 {
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    }
    for size in [photocraft_engine::prefs::UiFontSize::Large, photocraft_engine::prefs::UiFontSize::Small] {
        crate::theme::set_ui_font_size(&ctx, size);
        crate::theme::install_fonts(&ctx);
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        }
        let definitions = ctx.fonts(|fonts| fonts.definitions().clone());
        for family in
            [egui::FontFamily::Proportional, egui::FontFamily::Monospace, egui::FontFamily::Name("medium".into()), egui::FontFamily::Name("semibold".into())]
        {
            let chain = &definitions.families[&family];
            let hosts: Vec<_> = chain.iter().filter(|name| name.starts_with("photocraft-host-font-")).collect();
            assert_eq!(hosts.len(), 1);
            assert_ne!(chain.first(), hosts.first().copied(), "bundled primary face remains first");
            let expected = if size == photocraft_engine::prefs::UiFontSize::Large { 16.0 / 12.0 } else { 1.0 };
            assert_eq!(definitions.font_data[hosts[0]].tweak.scale, expected);
        }
    }
    assert!(!cache(&ctx, inter()), "repeated delivery neither copies nor reinstalls");
    for language in ["ar", "ja"] {
        let fonts = append(&ctx, UiFonts::new(&I18nScripts).system_fonts(SystemFonts::Off), language);
        fonts.install(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        assert!(ctx.fonts_mut(|fonts| fonts.has_glyphs(&egui::FontId::proportional(12.0), "Hello")));
    }
}

#[test]
fn selection_follows_requested_family_order() {
    let mut loaded = Loaded::default();
    let families = vec!["Inter".into(), "JetBrains Mono".into()];
    assert!(loaded.add(inter(), &families));
    assert!(loaded.add(photocraft_text::fonts::JETBRAINS_MONO_REGULAR.as_slice(), &families));
    let forward = loaded.selected(&families);
    let reverse = loaded.selected(&families.into_iter().rev().collect::<Vec<_>>());
    assert_eq!(forward[0].0, reverse[1].0);
    assert_eq!(forward[1].0, reverse[0].0);
}

#[test]
fn italic_faces_do_not_replace_upright_ui_fallbacks() {
    let mut italic = inter().to_vec();
    let tables = u16::from_be_bytes([italic[4], italic[5]]) as usize;
    let table = (0..tables).map(|i| 12 + 16 * i).find(|&i| &italic[i..i + 4] == b"OS/2").unwrap();
    let offset = u32::from_be_bytes(italic[table + 8..table + 12].try_into().unwrap()) as usize;
    italic[offset + 63] |= 1; // OS/2 fsSelection ITALIC bit.
    assert!(read_faces_bytes(&italic)[0].italic);
    assert!(!Loaded::default().add(&italic, &["Inter".into()]));
}
