//! The Type tool's searchable font menu (#1369): it keeps its height and focuses its search field.

use super::font_picker_in;
use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::prefs::FontPreview;

struct State {
    current: String,
    families: Vec<String>,
    popup: egui::Id,
    preview: FontPreview,
}

fn harness() -> Harness<'static, State> {
    harness_with(FontPreview::Medium)
}

fn harness_with(preview: FontPreview) -> Harness<'static, State> {
    // Enough families that the unfiltered menu hits its maximum height and scrolls.
    let mut families: Vec<String> = (0..60).map(|i| format!("Family {i:02}")).collect();
    families.push("Zeta Sans".into());
    let state = State { current: "Family 00".into(), families, popup: egui::Id::NULL, preview };
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).build_ui_state(
        |ui, s: &mut State| {
            // `ComboBox::from_id_salt` wraps its salt in an `IdSalt`; the popup id is the button's + "popup".
            s.popup = ui.make_persistent_id(egui::IdSalt::new("type-font")).with("popup");
            let fams = s.families.clone();
            font_picker_in(ui, &mut s.current, 170.0, &fams, s.preview);
        },
        state,
    );
    h.run();
    h
}

fn popup_rect(h: &Harness<'static, State>) -> Option<Rect> {
    let id = h.state().popup;
    let open = egui::Popup::is_id_open(&h.ctx, id);
    if open { h.ctx.memory(|m| m.area_rect(id)) } else { None }
}

fn open(h: &mut Harness<'static, State>) {
    h.get_by_role(Role::ComboBox).click();
    h.run();
}

#[test]
fn clearing_the_query_restores_the_menu_height() {
    let mut h = harness();
    open(&mut h);
    let full = popup_rect(&h).expect("menu open").height();
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    assert!(h.query_by_label("Family 01").is_none(), "the query filters the list");
    let searching = popup_rect(&h).expect("still open").height();
    for _ in 0.."zeta".len() {
        h.key_press(Key::Backspace);
    }
    h.run();
    assert!(h.query_by_label("Family 01").is_some(), "the cleared query lists every family");
    let cleared = popup_rect(&h).expect("still open").height();
    assert!((cleared - full).abs() < 0.5, "height after clearing {cleared} != original {full}");
    assert!((searching - full).abs() < 0.5, "the menu keeps its height while searching: {searching} vs {full}");
}

#[test]
fn reopening_focuses_the_search_field_and_clicking_it_keeps_the_menu_open() {
    let mut h = harness();
    open(&mut h);
    assert!(h.get_by_role(Role::TextInput).is_focused(), "search is focused on first open");
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    // Close without choosing, then reopen: the query stays and the field is focused again.
    h.key_press(Key::Escape);
    h.run();
    assert!(popup_rect(&h).is_none(), "Escape closes the menu");
    open(&mut h);
    let search = h.get_by_role(Role::TextInput);
    assert_eq!(search.value().as_deref(), Some("zeta"));
    assert!(search.is_focused(), "search is focused on reopen with a query");
    // Clicking the search field (to edit the query) must not close the menu.
    h.get_by_role(Role::TextInput).click();
    h.run();
    assert!(popup_rect(&h).is_some(), "clicking the search field keeps the menu open");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role(Role::TextInput).type_text("Zeta S");
    h.run();
    assert!(popup_rect(&h).is_some());
    // Picking a font still closes the menu.
    h.get_by_label("Zeta Sans").click();
    h.run();
    assert_eq!(h.state().current, "Zeta Sans");
    assert!(popup_rect(&h).is_none(), "choosing a font closes the menu");
}

#[test]
fn arrows_apply_fonts_without_closing_and_enter_accepts() {
    let mut h = harness();
    open(&mut h);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 01");
    assert!(popup_rect(&h).is_some());
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 01");
    h.key_press(Key::Enter);
    h.run();
    assert!(popup_rect(&h).is_none());
}

#[test]
fn arrows_follow_the_filtered_list_and_handle_no_matches() {
    let mut h = harness();
    open(&mut h);
    h.get_by_role(Role::TextInput).type_text("Family 5");
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 50");
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 50", "clamp at the first match");
    h.get_by_role(Role::TextInput).type_text("missing");
    h.run();
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().current, "Family 50");
    assert!(popup_rect(&h).is_some(), "no match cannot accept an unrelated font");
}

#[test]
fn arrows_scroll_the_selected_font_into_view() {
    let mut h = harness();
    open(&mut h);
    for _ in 0..59 {
        h.key_press(Key::ArrowDown);
        h.run();
    }
    assert_eq!(h.state().current, "Family 59");
    let row = h.get_by_label("Family 59");
    assert!(popup_rect(&h).unwrap().contains(row.rect().center()));
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Zeta Sans", "clamp at the last font");
}

#[test]
fn empty_font_list_ignores_navigation() {
    let mut h = harness();
    h.state_mut().families.clear();
    open(&mut h);
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().current, "Family 00");
    assert!(popup_rect(&h).is_some());
}

#[test]
fn font_cycle_in_options_bar_edits_the_layer_and_can_be_undone() {
    use crate::{PhotocraftApp, Services};
    use serde_json::json;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| crate::type_tool::options_bar(app, ui));
            }
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("JetBrains");
    h.key_press(Key::ArrowDown);
    h.run();
    let font = |app: &PhotocraftApp| {
        let doc = &app.session.active().unwrap().doc;
        match &doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => t.font_family.clone(),
            _ => panic!("type layer"),
        }
    };
    assert_eq!(font(h.state()), "JetBrains Mono");
    h.key_press(Key::Enter);
    h.run();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(font(h.state()), "Inter");
}

/// Preferences ▸ Type ▸ Font Preview Size: Extra Large grows the sample rows, Off drops the
/// samples (rows shrink to the plain height).
#[test]
fn the_preview_size_preference_resizes_or_removes_the_samples() {
    let mut h = harness_with(FontPreview::Medium);
    open(&mut h);
    let medium = h.get_by_label("Family 00").rect().height();
    assert!((medium - 28.0).abs() < 0.5, "medium rows hold the 24 px sample: {medium}");
    let mut h = harness_with(FontPreview::ExtraLarge);
    open(&mut h);
    let extra = h.get_by_label("Family 00").rect().height();
    assert!(extra > medium, "extra large rows are taller: {extra} vs {medium}");
    let mut h = harness_with(FontPreview::Off);
    open(&mut h);
    let off = h.get_by_label("Family 00").rect().height();
    assert!((off - 28.0).abs() < 0.5, "off keeps the plain row height: {off}");
}
