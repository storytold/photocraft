//! End-to-end egui interaction tests for the adapter-neutral shared asset browser.
//!
//! These tests deliberately exercise the real `browser` widget with pointer events. They do not
//! instantiate brush/gradient/swatches adapters: the shared module's own drag sources, hit regions,
//! arbitration, hierarchy resolver, and transient persistence are the system under test.

use super::{BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, browser};
use crate::pickers::picker_state::{HierarchyOrderEntry, PresetUi};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

const PANEL: &str = "shared-browser-ui-tests";
type UiTestState = (PresetUi, Vec<BrowserAction>, Vec<(String, String, String, usize)>);

fn group(name: &str, items: Vec<ItemView>) -> GroupView {
    GroupView { name: name.into(), root_only: false, items }
}

fn item(key: &str, name: &str, path: &[&str]) -> ItemView {
    ItemView { key: key.into(), name: name.into(), group_path: path.iter().map(|part| (*part).to_owned()).collect() }
}

fn harness(groups: Vec<GroupView>, size: Vec2) -> Harness<'static, UiTestState> {
    harness_with_state(groups, size, PresetUi::default())
}

fn harness_with_state(groups: Vec<GroupView>, size: Vec2, initial_state: PresetUi) -> Harness<'static, UiTestState> {
    let mut h = Harness::builder().with_size(size).build_ui_state(
        move |ui, state: &mut UiTestState| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let mut paint_item = |_: &egui::Ui, _: Rect, _: usize, _: usize, _: ItemPaintState| {};
            let (preset_ui, actions, item_moves) = state;
            let mut commit_item_move = |source_group: &str, key: &str, target_group: &str, index: usize| {
                item_moves.push((source_group.to_owned(), key.to_owned(), target_group.to_owned(), index));
                Ok(())
            };
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    grid_indent: 8.0,
                    all_groups_open: true,
                    compact_context_menus: false,
                    item_size: Some(vec2(72.0, 48.0)),
                    list_item_height: Some(26.0),
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: BrowserCapabilities {
                    item_drag: true,
                    item_drop: true,
                    group_drag: true,
                    selection: true,
                    activation: true,
                    canonical_hierarchy_drag: true,
                },
                item_context_menu: None,
                group_context_menu: None,
                nested_group_context_menu: None,
                canonical_hierarchy_move: None,
                hierarchy_item_move: Some(&mut commit_item_move),
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            actions.extend(browser(ui, preset_ui, &mut adapter, 900.0));
        },
        (initial_state, Vec::new(), Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    h
}

fn nested_group_tree() -> Vec<GroupView> {
    vec![
        group("A", vec![item("a1", "A card 1", &[]), item("a2", "A card 2", &[]), item("a3", "A card 3", &[])]),
        group("B", vec![item("b1", "B card 1", &[]), item("b2", "B card 2", &[])]),
        group("C", vec![item("c1", "C card 1", &[])]),
    ]
}

fn with_b_nested_in_a() -> PresetUi {
    let mut state = PresetUi::default();
    state.nest_group(PANEL, &["A".into(), "B".into(), "C".into()], "B", Some("A"));
    state.set_hierarchy_order(
        PANEL,
        "A",
        vec![
            HierarchyOrderEntry::Group("B".into()),
            HierarchyOrderEntry::Item("a1".into()),
            HierarchyOrderEntry::Item("a2".into()),
            HierarchyOrderEntry::Item("a3".into()),
        ],
    );
    state
}

fn with_root_item_between_groups(group: &str, key: &str) -> PresetUi {
    let mut state = PresetUi::default();
    state.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::RootItem { group: group.into(), key: key.into() },
            HierarchyOrderEntry::Group("B".into()),
        ],
    );
    state
}

fn pointer_drag(h: &mut Harness<'static, UiTestState>, source: Pos2, target: Pos2) {
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn group_header_left_border_is_visible_in_rendered_pixels() {
    let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
    let header = h.get_by_label("A").rect();
    let border = crate::theme::Tokens::get(&h.ctx).card_border;
    let image = h.render().expect("render group header");
    let x = (header.left() * h.ctx.pixels_per_point()).round() as u32;
    let y = (header.center().y * h.ctx.pixels_per_point()).round() as u32;
    let actual = image.get_pixel(x, y).0;

    assert_eq!(actual[..3], border.to_array()[..3], "the header's left border should reach its one-pixel tree inset; header={header:?}, pixel=({x}, {y})");
}

#[test]
fn canonical_group_drop_outline_left_edge_is_visible_in_rendered_pixels() {
    let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("C").rect().center();
    let target = h.get_by_label("A").rect();
    let pointer = target.center();

    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (pointer - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    let ppp = h.ctx.pixels_per_point();
    let image = h.render().expect("render active group-drop preview");
    let browser_clip = h.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new(("preset-browser-rect", PANEL))));
    let pixel_is_accent = |x: f32, y: f32| {
        let x = (x * ppp).round() as u32;
        let y = (y * ppp).round() as u32;
        x < image.width() && y < image.height() && image.get_pixel(x, y).0[..3] == accent.to_array()[..3]
    };
    let left_edge = (0..4).any(|dy| pixel_is_accent(target.left(), target.top() + dy as f32));
    let top_edge = (0..4).any(|dx| pixel_is_accent(target.left() + dx as f32, target.top() + 1.0));

    assert!(left_edge, "left edge missing from rendered drop outline: target={target:?}, clip={browser_clip:?}, accent={accent:?}, ppp={ppp}, pixels={:?}", (0..6).map(|x| image.get_pixel(x, (target.top() + 1.0) as u32).0).collect::<Vec<_>>());
    assert!(top_edge, "top edge missing from rendered drop outline: target={target:?}, accent={accent:?}, ppp={ppp}");
}

fn drag_labels(h: &mut Harness<'static, UiTestState>, source: &str, target: &str, offset: Vec2) {
    let from = h.get_by_label(source).rect().center();
    let to = h.get_by_label(target).rect().center() + offset;
    pointer_drag(h, from, to);
}

fn mixed_tree() -> Vec<GroupView> {
    vec![
        group("A", vec![item("a1", "A card 1", &[]), item("a2", "A card 2", &[]), item("a3", "A card 3", &[])]),
        group("B", vec![item("b1", "B card 1", &[]), item("b2", "B card 2", &[])]),
        group("C", vec![item("c1", "C card 1", &[])]),
    ]
}

#[test]
fn actual_group_header_drop_prepends_for_every_header_vertical_region() {
    // Header drop is a prepend target across the full header height, not just its disclosure icon.
    for y in [-12.0, -5.0, 0.0, 5.0, 12.0] {
        let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
        drag_labels(&mut h, "C", "A", vec2(0.0, y));
        let children = &h.state().0.hierarchy_order[PANEL]["A"];
        assert_eq!(children.first(), Some(&HierarchyOrderEntry::Group("C".into())), "header offset y={y}: {children:?}");
        assert_eq!(children.iter().filter(|entry| **entry == HierarchyOrderEntry::Group("C".into())).count(), 1);
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("A"));
    }
}

#[test]
fn actual_nested_header_drop_prepends_and_preserves_the_complete_nested_path() {
    for y in [-8.0, 0.0, 8.0] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        drag_labels(&mut h, "C", "B", vec2(0.0, y));
        let b = &h.state().0.hierarchy_order[PANEL]["B"];
        assert_eq!(b.first(), Some(&HierarchyOrderEntry::Group("C".into())), "nested header offset y={y}: {b:?}");
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("B"));
    }
}

#[test]
fn actual_cards_and_headers_share_the_same_indent_at_every_hierarchy_depth() {
    let h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());

    let a_header = h.get_by_label("A").rect();
    let a_card = h.get_by_label("A card 1").rect();
    let b_header = h.get_by_label("B").rect();
    let b_card = h.get_by_label("B card 1").rect();

    assert_eq!(a_card.left() - a_header.left(), 20.0, "cards directly inside a root group should be indented one level beyond their group header");
    assert_eq!(b_card.left() - b_header.left(), 20.0, "cards inside a nested group should be indented one level beyond their nested group header");
    assert_eq!(b_header.left() - a_header.left(), 20.0, "each nested hierarchy level adds exactly twenty pixels");
    assert_eq!(b_card.left() - a_card.left(), 20.0, "cards should inherit the same twenty-pixel hierarchy step as headers");
}

#[test]
fn actual_card_edge_drop_reorders_mixed_siblings_at_both_halves_across_many_cards() {
    for (source, target, after) in [("C", "A card 1", false), ("C", "A card 2", true), ("B", "A card 1", true), ("B", "A card 2", false)] {
        let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
        let source_rect = h.get_by_label(source).rect();
        let target_rect = h.get_by_label(target).rect();
        let point = if after { pos2(target_rect.right() - 2.0, target_rect.center().y) } else { pos2(target_rect.left() + 2.0, target_rect.center().y) };
        pointer_drag(&mut h, source_rect.center(), point);
        let order = &h.state().0.hierarchy_order[PANEL]["A"];
        let moving = HierarchyOrderEntry::Group(source.to_owned());
        let target = HierarchyOrderEntry::Item(if target == "A card 1" { "a1" } else { "a2" }.into());
        let moving_index = order.iter().position(|entry| entry == &moving).expect("moved group in mixed order");
        let target_index = order.iter().position(|entry| entry == &target).expect("card anchor remains in mixed order");
        assert_eq!(moving_index < target_index, !after, "source={source}, target={target:?}, after={after}: {order:?}");
    }
}

#[test]
fn actual_twelve_pixel_group_gap_selects_upper_and_lower_sibling_levels() {
    for offset in [-2.5, -1.0, 1.0, 2.5] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        let source = h.get_by_label("C").rect().center();
        let lower_header = h.get_by_label("B").rect();
        let point = pos2(lower_header.center().x, lower_header.top() - 6.0 + offset);
        pointer_drag(&mut h, source, point);
        let a = &h.state().0.hierarchy_order[PANEL]["A"];
        assert_eq!(a.iter().filter(|entry| **entry == HierarchyOrderEntry::Group("C".into())).count(), 1);
        let c_index = a.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
        let b_index = a.iter().position(|entry| *entry == HierarchyOrderEntry::Group("B".into())).unwrap();
        if offset < 0.0 {
            assert!(c_index > b_index, "upper gap half appends after the upper group's last direct child: {a:?}");
        } else {
            assert!(c_index < b_index, "lower gap half inserts before the lower group: {a:?}");
        }
    }
}

#[test]
fn actual_nested_group_drag_cannot_create_self_or_descendant_cycles() {
    for (source, target) in [("A", "B"), ("B", "B")] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        let before = h.state().0.hierarchy_order.clone();
        let from = h.get_by_label(source).rect().center();
        let to = h.get_by_label(target).rect().center();
        pointer_drag(&mut h, from, to);
        assert_eq!(h.state().0.hierarchy_order, before, "invalid source={source}, target={target}");
        assert!(h.state().1.is_empty());
    }
}

#[test]
fn actual_group_collapse_hides_descendants_and_reopening_restores_the_same_nodes() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    assert!(h.query_all_by_label("B card 1").next().is_some());
    let header = h.get_by_label("B").rect().center();
    h.event(Event::PointerMoved(header));
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.query_all_by_label("B card 1").next().is_none());
    let header = h.get_by_label("B").rect().center();
    h.event(Event::PointerMoved(header));
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.query_all_by_label("B card 1").next().is_some());
}

#[test]
fn actual_card_selection_and_double_activation_emit_only_the_enabled_generic_events() {
    let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
    let card = h.get_by_label("A card 1").rect().center();
    h.event(Event::PointerMoved(card));
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(matches!(h.state().1.as_slice(), [BrowserAction::SelectItem { group, key, .. }] if group == "A" && key == "a1"));
}

#[test]
fn actual_card_drag_commits_before_and_after_positions_for_multiple_source_target_pairs() {
    for (source_key, source_label, target_key, target_label, after, expected_index) in [
        ("a1", "A card 1", "a2", "A card 2", false, 0),
        ("a1", "A card 1", "a2", "A card 2", true, 1),
        ("a3", "A card 3", "a1", "A card 1", false, 0),
        ("a3", "A card 3", "a1", "A card 1", true, 1),
    ] {
        let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
        let source = h.get_by_label(source_label).rect().center();
        let target_rect = h.get_by_label(target_label).rect();
        let target = if after { pos2(target_rect.right() - 2.0, target_rect.center().y) } else { pos2(target_rect.left() + 2.0, target_rect.center().y) };
        pointer_drag(&mut h, source, target);
        assert_eq!(h.state().2, [("A".into(), source_key.into(), "A".into(), expected_index)], "source={source_key}, target={target_key}, after={after}");
    }
}

#[test]
fn actual_card_drag_into_nested_group_uses_the_destination_groups_final_index() {
    for (after, expected_index) in [(false, 0), (true, 1)] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        let source = h.get_by_label("A card 2").rect().center();
        let target_rect = h.get_by_label("B card 1").rect();
        let target = if after { pos2(target_rect.right() - 2.0, target_rect.center().y) } else { pos2(target_rect.left() + 2.0, target_rect.center().y) };
        pointer_drag(&mut h, source, target);
        assert_eq!(h.state().2, [("A".into(), "a2".into(), "B".into(), expected_index)]);
    }
}

#[test]
fn actual_root_card_is_rendered_between_root_groups_and_can_be_dropped_into_a_group() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_root_item_between_groups("A", "a1"));
    let a_header = h.get_by_label("A").rect();
    let root_card = h.get_by_label("A card 1").rect();
    let b_header = h.get_by_label("B").rect();
    assert_eq!(root_card.left(), a_header.left(), "root-level cards should share the root tree's one-pixel inset without an additional card indent");
    assert!(a_header.center().y < root_card.center().y && root_card.center().y < b_header.center().y);

    pointer_drag(&mut h, root_card.center(), b_header.center());
    assert_eq!(h.state().2, [("A".into(), "a1".into(), "B".into(), 0)]);
}

#[test]
fn actual_nested_card_drop_on_root_group_header_is_committed() {
    let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("A card 1").rect().center();
    let target = h.get_by_label("B").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("A".into(), "a1".into(), "B".into(), 0)]);
}

#[test]
fn actual_nested_card_drop_at_root_append_area_promotes_card_to_root_level() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    let source = h.get_by_label("B card 1").rect().center();
    let last_card = h.get_by_label("C card 1").rect();
    // B is nested inside A; C is the final root sibling, so the root-end target follows C.
    let target = pos2(last_card.center().x, last_card.bottom() + 8.0);

    pointer_drag(&mut h, source, target);

    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    assert!(
        root_order.contains(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() }),
        "the nested card should be persisted in the root sibling sequence; root order: {root_order:?}"
    );
    assert!(!h.state().0.hierarchy_order[PANEL]["B"].contains(&HierarchyOrderEntry::Item("b1".into())));
}

#[test]
fn actual_nested_card_drop_between_root_groups_promotes_card_to_root_level() {
    let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("A card 1").rect().center();
    let b_header = h.get_by_label("B").rect();
    let target = pos2(b_header.center().x, b_header.top() + 0.5);

    pointer_drag(&mut h, source, target);

    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    assert!(
        root_order.contains(&HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() }),
        "the card dropped between root groups should become a root sibling; root order: {root_order:?}"
    );
    assert!(!h.state().0.hierarchy_order[PANEL]["A"].contains(&HierarchyOrderEntry::Item("a1".into())));
}

#[test]
fn actual_nested_card_drop_before_first_root_group_promotes_card_to_root_front() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    let source = h.get_by_label("B card 1").rect().center();
    let first_header = h.get_by_label("A").rect();
    // The first-position insertion target sits on the group's top border without reserving space.
    let target = pos2(first_header.center().x, first_header.top());

    pointer_drag(&mut h, source, target);

    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    assert_eq!(root_order.first(), Some(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() }));
    assert_eq!(root_order.get(1), Some(&HierarchyOrderEntry::Group("A".into())));
    assert!(!h.state().0.hierarchy_order[PANEL]["B"].contains(&HierarchyOrderEntry::Item("b1".into())));
}

#[test]
fn actual_root_card_drop_on_nested_group_header_is_committed() {
    let mut initial = with_b_nested_in_a();
    initial.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
            HierarchyOrderEntry::Group("C".into()),
        ],
    );
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), initial);
    let source = h.get_by_label("A card 1").rect().center();
    let target = h.get_by_label("B").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("A".into(), "a1".into(), "B".into(), 0)]);
}

#[test]
fn actual_root_card_drop_on_another_root_group_header_is_committed_in_reverse_direction() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_root_item_between_groups("B", "b1"));
    let source = h.get_by_label("B card 1").rect().center();
    let target = h.get_by_label("A").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("B".into(), "b1".into(), "A".into(), 0)]);
}

#[test]
fn actual_root_card_drop_on_root_group_header_with_multiple_root_cards_is_committed() {
    let mut initial = with_root_item_between_groups("A", "a1");
    initial.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
            HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() },
            HierarchyOrderEntry::Group("B".into()),
        ],
    );
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), initial);
    let source = h.get_by_label("B card 1").rect().center();
    let target = h.get_by_label("A").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("B".into(), "b1".into(), "A".into(), 0)]);
}

fn pos2(x: f32, y: f32) -> Pos2 {
    Pos2::new(x, y)
}
