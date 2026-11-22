//! The Type tool's family field: the options bar's `FontPicker`, the font state it edits, and the preview text.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{PhotocraftApp, Services};

/// Frames a harness may take to settle: opening the list loads the rows' samples over several passes.
const SETTLE_STEPS: u64 = 64;

fn app_with_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    app.ui.tool = crate::state::Tool::Type;
    app.sync_views();
    app
}

/// The options bar over `app`, on the app's own context.
fn bar_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    // The full app frame: the app takes the window's context on its first pass.
    Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(SETTLE_STEPS).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        app
    })
}

fn field<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    h.query_all_by_label("Font family").next().expect("the family field")
}

/// Opens the family field and types `name` into it.
fn type_family(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str) {
    field(h).click();
    h.run();
    field(h).type_text(name);
    h.run();
}

fn layer_font(app: &PhotocraftApp) -> String {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.font_family.clone(),
        _ => panic!("type layer"),
    }
}

#[test]
fn font_cycle_in_options_bar_edits_the_layer_and_can_be_undone() {
    let mut h = bar_harness(app_with_layer());
    h.run_steps(4);
    type_family(&mut h, "JetBrains");
    assert_eq!(layer_font(h.state()), "Inter", "typing creates no document edit");
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(layer_font(h.state()), "JetBrains Mono");
    let steps = h.state().session.active().unwrap().history.entries().len();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer_font(h.state()), "Inter");
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps - 1, "one pick is one undoable step");
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(layer_font(h.state()), "JetBrains Mono");
}

#[test]
fn hovering_a_font_previews_only_the_selected_characters() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "abcdef", "font": "Inter", "size": 18})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("type.setStyle", json!({"layer": layer.0, "range": [0, 2], "font": "JetBrains Mono"})).unwrap();
    app.ui.text_edit =
        Some(crate::state::TextEdit { layer: layer.0, document: None, caret: 5, anchor: 3, session: "font-preview-test".into(), created: false, mask: false });

    crate::type_font_preview::set(&mut app, "Inter".into());
    let (preview, _) = crate::type_font_preview::display_doc(&mut app, 0).expect("preview document");
    let fonts = |doc: &photocraft_doc::Document| {
        let photocraft_doc::LayerContent::Text(text) = &doc.layer(layer).unwrap().content else {
            panic!("type layer");
        };
        text.char_runs().into_iter().flat_map(|run| std::iter::repeat_n(run.style.font_family, run.len)).collect::<Vec<_>>()
    };

    assert_eq!(fonts(&preview), ["JetBrains Mono", "JetBrains Mono", "Inter", "Inter", "Inter", "Inter"]);
    assert_eq!(fonts(&app.session.active().unwrap().doc), ["JetBrains Mono", "JetBrains Mono", "Inter", "Inter", "Inter", "Inter"]);
}

/// A star goes to the engine's preferences library; a changed recent list lands in `UiState.recent_fonts`, capped
/// by the preference; the next pass's state starts from what was stored.
#[test]
fn a_clicked_star_is_saved_to_the_preferences_library() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    let mut state = crate::type_tool::font_state(&app);
    state.settings.set_favorite("Bebas Neue", true);
    state.settings.note_recent("Arial");
    crate::type_tool::finish_font_state(&mut app, state);
    assert_eq!(app.session.prefs().type_.favorite_fonts, ["Bebas Neue"]);
    assert_eq!(app.ui.recent_fonts, ["Arial"]);
    let next = crate::type_tool::font_state(&app);
    assert_eq!(next.settings.favorites, ["Bebas Neue"]);
    assert_eq!(next.settings.recent, ["Arial"]);

    let mut state = crate::type_tool::font_state(&app);
    state.settings.set_favorite("Bebas Neue", false);
    crate::type_tool::finish_font_state(&mut app, state);
    assert!(app.session.prefs().type_.favorite_fonts.is_empty(), "un-starring removes the favourite");

    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 1})).unwrap();
    let mut state = crate::type_tool::font_state(&app);
    state.settings.note_recent("Zeta");
    crate::type_tool::finish_font_state(&mut app, state);
    assert_eq!(app.ui.recent_fonts.len(), 1, "the recent list is capped at once");
}

#[test]
fn the_preview_sets_the_type_layers_text_or_the_selection_being_typed() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert_eq!(super::preview_text(&app), None, "no document");
    app.run("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
    let id = app.run("type.create", serde_json::json!({"text": "Hello\rworld  ", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", serde_json::json!({"id": id})).ok();
    app.ui.text_edit = Some(crate::state::TextEdit { layer: id, document: None, caret: 0, anchor: 0, session: "s".into(), created: false, mask: false });
    assert_eq!(super::preview_text(&app).as_deref(), Some("Hello world"), "the whole text, on one line");
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = 6;
        ed.caret = 11;
    }
    assert_eq!(super::preview_text(&app).as_deref(), Some("world"), "only the selection while typing");
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = 50;
        ed.caret = 400;
    }
    assert_eq!(super::preview_text(&app), None, "an out-of-range selection is blank, not a panic");
}

/// #5152: with the Character panel's picker also shown, the open picker previews and the closed one never clears it.
#[test]
fn options_bar_hover_previews_with_the_character_picker_also_shown() {
    use photocraft_doc::LayerContent;

    let theme = crate::theme::AppTheme::pro();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(SETTLE_STEPS).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Services::default());
        app.ui.theme = theme;
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
        app.ui.tool = crate::state::Tool::Type;
        app.sync_views();
        app
    });
    h.run_steps(8);
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let font = |h: &mut Harness<'static, Box<PhotocraftApp>>| {
        let (shown, _) = crate::canvas::display_doc(h.state_mut(), 0);
        let LayerContent::Text(t) = &shown.layer(layer).unwrap().content else { panic!("type layer") };
        t.char_runs()[0].style.font_family.clone()
    };
    assert_eq!(h.query_all_by_label("Font family").count(), 2, "options bar and Character section both show the font picker");
    h.query_all_by_label("Font family").next().unwrap().click();
    h.run();
    h.query_all_by_label("Font family").next().unwrap().type_text("JetBrains Mono");
    h.run();
    h.get_by_role_and_label(Role::TreeItem, "JetBrains Mono").hover();
    h.run();
    assert_eq!(font(&mut h), "JetBrains Mono");
    h.query_all_by_label("Font family").next().unwrap().hover();
    h.run();
    assert_eq!(font(&mut h), "Inter", "leaving the row restores the committed font");
    let _ = Role::ComboBox;
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// Keys typed into the font search stay there, on the first opening and during an edit session: the first key
/// switches no tool, and no text reaches the type layer.
#[test]
fn typed_keys_stay_in_the_font_search_and_reach_no_tool_or_layer() {
    for editing in [false, true] {
        let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(SETTLE_STEPS).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Services::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
            app.ui.tool = crate::state::Tool::Type;
            if editing {
                super::edit_active(&mut app, &cc.egui_ctx).unwrap();
            }
            app.sync_views();
            app
        });
        h.run_steps(8);
        let layers = h.state().session.active().unwrap().doc.layer_count();
        field(&h).click();
        h.run();
        // Native typing carries a key event with the text: a key that left the search would pick a tool (J) or
        // replace the edited text.
        h.event(egui::Event::Key { key: egui::Key::J, physical_key: Some(egui::Key::J), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::Text("JetBrains".into()));
        h.run();
        assert_eq!(field(&h).value().as_deref(), Some("JetBrains"), "the search took the keys (editing: {editing})");
        assert_eq!(h.state().ui.tool, crate::state::Tool::Type, "no tool shortcut ran (editing: {editing})");
        let st = h.state().session.active().unwrap();
        let photocraft_doc::LayerContent::Text(text) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("type layer") };
        assert_eq!(text.text, "Sample", "the layer's text is unchanged (editing: {editing})");
        assert_eq!(st.doc.layer_count(), layers, "no layer was added (editing: {editing})");
    }
}

/// Hovering a family previews it on the canvas and adds no history step; picking it commits one step.
#[test]
fn hovering_a_font_adds_no_history_step_and_a_pick_adds_one() {
    let mut h = bar_harness(app_with_layer());
    h.run_steps(4);
    let before = history_len(h.state());
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    type_family(&mut h, "JetBrains Mono");
    h.get_by_role_and_label(Role::TreeItem, "JetBrains Mono").hover();
    h.run();
    let (shown, _) = crate::canvas::display_doc(h.state_mut(), 0);
    let photocraft_doc::LayerContent::Text(text) = &shown.layer(layer).unwrap().content else { panic!("type layer") };
    assert_eq!(text.char_runs()[0].style.font_family, "JetBrains Mono", "the canvas previews the hovered family");
    assert_eq!(layer_font(h.state()), "Inter", "hover commits nothing");
    assert_eq!(history_len(h.state()), before, "hover adds no history step");
    h.get_by_role_and_label(Role::TreeItem, "JetBrains Mono").click();
    h.run();
    assert_eq!(layer_font(h.state()), "JetBrains Mono");
    assert_eq!(history_len(h.state()), before + 1, "the pick is one history step");
}
