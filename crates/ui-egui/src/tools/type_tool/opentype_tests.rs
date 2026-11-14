//! UI policy and side effects, independent of engine formatting support.
use super::opentype::{Availability, availability};
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_engine::type_extra_cmds::feature_on;

const DLIG: &str = "type.openType.discretionaryLigatures";

/// A face with only contextual ligatures can still respond to Standard Ligatures. A mixed
/// target containing a second face with neither default ligature tag remains unsupported.
#[test]
fn standard_ligatures_checks_both_tags_per_face() {
    use photocraft_text::{FaceRequest, FontDb, fonts::INTER_REGULAR};

    let mut font = INTER_REGULAR.to_vec();
    let tables = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
    let record = (0..tables).map(|index| 12 + 16 * index).find(|&at| &font[at..at + 4] == b"GSUB").unwrap();
    let gsub = u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
    let features = gsub + u16::from_be_bytes(font[gsub + 6..gsub + 8].try_into().unwrap()) as usize;
    let count = u16::from_be_bytes(font[features..features + 2].try_into().unwrap()) as usize;
    let dlig = (0..count).map(|index| features + 2 + 6 * index).find(|&at| &font[at..at + 4] == b"dlig").unwrap();
    font[dlig..dlig + 4].copy_from_slice(b"clig");

    let mut db = FontDb::new();
    let family = "Contextual Ligatures Fixture";
    assert_eq!(db.register_font_data_as(font, family), [family]);
    let request = FaceRequest { family, style: "", postscript: None, weight: 400, italic: false };
    let known = db.face_features_for_requests([request]).unwrap();
    assert!(known[0].contains("clig") && !known[0].contains("liga"));
    let command = "type.openType.standardLigatures";
    assert_eq!(opentype::TargetFeatures::Ready(Some(known)).availability(command), Some(Availability::Available));
    let mixed = db.face_features_for_requests([request, FaceRequest { family: "Inter", ..request }]).unwrap();
    assert_eq!(opentype::TargetFeatures::Ready(Some(mixed)).availability(command), Some(Availability::Unsupported));
}

/// A deterministic document with two text layers whose GSUB sets differ; system fonts do not
/// affect Inter/JetBrains Mono's bundled faces. The active layer is also the selected layer.
fn fixture() -> (PhotocraftApp, [LayerId; 2]) {
    let _ = photocraft_text::shared();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 240})).unwrap();
    let mut layer = |font| LayerId(app.run("type.create", json!({"text": "AéBCD", "font": font, "size": 24})).unwrap()["layer"].as_u64().unwrap());
    let ids = [layer("Inter"), layer("JetBrains Mono")];
    select(&mut app, &ids[..1]);
    (app, ids)
}

/// Keep native layer selection and the active representative aligned, as the Layers panel does.
fn select(app: &mut PhotocraftApp, ids: &[LayerId]) {
    for (index, id) in ids.iter().enumerate() {
        app.run("layer.select", json!({"layer": id.0, "mode": if index == 0 { "replace" } else { "add" }})).unwrap();
    }
}

/// Read the actual stored style, independently of button state or cached capability metadata.
fn style(app: &PhotocraftApp, id: LayerId, at: usize) -> photocraft_doc::text::CharStyle {
    let t = text_layer(&app.session.active().unwrap().doc, id).unwrap();
    let byte = byte_of(&t.text, at);
    let mut end = 0;
    t.char_runs()
        .into_iter()
        .find(|run| {
            end += run.len;
            byte < end
        })
        .unwrap()
        .style
}

/// Editing selects character indices 1..3, which include a multibyte character.
fn editing(app: &mut PhotocraftApp, id: LayerId) {
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: None,
        caret: 3,
        anchor: 1,
        session: "opentype-availability".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
}

/// Model a click by using the same snapshot that would have enabled the Character row.
fn toggle(app: &mut PhotocraftApp, id: &str, shown_on: bool, defaults: Option<&photocraft_doc::text::CharStyle>) {
    let features = opentype::target_features_with_defaults(app, defaults);
    panel::toggle_opentype(app, id, shown_on, &features);
}

/// The same whole-layer and mixed-selection scope gates buttons and menus. Font changes retain
/// unavailable saved flags; direct engine automation still sets them without UI permission.
#[test]
fn target_intersection_preserves_flags_history_and_engine_commands() {
    let (mut app, ids) = fixture();
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    assert!(crate::menus::is_enabled(&app, DLIG));
    select(&mut app, &ids);
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert!(!crate::menus::is_enabled(&app, DLIG));
    let before = app.session.active().unwrap().history.entries().len();
    toggle(&mut app, DLIG, false, None);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before);
    assert!(ids.iter().all(|id| !style(&app, *id, 0).discretionary_ligatures));
    app.run(DLIG, json!({"layers": ids.map(|id| id.0), "on": true})).unwrap();
    assert!(ids.iter().all(|id| style(&app, *id, 0).discretionary_ligatures));
    let before = app.session.active().unwrap().history.entries().len();
    toggle(&mut app, DLIG, true, None);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before);
    assert!(ids.iter().all(|id| style(&app, *id, 0).discretionary_ligatures), "unavailable pressed flags survive activation attempts");
    app.run("type.setStyle", json!({"layer": ids[0].0, "font": "JetBrains Mono"})).unwrap();
    assert!(style(&app, ids[0], 0).discretionary_ligatures, "font switch preserves styling");
    app.run("type.setStyle", json!({"layer": ids[0].0, "font": "Inter"})).unwrap();
    select(&mut app, &ids[..1]);
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    toggle(&mut app, DLIG, true, None);
    assert!(!style(&app, ids[0], 0).discretionary_ligatures);
    assert!(style(&app, ids[1], 0).discretionary_ligatures, "unselected peer is untouched");
}

/// Selection-start controls keep their checked semantics, but a mixed target cannot imply all
/// selected faces support a feature. A collapsed caret follows the existing whole-layer command.
#[test]
fn selected_range_and_collapsed_caret_follow_actual_formatting_scope() {
    let (mut app, ids) = fixture();
    app.run("type.setStyle", json!({"layer": ids[0].0, "range": [3, 5], "font": "JetBrains Mono"})).unwrap();
    editing(&mut app, ids[0]);
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    toggle(&mut app, DLIG, false, None);
    assert!(!style(&app, ids[0], 0).discretionary_ligatures);
    assert!(style(&app, ids[0], 1).discretionary_ligatures && style(&app, ids[0], 2).discretionary_ligatures);
    assert!(!style(&app, ids[0], 3).discretionary_ligatures);
    app.ui.text_edit.as_mut().unwrap().caret = 5;
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert!(styles_at(&app).unwrap().0.discretionary_ligatures, "pressed state stays at selection start");
    app.ui.text_edit.as_mut().unwrap().caret = 1;
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported), "collapsed formatting targets the whole layer");
    assert_eq!(availability(&app, "type.openType.numerals.tabular"), Some(Availability::Unsupported));
    assert_eq!(availability(&app, "type.openType.numerals.default"), Some(Availability::Available));
    assert_eq!(availability(&app, "type.openType.stylisticSets.set20"), Some(Availability::Unsupported));
    assert_eq!(availability(&app, "type.openType.stylisticSets.set1"), Some(Availability::Available));
    assert_eq!(availability(&app, "type.setStyle"), None);
}

/// Saved defaults remain usable without a document and across UiState round-trip. An unknown
/// family disables its controls without erasing stored features or advertising fallback support.
#[test]
fn defaults_and_missing_fonts_survive_restart_without_losing_features() {
    let _ = photocraft_text::shared();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let available_default = |app: &PhotocraftApp| {
        let style = default_styles(app).0;
        opentype::target_features_with_defaults(app, Some(&style)).availability(DLIG)
    };
    app.ui.tool_options.type_font = "Inter".into();
    assert_eq!(available_default(&app), Some(Availability::Available));
    let style = default_styles(&app).0;
    toggle(&mut app, DLIG, false, Some(&style));
    assert!(default_styles(&app).0.discretionary_ligatures);
    app.ui.tool_options.type_font = "Missing OpenType Font".into();
    assert_eq!(available_default(&app), Some(Availability::Unknown));
    let style = default_styles(&app).0;
    toggle(&mut app, DLIG, true, Some(&style));
    assert!(default_styles(&app).0.discretionary_ligatures);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let mut restarted = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    restarted.ui = serde_json::from_str(&saved).unwrap();
    assert_eq!(available_default(&restarted), Some(Availability::Unknown));
    assert!(default_styles(&restarted).0.discretionary_ligatures);
    restarted.ui.tool_options.type_font = "Inter".into();
    assert_eq!(available_default(&restarted), Some(Availability::Available));
    assert!(default_styles(&restarted).0.discretionary_ligatures);
}

/// Real egui disabled buttons expose accessibility state and ignore pointer activation. A
/// supported button beside them still edits selected layers and creates exactly one undo step.
#[test]
fn character_buttons_disable_unsupported_features_and_keep_supported_clicks() {
    let (app, ids) = fixture();
    let before = app.session.active().unwrap().history.entries().len();
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(16).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.set_width(340.0);
                panel::character_panel(app, ui, false);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    assert!(h.get_by_label("Swash").accesskit_node().is_disabled());
    h.get_by_label("Swash").click();
    h.run();
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), before);
    assert!(!feature_on(&style(h.state(), ids[0], 0), "swsh"));
    assert!(!h.get_by_label("Fractions").accesskit_node().is_disabled());
    h.get_by_label("Fractions").click();
    h.run();
    assert!(feature_on(&style(h.state(), ids[0], 0), "frac"));
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), before + 1);
    assert!(!feature_on(&style(h.state(), ids[1], 0), "frac"));
}

/// Cached open/native menu rows must update on late registrations and programmatic selection
/// changes, even when no click or document edit forces a rebuild.
#[test]
fn menu_cache_tracks_catalog_and_edit_selection_without_document_edits() {
    let (mut app, ids) = fixture();
    let before = crate::native_menu::state_hash(&app, None);
    photocraft_text::shared().lock().unwrap().fonts.register_font_data(photocraft_text::fonts::INTER_REGULAR.to_vec());
    assert_ne!(crate::native_menu::state_hash(&app, None), before);
    editing(&mut app, ids[0]);
    let before = crate::native_menu::state_hash(&app, None);
    app.ui.text_edit.as_mut().unwrap().caret = 5;
    assert_ne!(crate::native_menu::state_hash(&app, None), before);
}

/// Switching documents cannot reuse another document's target fonts, even when layer ids match.
#[test]
fn document_switches_recompute_the_target_faces() {
    let (mut app, _) = fixture();
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    app.run("file.new", json!({"width": 400, "height": 240})).unwrap();
    app.run("type.create", json!({"text": "Second", "font": "JetBrains Mono"})).unwrap();
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert!(app.session.set_active(0));
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
}

/// Disabled scopes must not expand the eight-button row or change its hit targets. Check the
/// same containment/alignment invariants at compact/normal widths in every supported theme.
#[test]
fn opentype_buttons_remain_contained_across_themes_and_panel_widths() {
    for theme in crate::theme::ThemeKind::ALL {
        for width in [240.0, 340.0] {
            let (app, _) = fixture();
            let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(16).build_ui_state(
                move |ui, app| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        ui.set_width(width);
                        panel::character_panel(app, ui, false);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(4);
            let cells: Vec<_> = panel::OPENTYPE_BUTTONS.iter().map(|(_, _, label)| h.get_by_label(label).rect()).collect();
            let first = cells.first().unwrap();
            let last = cells.last().unwrap();
            assert!(last.right() <= first.left() + width + 1.0, "{theme:?}, width {width}: {cells:?}");
            for adjacent in cells.windows(2) {
                assert!(adjacent[0].right() <= adjacent[1].left());
                assert!((adjacent[0].top() - adjacent[1].top()).abs() < 1.0);
            }
            assert!(h.get_by_label("Swash").accesskit_node().is_disabled());
            assert!(!h.get_by_label("Fractions").accesskit_node().is_disabled());
        }
    }
}

/// Empty text still has a typing style: capabilities must not disappear just because there are
/// no glyphs, and a later font change must refresh them without inserting placeholder text.
#[test]
fn empty_type_layers_use_their_typing_style_for_capabilities() {
    let (mut app, _) = fixture();
    let id = LayerId(app.run("type.create", json!({"text": "", "font": "JetBrains Mono"})).unwrap()["layer"].as_u64().unwrap());
    select(&mut app, &[id]);
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert_eq!(availability(&app, "type.openType.fractions"), Some(Availability::Available));
    let before = app.session.active().unwrap().history.entries().len();
    toggle(&mut app, DLIG, false, None);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before);
    app.run("type.setStyle", json!({"layer": id.0, "font": "Inter"})).unwrap();
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    assert!(text_layer(&app.session.active().unwrap().doc, id).unwrap().text.is_empty());
}

/// Optional capability inspection borrows only identity fields even when runs contain large,
/// irrelevant formatting payloads. Normalization must agree with the document's existing API.
#[test]
fn borrowed_requests_preserve_normalized_ranges_without_copying_style_payloads() {
    use photocraft_doc::text::{CharStyle, FontFeature, TextRun};
    let mut t = TextLayer {
        text: "AéBCD".into(),
        font_family: "Inter".into(),
        runs: vec![
            TextRun { len: 0, style: CharStyle { font_family: "Ignored".into(), ..Default::default() } },
            TextRun {
                len: 2,
                style: CharStyle { font_family: "Inter".into(), features: vec![FontFeature { tag: "x".repeat(4096), value: 1 }; 16], ..Default::default() },
            },
            TextRun { len: 1, style: CharStyle { font_family: "JetBrains Mono".into(), ..Default::default() } },
        ],
        ..Default::default()
    };
    for range in [None, Some([0, 1]), Some([1, 2]), Some([2, 5]), Some([5, 2]), Some([50, 100])] {
        let expected = match range {
            Some([a, b]) => editing::selected_styles(&t, a, b),
            None => t.char_runs().into_iter().map(|run| run.style).collect(),
        };
        let actual: Vec<_> = opentype::layer_requests(&t, range).collect();
        assert_eq!(actual, expected.iter().map(photocraft_text::FaceRequest::from).collect::<Vec<_>>());
        assert!(actual.iter().all(|request| t.runs.iter().any(|run| std::ptr::eq(request.family.as_ptr(), run.style.font_family.as_ptr()))));
    }
    t.text.clear();
    assert_eq!(opentype::layer_requests(&t, None).next().unwrap().family, "Ignored");
    t.text = "abc".into();
    t.runs.clear();
    assert_eq!(opentype::layer_requests(&t, None).next().unwrap().family, "Inter");
    t.runs.push(TextRun { len: 0, style: CharStyle { font_family: "Zero".into(), ..Default::default() } });
    assert_eq!(opentype::layer_requests(&t, None).next().unwrap().family, "Inter");
}

/// The actual UI query stops a distinct over-budget target before scanning its remaining runs.
/// Repeated-font runs with rich styling still use one face, and menus take just one snapshot.
#[test]
fn ui_admission_bounds_oversized_targets_and_shares_the_menu_snapshot() {
    use photocraft_doc::text::{CharStyle, FontFeature, TextRun};
    let (mut app, ids) = fixture();
    let text = match &mut std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layer_mut(ids[0]).unwrap().content {
        LayerContent::Text(text) => text,
        _ => panic!("fixture must select text"),
    };
    text.text = "A".repeat(5000);
    text.runs = (0..5000)
        .map(|index| TextRun { len: 1, style: CharStyle { font_family: "Inter".into(), font_style: format!("Style{index}"), ..Default::default() } })
        .collect();
    opentype::RUN_VISITS.with(|count| count.set(0));
    assert_eq!(availability(&app, DLIG), Some(Availability::Unknown));
    assert!(opentype::RUN_VISITS.with(|count| count.get()) <= 4098, "the metadata budget must precede a full-target snapshot");
    editing(&mut app, ids[0]);
    let edit = app.ui.text_edit.as_mut().unwrap();
    (edit.anchor, edit.caret) = (0, 1);
    opentype::RUN_VISITS.with(|count| count.set(0));
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    assert_eq!(opentype::RUN_VISITS.with(|count| count.get()), 1, "runs after the selected range do not affect its capabilities");
    app.ui.text_edit = None;
    let text = match &mut std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layer_mut(ids[0]).unwrap().content {
        LayerContent::Text(text) => text,
        _ => panic!("fixture must select text"),
    };
    let payload = CharStyle { font_family: "Inter".into(), features: vec![FontFeature { tag: "z".repeat(1024), value: 1 }; 16], ..Default::default() };
    text.text = "A".repeat(128);
    text.runs = vec![TextRun { len: 1, style: payload }; 128];
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    opentype::TARGET_SNAPSHOTS.with(|count| count.set(0));
    let items = crate::menus::menu_items(&app);
    assert_eq!(opentype::TARGET_SNAPSHOTS.with(|count| count.get()), 1);
    assert!(items.iter().find(|item| item.id == DLIG).unwrap().enabled);
    assert!(!items.iter().find(|item| item.id == "type.openType.swash").unwrap().enabled);
}

/// A native click can arrive after a control message changes the selection but before the
/// native rows refresh. An unavailable click or an old edit with a same-id layer in the
/// new document must not write style or create history.
#[test]
fn native_opentype_click_rechecks_the_current_target() {
    use crate::menus::native_menu::{self, Backend, Event, MenuBar, NativeMenu};

    struct ClickOnce(Vec<Event>);

    impl Backend for ClickOnce {
        fn sync(&mut self, _bar: &MenuBar) {}

        /// The backend delivers the one queued click only once.
        fn drain(&mut self) -> Vec<Event> {
            std::mem::take(&mut self.0)
        }
    }

    let (mut app, ids) = fixture();
    let peer = LayerId(app.run("type.create", json!({"text": "ABC", "font": "Inter"})).unwrap()["layer"].as_u64().unwrap());
    let ctx = egui::Context::default();
    app.session.restore_layer_selection(Some(ids[0]), &[ids[0], peer], Some(ids[0])).unwrap();
    assert!(crate::menus::is_enabled(&app, DLIG));
    let mut menu = NativeMenu::new(Box::new(ClickOnce(vec![Event::Click(DLIG.into())])));
    menu.raw_input(&mut egui::RawInput::default());
    app.services.native_menu = Some(menu);
    app.session.restore_layer_selection(Some(ids[0]), &ids, Some(ids[0])).unwrap();
    assert!(!crate::menus::is_enabled(&app, DLIG));
    let history = app.session.active().unwrap().history.entries().len();
    ctx.run_ui(egui::RawInput::default(), |ui| native_menu::run(&mut app, ui.ctx())).textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    assert!(!style(&app, ids[1], 0).discretionary_ligatures);

    select(&mut app, &ids[..1]);
    let mut menu = NativeMenu::new(Box::new(ClickOnce(vec![Event::Click(DLIG.into())])));
    menu.raw_input(&mut egui::RawInput::default());
    app.services.native_menu = Some(menu);
    ctx.run_ui(egui::RawInput::default(), |ui| native_menu::run(&mut app, ui.ctx())).textures_delta.clear();
    assert!(style(&app, ids[0], 0).discretionary_ligatures);

    let original = app.session.active().unwrap().doc.id.0;
    // Opening the same saved document twice keeps its layer ids but assigns a distinct document id.
    let copy = app.session.active().unwrap().doc.as_ref().clone();
    let other = ids[0];
    let other_index = app.session.add_document(copy, None);
    app.run("type.setStyle", json!({"layer": other.0, "font": "JetBrains Mono", "discretionaryLigatures": false})).unwrap();
    select(&mut app, &[other]);
    assert_ne!(app.session.active().unwrap().doc.id.0, original);
    assert_eq!(style(&app, other, 1).font_family, "JetBrains Mono");
    assert!(app.session.set_active(0));
    editing(&mut app, ids[0]);
    app.ui.text_edit.as_mut().unwrap().document = Some(original);
    assert!(crate::menus::is_enabled(&app, DLIG));
    let mut menu = NativeMenu::new(Box::new(ClickOnce(vec![Event::Click(DLIG.into())])));
    menu.raw_input(&mut egui::RawInput::default());
    app.services.native_menu = Some(menu);
    assert!(app.session.set_active(other_index));
    let history = app.session.active().unwrap().history.entries().len();
    ctx.run_ui(egui::RawInput::default(), |ui| native_menu::run(&mut app, ui.ctx())).textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    assert!(!style(&app, other, 0).discretionary_ligatures);
    assert!(!style(&app, other, 1).discretionary_ligatures, "the old edit's selected range did not style the new document");
}

/// A native row published before a font lock can still deliver a click. Report the retry,
/// keep style and history intact, refresh without input, then accept a fresh click.
#[test]
fn native_menu_retries_open_type_after_background_font_lock() {
    use crate::menus::native_menu::{self, Backend, Event, MenuBar, NativeMenu};
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::mpsc,
        time::{Duration, Instant},
    };

    struct RecordDlig {
        enabled: Rc<RefCell<Vec<bool>>>,
        clicks: Rc<RefCell<Vec<Event>>>,
    }

    impl Backend for RecordDlig {
        /// Record the actual published availability, including the first busy frame.
        fn sync(&mut self, bar: &MenuBar) {
            self.enabled.borrow_mut().push(bar.find(DLIG).unwrap().enabled);
        }

        /// Deliver each queued native click once, even if the target now refuses it.
        fn drain(&mut self) -> Vec<Event> {
            std::mem::take(&mut *self.clicks.borrow_mut())
        }
    }

    let (mut app, ids) = fixture();
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let clicks = Rc::new(RefCell::new(Vec::new()));
    app.services.native_menu = Some(NativeMenu::new(Box::new(RecordDlig { enabled: rendered.clone(), clicks: clicks.clone() })));
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| native_menu::sync(&mut app, ui.ctx())).textures_delta.clear();
    assert_eq!(*rendered.borrow(), vec![true], "the click originates from an enabled row");
    let history = app.session.active().unwrap().history.entries().len();
    let (locked, waiting) = mpsc::sync_channel(0);
    let (release, finished) = mpsc::channel();
    let photocraft_engine::jobs::Started::Job(_) = app
        .session
        .start_job(
            "test.holdFontEngine",
            json!({}),
            "Hold font engine",
            false,
            move |_| {
                let _engine = photocraft_text::shared().lock().unwrap();
                locked.send(()).unwrap();
                // Bound the counterexample: blocking inspection must fail the availability
                // assertion after the wait instead of hanging the entire test binary.
                let _ = finished.recv_timeout(Duration::from_secs(2));
                Ok(())
            },
            |_, ()| Ok(json!(null)),
        )
        .unwrap()
    else {
        panic!("the font engine fixture needs a native worker");
    };
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    clicks.borrow_mut().push(Event::Click(DLIG.into()));
    let mut raw = egui::RawInput::default();
    app.services.native_menu.as_mut().unwrap().raw_input(&mut raw);
    ctx.run_ui(raw, |ui| {
        native_menu::run(&mut app, ui.ctx());
        native_menu::sync(&mut app, ui.ctx());
    })
    .textures_delta
    .clear();
    let _ = release.send(());
    let retry = tl!("Font engine is busy. Try the OpenType command again.");
    assert_eq!(app.ui.status, retry);
    assert!(app.ui.status_error);
    assert!(app.ui.notices.iter().any(|notice| notice.title == retry && notice.error), "the feedback must survive job progress replacing the status line");
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    assert!(!style(&app, ids[0], 0).discretionary_ligatures);
    assert_eq!(*rendered.borrow(), vec![true, false], "busy metadata must not wait for or borrow the rendered face");
    assert!(!app.services.native_menu.as_ref().unwrap().has_stable_snapshot(), "a busy menu cannot be cached as stable");
    assert!(app.session.has_jobs(), "keep the same menu hash while the worker releases the engine");
    let deadline = Instant::now() + Duration::from_secs(5);
    while rendered.borrow().last() != Some(&true) && Instant::now() < deadline {
        ctx.run_ui(egui::RawInput::default(), |ui| native_menu::sync(&mut app, ui.ctx())).textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(rendered.borrow().last(), Some(&true), "a menu published while busy must refresh without input");
    while app.session.has_jobs() && Instant::now() < deadline {
        ctx.run_ui(egui::RawInput::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx())).textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.session.has_jobs());
    assert_eq!(app.ui.status, "Hold font engine", "completion replaced the status line");
    assert!(app.ui.notices.iter().any(|notice| notice.title == retry), "completion must not remove the retry notice");
    assert!(!style(&app, ids[0], 0).discretionary_ligatures, "refresh must not replay the refused click");
    clicks.borrow_mut().push(Event::Click(DLIG.into()));
    let mut raw = egui::RawInput::default();
    app.services.native_menu.as_mut().unwrap().raw_input(&mut raw);
    ctx.run_ui(raw, |ui| native_menu::run(&mut app, ui.ctx())).textures_delta.clear();
    assert!(style(&app, ids[0], 0).discretionary_ligatures);
    assert_eq!(app.session.active().unwrap().history.entries().len(), history + 1);
    assert!(!app.ui.status_error);
}

/// Capability inspection must not advance the engine's active-layer marker or create history.
/// Direct selection and undo/redo must refresh the target while preserving unavailable flags.
#[test]
fn active_layer_markers_and_history_keep_capability_queries_read_only() {
    let (mut app, ids) = fixture();
    let marker = app.session.active_layer_changes();
    let history = app.session.active().unwrap().history.entries().len();
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    let _ = crate::menus::menu_items(&app);
    assert_eq!(app.session.active_layer_changes(), marker);
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);

    app.session.select_layer(ids[1]).unwrap();
    assert_ne!(app.session.active_layer_changes(), marker);
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    app.session.restore_layer_selection(Some(ids[0]), &ids[..1], Some(ids[0])).unwrap();
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    let marker = app.session.active_layer_changes();
    toggle(&mut app, DLIG, false, None);
    assert!(style(&app, ids[0], 0).discretionary_ligatures);
    assert_eq!(app.session.active_layer_changes(), marker);
    app.run("type.setStyle", json!({"layer": ids[0].0, "font": "JetBrains Mono"})).unwrap();
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert!(style(&app, ids[0], 0).discretionary_ligatures);
    assert!(app.session.undo());
    assert_eq!(availability(&app, DLIG), Some(Availability::Available));
    assert!(style(&app, ids[0], 0).discretionary_ligatures);
    assert!(app.session.redo());
    assert_eq!(availability(&app, DLIG), Some(Availability::Unsupported));
    assert!(style(&app, ids[0], 0).discretionary_ligatures);
}
