//! #540: the font menu shows each family in its own face and the recently used fonts first.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::prefs::FontPreview;
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn installed() -> &'static [String] {
    crate::type_tool::families()
}

#[test]
fn preview_sizes_follow_the_preference_and_off_hides_samples() {
    assert_eq!(sample_height(FontPreview::Off), None);
    let sizes: Vec<f32> = [FontPreview::Small, FontPreview::Medium, FontPreview::Large, FontPreview::ExtraLarge, FontPreview::Huge]
        .into_iter()
        .filter_map(sample_height)
        .collect();
    assert_eq!(sizes.len(), 5);
    assert!(sizes.windows(2).all(|w| w[0] < w[1]), "{sizes:?}");
}

#[test]
fn recent_fonts_are_newest_first_without_duplicates_and_capped() {
    let mut app = app();
    let fams = installed();
    assert!(fams.len() >= 2, "the bundled fonts give at least two families");
    let (a, b) = (&fams[0], &fams[1]);
    for f in [a, b, a] {
        note_used(&mut app, f);
    }
    note_used(&mut app, "No Such Font");
    assert_eq!(recent(&app, fams), vec![a.clone(), b.clone()], "uninstalled fonts aren't listed");
    app.session.prefs.edit(|p| p.type_.recent_fonts = 1);
    assert_eq!(recent(&app, fams), vec![a.clone()], "Number of Recent Fonts to Display");
    app.session.prefs.edit(|p| p.type_.recent_fonts = 0);
    assert!(recent(&app, fams).is_empty());
    for i in 0..80 {
        note_used(&mut app, &format!("Font {i}"));
    }
    assert_eq!(app.session.prefs().type_.recent_font_list.len(), RECENT_MAX);
}

#[test]
fn type_menu_font_preview_size_is_the_preference() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::view_cmds::invoke(&mut app, &ctx, "type.fontPreviewSize.large", &json!({})).unwrap().unwrap();
    assert_eq!(app.session.prefs().type_.font_preview, FontPreview::Large);
    assert_eq!(crate::view_cmds::checked(&app, "type.fontPreviewSize.large"), Some(true));
    assert_eq!(crate::view_cmds::checked(&app, "type.fontPreviewSize.medium"), Some(false));
}

#[test]
fn samples_render_within_a_frame_budget_and_are_cached() {
    let ctx = egui::Context::default();
    let fam = photocraft_text::fonts::DEFAULT_FAMILY;
    let mut budget = 0;
    assert!(sample(&ctx, fam, 20, &mut budget).is_none(), "no budget left: drawn on a later frame");
    budget = 1;
    let tex = sample(&ctx, fam, 20, &mut budget).unwrap();
    assert_eq!((tex.size()[1], budget), (20, 0));
    assert!(tex.size()[0] > 30, "as wide as the word: {:?}", tex.size());
    assert!(sample(&ctx, fam, 20, &mut budget).is_some(), "cached: no budget needed");
    let mut cache = Samples::default();
    for i in 0..CACHE_CAP + 5 {
        cache.insert((format!("F{i}"), 20), None);
    }
    assert_eq!(cache.map.len(), CACHE_CAP);
    assert!(cache.get(&("F0".into(), 20)).is_none() && cache.get(&(format!("F{}", CACHE_CAP + 4), 20)).is_some(), "the oldest went first");
}

#[test]
fn the_menu_lists_recent_fonts_first_and_remembers_a_pick() {
    let fams = installed();
    assert!(fams.len() >= 3, "the bundled fonts give at least three families");
    let mut app = app();
    app.ui.tool_options.type_font = fams[0].clone();
    note_used(&mut app, &fams[2]);
    let mut h = Harness::builder().with_size(vec2(600.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let mut f = app.ui.tool_options.type_font.clone();
            if picker(app, ui, "test-font", &mut f, 200.0) {
                app.ui.tool_options.type_font = f;
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(3);
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run_steps(4);
    let recent: Vec<_> = h.query_all_by_label(&fams[2]).map(|n| n.rect().min.y).collect();
    assert_eq!(recent.len(), 2, "listed among the recent fonts and in the full list");
    assert!(recent[0] < h.query_all_by_label(&fams[1]).map(|n| n.rect().min.y).fold(f32::MAX, f32::min), "the recent section comes first");
    if fams.len() > 200 {
        assert_eq!(h.query_all_by_label(&fams[fams.len() - 1]).count(), 0, "rows below the fold aren't laid out");
    }
    h.query_all_by_label(&fams[1]).last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.type_font, fams[1]);
    assert_eq!(h.state().session.prefs().type_.recent_font_list[..2], [fams[1].clone(), fams[2].clone()]);
}
