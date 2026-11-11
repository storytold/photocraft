//! Issue #11753: English Sample labels and stable ids; catalog changes live in craft-i18n.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::PhotocraftApp;
use crate::i18n::{self, Lang};
use crate::state::Tool;
use crate::theme::ThemeKind;

const IDS: [&str; 5] = ["current", "currentAndBelow", "all", "allNoAdjustments", "currentAndBelowNoAdjustments"];
const LABELS: [&str; 5] = ["Current Layer", "Current & Below", "All Layers", "All Layers no Adjustments", "Current & Below no Adjustments"];
const OLD_LABELS: [&str; 2] = ["All Layers No Adjustments", "Current & Below No Adjustments"];

#[test]
fn all_layers_label_uses_lower_case_no() {
    i18n::with_language(Lang::EN, || {
        assert_eq!(super::layer_options().get(3).unwrap().1, "All Layers no Adjustments");
    });
}

#[test]
fn current_and_below_label_uses_lower_case_no() {
    i18n::with_language(Lang::EN, || {
        assert_eq!(super::layer_options().get(4).unwrap().1, "Current & Below no Adjustments");
    });
}

#[test]
fn sample_layer_ids_keep_the_exact_five_entry_order() {
    i18n::with_language(Lang::EN, || {
        assert_eq!(super::layer_options().map(|(id, _)| id), IDS.map(str::to_owned));
    });
}

fn options_bar(lang: Lang, kind: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", serde_json::json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Eyedropper;
    app.ui.tool_options.eyedropper_sample = "current".into();
    app.ui.tool_options.eyedropper_size = 31;
    app.ui.tool_options.eyedropper_ring = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 600.0))
        // Construction draws immediately: install the production named font families first.
        .with_context_setup(move |ctx| i18n::with_language(lang, || PhotocraftApp::setup_context(ctx, kind)))
        .build_ui_state(move |ui, app| i18n::with_language(lang, || crate::panels::options_bar(app, ui)), app);
    h.run_steps(3);
    h
}

/// Inspect production paint output, not only an accessible name that may be clipped away.
/// This is a text/clip geometry oracle, not a GPU pixel or native-window snapshot.
fn painted_text_in(shape: &egui::Shape, clip: egui::Rect, label: &str, target: egui::Rect) -> bool {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| painted_text_in(shape, clip, label, target)),
        egui::Shape::Text(text) if text.galley.text() == label && text.opacity_factor > 0.0 => {
            let visible = text.visual_bounding_rect().intersect(clip).intersect(target);
            visible.is_positive()
        }
        _ => false,
    }
}

fn assert_painted(h: &Harness<'_, PhotocraftApp>, label: &str, target: egui::Rect) {
    assert!(h.output().shapes.iter().any(|shape| painted_text_in(&shape.shape, shape.clip_rect, label, target)), "{label:?} is painted in {target:?}");
}

fn open_sample_menu(h: &mut Harness<'_, PhotocraftApp>, lang: Lang) {
    let source = i18n::tr(lang, "Source");
    let combo = h.get_by_role_and_label(Role::ComboBox, source);
    assert!(combo.rect().is_positive());
    combo.click();
    h.run_steps(3);
}

fn choose_sample(h: &mut Harness<'_, PhotocraftApp>, lang: Lang, id: &str, label: &str) {
    let mut expected = h.state().ui.tool_options.clone();
    expected.eyedropper_sample = id.into();
    open_sample_menu(h, lang);
    // These five rows fit in the production 420-point popup. Verify actual paint at the
    // freshly queried physical click target rather than clicking a hidden AccessKit row.
    let row = h.get_by_role_and_label(Role::Button, label);
    assert!(row.rect().is_positive());
    assert_painted(h, label, row.rect());
    row.click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options, expected, "only the Sample id changes");
    let combo = h.get_by_role_and_label(Role::ComboBox, i18n::tr(lang, "Source"));
    assert_painted(h, label, combo.rect());
    assert!(h.query_by_role_and_label(Role::Button, label).is_none(), "the selected popup closes");
}

#[test]
fn sample_dropdown_paints_the_five_labels_in_order() {
    for kind in [ThemeKind::Studio, ThemeKind::ProMedium] {
        let mut h = options_bar(Lang::EN, kind);
        open_sample_menu(&mut h, Lang::EN);
        let mut previous_bottom = f32::NEG_INFINITY;
        for label in LABELS {
            let row = h.get_by_role_and_label(Role::Button, label);
            assert!(row.rect().is_positive());
            assert!(row.rect().top() >= previous_bottom, "{kind:?}: {label} keeps its row order");
            assert_painted(&h, label, row.rect());
            previous_bottom = row.rect().bottom();
        }
        for old in OLD_LABELS {
            assert!(h.query_by_role_and_label(Role::Button, old).is_none());
        }
    }
}

#[test]
fn sample_dropdown_keeps_the_first_three_selection_ids() {
    let mut h = options_bar(Lang::EN, ThemeKind::Studio);
    for index in [2, 1, 0] {
        choose_sample(&mut h, Lang::EN, IDS[index], LABELS[index]);
    }
}

#[test]
fn sample_dropdown_selects_all_layers_no_adjustments_by_unchanged_id() {
    let mut h = options_bar(Lang::EN, ThemeKind::Studio);
    choose_sample(&mut h, Lang::EN, "allNoAdjustments", "All Layers no Adjustments");
}

#[test]
fn sample_dropdown_selects_current_and_below_no_adjustments_by_unchanged_id() {
    let mut h = options_bar(Lang::EN, ThemeKind::Studio);
    choose_sample(&mut h, Lang::EN, "currentAndBelowNoAdjustments", "Current & Below no Adjustments");
}

#[test]
fn sample_layer_ids_keep_the_five_entry_order_in_every_language() {
    for lang in Lang::all() {
        i18n::with_language(lang, || {
            let options = super::layer_options();
            assert_eq!(options.map(|(id, _)| id), IDS.map(str::to_owned), "{}", lang.code());
        });
    }
}

#[test]
fn sample_dropdown_keeps_selection_ids_and_sibling_options_in_other_language() {
    // Keep the real locale when embedded; an English-only build needs no external catalog.
    let lang = Lang::from_code("fr").unwrap_or_else(|| i18n::fixture_lang("fr", ""));
    let mut h = options_bar(lang, ThemeKind::Studio);
    // Missing translations remain a supported English fallback, never a test failure.
    choose_sample(&mut h, lang, "allNoAdjustments", i18n::tr(lang, "All Layers no Adjustments"));
    choose_sample(&mut h, lang, "currentAndBelowNoAdjustments", i18n::tr(lang, "Current & Below no Adjustments"));
}
