use super::*;

fn text_sizes(ctx: &egui::Context) -> Vec<egui::Vec2> {
    let mut sizes = Vec::new();
    // The plugin queues changed fonts at the end of a pass; egui applies them next pass.
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    ctx.run_ui(Default::default(), |ui| {
        // Include custom painter fonts and explicit RichText sizes, not just text styles.
        for font in [FontId::proportional(11.5), medium(12.0), semibold(15.0), mono(12.0), TextStyle::Body.resolve(ui.style())] {
            sizes.push(ui.painter().layout_no_wrap("Interface 123".into(), font, Color32::WHITE).size());
        }
        sizes.push(ui.label(egui::RichText::new("Interface 123").size(13.0)).rect.size());
    })
    .textures_delta
    .clear();
    sizes
}

#[test]
fn ui_font_sizes_scale_shaping_and_row_height_without_accumulating() {
    let ctx = egui::Context::default();
    install_fonts(&ctx);
    apply(&ctx, &AppTheme::pro());
    let original = text_sizes(&ctx);
    let definitions = ctx.fonts(|f| f.definitions().clone());
    for size in [UiFontSize::Tiny, UiFontSize::Small, UiFontSize::Medium, UiFontSize::Large, UiFontSize::Tiny, UiFontSize::Small] {
        set_ui_font_size(&ctx, size);
        let measured = text_sizes(&ctx);
        for (before, after) in original.iter().zip(&measured) {
            let expected = *before * font_scale(size);
            assert!((after.x - expected.x).abs() < 1.0, "{size:?}: width {after:?} vs {expected:?}");
            // egui rounds ascent/descent and the final row height to pixels separately.
            assert!((after.y - expected.y).abs() < 1.1, "{size:?}: height {after:?} vs {expected:?}");
        }
        set_ui_font_size(&ctx, size);
        assert_eq!(text_sizes(&ctx), measured, "setting the same size twice is stable");
    }
    assert_eq!(ctx.fonts(|f| f.definitions().clone()), definitions, "Small restores original font tweaks exactly");
    assert_eq!(ctx.zoom_factor(), 1.0);
}

// Regression test for #4630: the stock thin scroll bar fades its handle to the same opacity
// as its track while the pointer is on it, so the thumb disappeared when it was grabbed.
#[test]
fn scrollbar_thumb_stays_more_visible_than_its_track_in_every_theme() {
    for kind in AppTheme::all() {
        let ctx = egui::Context::default();
        apply(&ctx, &kind);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let scroll = ctx.style_of(theme).spacing.scroll;
            if !scroll.floating {
                continue;
            }
            assert!(
                scroll.interact_handle_opacity > scroll.interact_background_opacity,
                "{kind:?}/{theme:?}: handle {} vs track {} under the pointer",
                scroll.interact_handle_opacity,
                scroll.interact_background_opacity
            );
            assert_eq!(scroll.interact_handle_opacity, 1.0, "{kind:?}/{theme:?}: the thumb is opaque under the pointer");
        }
    }
}

/// The desktop app installs fonts and loads the saved UI Font Size in its first frame, while
/// egui's default fonts are still active. Any size but Small then panicked on the next pass
/// ("FontFamily::Name("medium") is not bound to any fonts") because the plugin rescaled a
/// copy of the defaults and that replaced the queued Inter stack.
#[test]
fn saved_ui_font_size_survives_first_frame_install() {
    let ctx = egui::Context::default();
    let medium_bound = |ctx: &egui::Context| ctx.fonts(|f| f.families().contains(&FontFamily::Name("medium".into())));
    ctx.run_ui(Default::default(), |ui| {
        assert!(!medium_bound(ui.ctx()), "egui's defaults are active in the first frame");
        install_fonts(ui.ctx());
        apply(ui.ctx(), &AppTheme::pro());
        set_ui_font_size(ui.ctx(), UiFontSize::Medium);
    })
    .textures_delta
    .clear();
    let mut width = 0.0;
    ctx.run_ui(Default::default(), |ui| {
        assert!(medium_bound(ui.ctx()), "the installed stack reaches the second pass");
        width = ui.painter().layout_no_wrap("PhotoCraft".into(), medium(13.0), Color32::WHITE).size().x;
    })
    .textures_delta
    .clear();
    assert!(width > 0.0);
    // The saved size applies one pass later, from the installed stack.
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let mut scaled = 0.0;
    ctx.run_ui(Default::default(), |ui| {
        assert!(medium_bound(ui.ctx()));
        scaled = ui.painter().layout_no_wrap("PhotoCraft".into(), medium(13.0), Color32::WHITE).size().x;
    })
    .textures_delta
    .clear();
    assert!((scaled - width * font_scale(UiFontSize::Medium)).abs() < 1.0, "{scaled} vs {width} × 14/12");
    // Re-installing (a language change) keeps the size and the named families.
    ctx.run_ui(Default::default(), |ui| install_fonts(ui.ctx())).textures_delta.clear();
    for _ in 0..2 {
        ctx.run_ui(Default::default(), |ui| {
            assert!(medium_bound(ui.ctx()));
            let w = ui.painter().layout_no_wrap("PhotoCraft".into(), medium(13.0), Color32::WHITE).size().x;
            assert!((w - scaled).abs() < 1.0, "{w} vs {scaled}");
        })
        .textures_delta
        .clear();
    }
}

#[test]
fn every_palette_keeps_complete_egui_styles_when_the_os_changes() {
    for kind in AppTheme::all() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() }, |ui| apply(ui.ctx(), &kind)).textures_delta.clear();
        let expected = ctx.global_style();
        for appearance in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            ctx.run_ui(egui::RawInput { system_theme: appearance, ..Default::default() }, |_| {}).textures_delta.clear();
            assert_eq!(Tokens::get(&ctx), kind.tokens());
            assert_eq!(ctx.global_style(), expected, "{kind:?} changed with {appearance:?}");
            assert_eq!(ctx.style_of(egui::Theme::Light), expected);
            assert_eq!(ctx.style_of(egui::Theme::Dark), expected);
        }
    }
}

#[test]
fn applying_a_palette_clears_a_pinned_native_appearance_preference() {
    for pinned in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(pinned);
        apply(&ctx, &AppTheme::pro_medium());
        assert_eq!(ctx.options(|o| o.theme_preference), egui::ThemePreference::System);
    }
}

/// WCAG 2 contrast ratio, the measure libadwaita's own palette is held to.
fn contrast(a: Color32, b: Color32) -> f32 {
    let lin = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let lum = |c: Color32| 0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b());
    let (hi, lo) = if lum(a) > lum(b) { (lum(a), lum(b)) } else { (lum(b), lum(a)) };
    (hi + 0.05) / (lo + 0.05)
}

/// The light themes outline their white slider knob so its edge holds WCAG's 3:1 non-text
/// contrast against the panels sliders sit on (#7927); the dark ones keep the plain knob.
#[test]
fn light_themes_outline_the_slider_knob() {
    for kind in AppTheme::all() {
        let t = kind.tokens();
        // The Pro layouts keep their dark ring (it rims a grey knob), Classic its bevelled knob.
        if t.pro || t.bevel {
            continue;
        }
        match t.knob_outline() {
            Some(outline) => {
                assert!(!t.dark(), "{kind:?}");
                for panel in [t.dock, t.card, t.chrome] {
                    assert!(contrast(outline, panel) >= 3.0, "{kind:?}: knob outline {outline:?} on {panel:?}");
                }
            }
            None => assert!(t.dark(), "{kind:?}: a light theme without a knob outline"),
        }
    }
    assert_eq!(AppTheme::pro().tokens().knob_outline(), Some(Color32::from_gray(40)), "Pro is unchanged");
    for kind in [AppTheme::studio_light(), AppTheme::adwaita(), AppTheme::breeze_light(), AppTheme::soft_light()] {
        assert!(kind.tokens().knob_outline().is_some(), "{kind:?}");
    }
}

/// Every palette's labels and icons read on the surfaces they sit on, its text keeps a hierarchy,
/// and its button labels read on their fills. The floors are those the lowest-contrast palette
/// (Solarized Dark, whose base0 text is the palette's own) holds.
#[test]
fn every_palette_holds_text_contrast() {
    for kind in AppTheme::all() {
        let t = kind.tokens();
        for surface in [t.chrome, t.dock, t.card, t.field] {
            assert!(contrast(t.text, surface) >= 4.0, "{kind:?}: text on {surface:?}");
            assert!(contrast(t.icon, surface) >= 4.0, "{kind:?}: icon on {surface:?}");
            assert!(contrast(t.text_dim, surface) >= 2.5, "{kind:?}: dim text on {surface:?}");
            assert!(contrast(t.text_faint, surface) >= 2.0, "{kind:?}: faint text on {surface:?}");
        }
        assert!(contrast(t.text, t.card) > contrast(t.text_dim, t.card) && contrast(t.text_dim, t.card) > contrast(t.text_faint, t.card), "{kind:?}");
        assert!(contrast(t.primary_text, t.primary_bg) >= 3.0, "{kind:?}: primary button label");
        assert!(contrast(t.caption_close_text, t.caption_close) >= 4.0, "{kind:?}: close button glyph");
    }
}
