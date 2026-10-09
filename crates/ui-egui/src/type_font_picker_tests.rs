//! The Type tool's searchable font menu (#1369): it keeps its height and focuses its search field.

use super::font_picker_in;
use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

struct State {
    current: String,
    families: Vec<String>,
    popup: egui::Id,
}

fn harness() -> Harness<'static, State> {
    // Enough families that the unfiltered menu hits its maximum height and scrolls.
    let mut families: Vec<String> = (0..60).map(|i| format!("Family {i:02}")).collect();
    families.push("Zeta Sans".into());
    let state = State { current: "Family 00".into(), families, popup: egui::Id::NULL };
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).build_ui_state(
        |ui, s: &mut State| {
            // `ComboBox::from_id_salt` wraps its salt in an `IdSalt`; the popup id is the button's + "popup".
            s.popup = ui.make_persistent_id(egui::IdSalt::new("type-font")).with("popup");
            let fams = s.families.clone();
            font_picker_in(ui, &mut s.current, 170.0, &fams);
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
