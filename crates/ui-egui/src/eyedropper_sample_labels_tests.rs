//! Issue #11753: only the Eyedropper Sample labels change, not their ids or translations.

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
fn translated_sample_labels_preserve_the_five_ids_and_existing_values() {
    for lang in Lang::all() {
        i18n::with_language(lang, || {
            let options = super::layer_options();
            assert_eq!(options.clone().map(|(id, _)| id), IDS.map(str::to_owned), "{}", lang.code());
            assert_eq!(options.map(|(_, label)| label), LABELS.map(|label| i18n::tr(lang, label)), "{}", lang.code());
        });
    }
}

#[test]
fn translated_sample_dropdown_keeps_selection_ids_and_sibling_options() {
    let lang = Lang::from_code("fr").unwrap();
    let mut h = options_bar(lang, ThemeKind::Studio);
    // Missing translations remain a supported English fallback, never a test failure.
    choose_sample(&mut h, lang, "allNoAdjustments", i18n::tr(lang, "All Layers no Adjustments"));
    choose_sample(&mut h, lang, "currentAndBelowNoAdjustments", i18n::tr(lang, "Current & Below no Adjustments"));
}

// Frozen pre-change values: rekey existing entries without editing their translated bytes.
// Do not require a row to exist: catalogs may legitimately fall back to English.
const TRANSLATIONS: [(&str, [&str; 2]); 32] = [
    ("ar", ["كل الطبقات بدون تعديلات", "الحالية وما تحتها دون تعديلات"]),
    ("az", ["Bütün qatlardə düzəliş yoxdur", "Cari və Aşağıda Düzəlişlər Yoxdur"]),
    ("bs", ["Svi slojevi bez podešavanja", "Trenutni i ispod bez podešavanja"]),
    ("cs", ["Všechny vrstvy bez korekcí", "Aktuální a pod ní bez korekcí"]),
    ("da", ["Alle lag uden justeringer", "Aktuelt og nedenunder uden justeringer"]),
    ("de", ["Alle Ebenen ohne Einstellungen", "Aktuelle und darunter ohne Einstellungen"]),
    ("el", ["Όλα τα επίπεδα χωρίς προσαρμογές", "Τρέχον και υποκείμενα χωρίς προσαρμογές"]),
    ("es", ["Todas las capas sin ajustes", "Actual e inferiores sin ajustes"]),
    ("fi", ["Kaikki tasot ilman säätöjä", "Nykyinen ja alemmat ilman säätöjä"]),
    ("fr", ["Tous les calques sans ajustements", "Actif et inférieurs sans ajustements"]),
    ("he", ["כל השכבות ללא התאמות", "נוכחי ומתחת ללא התאמות"]),
    ("hu", ["Minden réteg, korrekciók nélkül", "Jelenlegi és alatta lévők, korrekciók nélkül"]),
    ("id", ["Semua Layer Tanpa Penyesuaian", "Saat Ini & Di Bawahnya Tanpa Penyesuaian"]),
    ("it", ["Tutti i livelli senza regolazioni", "Corrente e inferiori senza regolazioni"]),
    ("ja", ["すべてのレイヤー (色調補正なし)", "現在のレイヤーとその下 (色調補正なし)"]),
    ("ko", ["모든 레이어(조정 제외)", "현재 및 이하(조정 제외)"]),
    ("nb", ["Alle lag uten justeringer", "Gjeldende og under uten justeringer"]),
    ("nl", ["Alle lagen zonder aanpassingen", "Huidige en onderliggende zonder aanpassingen"]),
    ("pl", ["Wszystkie warstwy bez korekt", "Bieżąca i poniżej bez korekt"]),
    ("pt-br", ["Todas as camadas sem ajustes", "Atual e abaixo sem ajustes"]),
    ("ro", ["Toate straturile fără ajustări", "Curent și inferioare fără ajustări"]),
    ("ru", ["Все слои без коррекции", "Текущий и ниже без коррекции"]),
    ("sr", ["Svi slojevi bez korekcija", "Tekući i ispod bez korekcija"]),
    ("sv", ["Alla lager utan justeringar", "Aktuellt och under utan justeringar"]),
    ("ta", ["அனைத்து அடுக்குகள், சரிசெய்தல்கள் இல்லாமல்", "தற்போதையது & கீழுள்ளவை, சரிசெய்தல்கள் இல்லாமல்"]),
    ("th", ["ทุกเลเยอร์ไม่มีการปรับเปลี่ยน", "ปัจจุบันและต่ำกว่าไม่มีการปรับเปลี่ยน"]),
    ("tr", ["Tüm Katmanlar Ayarlama Yok", "Mevcut ve Altında Ayarlama Yok"]),
    ("uk", ["Усі шари без коригувань", "Поточний і нижчі без коригувань"]),
    ("ur", ["تمام تہیں بغیر ایڈجسٹمنٹ", "موجودہ اور نیچے بغیر ایڈجسٹمنٹ"]),
    ("vi", ["Mọi lớp không điều chỉnh", "Hiện tại & dưới không điều chỉnh"]),
    ("zh-hans", ["所有图层（不含调整）", "当前及下方图层（不含调整）"]),
    ("zh-hant", ["所有圖層（不含調整）", "目前及以下（不含調整）"]),
];

#[test]
fn existing_catalog_keys_are_rekeyed_without_changing_translation_bytes() {
    for info in &i18n::LANGUAGES {
        let lang = Lang::from_code(info.code).unwrap();
        for old in OLD_LABELS {
            assert!(!i18n::has(lang, old), "{} still carries the superseded English key {old:?}", info.code);
        }
        if let Some((_, expected)) = TRANSLATIONS.iter().find(|(code, _)| *code == info.code) {
            for (key, value) in LABELS.iter().skip(3).zip(expected) {
                if i18n::has(lang, key) {
                    assert_eq!(i18n::tr(lang, key).as_bytes(), value.as_bytes(), "{}: {key}", info.code);
                }
            }
        }
    }
}
