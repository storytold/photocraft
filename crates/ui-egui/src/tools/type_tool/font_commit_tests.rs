//! Selected-range font choices, caret typing styles and the commit, through the family field of the options bar
//! and the Character panel.
use crate::{PhotocraftApp, Services};
use egui::{Key, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::{Value, json};

const FONTS: [&str; 2] = ["Inter", "JetBrains Mono"];

type H = Harness<'static, Box<PhotocraftApp>>;

use super::commit_tests::ime;

/// A key press through the window host, as the platform's reaches the focused editor.
fn key(app: &mut PhotocraftApp, key: Key) {
    crate::canvas::testing::key(app, key, egui::Modifiers::NONE);
}

fn layer(doc: &Document, id: LayerId) -> &photocraft_doc::TextLayer {
    let LayerContent::Text(t) = &doc.layer(id).unwrap().content else { panic!("text layer") };
    t
}

fn styles(doc: &Document, id: LayerId) -> Value {
    let t = layer(doc, id);
    let mut at = 0;
    let mut chars = Vec::new();
    for r in t.char_runs() {
        let end = at + r.len;
        for ch in t.text.get(at..end).unwrap().chars() {
            chars.push(json!({"char": ch.to_string(), "font": r.style.font_family,
                "face": r.style.postscript_name, "weight": r.style.weight, "italic": r.style.italic}));
        }
        at = end;
    }
    json!({"text": t.text, "summary": t.font_family, "chars": chars})
}

fn saved(h: &H, id: LayerId) -> Value {
    styles(&h.state().session.active().unwrap().doc, id)
}

fn families(s: &Value) -> Vec<String> {
    s["chars"].as_array().unwrap().iter().map(|c| c["font"].as_str().unwrap().to_owned()).collect()
}

fn select(h: &mut H, range: [usize; 2]) {
    let e = h.state_mut().ui.text_edit.as_mut().unwrap();
    e.anchor = range[0];
    e.caret = range[1];
}

/// Takes the keyboard off the family field and gives it to the session's editor, as the field's Enter does in the app:
/// the pass that shows the editor asks for the focus and the next one applies it, so input sent after this reaches it.
fn surrender_focus(h: &mut H) {
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    super::editing::focus_editor(&h.ctx);
    h.run();
}

/// A text layer `content` in an edit session in `FONTS[pattern % 2]`; patterns 2 and 3 style characters 2..4 in the other font.
/// The full app frame, so the options bar and the Character panel both show their family fields.
fn fixture(pattern: usize, content: &str) -> (H, LayerId) {
    let mut h = document();
    let id = text_layer(&mut h, pattern, content);
    (h, id)
}

/// [`fixture`]'s app with its document and the Type tool, before the text layer.
fn document() -> H {
    Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Services::default());
        app.session.edit_prefs(|p| p.type_.font_preview = photocraft_engine::prefs::FontPreview::Off);
        app.run("file.new", json!({"width": 240, "height": 160})).unwrap();
        app.sync_views();
        app.ui.tool = crate::state::Tool::Type;
        app
    })
}

/// [`fixture`]'s text layer, made on `h` and edited.
fn text_layer(h: &mut H, pattern: usize, content: &str) -> LayerId {
    h.state_mut().run("type.create", json!({"text": content, "font": FONTS[pattern % 2], "size": 18})).unwrap();
    // Settle as the harness does after its first frame.
    let _ = h.run_ok();
    let id = h.state().session.active().unwrap().active_layer.unwrap();
    let ctx = h.ctx.clone();
    super::edit_active(h.state_mut(), &ctx).unwrap();
    if pattern >= 2 {
        let key = h.state().ui.text_edit.as_ref().unwrap().session.clone();
        h.state_mut().run("type.setStyle", json!({"layer": id.0, "range": [2, 4], "font": FONTS[1 - pattern % 2], "coalesce": key})).unwrap();
    }
    h.run_steps(3);
    id
}

/// The past states of the document's history.
fn past_len(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// Undo back to `steps` past states: the document before the last case's text layer.
fn undo_to(h: &mut H, steps: usize) {
    while past_len(h) > steps {
        let before = past_len(h);
        h.state_mut().run("edit.undo", json!({})).unwrap_or_else(|e| panic!("undo: {e}"));
        assert!(past_len(h) < before, "Undo takes back the last case");
    }
    let _ = h.run_ok();
}

/// Picks `font` in the first family field: open it, type the name, then accept it as `accept` says
/// (Enter, ArrowDown then Enter, a click on its row, or a hover).
fn choose(h: &mut H, font: &str, accept: &str) {
    h.query_all_by_label("Font family").next().unwrap().click();
    h.run();
    h.query_all_by_label("Font family").next().unwrap().type_text(font);
    h.run();
    match accept {
        "enter" => h.key_press(Key::Enter),
        "arrow-enter" => {
            h.key_press(Key::ArrowDown);
            h.run();
            h.key_press(Key::Enter);
        }
        "click" => h.get_by_role_and_label(egui::accesskit::Role::TreeItem, font).click(),
        "hover" => h.get_by_role_and_label(egui::accesskit::Role::TreeItem, font).hover(),
        _ => panic!("unknown acceptance"),
    }
    h.run();
}

/// Commits the edit through the bar's check mark.
fn checkmark(h: &mut H) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Commit any current edits").click();
    h.run();
}

#[test]
fn caret_font_choices_style_only_subsequent_typing_including_ime() {
    // One app: each case's text layer goes with Undo before the next case makes its own.
    // Every acceptance, as each reaches the caret's pending style by its own route through the
    // family field (ArrowDown applies its row once per step). One original font, as it only decides
    // which font is chosen. The caret at the start of the text, where any text inserts at byte 0 the same way,
    // and at the end of each text, where the inserted text and its pending font land after its last character
    // boundary (a plain letter, a joined emoji, a letter after a paragraph break). The acceptance only sets the
    // pending font (`type_insertion_style`, checked before anything is typed), wherever the caret is, so each
    // acceptance runs once or more and need not meet every caret or text.
    let mut h = document();
    let base = past_len(&h);
    let original = 0;
    for (accept, text, end) in [("enter", "abcdef", false), ("click", "abcdef", true), ("arrow-enter", "AéB👩‍💻", true), ("enter", "ab\ncdé", true)] {
        let caret = if end { text.chars().count() } else { 0 };
        let label = (accept, text, caret);
        undo_to(&mut h, base);
        let id = text_layer(&mut h, original, text);
        select(&mut h, [caret, caret]);
        h.run();
        let before = saved(&h, id);
        let steps = h.state().session.active().unwrap().history.entries().len();
        let requested = FONTS[1 - original];
        choose(&mut h, requested, accept);
        // ArrowDown steps to the row below the typed match, which depends on the installed fonts
        // ("JetBrains Mono NL" where it is installed), so the pending font is the row it accepted.
        let font =
            h.state().ui.type_insertion_style.as_ref().map(|s| s.style.font_family.clone()).unwrap_or_else(|| panic!("a pending font at the caret: {label:?}"));
        let font = font.as_str();
        assert!(font.starts_with(requested) && font != FONTS[original], "the accepted row matches the typed name: {font} {label:?}");
        if accept != "arrow-enter" {
            assert_eq!(font, requested, "{label:?}");
        }
        assert_eq!(saved(&h, id), before, "a font choice at a caret never restyles existing characters: {label:?}");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps, "{label:?}");
        crate::type_font_preview::set(h.state_mut(), font.into());
        assert!(crate::type_font_preview::display_doc(h.state_mut(), 0).is_none());
        // Two preedit replacements and the final commit keep the pending font.
        surrender_focus(&mut h);
        ime(h.state_mut(), egui::ImeEvent::Preedit { text: "é".into(), active_range_chars: None });
        h.run();
        ime(h.state_mut(), egui::ImeEvent::Preedit { text: "éX".into(), active_range_chars: None });
        h.run();
        ime(h.state_mut(), egui::ImeEvent::Commit("éX".into()));
        h.run();
        super::host::type_text(h.state_mut(), "Z");
        h.run();
        let mut expected = families(&before);
        expected.splice(caret..caret, std::iter::repeat_n(font.to_string(), 3));
        assert_eq!(families(&saved(&h, id)), expected, "{label:?}");
        let mut expected_text: Vec<char> = text.chars().collect();
        expected_text.splice(caret..caret, "éXZ".chars());
        let typed = saved(&h, id);
        assert_eq!(typed["text"], expected_text.into_iter().collect::<String>(), "the IME and typed text land at the caret: {label:?}");
        assert!(h.state().ui.type_insertion_style.is_some(), "the pending font is still set before the commit: {label:?}");
        checkmark(&mut h);
        assert_eq!(saved(&h, id), typed, "{label:?}");
        assert!(h.state().ui.type_insertion_style.is_none(), "the commit clears the pending font: {label:?}");
    }
}

/// A commit with a hover preview running ends the preview first, so it never spreads over the whole layer.
#[test]
fn global_commit_clears_hover_before_preview_can_expand_to_the_whole_layer() {
    let (mut h, id) = fixture(2, "abcdef");
    select(&mut h, [2, 4]);
    h.run();
    choose(&mut h, FONTS[1], "hover");
    checkmark(&mut h);
    assert!(h.state().ui.text_edit.is_none());
    let (shown, _) = crate::canvas::display_doc(h.state_mut(), 0);
    assert_eq!(styles(&shown, id), saved(&h, id), "the first committed frame shows real runs, not a full-layer hover preview");
}

/// The pending caret style goes with a caret move, a cancel, an undo and a document switch.
#[test]
fn caret_style_is_discarded_by_navigation_cancel_undo_and_document_switch() {
    for action in ["navigation", "cancel", "undo", "document-switch"] {
        let (mut h, id) = fixture(0, "abcdef");
        select(&mut h, [3, 3]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        match action {
            "navigation" => {
                surrender_focus(&mut h);
                key(h.state_mut(), Key::ArrowLeft);
                h.run();
                key(h.state_mut(), Key::ArrowRight);
                h.run();
                super::host::type_text(h.state_mut(), "Z");
                assert!(families(&saved(&h, id)).iter().all(|f| f == FONTS[0]));
            }
            "cancel" => {
                super::cancel(h.state_mut());
                assert!(h.state().ui.type_insertion_style.is_none());
            }
            "undo" => {
                h.state_mut().run("type.setStyle", json!({"layer": id.0, "range": [0, 1], "font": FONTS[1]})).unwrap();
                assert!(h.state_mut().session.undo());
                assert!(super::insertion_style::pending(h.state()).is_none());
            }
            _ => {
                h.state_mut().run("file.new", json!({"width": 80, "height": 60})).unwrap();
                super::end_if_left(h.state_mut());
                h.state_mut().session.set_active(0);
                h.run();
                assert!(h.state().ui.type_insertion_style.is_none());
            }
        }
    }
}

/// Focus returning to the same caret keeps the pending style.
#[test]
fn returning_focus_to_the_same_canvas_caret_keeps_the_chosen_typing_font() {
    let (mut h, id) = fixture(0, "abcdef");
    select(&mut h, [6, 6]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    select(&mut h, [6, 6]);
    h.run();
    assert_eq!(h.state().ui.text_edit.as_ref().unwrap().caret, 6);
    assert!(super::insertion_style::pending(h.state()).is_some());
    surrender_focus(&mut h);
    super::host::type_text(h.state_mut(), "XYZ");
    h.run();
    assert_eq!(families(&saved(&h, id)), [FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[1], FONTS[1], FONTS[1]]);
}

/// Deleting keeps the pending caret font (`EditKind::Deleted`, `pending_across_deletion`).
#[test]
fn backspace_and_delete_preserve_pending_caret_font() {
    for key in [Key::Backspace, Key::Delete] {
        let (mut h, id) = fixture(0, "abcdef");
        select(&mut h, [3, 3]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        surrender_focus(&mut h);
        self::key(h.state_mut(), key);
        h.run();
        assert!(super::insertion_style::pending(h.state()).is_some());
        super::host::type_text(h.state_mut(), "Z");
        h.run();
        let expected = if key == Key::Backspace {
            vec![FONTS[0], FONTS[0], FONTS[1], FONTS[0], FONTS[0], FONTS[0]]
        } else {
            vec![FONTS[0], FONTS[0], FONTS[0], FONTS[1], FONTS[0], FONTS[0]]
        };
        assert_eq!(families(&saved(&h, id)), expected);
    }

    let (mut h, id) = fixture(0, "abcdef");
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    surrender_focus(&mut h);
    key(h.state_mut(), Key::Backspace);
    h.run();
    key(h.state_mut(), Key::Backspace);
    h.run();
    assert!(super::insertion_style::pending(h.state()).is_some());
    super::host::type_text(h.state_mut(), "Z");
    h.run();
    assert_eq!(families(&saved(&h, id)), [FONTS[0], FONTS[1], FONTS[0], FONTS[0], FONTS[0]]);
}

#[test]
fn caret_font_insertion_preserves_variable_axes_and_style_changes_can_clear_weight_axis() {
    let (mut h, id) = fixture(0, "abcdef");
    h.state_mut().run("type.setStyle", json!({"layer": id.0, "variations": {"wght": 650, "wdth": 95}})).unwrap();
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    super::host::type_text(h.state_mut(), "X");
    let variations = |app: &PhotocraftApp, at: usize| {
        let mut offset = 0;
        layer(&app.session.active().unwrap().doc, id)
            .char_runs()
            .into_iter()
            .find_map(|r| {
                offset += r.len;
                (offset > at).then_some(r.style.variations)
            })
            .unwrap()
    };
    let axes = variations(h.state(), 3);
    assert!(axes.iter().any(|v| v.axis == "wght" && v.value == 650.0));
    assert!(axes.iter().any(|v| v.axis == "wdth" && v.value == 95.0));
    super::apply(h.state_mut(), json!({"fontStyle": "Bold"}), None);
    super::host::type_text(h.state_mut(), "Y");
    let axes = variations(h.state(), 4);
    assert!(!axes.iter().any(|v| v.axis == "wght"));
    assert!(axes.iter().any(|v| v.axis == "wdth" && v.value == 95.0));
}

#[test]
fn caret_font_and_face_persist_across_more_font_choices_and_paste() {
    let (mut h, id) = fixture(0, "abcdef");
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    super::apply(h.state_mut(), json!({"fontStyle": "Bold"}), None);
    choose(&mut h, FONTS[0], "enter");
    surrender_focus(&mut h);
    h.event(egui::Event::Paste("éX".into()));
    h.run();
    let t = saved(&h, id);
    for (i, c) in t["chars"].as_array().unwrap().iter().enumerate() {
        assert_eq!(c["font"], FONTS[0]);
        assert_eq!(c["weight"], if (3..5).contains(&i) { 700 } else { 400 });
    }
    let applied = saved(&h, id);
    checkmark(&mut h);
    assert_eq!(saved(&h, id), applied);
    for name in ["probe.pcraft", "probe.psd"] {
        let doc = h.state().session.active().unwrap().doc.clone();
        let out = photocraft_io::export(&doc, name, &Default::default()).unwrap();
        let back = photocraft_io::import(name, &out.bytes).unwrap().document;
        let imported_id = back.walk().into_iter().find_map(|(_, _, l)| matches!(l.content, LayerContent::Text(_)).then_some(l.id)).unwrap();
        assert_eq!(families(&styles(&back, imported_id)), families(&applied));
    }
    assert!(h.state_mut().session.undo());
    assert_eq!(saved(&h, id)["text"], "abcdef");
    assert!(h.state_mut().session.redo());
    assert_eq!(saved(&h, id), applied);
}

#[test]
fn an_existing_empty_layer_keeps_its_font_when_committed() {
    {
        let (mut h, id) = fixture(0, "abcdef");
        h.state_mut().run("type.edit", json!({"layer": id.0, "text": ""})).unwrap();
        select(&mut h, [0, 0]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        checkmark(&mut h);
        assert_eq!(saved(&h, id)["summary"], FONTS[1]);
        let ctx = h.ctx.clone();
        super::edit_active(h.state_mut(), &ctx).unwrap();
        super::host::type_text(h.state_mut(), "X");
        assert_eq!(families(&saved(&h, id)), [FONTS[1]]);
    }
}
