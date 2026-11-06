//! End-to-end egui interaction tests for the adapter-neutral shared asset browser.
//!
//! These tests deliberately exercise the real `browser` widget with pointer events. They do not
//! instantiate brush/gradient/swatches adapters: the shared module's own drag sources, hit regions,
//! arbitration, hierarchy resolver, and transient persistence are the system under test.

use super::*;
use crate::pickers::picker_browser_dragging::DropMoveIntent;
use crate::pickers::picker_browser_identity::PickerIdentityKey;
use crate::pickers::picker_browser_model::PickerId;
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
    harness_with_state_layout(groups, size, initial_state, false)
}

fn harness_with_state_layout(groups: Vec<GroupView>, size: Vec2, mut initial_state: PresetUi, list: bool) -> Harness<'static, UiTestState> {
    initial_state.sizes.insert(PANEL.to_owned(), if list { super::MIN_THUMB } else { 40.0 });
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
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
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
                rekey_hierarchy_after_move: None,
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

fn harness_with_intent_capture(
    groups: Vec<GroupView>,
    size: Vec2,
    mut initial_state: PresetUi,
    list: bool,
) -> Harness<'static, (PresetUi, Vec<DropMoveIntent>)> {
    initial_state.sizes.insert(PANEL.to_owned(), if list { super::MIN_THUMB } else { 40.0 });
    let mut h = Harness::builder().with_size(size).build_ui_state(
        move |ui, state: &mut (PresetUi, Vec<DropMoveIntent>)| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let (preset_ui, intents) = state;
            let mut capture_intent =
                |intent: &DropMoveIntent, _identities: &std::collections::BTreeMap<PickerId, crate::pickers::picker_browser_identity::PickerNodeIdentity>| {
                    intents.push(intent.clone());
                    Ok(())
                };
            let mut paint_item = |_: &egui::Ui, _: Rect, _: usize, _: usize, _: ItemPaintState| {};
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
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
                canonical_hierarchy_move: Some(&mut capture_intent),
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: None,
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            browser(ui, preset_ui, &mut adapter, 900.0);
        },
        (initial_state, Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    h
}

#[test]
fn all_display_modes_allocate_the_exact_requested_card_height() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let groups = vec![group("A", vec![item("one", "One card", &[])])];
    for (mode, expected_height) in
        [(Mode::TextOnly, 20.0), (Mode::SmallThumbnail, 32.0), (Mode::LargeThumbnail, 64.0), (Mode::SmallList, 20.0), (Mode::LargeList, 34.0)]
    {
        let mut state = PresetUi::default();
        state.display_modes.insert(PANEL.to_owned(), mode);
        let harness = harness_with_state(groups.clone(), vec2(520.0, 520.0), state);
        let rect = harness.query_all_by_label("One card").map(|node| node.rect()).max_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
        assert_eq!(rect.height(), expected_height, "display mode {mode:?} allocated {rect:?}");
    }
}

#[test]
fn wrapped_list_uses_fixed_width_column_major_flow_and_shift_selection_order() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let groups = vec![group("A", vec![item("a1", "Card 1", &[]), item("a2", "Card 2", &[]), item("a3", "Card 3", &[]), item("a4", "Card 4", &[])])];
    let mut state = PresetUi::default();
    state.display_modes.insert(PANEL.to_owned(), Mode::SmallList);
    state.wrap_list.insert(PANEL.to_owned(), true);
    let mut harness = harness_with_state(groups, vec2(520.0, 520.0), state);

    let first = harness.get_by_label("Card 1").rect();
    let second = harness.get_by_label("Card 2").rect();
    let third = harness.get_by_label("Card 3").rect();
    assert_eq!(first.width(), 198.0);
    assert_eq!(second.width(), 198.0);
    assert_eq!(first.left(), second.left());
    assert!(second.top() > first.top(), "second item should be below the first in column one");
    assert!(third.left() > first.left(), "third item should begin column two");
    assert_eq!(third.top(), first.top(), "third item should return to the top of column two");

    let click = |harness: &mut Harness<'static, UiTestState>, point: Pos2, modifiers: Modifiers| {
        harness.hover_at(point);
        harness.event(Event::ModifiersChanged(modifiers));
        harness.run_steps(1);
        harness.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers });
        harness.run_steps(1);
        harness.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers });
        harness.run_steps(2);
        harness.event(Event::ModifiersChanged(Modifiers::NONE));
        harness.run_steps(1);
    };
    click(&mut harness, second.center(), Modifiers::NONE);
    click(&mut harness, third.center(), Modifiers::SHIFT);

    let selected = &harness.state().0.browser_selection[PANEL];
    assert!(selected.contains(&PickerId::item("A", "a2")));
    assert!(selected.contains(&PickerId::item("A", "a3")));
    assert_eq!(selected.len(), 2, "Shift-selection should follow the visual column-major order");
}

#[test]
fn large_wrapped_list_uses_245px_content_width_plus_its_two_pixel_border() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let groups = vec![group("A", vec![item("a1", "Large card 1", &[]), item("a2", "Large card 2", &[]), item("a3", "Large card 3", &[])])];
    let mut state = PresetUi::default();
    state.display_modes.insert(PANEL.to_owned(), Mode::LargeList);
    state.wrap_list.insert(PANEL.to_owned(), true);
    let harness = harness_with_state(groups, vec2(620.0, 520.0), state);

    let first = harness.get_by_label("Large card 1").rect();
    let second = harness.get_by_label("Large card 2").rect();
    let third = harness.get_by_label("Large card 3").rect();
    assert_eq!(first.height(), 34.0);
    assert_eq!(first.width(), 247.0);
    assert_eq!(second.width(), 247.0);
    assert_eq!(first.left(), second.left());
    assert!(second.top() > first.top());
    assert!(third.left() > first.left());
    assert_eq!(third.top(), first.top());
}

#[test]
fn unwrapped_list_modes_use_full_width_and_keep_one_item_per_row() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let groups = vec![group("A", vec![item("a1", "Unwrapped card 1", &[]), item("a2", "Unwrapped card 2", &[])])];
    for mode in [Mode::SmallList, Mode::LargeList] {
        let mut state = PresetUi::default();
        state.display_modes.insert(PANEL.to_owned(), mode);
        state.wrap_list.insert(PANEL.to_owned(), false);
        let harness = harness_with_state(groups.clone(), vec2(520.0, 520.0), state);
        let first = harness.get_by_label("Unwrapped card 1").rect();
        let second = harness.get_by_label("Unwrapped card 2").rect();
        let header = harness.get_by_label("A").rect();
        assert!(first.width() > 400.0, "unwrapped {mode:?} should span the available row: {first:?}");
        assert!(
            first.right() <= header.right() + 0.5 && first.right() >= header.right() - 8.0,
            "unwrapped {mode:?} cards align with the header's visible right edge while retaining the ScrollArea scrollbar gutter: card={first:?}, header={header:?}"
        );
        assert!(first.left() > header.left(), "unwrapped {mode:?} cards retain their hierarchy inset");
        assert_eq!(first.left(), second.left());
        assert!(second.top() > first.top(), "unwrapped {mode:?} must keep one item per row");
    }
}

#[test]
fn text_only_is_a_list_mode_but_keeps_full_width_rows() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let groups = vec![group("A", vec![item("a1", "Text card 1", &[]), item("a2", "Text card 2", &[])])];
    let mut state = PresetUi::default();
    state.display_modes.insert(PANEL.to_owned(), Mode::TextOnly);
    state.wrap_list.insert(PANEL.to_owned(), true);
    let harness = harness_with_state(groups, vec2(520.0, 520.0), state);

    let first = harness.query_all_by_label("Text card 1").map(|node| node.rect()).max_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
    let second = harness.query_all_by_label("Text card 2").map(|node| node.rect()).max_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
    assert!(first.width() > 400.0, "Text Only must remain full-width despite a stored Wrap List preference: {first:?}");
    assert_eq!(first.left(), second.left());
    assert!(second.top() > first.top());
    assert!(harness.state().0.wrap_list_enabled(PANEL), "Text Only participates in the shared list-mode preference");
}

#[test]
fn text_only_elides_long_fixed_width_labels_without_losing_the_full_accessible_name() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let name = "A very long preset name that must be elided rather than painted over adjacent controls";
    let groups = vec![group("A", vec![item("long", name, &[])])];
    let mut state = PresetUi::default();
    state.display_modes.insert(PANEL.to_owned(), Mode::TextOnly);
    let mut harness = harness_with_state(groups, vec2(520.0, 520.0), state);

    let row = harness.query_all_by_label(name).map(|node| node.rect()).max_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
    let label = harness.query_all_by_label(name).map(|node| node.rect()).min_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
    assert!(label.width() < row.width(), "the label is width-constrained within its full-width row: {label:?} vs {row:?}");
    harness.hover_at(label.center());
    harness.run_steps(3);
    assert!(harness.query_all_by_label(name).next().is_some(), "the full preset name remains available to accessibility and hover help");
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

fn with_nested_item_before_group() -> PresetUi {
    let mut state = with_b_nested_in_a();
    state.set_hierarchy_order(
        PANEL,
        "A",
        vec![
            HierarchyOrderEntry::Item("a1".into()),
            HierarchyOrderEntry::Group("B".into()),
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

fn with_root_item_before_group(group: &str, key: &str, lower_group: &str) -> PresetUi {
    let mut state = PresetUi::default();
    state.set_hierarchy_order(
        PANEL,
        "",
        vec![HierarchyOrderEntry::RootItem { group: group.into(), key: key.into() }, HierarchyOrderEntry::Group(lower_group.into())],
    );
    state
}

fn with_consecutive_root_cards_from_different_groups() -> PresetUi {
    let mut state = PresetUi::default();
    state.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
            HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() },
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::Group("B".into()),
            HierarchyOrderEntry::Group("C".into()),
        ],
    );
    state
}

fn pointer_drag<S>(h: &mut Harness<'static, S>, source: Pos2, target: Pos2) {
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
fn dragging_an_unselected_group_selects_it_after_the_drop() {
    let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("C").rect().center();
    let target = h.get_by_label("A").rect().center();

    assert!(!h.state().0.browser_node_selected(PANEL, &PickerId::group("C")));
    pointer_drag(&mut h, source, target);

    assert!(h.state().0.browser_node_selected(PANEL, &PickerId::group("C")), "the dragged group should become selected when its drop finishes");
}

#[test]
fn dragging_an_unselected_card_selects_it_after_the_drop() {
    let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("A card 2").rect().center();
    let target = h.get_by_label("A card 1").rect().center();
    let dragged = PickerId::item("A", "a2");

    assert!(!h.state().0.browser_node_selected(PANEL, &dragged));
    pointer_drag(&mut h, source, target);

    assert!(h.state().0.browser_node_selected(PANEL, &dragged), "the dragged card should become selected when its drop finishes");
    assert!(h.state().0.is_selected_item(PANEL, "A", &[], "a2"), "the dragged card should become the domain-selected preset");
}

#[test]
fn dropping_a_card_into_a_folder_selects_it_at_its_new_location() {
    let groups = vec![group("A", vec![item("folder-card", "Folder card", &["Folder"])]), group("B", vec![item("b1", "B card 1", &[])])];
    let mut h = harness_with_intent_capture(groups, vec2(520.0, 900.0), PresetUi::default(), false);
    let source = h.get_by_label("B card 1").rect().center();
    let target = h.get_by_label("Folder").rect().center();

    pointer_drag(&mut h, source, target);

    assert!(
        h.state().0.is_selected_item(PANEL, "A", &["Folder".into()], "b1"),
        "the dropped card must be selected using its destination group and folder path"
    );
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

    assert!(
        left_edge,
        "left edge missing from rendered drop outline: target={target:?}, clip={browser_clip:?}, accent={accent:?}, ppp={ppp}, pixels={:?}",
        (0..6).map(|x| image.get_pixel(x, (target.top() + 1.0) as u32).0).collect::<Vec<_>>()
    );
    assert!(top_edge, "top edge missing from rendered drop outline: target={target:?}, accent={accent:?}, ppp={ppp}");
}

#[test]
fn item_drag_preview_presses_against_asset_browser_viewport_edge() {
    let groups = vec![group("A", vec![item("a1", "A card", &[])])];
    let ghost_rect = std::rc::Rc::new(std::cell::Cell::new(None::<Rect>));
    let ghost_rect_ui = ghost_rect.clone();
    let mut initial_state = PresetUi::default();
    initial_state.display_modes.insert(PANEL.to_owned(), crate::pickers::picker_state::PickerDisplayMode::SmallList);
    let mut h = Harness::builder().with_size(vec2(420.0, 420.0)).build_ui_state(
        move |ui, state: &mut (PresetUi, Vec<BrowserAction>)| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let picker_clip = Rect::from_min_size(pos2(32.0, 24.0), vec2(300.0, 220.0));
            ui.set_clip_rect(ui.clip_rect().intersect(picker_clip));
            ui.add_space(24.0);
            let mut paint_item = |ui: &egui::Ui, rect: Rect, _: usize, _: usize, state: ItemPaintState| {
                if matches!(state, ItemPaintState::DragGhost) {
                    ghost_rect_ui.set(Some(rect));
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(255, 0, 255));
                }
            };
            let mut paint_list_item = |ui: &egui::Ui, rect: Rect, _: usize, _: usize, _: bool| {
                ghost_rect_ui.set(Some(rect));
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(255, 0, 255));
            };
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: BrowserCapabilities {
                    item_drag: true,
                    item_drop: false,
                    group_drag: false,
                    selection: false,
                    activation: false,
                    canonical_hierarchy_drag: true,
                },
                item_context_menu: None,
                group_context_menu: None,
                nested_group_context_menu: None,
                canonical_hierarchy_move: None,
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: None,
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: Some(&mut paint_list_item),
            };
            let (preset_ui, actions) = state;
            actions.extend(browser(ui, preset_ui, &mut adapter, 180.0));
        },
        (initial_state, Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);

    let source = h.get_by_label("A card").rect();
    let browser_clip = h.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new(("preset-browser-rect", PANEL)))).expect("browser viewport");
    let picker_clip = Rect::from_min_size(pos2(32.0, 24.0), vec2(300.0, 220.0));
    let app_clip = h.ctx.content_rect();
    let pointer = pos2(source.center().x, browser_clip.bottom() - 8.0);
    assert!(browser_clip.contains(source.center()));
    assert!(browser_clip.contains(pointer), "test pointer must remain inside the asset-browser viewport: {browser_clip:?}, pointer={pointer:?}");
    assert!(picker_clip.contains(pointer), "test pointer must remain inside the picker shell: {picker_clip:?}, pointer={pointer:?}");
    assert!(app_clip.contains(pointer), "test pointer must remain inside the app window: {app_clip:?}, pointer={pointer:?}");

    h.event(Event::PointerMoved(source.center()));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source.center(), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let drag_start = source.center() + vec2(12.0, 0.0);
    h.event(Event::PointerMoved(drag_start));
    h.run_steps(1);
    h.event(Event::PointerMoved(pointer));
    h.run_steps(2);

    let expected = Rect::from_min_size(pos2(source.center().x - source.width() * 0.5, browser_clip.bottom() - source.height()), source.size());
    assert_eq!(ghost_rect.get(), Some(expected), "list ghost should reuse the complete source row and remain clamped inside the asset-browser viewport");
    let image = h.render().expect("render dragged item preview against picker edge");
    let x = (expected.center().x * h.ctx.pixels_per_point()).round() as u32;
    let y = (expected.center().y * h.ctx.pixels_per_point()).round() as u32;
    let edge_pixel = image.get_pixel(x, y).0;
    assert!(ghost_rect.get().is_some(), "the adapter's drag-preview painter should run outside the ScrollArea");
    assert!(
        edge_pixel[0] > edge_pixel[1] && edge_pixel[2] > edge_pixel[1],
        "drag preview should remain visible while pressed against the asset-browser viewport edge; picker={picker_clip:?}, browser={browser_clip:?}, pointer={pointer:?}, edge_pixel={edge_pixel:?}"
    );
}

#[test]
fn item_drag_preview_follows_cursor_with_original_pickup_offset_after_leaving_asset_browser_viewport() {
    let groups = vec![group("A", vec![item("a1", "A card", &[])])];
    let ghost_rect = std::rc::Rc::new(std::cell::Cell::new(None::<Rect>));
    let ghost_rect_ui = ghost_rect.clone();
    let mut h = Harness::builder().with_size(vec2(420.0, 420.0)).build_ui_state(
        move |ui, state: &mut (PresetUi, Vec<BrowserAction>)| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            // Reset every frame so the assertion below proves the ghost was painted again,
            // rather than observing a rectangle retained from the previous frame.
            ghost_rect_ui.set(None);
            let picker_clip = Rect::from_min_size(pos2(32.0, 24.0), vec2(300.0, 220.0));
            ui.set_clip_rect(ui.clip_rect().intersect(picker_clip));
            ui.add_space(24.0);
            let mut paint_item = |ui: &egui::Ui, rect: Rect, _: usize, _: usize, state: ItemPaintState| {
                if matches!(state, ItemPaintState::DragGhost) {
                    ghost_rect_ui.set(Some(rect));
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(255, 0, 255));
                }
            };
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: BrowserCapabilities {
                    item_drag: true,
                    item_drop: false,
                    group_drag: false,
                    selection: false,
                    activation: false,
                    canonical_hierarchy_drag: true,
                },
                item_context_menu: None,
                group_context_menu: None,
                nested_group_context_menu: None,
                canonical_hierarchy_move: None,
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: None,
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            let (preset_ui, actions) = state;
            actions.extend(browser(ui, preset_ui, &mut adapter, 180.0));
        },
        (PresetUi::default(), Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);

    let source = h.get_by_label("A card").rect();
    let app_clip = h.ctx.content_rect();
    let picker_clip = Rect::from_min_size(pos2(32.0, 24.0), vec2(300.0, 220.0));
    let browser_clip = h.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new(("preset-browser-rect", PANEL)))).expect("browser viewport");
    let pointer = pos2(source.center().x, browser_clip.bottom() + 8.0);
    assert!(!browser_clip.contains(pointer), "test pointer must be outside the ScrollArea: {browser_clip:?}, pointer={pointer:?}");
    assert!(picker_clip.contains(pointer), "test pointer must remain inside the picker shell, in its footer region: pointer={pointer:?}");
    assert!(app_clip.contains(pointer));
    h.event(Event::PointerMoved(source.center()));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source.center(), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source.center() + vec2(12.0, 0.0)));
    h.run_steps(1);
    h.event(Event::PointerMoved(pointer));
    h.run_steps(2);
    let expected_outside = Rect::from_min_size(pointer - source.size() * 0.5, source.size());
    assert_eq!(
        ghost_rect.get(),
        Some(expected_outside),
        "outside the asset-browser viewport, the ghost must follow the cursor while preserving the original center grab offset"
    );

    // Also cover the native PointerGone event path. Do not first send an artificial out-of-bounds
    // PointerMoved event here; the preceding assertion already covers leaving the browser viewport.
    h.remove_cursor();
    h.run_steps(2);
    assert_eq!(
        ghost_rect.get(),
        Some(source),
        "without a pointer position, the ghost falls back to its pickup bounds; app={app_clip:?}, picker={picker_clip:?}"
    );
    let image = h.render().expect("render dragged item preview after PointerGone");
    let source_center = source.center();
    let ppp = h.ctx.pixels_per_point();
    let x = (source_center.x * ppp).round() as u32;
    let y = (source_center.y * ppp).round() as u32;
    let snapped_pixel = image.get_pixel(x, y).0;
    assert!(
        snapped_pixel[0] > snapped_pixel[1] && snapped_pixel[2] > snapped_pixel[1],
        "drag preview should fall back to its pickup position when PointerGone removes the cursor; source={source:?}, app={app_clip:?}, picker={picker_clip:?}, pixel={snapped_pixel:?}"
    );
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
fn actual_open_nonempty_group_header_drops_into_the_group_across_its_full_height() {
    for y in [-12.0, -5.0, 0.0] {
        let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
        drag_labels(&mut h, "C", "A", vec2(0.0, y));
        let children = &h.state().0.hierarchy_order[PANEL]["A"];
        assert_eq!(children.first(), Some(&HierarchyOrderEntry::Group("C".into())), "header offset y={y}: {children:?}");
        assert_eq!(children.iter().filter(|entry| **entry == HierarchyOrderEntry::Group("C".into())).count(), 1);
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("A"));
    }

    for y in [5.0, 12.0] {
        let mut h = harness(mixed_tree(), vec2(520.0, 900.0));
        drag_labels(&mut h, "C", "A", vec2(0.0, y));
        let children = &h.state().0.hierarchy_order[PANEL]["A"];
        assert_eq!(children.first(), Some(&HierarchyOrderEntry::Group("C".into())), "header offset y={y}: {children:?}");
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("A"), "lower-half drop into an open non-empty group");
    }
}

#[test]
fn actual_open_nested_nonempty_header_drops_into_the_group_across_its_full_height() {
    for y in [-8.0, 0.0] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        drag_labels(&mut h, "C", "B", vec2(0.0, y));
        let b = &h.state().0.hierarchy_order[PANEL]["B"];
        assert_eq!(b.first(), Some(&HierarchyOrderEntry::Group("C".into())), "nested header offset y={y}: {b:?}");
        assert_eq!(h.state().0.parent_of(PANEL, "B"), Some("A"));
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("B"));
    }

    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    drag_labels(&mut h, "C", "B", vec2(0.0, 8.0));
    let b = &h.state().0.hierarchy_order[PANEL]["B"];
    assert_eq!(b.first(), Some(&HierarchyOrderEntry::Group("C".into())), "lower-half drop into open non-empty B: {b:?}");
    assert_eq!(h.state().0.parent_of(PANEL, "B"), Some("A"));
    assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("B"));
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
fn actual_six_pixel_group_gap_selects_upper_append_and_lower_sibling_levels() {
    for offset in [-5.0, -4.0, -2.0, -1.0] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        let source = h.get_by_label("C").rect().center();
        let parent_header = h.get_by_label("A").rect();
        let lower_header = h.get_by_label("B").rect();
        let x = if offset < -3.0 { parent_header.left() + 2.0 } else { lower_header.left() + 2.0 };
        let point = pos2(x.min(lower_header.right() - 2.0), lower_header.top() + offset);
        pointer_drag(&mut h, source, point);
        let a = &h.state().0.hierarchy_order[PANEL]["A"];
        assert_eq!(a.iter().filter(|entry| **entry == HierarchyOrderEntry::Group("C".into())).count(), 1);
        let c_index = a.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
        let b_index = a.iter().position(|entry| *entry == HierarchyOrderEntry::Group("B".into())).unwrap();
        if offset < -3.0 {
            let last_inner_index = a.iter().position(|entry| *entry == HierarchyOrderEntry::Item("a3".into())).unwrap();
            assert!(c_index > last_inner_index, "upper gap half appends after the upper group's last direct child: {a:?}");
        } else {
            assert!(c_index < b_index, "lower gap half inserts before the lower group: {a:?}");
        }
    }
}

#[test]
fn actual_card_above_group_upper_half_shows_only_its_right_vertical_marker() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_root_item_before_group("A", "a1", "B"));
    let source = h.get_by_label("C").rect().center();
    let card = h.get_by_label("A card 1").rect();
    let lower_group = h.get_by_label("B").rect();
    assert!(card.bottom() <= lower_group.top(), "the root card should be the immediately preceding sibling: card={card:?}, group={lower_group:?}");

    let target = pos2(card.center().x, lower_group.top() - 4.0);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render card-to-group sibling marker");
    let ppp = h.ctx.pixels_per_point();
    let x = (card.right() * ppp).round() as i32;
    let y0 = (card.top() * ppp).round() as i32;
    let y1 = (card.bottom() * ppp).round() as i32;
    let has_accent = |y: i32| {
        (x - 3..=x + 3).any(|px| {
            px >= 0 && y >= 0 && (px as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(px as u32, y as u32).0[..3] == accent[..3]
        })
    };
    assert!(
        (y0..=y1).filter(|y| has_accent(*y)).count() > (y1 - y0) as usize * 3 / 4,
        "upper half of a card→group boundary must paint only the card's right vertical edge; card={card:?}, group={lower_group:?}, x={x}, accent_pixels={}, height={}, order={:?}",
        (y0..=y1).filter(|y| has_accent(*y)).count(),
        y1 - y0,
        h.state().0.hierarchy_order[PANEL]
    );
    let group_line_y = (lower_group.top() * ppp).round() as i32;
    let group_x0 = (lower_group.left() * ppp).round() as i32;
    let group_x1 = (lower_group.right() * ppp).round() as i32;
    let has_group_line = |x: i32| {
        (group_line_y - 3..=group_line_y + 3)
            .any(|y| x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(x as u32, y as u32).0[..3] == accent[..3])
    };
    assert!(
        (group_x0..=group_x1).filter(|x| has_group_line(*x)).count() < (group_x1 - group_x0) as usize / 4,
        "upper half must not also paint the lower group's horizontal boundary; group={lower_group:?}, y={group_line_y}"
    );
    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    let card_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() }).unwrap();
    let dragged_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
    let lower_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group("B".into())).unwrap();
    assert!(
        card_index < dragged_index && dragged_index < lower_index,
        "the upper half of the card→group boundary should insert the dragged group after the card and before the lower group: {root_order:?}"
    );
}

#[test]
fn actual_card_above_group_lower_half_shows_only_the_groups_upper_horizontal_marker() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_root_item_before_group("A", "a1", "B"));
    let source = h.get_by_label("C").rect().center();
    let card = h.get_by_label("A card 1").rect();
    let lower_group = h.get_by_label("B").rect();
    let target = pos2(card.center().x, lower_group.top() - 2.0);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render lower card-to-group sibling marker");
    let ppp = h.ctx.pixels_per_point();
    let group_line_y = (lower_group.top() * ppp).round() as i32;
    let group_x0 = (lower_group.left() * ppp).round() as i32;
    let group_x1 = (lower_group.right() * ppp).round() as i32;
    let has_group_line = |x: i32| {
        (group_line_y - 3..=group_line_y + 3)
            .any(|y| x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(x as u32, y as u32).0[..3] == accent[..3])
    };
    assert!(
        (group_x0..=group_x1).filter(|x| has_group_line(*x)).count() > (group_x1 - group_x0) as usize * 3 / 4,
        "lower half of a card→group boundary must paint the group's upper horizontal edge; group={lower_group:?}, y={group_line_y}"
    );
    let card_x = (card.right() * ppp).round() as i32;
    let card_y0 = (card.top() * ppp).round() as i32;
    let card_y1 = (card.bottom() * ppp).round() as i32;
    let has_card_edge = |y: i32| {
        (card_x - 3..=card_x + 3)
            .any(|x| x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(x as u32, y as u32).0[..3] == accent[..3])
    };
    assert!(
        (card_y0..=card_y1).filter(|y| has_card_edge(*y)).count() < (card_y1 - card_y0) as usize / 4,
        "lower half must not also paint the preceding card's right edge; card={card:?}, x={card_x}"
    );

    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    let card_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() }).unwrap();
    let dragged_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
    let lower_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group("B".into())).unwrap();
    assert_eq!(dragged_index, card_index + 1, "the lower half should insert the dragged group immediately after the root card: {root_order:?}");
    assert_eq!(lower_index, dragged_index + 1, "the lower group should remain immediately after the dragged group: {root_order:?}");
    assert_eq!(h.state().0.parent_of(PANEL, "C"), None, "the lower-half drop remains at the root sibling level");
}

#[test]
fn actual_nested_item_above_group_lower_half_drops_group_as_parent_sibling() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_nested_item_before_group());
    let source = h.get_by_label("C").rect().center();
    let card = h.get_by_label("A card 1").rect();
    let lower_group = h.get_by_label("B").rect();
    assert!(card.bottom() <= lower_group.top(), "the nested item should be the immediately preceding sibling: card={card:?}, group={lower_group:?}");

    let target = pos2(card.center().x, lower_group.top() - 2.0);
    pointer_drag(&mut h, source, target);

    let siblings = &h.state().0.hierarchy_order[PANEL]["A"];
    let card_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Item("a1".into())).unwrap();
    let dragged_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
    let lower_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Group("B".into())).unwrap();
    assert_eq!(dragged_index, card_index + 1, "the lower half should insert C immediately after the nested item: {siblings:?}");
    assert_eq!(lower_index, dragged_index + 1, "B should remain immediately after C under A: {siblings:?}");
    assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("A"));
}

#[test]
fn actual_parent_indent_gutter_drops_into_parent_across_nested_child_header() {
    for offset in [-1.0, 1.0] {
        let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
        let source = h.get_by_label("C").rect().center();
        let parent_header = h.get_by_label("A").rect();
        let child_header = h.get_by_label("B").rect();
        let boundary = child_header.top();
        let point = pos2(parent_header.left() + 2.0, boundary + offset);

        h.event(Event::PointerMoved(source));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=8 {
            h.event(Event::PointerMoved(source + (point - source) * (step as f32 / 8.0)));
            h.run_steps(1);
        }

        h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        let children = &h.state().0.hierarchy_order[PANEL]["A"];
        assert!(children.contains(&HierarchyOrderEntry::Group("C".into())), "parent indent gutter appends into A at offset {offset}: {children:?}");
        assert_eq!(h.state().0.parent_of(PANEL, "C"), Some("A"));
    }
}

#[test]
fn actual_gap_after_a_nested_subtree_anchors_after_the_preceding_root_group() {
    for (offset, upper) in [(-5.0, true), (-1.0, false)] {
        let mut groups = nested_group_tree();
        groups.push(group("D", vec![]));
        let mut h = harness_with_state(groups, vec2(520.0, 900.0), with_b_nested_in_a());
        let source = h.get_by_label("D").rect().center();
        let lower_header = h.get_by_label("C").rect();
        let point = pos2(lower_header.center().x, lower_header.top() + offset);
        pointer_drag(&mut h, source, point);

        if upper {
            let root = &h.state().0.hierarchy_order[PANEL][""];
            let d_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("D".into())).expect("D should remain a root sibling");
            let a_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("A".into())).unwrap();
            let c_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
            assert!(a_index < d_index && d_index < c_index, "upper half should append after the preceding root subtree: {root:?}");
            assert!(!h.state().0.hierarchy_order[PANEL]["A"].contains(&HierarchyOrderEntry::Group("D".into())));
        } else {
            let root = &h.state().0.hierarchy_order[PANEL][""];
            let d_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("D".into())).unwrap();
            let c_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("C".into())).unwrap();
            assert!(d_index < c_index, "lower half should insert D before C at the root level: {root:?}");
        }
    }
}

#[test]
fn actual_gap_between_a_group_and_root_card_offers_append_and_root_sibling_destinations() {
    for (offset, append_into_group) in [(-1.0, true), (1.0, false)] {
        let groups = vec![group("A", vec![item("a1", "A card", &[])]), group("B", vec![item("b1", "B card", &[])])];
        let mut h = harness_with_state(groups, vec2(520.0, 900.0), with_root_item_between_groups("B", "b1"));
        let group_card = h.get_by_label("A card").rect();
        let source_rect = h.get_by_label("B card").rect();
        let source = source_rect.center();
        let boundary = (group_card.bottom() + source_rect.top()) * 0.5;
        let point = pos2(group_card.center().x, boundary + offset);
        pointer_drag(&mut h, source, point);

        let a = h.state().0.hierarchy_order[PANEL].get("A").cloned().unwrap_or_default();
        let root = h.state().0.hierarchy_order[PANEL].get("").cloned().unwrap_or_default();
        if append_into_group {
            assert!(
                a.contains(&HierarchyOrderEntry::Item("b1".into())),
                "upper half should append the root card to Group A: {a:?}; root={root:?}; all={:?}",
                h.state().0.hierarchy_order
            );
            assert!(!root.contains(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() }));
        } else {
            assert!(!a.contains(&HierarchyOrderEntry::Item("b1".into())), "lower half should keep the card at the root level: {a:?}");
            let group_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::Group("A".into())).unwrap();
            let card_index = root.iter().position(|entry| *entry == HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() }).unwrap();
            assert_eq!(card_index, group_index + 1, "root card should remain immediately after Group A: {root:?}");
        }
    }
}

#[test]
fn actual_gap_between_nested_group_and_its_sibling_card_works_at_nested_depth() {
    for (offset, append_into_group) in [(-1.0, true), (1.0, false)] {
        let groups =
            vec![group("A", vec![item("a1", "A card", &[])]), group("B", vec![item("b1", "B card", &[])]), group("C", vec![item("c1", "C card", &[])])];
        let mut initial = PresetUi::default();
        initial.nest_group(PANEL, &["A".into(), "B".into(), "C".into()], "A", Some("B"));
        initial.set_hierarchy_order(PANEL, "B", vec![HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Item("b1".into())]);
        let mut h = harness_with_state(groups, vec2(520.0, 900.0), initial);
        let group_card = h.get_by_label("A card").rect();
        let sibling_card = h.get_by_label("B card").rect();
        let source = h.get_by_label("C card").rect().center();
        let group_header = h.get_by_label("A").rect();
        let boundary = (group_card.bottom() + sibling_card.top()) * 0.5;
        let point = pos2(group_card.center().x, boundary + offset);
        if append_into_group {
            pointer_drag(&mut h, source, point);
        } else {
            h.event(Event::PointerMoved(source));
            h.run_steps(1);
            h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            for step in 1..=8 {
                h.event(Event::PointerMoved(source + (point - source) * (step as f32 / 8.0)));
                h.run_steps(1);
            }
            let accent = crate::theme::Tokens::get(&h.ctx).accent;
            let ppp = h.ctx.pixels_per_point();
            let image = h.render().expect("render lower group-boundary drop preview");
            let x = (group_header.center().x * ppp).round() as u32;
            let y = (group_card.bottom() * ppp).round() as u32;
            assert!(
                (y.saturating_sub(2)..=(y + 2)).any(|py| x < image.width() && py < image.height() && image.get_pixel(x, py).0[..3] == accent.to_array()[..3]),
                "lower half should paint a horizontal marker on Group A's lower border, not a vertical card-edge marker"
            );
            h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
        }

        if append_into_group {
            let children = &h.state().0.hierarchy_order[PANEL]["A"];
            assert!(children.contains(&HierarchyOrderEntry::Item("c1".into())), "upper half should append into nested Group A: {children:?}");
        } else {
            let siblings = &h.state().0.hierarchy_order[PANEL]["B"];
            let group_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Group("A".into())).unwrap();
            let moved_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Item("c1".into())).unwrap();
            let next_card_index = siblings.iter().position(|entry| *entry == HierarchyOrderEntry::Item("b1".into())).unwrap();
            assert_eq!(moved_index, group_index + 1, "lower half should insert immediately after Group A at parent X: {siblings:?}");
            assert_eq!(next_card_index, moved_index + 1, "the existing sibling card should remain after the moved card: {siblings:?}");
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
    let header_rect = h.get_by_label("B").rect();
    let header = header_rect.center();
    h.event(Event::PointerMoved(header));
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.query_all_by_label("B card 1").next().is_some(), "clicking the group body selects it but does not collapse it");
    assert!(h.state().0.browser_node_selected(PANEL, &PickerId::group("B")));

    let disclosure = pos2(header_rect.left() + 9.0, header_rect.center().y);
    h.event(Event::PointerMoved(disclosure));
    h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.query_all_by_label("B card 1").next().is_none());
    h.event(Event::PointerMoved(disclosure));
    h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: disclosure, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
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
fn domain_active_item_does_not_select_a_card_in_a_fresh_browser() {
    let groups = vec![group("A", vec![item("a1", "A card 1", &[])])];
    let mut initial_state = PresetUi::default();
    initial_state.set_selected_item(PANEL, "A", &[], "a1");
    let mut h = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
        move |ui, state: &mut (PresetUi, Vec<bool>)| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let (preset_ui, painted_selection) = state;
            let mut paint_item = |_: &egui::Ui, _: Rect, _: usize, _: usize, state: ItemPaintState| {
                if let ItemPaintState::Item { selected } = state {
                    painted_selection.push(selected);
                }
            };
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: BrowserCapabilities {
                    item_drag: false,
                    item_drop: false,
                    group_drag: false,
                    selection: true,
                    activation: true,
                    canonical_hierarchy_drag: false,
                },
                item_context_menu: None,
                group_context_menu: None,
                nested_group_context_menu: None,
                canonical_hierarchy_move: None,
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: None,
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            let _ = browser(ui, preset_ui, &mut adapter, 300.0);
        },
        (initial_state, Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);

    let painted_selection = &h.state().1;
    assert!(!painted_selection.is_empty(), "the browser should paint its item cards");
    assert!(!painted_selection.iter().any(|selected| *selected), "the active preset belongs to the domain, not the browser's transient selection");
}

#[test]
fn browser_selection_clears_only_when_empty_browser_space_is_clicked() {
    let groups = nested_group_tree();
    let mut h = Harness::builder().with_size(vec2(520.0, 520.0)).build_ui_state(
        move |ui, state: &mut UiTestState| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            ui.label("Shell header");
            let mut paint_item = |_: &egui::Ui, _: Rect, _: usize, _: usize, _: ItemPaintState| {};
            let (preset_ui, actions, item_moves) = state;
            let mut commit_item_move = |_: &str, _: &str, _: &str, _: usize| Ok(());
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: BrowserCapabilities {
                    item_drag: false,
                    item_drop: false,
                    group_drag: false,
                    selection: true,
                    activation: true,
                    canonical_hierarchy_drag: false,
                },
                item_context_menu: None,
                group_context_menu: None,
                nested_group_context_menu: None,
                canonical_hierarchy_move: None,
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: Some(&mut commit_item_move),
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            actions.extend(browser(ui, preset_ui, &mut adapter, 300.0));
            ui.label("Shell footer");
            let _ = item_moves;
        },
        (PresetUi::default(), Vec::new(), Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);

    let card = h.get_by_label("A card 1").rect().center();
    h.event(Event::PointerMoved(card));
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(!h.state().0.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty()));

    let header = h.get_by_label("Shell header").rect().center();
    h.event(Event::PointerMoved(header));
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(
        !h.state().0.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty()),
        "shell header is outside the browser selection-clear region"
    );

    let footer = h.get_by_label("Shell footer").rect().center();
    h.event(Event::PointerMoved(footer));
    h.event(Event::PointerButton { pos: footer, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: footer, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(
        !h.state().0.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty()),
        "shell footer is outside the browser selection-clear region"
    );

    let browser = super::last_browser_rect(&h.ctx, PANEL).expect("browser viewport is recorded");
    let empty = pos2(browser.right() - 8.0, browser.bottom() - 8.0);
    h.event(Event::PointerMoved(empty));
    h.event(Event::PointerButton { pos: empty, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: empty, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().0.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty()), "empty space within the browser viewport clears selection");
}

#[test]
fn actual_root_card_selection_keeps_its_source_group_identity() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_root_item_between_groups("A", "a1"));
    let card = h.get_by_label("A card 1").rect().center();
    h.event(Event::PointerMoved(card));
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: card, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);

    assert!(matches!(h.state().1.as_slice(), [BrowserAction::SelectItem { group, group_path, key }] if group == "A" && group_path.is_empty() && key == "a1"));
}

#[test]
fn root_cards_from_different_backing_groups_share_one_grid_and_keep_owner_identity() {
    let groups = nested_group_tree();
    let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
        move |ui, state: &mut (PresetUi, Vec<BrowserAction>, Vec<(usize, usize)>)| {
            if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let (preset_ui, actions, painted) = state;
            let mut paint_item = |_: &egui::Ui, _: Rect, group: usize, item: usize, _: ItemPaintState| painted.push((group, item));
            let mut adapter = BrowserAdapter {
                panel: PANEL,
                groups: &groups,
                projection_keys: None,
                options: super::BrowserOptions {
                    canvas: Rect::NOTHING,
                    flush: false,
                    center_grid: false,
                    all_groups_open: true,
                    compact_context_menus: false,
                    domain_hierarchy_actions: false,
                    root_item_group: None,
                },
                capabilities: super::BrowserCapabilities {
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
                rekey_hierarchy_after_move: None,
                hierarchy_item_move: None,
                paint_group: None,
                paint_item: &mut paint_item,
                paint_list_item: None,
            };
            actions.extend(browser(ui, preset_ui, &mut adapter, 900.0));
        },
        (with_consecutive_root_cards_from_different_groups(), Vec::new(), Vec::new()),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);

    let first = h.get_by_label("A card 1").rect();
    let second = h.get_by_label("B card 1").rect();
    assert_eq!(first.top(), second.top(), "adjacent root cards must occupy the same grid row");
    assert!(second.left() >= first.right(), "the second backing group's card must share the horizontal bunch: {first:?}, {second:?}");
    assert!(h.state().2.contains(&(0, 0)), "the first card must paint through its owning group and item index");
    assert!(h.state().2.contains(&(1, 0)), "the second card must paint through its owning group and item index");

    let point = second.center();
    h.event(Event::PointerMoved(point));
    h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().0.browser_node_selected(PANEL, &PickerId::item("B", "b1")));
    assert!(matches!(h.state().1.last(), Some(BrowserAction::SelectItem { group, key, .. }) if group == "B" && key == "b1"));
}

#[test]
fn root_cards_from_different_backing_groups_remain_rows_in_list_mode() {
    use crate::pickers::picker_state::PickerDisplayMode as Mode;

    let mut state = with_consecutive_root_cards_from_different_groups();
    state.display_modes.insert(PANEL.to_owned(), Mode::SmallList);
    state.wrap_list.insert(PANEL.to_owned(), false);
    let h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), state, true);
    let first = h.get_by_label("A card 1").rect();
    let second = h.get_by_label("B card 1").rect();

    assert_eq!(first.left(), second.left());
    assert!(second.top() > first.top(), "list mode keeps one card per row: {first:?}, {second:?}");
}

#[test]
fn actual_root_cards_reorder_each_other_without_losing_owner_identity() {
    for (source_label, target_label, after) in [("B card 1", "A card 1", false), ("A card 1", "B card 1", true)] {
        let mut initial = PresetUi::default();
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
        let source = h.get_by_label(source_label).rect().center();
        let target_rect = h.get_by_label(target_label).rect();
        let source_rect = h.get_by_label(source_label).rect();
        assert_eq!(source_rect.top(), target_rect.top(), "cross-owner root cards must be one horizontal grid before drag");
        assert!(source_rect.left() != target_rect.left(), "the two cards must have distinct columns");
        let target = if after { pos2(target_rect.right() - 2.0, target_rect.center().y) } else { pos2(target_rect.left() + 2.0, target_rect.center().y) };

        pointer_drag(&mut h, source, target);

        assert_eq!(
            h.state().0.hierarchy_order[PANEL][""],
            [
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() },
                HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
                HierarchyOrderEntry::Group("B".into()),
                HierarchyOrderEntry::Group("C".into()),
            ],
            "source={source_label}, target={target_label}, after={after}"
        );
    }
}

#[test]
fn active_root_card_drop_between_cards_paints_a_vertical_insertion_marker() {
    let mut initial = PresetUi::default();
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
    let target_card = h.get_by_label("A card 1").rect();
    let target = pos2(target_card.left() + 0.5, target_card.center().y);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render root card insertion preview");
    let ppp = h.ctx.pixels_per_point();
    let x = (target_card.left() * ppp).round() as i32;
    let y = (target_card.center().y * ppp).round() as i32;
    let has_accent = (x - 2..=x + 2).any(|px| {
        (y - 2..=y + 2).any(|py| {
            px >= 0 && py >= 0 && (px as u32) < image.width() && (py as u32) < image.height() && image.get_pixel(px as u32, py as u32).0[..3] == accent[..3]
        })
    });

    assert!(has_accent, "the insertion marker between root cards should be vertical at the target card's leading edge");
    let horizontal_marker_pixels = (x + 3..((target_card.right() * ppp).round() as i32 - 3))
        .filter(|px| {
            (y - 1..=y + 1).any(|py| {
                *px >= 0
                    && py >= 0
                    && (*px as u32) < image.width()
                    && (py as u32) < image.height()
                    && image.get_pixel(*px as u32, py as u32).0[..3] == accent[..3]
            })
        })
        .count();
    assert_eq!(horizontal_marker_pixels, 0, "an interior card insertion must not paint a horizontal marker across the card");
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
fn actual_list_row_drag_commits_before_and_after_positions() {
    for (after, expected_index) in [(false, 0), (true, 1)] {
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), PresetUi::default(), true);
        let source = h.get_by_label("A card 1").rect().center();
        let target_row = h.get_by_label("A card 2").rect();
        let target = pos2(target_row.center().x, if after { target_row.bottom() - 2.0 } else { target_row.top() + 2.0 });

        pointer_drag(&mut h, source, target);

        assert_eq!(h.state().2, [("A".into(), "a1".into(), "A".into(), expected_index)], "list after={after}, target={target_row:?}");
    }
}

#[test]
fn actual_list_drag_moves_the_entire_selected_item_set() {
    for list in [true, false] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::item("A", "a1"), false);
        initial.select_browser_node(PANEL, PickerId::item("A", "a2"), true);
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A card 1").rect().center();
        let target_row = h.get_by_label("B card 1").rect();
        let target = if list { pos2(target_row.center().x, target_row.top() + 2.0) } else { pos2(target_row.left() + 2.0, target_row.center().y) };

        pointer_drag(&mut h, source, target);

        let mut moved: Vec<_> = h.state().2.iter().map(|(_, key, group, _)| (key.as_str(), group.as_str())).collect();
        moved.sort_unstable();
        assert_eq!(moved, [("a1", "B"), ("a2", "B")], "list layout={list}");
        assert_eq!(
            h.state().2.iter().map(|(_, key, _, _)| key.as_str()).collect::<Vec<_>>(),
            ["a1", "a2"],
            "simulated insertion indices must advance with the selected nodes in list={list}"
        );
        assert_eq!(
            h.state().2.iter().map(|(_, _, _, index)| *index).collect::<Vec<_>>(),
            [0, 1],
            "domain insertion indices must preserve selection order in list={list}"
        );
        let destination = &h.state().0.hierarchy_order[PANEL]["B"];
        let moved_a1 = destination.iter().position(|entry| *entry == HierarchyOrderEntry::Item("a1".into())).unwrap();
        let moved_a2 = destination.iter().position(|entry| *entry == HierarchyOrderEntry::Item("a2".into())).unwrap();
        let target = destination.iter().position(|entry| *entry == HierarchyOrderEntry::Item("b1".into())).unwrap();
        assert!(moved_a1 < moved_a2 && moved_a2 < target, "selection order must be retained in list={list}: {destination:?}");
    }
}

#[test]
fn actual_multi_card_drag_preserves_selection_order_for_every_drop_region() {
    for list in [false, true] {
        for (region, expected_order, expected_callbacks) in [
            ("before", vec!["a1", "a2", "b1", "b2"], vec![("a1", 0), ("a2", 1)]),
            ("after", vec!["b1", "a1", "a2", "b2"], vec![("a2", 1), ("a1", 1)]),
            ("prepend", vec!["a1", "a2", "b1", "b2"], vec![("a2", 0), ("a1", 0)]),
            ("append", vec!["b1", "b2"], vec![]),
        ] {
            let mut initial = PresetUi::default();
            initial.select_browser_node(PANEL, PickerId::item("A", "a1"), false);
            initial.select_browser_node(PANEL, PickerId::item("A", "a2"), true);
            let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
            let source = h.get_by_label("A card 1").rect().center();
            let target = match region {
                "before" | "after" => {
                    let rect = h.get_by_label("B card 1").rect();
                    if list {
                        pos2(rect.center().x, if region == "before" { rect.top() + 2.0 } else { rect.bottom() - 2.0 })
                    } else {
                        pos2(if region == "before" { rect.left() + 2.0 } else { rect.right() - 2.0 }, rect.center().y)
                    }
                }
                "prepend" => h.get_by_label("B").rect().center(),
                "append" => {
                    let last = h.get_by_label("C card 1").rect();
                    pos2(last.center().x, last.bottom() + 10.0)
                }
                _ => {
                    assert!(["before", "after", "prepend", "append"].contains(&region), "unexpected drop region: {region}");
                    Pos2::ZERO
                }
            };

            pointer_drag(&mut h, source, target);

            let actual_order: Vec<_> = h.state().0.hierarchy_order[PANEL]["B"]
                .iter()
                .filter_map(|entry| match entry {
                    HierarchyOrderEntry::Item(key) => Some(key.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(actual_order, expected_order, "region={region}, list={list}");
            assert_eq!(
                h.state().2.iter().map(|(_, key, _, index)| (key.as_str(), *index)).collect::<Vec<_>>(),
                expected_callbacks,
                "domain callback order and indices for region={region}, list={list}"
            );
            if region == "append" {
                assert_eq!(
                    h.state().0.hierarchy_order[PANEL][""],
                    [
                        HierarchyOrderEntry::Group("A".into()),
                        HierarchyOrderEntry::Group("B".into()),
                        HierarchyOrderEntry::Group("C".into()),
                        HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
                        HierarchyOrderEntry::RootItem { group: "A".into(), key: "a2".into() },
                    ],
                    "root append must retain the selected card order in list={list}"
                );
            }
        }
    }
}

#[test]
fn actual_multi_card_same_group_reorder_uses_indices_after_prior_selected_moves() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::item("A", "a1"), false);
        initial.select_browser_node(PANEL, PickerId::item("A", "a2"), true);
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A card 1").rect().center();
        let target_row = h.get_by_label("A card 3").rect();
        let target = if list { pos2(target_row.center().x, target_row.bottom() - 2.0) } else { pos2(target_row.right() - 2.0, target_row.center().y) };

        pointer_drag(&mut h, source, target);

        let actual: Vec<_> = h.state().0.hierarchy_order[PANEL]["A"]
            .iter()
            .filter_map(|entry| match entry {
                HierarchyOrderEntry::Item(key) => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(actual, ["a3", "a1", "a2"], "same-container reorder in list={list}");
        assert_eq!(
            h.state().2.iter().map(|(_, key, _, index)| (key.as_str(), *index)).collect::<Vec<_>>(),
            [("a2", 2), ("a1", 1)],
            "same-container domain indices in list={list}"
        );
    }
}

#[test]
fn actual_group_drag_moves_all_selected_groups_together() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        initial.select_browser_node(PANEL, PickerId::group("B"), true);
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert_eq!(
            h.state().0.hierarchy_order[PANEL]["C"],
            [HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Item("c1".into())],
            "group selection order must be retained in list={list}"
        );
        assert_eq!(h.state().2, Vec::<(String, String, String, usize)>::new());
    }
}

#[test]
fn actual_group_drag_moves_a_mixed_group_and_card_selection() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        initial.select_browser_node(PANEL, PickerId::item("B", "b1"), true);
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert_eq!(
            h.state().0.hierarchy_order[PANEL]["C"],
            [HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Item("b1".into()), HierarchyOrderEntry::Item("c1".into())],
            "mixed selection order must be retained in list={list}"
        );
        assert!(
            h.state().0.browser_node_selected(PANEL, &PickerId::group("A")),
            "dragging an already-selected group must preserve its selection in list={list}"
        );
        assert!(
            h.state().0.browser_node_selected(PANEL, &PickerId::item("B", "b1")),
            "dragging an already-selected group must preserve the selected card in list={list}"
        );
        assert_eq!(h.state().2, [("B".into(), "b1".into(), "C".into(), 0)]);
    }
}

#[test]
fn actual_card_drag_moves_a_mixed_card_and_group_selection() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        initial.select_browser_node(PANEL, PickerId::item("B", "b1"), true);
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("B card 1").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert_eq!(
            h.state().0.hierarchy_order[PANEL]["C"],
            [HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Item("b1".into()), HierarchyOrderEntry::Item("c1".into())],
            "mixed selection order must be independent of the dragged node type in list={list}"
        );
        assert!(
            h.state().0.browser_node_selected(PANEL, &PickerId::group("A")),
            "dragging an already-selected card must preserve the selected group in list={list}"
        );
        assert!(
            h.state().0.browser_node_selected(PANEL, &PickerId::item("B", "b1")),
            "dragging an already-selected card must preserve its selection in list={list}"
        );
        assert_eq!(h.state().2, [("B".into(), "b1".into(), "C".into(), 0)]);
    }
}

#[test]
fn actual_shared_picker_passes_the_complete_mixed_selection_to_its_commit_function() {
    let mut initial = PresetUi::default();
    initial.select_browser_node(PANEL, PickerId::group("A"), false);
    initial.select_browser_node(PANEL, PickerId::item("B", "b1"), true);
    let mut h = harness_with_intent_capture(nested_group_tree(), vec2(520.0, 900.0), initial, false);
    assert!(h.state().0.browser_node_selected(PANEL, &PickerId::group("A")));
    assert!(h.state().0.browser_node_selected(PANEL, &PickerId::item("B", "b1")));
    let source = h.get_by_label("A").rect().center();
    let target = h.get_by_label("C").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().1.len(), 1);
    assert_eq!(h.state().1[0].source, PickerId::group("A"));
    assert_eq!(h.state().1[0].selection, [PickerId::group("A"), PickerId::item("B", "b1")]);
    assert_eq!(h.state().1[0].position, crate::pickers::picker_browser_model::DropPosition::PrependTo { group: PickerId::group("C") });
}

#[test]
fn actual_open_empty_group_header_bottom_half_drops_into_the_group() {
    for list in [false, true] {
        let mut h = harness_with_state_layout(
            vec![group("A", Vec::new()), group("B", Vec::new()), group("C", Vec::new())],
            vec2(520.0, 900.0),
            PresetUi::default(),
            list,
        );
        let source = h.get_by_label("A").rect().center();
        let target_header = h.get_by_label("C").rect();
        let target = pos2(target_header.center().x, target_header.center().y + target_header.height() * 0.25);

        pointer_drag(&mut h, source, target);

        assert_eq!(h.state().0.hierarchy_order[PANEL][""], [HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Group("C".into())]);
        assert_eq!(
            h.state().0.hierarchy_order[PANEL]["C"],
            [HierarchyOrderEntry::Group("A".into())],
            "the lower half of an open empty group header also drops into it in list={list}"
        );
        assert_eq!(h.state().0.parent_of(PANEL, "A"), Some("C"), "A becomes a child of C in list={list}");
    }
}

#[test]
fn actual_closed_group_header_bottom_half_keeps_the_sibling_after_target() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.toggled.insert(format!("{PANEL}/B"));
        let mut h = harness_with_state_layout(vec![group("A", Vec::new()), group("B", Vec::new()), group("C", Vec::new())], vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A").rect().center();
        let target_header = h.get_by_label("B").rect();
        let target = pos2(target_header.center().x, target_header.bottom() - 2.0);

        pointer_drag(&mut h, source, target);

        assert_eq!(
            h.state().0.hierarchy_order[PANEL][""],
            [HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Group("C".into())],
            "a closed group's lower header half remains a sibling-after target in list={list}"
        );
        assert_eq!(h.state().0.parent_of(PANEL, "A"), None);
    }
}

#[test]
fn actual_unselected_drag_source_does_not_expand_to_other_selected_nodes() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        let mut h = harness_with_intent_capture(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("B card 1").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert_eq!(h.state().1.len(), 1, "list={list}");
        assert_eq!(h.state().1[0].source, PickerId::item("B", "b1"), "list={list}");
        assert!(h.state().1[0].selection.is_empty(), "an unselected source must use single-node fallback in list={list}");
    }
}

#[test]
fn actual_selected_ancestor_moves_once_when_a_selected_descendant_is_dragged() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        initial.select_browser_node(PANEL, PickerId::item("A", "a1"), true);
        let mut h = harness_with_intent_capture(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A card 1").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert_eq!(h.state().1.len(), 1, "list={list}");
        assert_eq!(h.state().1[0].source, PickerId::item("A", "a1"), "the pointer identity remains the dragged card in list={list}");
        assert_eq!(h.state().1[0].selection, [PickerId::group("A")], "the selected ancestor subsumes its descendant in list={list}");
    }
}

#[test]
fn actual_invalid_member_rejects_the_entire_multi_node_move_before_commit() {
    for list in [false, true] {
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::group("A"), false);
        initial.select_browser_node(PANEL, PickerId::group("C"), true);
        let mut h = harness_with_intent_capture(nested_group_tree(), vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("A").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert!(h.state().1.is_empty(), "the selected destination group makes the whole selection invalid in list={list}");
        assert_eq!(h.state().0.parent_of(PANEL, "A"), None, "no valid subset may be moved in list={list}");
        assert_eq!(h.state().0.parent_of(PANEL, "C"), None, "the invalid selected node remains at root in list={list}");
    }
}

#[test]
fn actual_multiple_group_drag_uses_before_after_prepend_and_append_targets() {
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition};

    for list in [false, true] {
        for (region, expected) in [
            ("before", DropPosition::Before { target: PickerId::group("D") }),
            ("after", DropPosition::After { target: PickerId::group("C") }),
            ("prepend", DropPosition::PrependTo { group: PickerId::group("C") }),
            ("append", DropPosition::AppendTo { container: ContainerId::Root }),
        ] {
            let mut initial = PresetUi::default();
            initial.select_browser_node(PANEL, PickerId::group("A"), false);
            initial.select_browser_node(PANEL, PickerId::group("B"), true);
            let groups = vec![group("A", Vec::new()), group("B", Vec::new()), group("C", Vec::new()), group("D", Vec::new())];
            let mut h = harness_with_intent_capture(groups, vec2(520.0, 900.0), initial, list);
            let source = h.get_by_label("A").rect().center();
            let target = match region {
                "before" | "after" => {
                    let lower = h.get_by_label("D").rect();
                    pos2(lower.center().x, lower.top() + if region == "before" { -1.0 } else { -5.0 })
                }
                "prepend" => h.get_by_label("C").rect().center(),
                "append" => {
                    let last = h.get_by_label("D").rect();
                    pos2(last.center().x, last.bottom() + 10.0)
                }
                _ => {
                    assert!(["before", "after", "prepend", "append"].contains(&region), "unexpected drop region: {region}");
                    Pos2::ZERO
                }
            };

            pointer_drag(&mut h, source, target);

            assert_eq!(h.state().1.len(), 1, "region={region}, list={list}");
            assert_eq!(h.state().1[0].source, PickerId::group("A"), "region={region}, list={list}");
            assert_eq!(h.state().1[0].selection, [PickerId::group("A"), PickerId::group("B")], "region={region}, list={list}");
            assert_eq!(h.state().1[0].position, expected, "region={region}, list={list}");
        }
    }
}

#[test]
fn actual_mixed_group_and_card_selection_uses_every_drop_position_from_either_source_type() {
    use crate::pickers::picker_browser_model::{ContainerId, DropPosition};

    for list in [false, true] {
        for source_kind in ["group", "card"] {
            for (region, expected) in [
                ("before", DropPosition::Before { target: PickerId::group("D") }),
                ("after", DropPosition::After { target: PickerId::group("C") }),
                ("prepend", DropPosition::PrependTo { group: PickerId::group("C") }),
                ("append", DropPosition::AppendTo { container: ContainerId::Root }),
            ] {
                let mut initial = PresetUi::default();
                initial.select_browser_node(PANEL, PickerId::group("A"), false);
                initial.select_browser_node(PANEL, PickerId::item("B", "b1"), true);
                let groups = vec![group("A", Vec::new()), group("B", vec![item("b1", "B card 1", &[])]), group("C", Vec::new()), group("D", Vec::new())];
                let mut h = harness_with_intent_capture(groups, vec2(520.0, 900.0), initial, list);
                let source = if source_kind == "group" { h.get_by_label("A").rect().center() } else { h.get_by_label("B card 1").rect().center() };
                let target = match region {
                    "before" | "after" => {
                        let lower = h.get_by_label("D").rect();
                        pos2(lower.center().x, lower.top() + if region == "before" { -1.0 } else { -5.0 })
                    }
                    "prepend" => h.get_by_label("C").rect().center(),
                    "append" => {
                        let last = h.get_by_label("D").rect();
                        pos2(last.center().x, last.bottom() + 10.0)
                    }
                    _ => {
                        assert!(["before", "after", "prepend", "append"].contains(&region), "unexpected drop region: {region}");
                        Pos2::ZERO
                    }
                };

                pointer_drag(&mut h, source, target);

                assert_eq!(h.state().1.len(), 1, "source={source_kind}, region={region}, list={list}");
                let intent = &h.state().1[0];
                assert_eq!(
                    intent.source,
                    if source_kind == "group" { PickerId::group("A") } else { PickerId::item("B", "b1") },
                    "source identity for source={source_kind}, region={region}, list={list}"
                );
                assert_eq!(
                    intent.selection,
                    [PickerId::group("A"), PickerId::item("B", "b1")],
                    "selection order for source={source_kind}, region={region}, list={list}"
                );
                assert_eq!(intent.position, expected, "destination for source={source_kind}, region={region}, list={list}");
            }
        }
    }
}

#[test]
fn actual_unsupported_path_selection_is_rejected_before_any_item_callback() {
    for list in [false, true] {
        let groups = vec![
            group("A", vec![item("folder-card", "Folder card", &["Folder"])]),
            group("B", vec![item("b1", "B card 1", &[])]),
            group("C", vec![item("c1", "C card 1", &[])]),
        ];
        let mut initial = PresetUi::default();
        initial.select_browser_node(PANEL, PickerId::item_in_path(["A", "Folder"], "folder-card"), false);
        initial.select_browser_node(PANEL, PickerId::item("B", "b1"), true);
        let mut h = harness_with_state_layout(groups, vec2(520.0, 900.0), initial, list);
        let source = h.get_by_label("Folder card").rect().center();
        let target = h.get_by_label("C").rect().center();

        pointer_drag(&mut h, source, target);

        assert!(h.state().2.is_empty(), "unsupported path-aware multi-move must not partially call item callbacks in list={list}");
    }
}

#[test]
fn actual_group_drag_onto_a_list_row_uses_the_rows_vertical_insertion_edge() {
    for (after, expected_order) in [
        (
            false,
            vec![
                HierarchyOrderEntry::Group("B".into()),
                HierarchyOrderEntry::Item("a1".into()),
                HierarchyOrderEntry::Item("a2".into()),
                HierarchyOrderEntry::Item("a3".into()),
            ],
        ),
        (
            true,
            vec![
                HierarchyOrderEntry::Item("a1".into()),
                HierarchyOrderEntry::Group("B".into()),
                HierarchyOrderEntry::Item("a2".into()),
                HierarchyOrderEntry::Item("a3".into()),
            ],
        ),
    ] {
        let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), PresetUi::default(), true);
        let source = h.get_by_label("B").rect().center();
        let target_row = h.get_by_label("A card 1").rect();
        let target = pos2(target_row.center().x, if after { target_row.bottom() - 2.0 } else { target_row.top() + 2.0 });

        pointer_drag(&mut h, source, target);

        assert_eq!(h.state().0.parent_of(PANEL, "B"), Some("A"));
        assert_eq!(h.state().0.hierarchy_order[PANEL]["A"], expected_order, "list after={after}");
    }
}

#[test]
fn actual_list_row_accepts_an_item_drop_from_a_different_group() {
    let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), PresetUi::default(), true);
    let source = h.get_by_label("B card 1").rect().center();
    let target_row = h.get_by_label("A card 2").rect();
    let target = pos2(target_row.center().x, target_row.bottom() - 2.0);

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("B".into(), "b1".into(), "A".into(), 2)]);
}

#[test]
fn actual_list_item_drop_on_a_group_header_moves_it_into_that_group() {
    let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), PresetUi::default(), true);
    let source = h.get_by_label("A card 1").rect().center();
    let target = h.get_by_label("B").rect().center();

    pointer_drag(&mut h, source, target);

    assert_eq!(h.state().2, [("A".into(), "a1".into(), "B".into(), 0)]);
}

#[test]
fn actual_nested_list_item_drop_at_root_append_promotes_it_to_a_root_sibling() {
    let mut h = harness_with_state_layout(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a(), true);
    let source = h.get_by_label("B card 1").rect().center();
    let last_row = h.get_by_label("C card 1").rect();
    let target = pos2(last_row.center().x, last_row.bottom() + 8.0);

    pointer_drag(&mut h, source, target);

    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    assert!(root_order.contains(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() }), "root order: {root_order:?}");
    assert!(!h.state().0.hierarchy_order[PANEL]["B"].contains(&HierarchyOrderEntry::Item("b1".into())));
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
fn actual_nested_card_drag_back_to_its_parent_group_shows_the_drop_indicator() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    let source = h.get_by_label("B card 1").rect().center();
    let target_row = h.get_by_label("A card 2").rect();
    let target = pos2(target_row.left() + 2.0, target_row.center().y);

    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    let ppp = h.ctx.pixels_per_point();
    let image = h.render().expect("render nested-to-parent card drop preview");
    let x = (target_row.left() * ppp).round() as u32;
    let y = (target_row.center().y * ppp).round() as u32;
    assert!(
        (x.saturating_sub(2)..=(x + 2)).any(|px| px < image.width() && y < image.height() && image.get_pixel(px, y).0[..3] == accent.to_array()[..3]),
        "moving a nested card into its parent group should show the leading-edge insertion indicator"
    );

    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().2, [("B".into(), "b1".into(), "A".into(), 1)]);
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
fn root_append_indicator_spans_the_full_browser_width() {
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), with_b_nested_in_a());
    let source = h.get_by_label("B card 1").rect().center();
    let last_card = h.get_by_label("C card 1").rect();
    let target = pos2(last_card.center().x, last_card.bottom() + 8.0);

    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render root append preview");
    let ppp = h.ctx.pixels_per_point();
    let y = (last_card.bottom() * ppp).round() as i32;
    let has_accent_at = |x: u32| (y - 3..=y + 3).any(|py| py >= 0 && (py as u32) < image.height() && image.get_pixel(x, py as u32).0[..3] == accent[..3]);
    assert!(has_accent_at(2), "root append marker should reach the left edge of the browser; image width={}", image.width());
    assert!(has_accent_at(image.width() - 3), "root append marker should reach the right edge of the browser; image width={}", image.width());
}

#[test]
fn root_end_after_a_card_uses_the_cards_vertical_trailing_marker() {
    let mut state = with_b_nested_in_a();
    state.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::Group("C".into()),
            HierarchyOrderEntry::RootItem { group: "B".into(), key: "b1".into() },
        ],
    );
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), state);
    let source = h.get_by_label("A").rect().center();
    let last_card = h.get_by_label("B card 1").rect();
    let target = pos2(last_card.center().x, last_card.bottom() + 8.0);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render root card append preview");
    let ppp = h.ctx.pixels_per_point();
    let x = (last_card.right() * ppp).round() as i32;
    let y = (last_card.center().y * ppp).round() as i32;
    let has_accent = |px: i32, py: i32| {
        (px - 2..=px + 2).any(|x| {
            (py - 2..=py + 2).any(|y| {
                x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(x as u32, y as u32).0[..3] == accent[..3]
            })
        })
    };
    assert!(
        has_accent(x, y),
        "root append after a card should show its vertical trailing edge; last_card={last_card:?}, ppp={ppp}, image={}x{}, order={:?}",
        image.width(),
        image.height(),
        h.state().0.hierarchy_order[PANEL]
    );
    assert!(!has_accent(2, (last_card.bottom() * ppp).round() as i32), "card append should not draw a full-width line");
    assert!(!has_accent(image.width() as i32 - 3, (last_card.bottom() * ppp).round() as i32), "card append should not draw a full-width line");
}

#[test]
fn root_leading_group_marker_renders_all_four_pixels_inside_the_picker_clip() {
    let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
    let first_header = h.get_by_label("A").rect();
    let source = h.get_by_label("B card 1").rect().center();
    let target = pos2(first_header.center().x, first_header.top() + 0.5);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render root leading preview");
    let ppp = h.ctx.pixels_per_point();
    let left = (first_header.left() * ppp).round() as u32;
    let right = (first_header.right() * ppp).round() as u32;
    let top = (first_header.top() * ppp).round() as i32;
    let has_accent_at = |x: u32, y: i32| y >= 0 && (y as u32) < image.height() && image.get_pixel(x, y as u32).0[..3] == accent[..3];
    let rows: Vec<bool> = (top - 8..=top + 12).map(|y| has_accent_at(left + 4, y)).collect();
    assert!(
        rows.windows(4).any(|window| window.iter().all(|pixel| *pixel)),
        "leading group marker should render four consecutive pixels at the left edge; first_header={first_header:?}, top={top}, rows={rows:?}, image={}x{}",
        image.width(),
        image.height()
    );
    let right_rows: Vec<bool> = (top - 8..=top + 12).map(|y| has_accent_at(image.width() - 5, y)).collect();
    assert!(
        right_rows.windows(4).any(|window| window.iter().all(|pixel| *pixel)),
        "leading group marker should span the full browser width; rows={right_rows:?}, first_header={first_header:?}, right={right}"
    );
}

#[test]
fn root_leading_card_uses_its_vertical_leading_marker() {
    let mut state = with_root_item_between_groups("A", "a1");
    state.set_hierarchy_order(
        PANEL,
        "",
        vec![
            HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() },
            HierarchyOrderEntry::Group("A".into()),
            HierarchyOrderEntry::Group("B".into()),
            HierarchyOrderEntry::Group("C".into()),
        ],
    );
    let mut h = harness_with_state(nested_group_tree(), vec2(520.0, 900.0), state);
    let first_card = h.get_by_label("A card 1").rect();
    let source = h.get_by_label("A card 2").rect().center();
    let target = pos2(first_card.center().x, first_card.top() + 0.5);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
        h.run_steps(1);
    }

    let accent = crate::theme::Tokens::get(&h.ctx).accent.to_array();
    let image = h.render().expect("render root leading card preview");
    let ppp = h.ctx.pixels_per_point();
    let x = (first_card.left() * ppp).round() as i32;
    let y = (first_card.center().y * ppp).round() as i32;
    let has_accent = |px: i32, py: i32| {
        (px - 2..=px + 2).any(|x| {
            (py - 2..=py + 2).any(|y| {
                x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() && image.get_pixel(x as u32, y as u32).0[..3] == accent[..3]
            })
        })
    };
    assert!(has_accent(x, y), "root prepend before a card should show its vertical leading edge");
    assert!(
        !has_accent(image.width() as i32 - 3, y),
        "card prepend should not draw a full-width horizontal line; first_card={first_card:?}, ppp={ppp}, image={}x{}",
        image.width(),
        image.height()
    );
}

#[test]
fn actual_nested_card_drop_between_root_groups_promotes_card_to_root_level() {
    let mut h = harness(nested_group_tree(), vec2(520.0, 900.0));
    let source = h.get_by_label("A card 1").rect().center();
    let b_header = h.get_by_label("B").rect();
    let target = pos2(b_header.center().x, b_header.top() - 1.0);

    pointer_drag(&mut h, source, target);

    let root_order = &h.state().0.hierarchy_order[PANEL][""];
    assert!(
        root_order.contains(&HierarchyOrderEntry::RootItem { group: "A".into(), key: "a1".into() }),
        "the card dropped between root groups should become a root sibling; root order: {root_order:?}"
    );
    assert!(!h.state().0.hierarchy_order[PANEL]["A"].contains(&HierarchyOrderEntry::Item("a1".into())));
    assert!(h.state().0.is_selected_item(PANEL, "A", &[], "a1"), "root promotion must retain the moved card's owning group identity");
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

#[cfg(test)]
mod filter_tests {
    use super::super::{BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, ItemPaintState, browser, last_group_headers};
    use super::{group, item};
    use crate::pickers::picker_state::{HierarchyOrderEntry, PresetUi};
    use egui_kittest::kittest::Queryable;

    fn nested_group_drop_harness(domain_hierarchy_actions: bool, target_nested_card: bool) -> egui_kittest::Harness<'static, (PresetUi, Vec<BrowserAction>)> {
        use egui_kittest::Harness;

        let groups = vec![group("A", vec![item("nested-card", "Nested item", &["Nested"])]), group("B", vec![item("root-card", "Root item", &[])])];
        let mut h = Harness::builder().with_size(egui::vec2(420.0, 360.0)).build_ui_state(
            move |ui, state: &mut (PresetUi, Vec<BrowserAction>)| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                let mut paint_item = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: ItemPaintState| {};
                let mut adapter = BrowserAdapter {
                    panel: "hierarchy-drop-test",
                    groups: &groups,
                    projection_keys: None,
                    options: BrowserOptions {
                        canvas: egui::Rect::NOTHING,
                        flush: false,
                        center_grid: false,
                        all_groups_open: true,
                        compact_context_menus: false,
                        domain_hierarchy_actions,
                        root_item_group: None,
                    },
                    capabilities: BrowserCapabilities {
                        item_drag: false,
                        item_drop: false,
                        group_drag: true,
                        selection: false,
                        activation: false,
                        canonical_hierarchy_drag: false,
                    },
                    item_context_menu: None,
                    group_context_menu: None,
                    nested_group_context_menu: None,
                    canonical_hierarchy_move: None,
                    rekey_hierarchy_after_move: None,
                    hierarchy_item_move: None,
                    paint_group: None,
                    paint_item: &mut paint_item,
                    paint_list_item: None,
                };
                let (preset_ui, actions) = state;
                actions.extend(browser(ui, preset_ui, &mut adapter, 300.0));
            },
            (PresetUi::default(), Vec::new()),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let source = last_group_headers(&h.ctx, "hierarchy-drop-test").into_iter().find(|(name, _)| name == "B").map(|(_, rect)| rect.center()).unwrap();
        let target = if target_nested_card { h.get_by_label("Nested item").rect() } else { h.get_by_label("Nested").rect() };
        let target = target.center();
        h.event(egui::Event::PointerMoved(source));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: source, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(egui::Event::PointerMoved(source + (target - source) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        h
    }

    #[test]
    fn root_group_drop_on_nested_header_resolves_once_in_domain_mode() {
        let h = nested_group_drop_harness(true, false);
        assert!(matches!(
            h.state().1.as_slice(),
            [BrowserAction::MoveInto { group, into: Some((target, path)), before: None, before_item: None, .. }]
                if group == "B" && target == "A" && path == &["Nested".to_string()]
        ));
        assert_eq!(h.state().1.len(), 1, "the nested header's MoveInto must suppress the source header's root MoveGroup");
    }

    #[test]
    fn unsupported_root_group_drop_on_nested_header_or_card_is_not_reinterpreted_as_root_reorder() {
        let h = nested_group_drop_harness(false, false);
        assert!(h.state().1.is_empty(), "unsupported nested header must not consume or misroute the release");
        assert_eq!(h.state().0.parent_of("hierarchy-drop-test", "B"), None);
        assert!(
            !h.state().0.hierarchy_order.get("hierarchy-drop-test").is_some_and(|orders| orders.contains_key("")),
            "an unsupported nested target must not mutate root group order"
        );
    }

    #[test]
    fn root_group_drop_on_nested_card_updates_the_unified_sibling_sequence() {
        let h = nested_group_drop_harness(false, true);
        assert!(h.state().1.is_empty(), "the transient shared hierarchy handles the drop without a domain command");
        assert_eq!(h.state().0.parent_of("hierarchy-drop-test", "B"), Some("A"));
        assert!(
            h.state().0.hierarchy_order["hierarchy-drop-test"]["A"].contains(&HierarchyOrderEntry::Group("B".into())),
            "the moved group must occupy the same sibling sequence as the target card"
        );
    }
}

// Unit and projection tests for private asset-browser behavior.
// Kept in this sibling test module so asset_browser.rs contains production code only.
const HIT_DEBUG_LABEL_HEIGHT: f32 = 9.0;
fn hit_debug_label_position(hit: &DropHitRegion) -> Pos2 {
    let label_y = if hit.hit_rect.height() <= 6.1 {
        match hit.region {
            // The upper label must not be placed above the gap: that part can be clipped by
            // the preceding group's row. Stack both labels below the gap instead.
            PointerRegion::TrailingEdge => hit.hit_rect.bottom() + 3.0,
            PointerRegion::LeadingEdge => hit.hit_rect.bottom() + HIT_DEBUG_LABEL_HEIGHT,
            _ => hit.hit_rect.top() + 2.0,
        }
    } else {
        hit.hit_rect.top() + 2.0
    };
    pos2(hit.hit_rect.left() + 3.0, label_y)
}
/// Test helper for the shared browser's root-group/nested-group/item query contract.
#[cfg(test)]
pub(crate) fn filter_groups(groups: &[GroupView], query: &str) -> Vec<GroupView> {
    let query = query.trim().to_lowercase();
    groups
        .iter()
        .filter_map(|group| {
            let group_matches = group_matches_query(group, &query);
            let items: Vec<ItemView> = group
                .items
                .iter()
                .filter(|item| item_visible_for_query(item, &query, group_matches))
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect();
            group_visible_for_query(&query, group_matches, !items.is_empty()).then(|| GroupView { name: group.name.clone(), root_only: group.root_only, items })
        })
        .collect()
}

fn group_render_segments(
    groups: &[GroupView],
    display: &[(usize, usize)],
    parents: &std::collections::BTreeMap<String, String>,
    orders: &std::collections::BTreeMap<String, Vec<HierarchyOrderEntry>>,
) -> Vec<crate::pickers::picker_browser_projection::GroupRenderSegment> {
    let keys = test_projection_keys(groups);
    build_hierarchy_projection(groups, display, parents, orders, &keys, &mut PickerIdRegistry::default()).map(|(segments, _, _)| segments).unwrap_or_default()
}
pub(super) fn test_projection_keys(groups: &[GroupView]) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("test-group", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
        for (item_index, item) in group.items.iter().enumerate() {
            let folder_path = &item.group_path[..item.group_path.len().min(crate::pickers::asset_browser_hierarchy::MAX_HIERARCHY_DEPTH)];
            keys.insert(
                ProjectionNodeRef::Item { group_index, item_index },
                PickerIdentityKey::from_parts("test-item", [group.name.as_str(), item.key.as_str()]),
                if folder_path.is_empty() {
                    PickerNodeIdentity::Item { group: group.name.clone(), key: item.key.clone() }
                } else {
                    PickerNodeIdentity::FolderItem { group: group.name.clone(), path: folder_path.to_vec(), key: item.key.clone() }
                },
            );
            for depth in 1..=folder_path.len() {
                keys.insert(
                    ProjectionNodeRef::Folder { group_index, path: folder_path[..depth].to_vec() },
                    PickerIdentityKey::from_parts("test-folder", std::iter::once(group.name.as_str()).chain(folder_path[..depth].iter().map(String::as_str))),
                    PickerNodeIdentity::Folder { group: group.name.clone(), path: folder_path[..depth].to_vec() },
                );
            }
        }
    }
    keys
}
fn full_hierarchy_selection_order(hierarchy: &PickerBrowserHierarchy) -> Vec<PickerId> {
    let mut order = Vec::new();
    let mut pending: Vec<_> = hierarchy.roots().iter().rev().collect();
    while let Some(node) = pending.pop() {
        order.push(node.id().clone());
        pending.extend(node.children().iter().rev());
    }
    order
}
/// Group header rects drawn last frame (name ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¾Ãƒâ€šÃ‚Â¢ rect), in display order; for tests and automation.
pub(crate) fn test_last_group_headers(ctx: &egui::Context, panel: &str) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-group-headers", panel)))).unwrap_or_default()
}
/// The allocated viewport from the last frame; useful to verify the shared browser's layout.
pub(crate) fn test_last_browser_rect(ctx: &egui::Context, panel: &str) -> Option<Rect> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-browser-rect", panel))))
}

mod moved_unit_tests {
    use super::*;

    #[cfg(test)]
    mod item_batch_tests {
        use super::{BrowserAction, GroupView, ItemView, advance_item_rename, begin_item_rename, delete_item_actions, item_context_targets, move_item_actions};
        use crate::pickers::picker_state::PresetUi;

        #[test]
        fn item_context_actions_target_the_selected_cards_and_keep_single_card_fallback() {
            let clicked_group = GroupView { name: "A".into(), root_only: false, items: Vec::new() };
            let clicked = ItemView { key: "one".into(), name: "One".into(), group_path: Vec::new() };
            let selected = vec![
                ("A".to_owned(), clicked.clone()),
                ("B".to_owned(), ItemView { key: "two".into(), name: "Two".into(), group_path: vec!["Folder".into()] }),
                ("C".to_owned(), ItemView { key: "three".into(), name: "Three".into(), group_path: Vec::new() }),
            ];

            let targets = item_context_targets(&clicked, &clicked_group, &selected);
            assert_eq!(targets.len(), 3);
            assert!(matches!(delete_item_actions(&targets).as_slice(), [
            BrowserAction::DeleteItem { group: a, key: one, .. },
            BrowserAction::DeleteItem { group: b, key: two, group_path, .. },
            BrowserAction::DeleteItem { group: c, key: three, .. },
        ] if a == "A" && one == "one" && b == "B" && two == "two" && group_path == &["Folder".to_owned()] && c == "C" && three == "three"));

            let moves = move_item_actions(&targets, "B");
            assert!(matches!(moves.as_slice(), [BrowserAction::Move(one, target), BrowserAction::Move(three, target_again)]
            if one == "one" && three == "three" && target == "B" && target_again == "B"));

            let unselected = ItemView { key: "outside".into(), name: "Outside".into(), group_path: Vec::new() };
            let fallback = item_context_targets(&unselected, &clicked_group, &selected);
            assert_eq!(fallback.len(), 1);
            assert_eq!(fallback[0].1.key, "outside");
        }

        #[test]
        fn multi_rename_processes_one_selected_item_after_another() {
            let targets = vec![
                ("A".into(), ItemView { key: "first".into(), name: "First".into(), group_path: Vec::new() }),
                ("B".into(), ItemView { key: "second".into(), name: "Second".into(), group_path: vec!["Folder".into()] }),
                ("C".into(), ItemView { key: "third".into(), name: "Third".into(), group_path: Vec::new() }),
            ];
            let mut state = PresetUi::default();
            begin_item_rename(&mut state, "panel", &targets);
            assert_eq!(state.item_renaming.as_ref().map(|rename| rename.key.as_str()), Some("first"));
            assert_eq!(state.item_rename_queue.iter().map(|rename| rename.key.as_str()).collect::<Vec<_>>(), ["second", "third"]);

            state.item_renaming = None; // the first inline editor was committed or cancelled
            advance_item_rename(&mut state, "panel");
            assert_eq!(
                state.item_renaming.as_ref().map(|rename| (rename.key.as_str(), rename.group.as_str(), rename.group_path.as_slice())),
                Some(("second", "B", &["Folder".to_owned()][..]))
            );

            state.item_renaming = None;
            advance_item_rename(&mut state, "panel");
            assert_eq!(state.item_renaming.as_ref().map(|rename| rename.key.as_str()), Some("third"));
            assert!(state.item_rename_queue.is_empty());
        }
    }
    #[cfg(test)]
    mod browser_drag_ghost_tests {
        use super::{Rect, browser_drag_ghost_position, clamp_drag_ghost, pos2, vec2};

        #[test]
        fn pointer_leaving_the_window_returns_the_ghost_to_its_exact_pickup_bounds() {
            let origin = Rect::from_min_size(pos2(12.0, 18.0), vec2(80.0, 40.0));
            let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
            assert_eq!(browser_drag_ghost_position(None, origin, vec2(15.0, 10.0), picker), origin);
        }

        #[test]
        fn pointer_outside_the_picker_shell_follows_cursor_with_original_pickup_offset() {
            let origin = Rect::from_min_size(pos2(30.0, 45.0), vec2(80.0, 40.0));
            let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
            let expected = Rect::from_min_size(pos2(85.0, 160.0), origin.size());
            assert_eq!(browser_drag_ghost_position(Some(pos2(100.0, 170.0)), origin, vec2(15.0, 10.0), picker), expected);
        }

        #[test]
        fn ghost_stays_fully_inside_browser_window_when_pointer_reaches_its_edges() {
            let origin = Rect::from_min_size(pos2(30.0, 45.0), vec2(80.0, 40.0));
            let picker = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
            let expected = Rect::from_min_size(pos2(140.0, 110.0), vec2(80.0, 40.0));
            assert_eq!(browser_drag_ghost_position(Some(pos2(215.0, 145.0)), origin, vec2(15.0, 10.0), picker), expected);
        }

        #[test]
        fn dragged_cards_and_groups_press_against_each_window_edge() {
            let window = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 120.0));
            let ghost = Rect::from_min_size(pos2(-15.0, 130.0), vec2(80.0, 40.0));
            assert_eq!(clamp_drag_ghost(ghost, window), Rect::from_min_size(pos2(20.0, 110.0), vec2(80.0, 40.0)));
        }

        #[test]
        fn oversized_drag_ghosts_anchor_to_the_window_origin() {
            let window = Rect::from_min_size(pos2(20.0, 30.0), vec2(80.0, 40.0));
            let ghost = Rect::from_min_size(pos2(-15.0, -15.0), vec2(100.0, 60.0));
            assert_eq!(clamp_drag_ghost(ghost, window), Rect::from_min_size(pos2(20.0, 30.0), vec2(100.0, 60.0)));
        }
    }
    #[cfg(test)]
    mod selected_drag_nodes_tests {
        use std::collections::BTreeMap;

        use super::{plan_hierarchy_item_moves, selected_drag_nodes};
        use crate::pickers::picker_browser_identity::PickerNodeIdentity;
        use crate::pickers::picker_browser_model::{DropPosition, HierarchyNode, PickerBrowserHierarchy, PickerId};
        use crate::pickers::picker_state::PresetUi;

        #[test]
        fn selected_ancestor_subsumes_descendants_and_keeps_independent_nodes_in_visible_order() {
            let hierarchy = PickerBrowserHierarchy::new(vec![
                HierarchyNode::group("A", vec![HierarchyNode::item("a1"), HierarchyNode::group("Nested", vec![HierarchyNode::item("nested-card")])]),
                HierarchyNode::group("B", vec![HierarchyNode::item("b1")]),
            ])
            .unwrap();
            let visible = ["A", "a1", "Nested", "nested-card", "B", "b1"].map(PickerId::from);
            let mut state = PresetUi::default();
            for (index, id) in ["A", "a1", "Nested", "nested-card", "b1"].into_iter().map(PickerId::from).enumerate() {
                state.select_browser_node("panel", id, index != 0);
            }

            assert_eq!(selected_drag_nodes(&state, "panel", &visible, &PickerId::from("a1"), &hierarchy), [PickerId::from("A"), PickerId::from("b1")]);
        }

        #[test]
        fn dragging_an_unselected_node_does_not_move_the_other_selected_nodes() {
            let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group("A", vec![HierarchyNode::item("a1")])]).unwrap();
            let mut state = PresetUi::default();
            state.select_browser_node("panel", PickerId::from("A"), false);

            assert!(selected_drag_nodes(&state, "panel", &[PickerId::from("A"), PickerId::from("a1")], &PickerId::from("a1"), &hierarchy).is_empty());
        }

        #[test]
        fn item_callback_indices_follow_prior_selected_moves_for_every_insertion_position() {
            let hierarchy = PickerBrowserHierarchy::new(vec![
                HierarchyNode::group(
                    PickerId::group("A"),
                    vec![
                        HierarchyNode::item(PickerId::item("A", "a1")),
                        HierarchyNode::item(PickerId::item("A", "a2")),
                        HierarchyNode::item(PickerId::item("A", "a3")),
                    ],
                ),
                HierarchyNode::group(
                    PickerId::group("B"),
                    vec![HierarchyNode::item(PickerId::item("B", "b1")), HierarchyNode::item(PickerId::item("B", "b2"))],
                ),
            ])
            .unwrap();
            let identities = BTreeMap::from([
                (PickerId::group("A"), PickerNodeIdentity::Group("A".into())),
                (PickerId::item("A", "a1"), PickerNodeIdentity::Item { group: "A".into(), key: "a1".into() }),
                (PickerId::item("A", "a2"), PickerNodeIdentity::Item { group: "A".into(), key: "a2".into() }),
                (PickerId::item("A", "a3"), PickerNodeIdentity::Item { group: "A".into(), key: "a3".into() }),
                (PickerId::group("B"), PickerNodeIdentity::Group("B".into())),
                (PickerId::item("B", "b1"), PickerNodeIdentity::Item { group: "B".into(), key: "b1".into() }),
                (PickerId::item("B", "b2"), PickerNodeIdentity::Item { group: "B".into(), key: "b2".into() }),
            ]);
            let sources = [PickerId::item("A", "a1"), PickerId::item("A", "a2")];
            let cases = [
                (DropPosition::Before { target: PickerId::item("B", "b1") }, vec![("a1", 0), ("a2", 1)]),
                (DropPosition::After { target: PickerId::item("B", "b1") }, vec![("a2", 1), ("a1", 1)]),
                (DropPosition::PrependTo { group: PickerId::group("B") }, vec![("a2", 0), ("a1", 0)]),
                (
                    DropPosition::AppendTo { container: crate::pickers::picker_browser_model::ContainerId::Group(PickerId::group("B")) },
                    vec![("a1", 2), ("a2", 3)],
                ),
                (DropPosition::After { target: PickerId::item("A", "a3") }, vec![("a2", 2), ("a1", 1)]),
            ];

            for (position, expected) in cases {
                let actual = plan_hierarchy_item_moves(&hierarchy, &sources, &position, &identities, None).unwrap();
                assert_eq!(actual.iter().map(|(_, key, _, index)| (key.as_str(), *index)).collect::<Vec<_>>(), expected, "position={position:?}");
            }
        }
    }
    #[cfg(test)]
    mod group_drop_tests {
        use super::{AssetDrag, hierarchy_drop_accepts, is_group_descendant};
        use std::collections::BTreeMap;

        #[test]
        fn rejects_dropping_a_group_into_any_descendant() {
            let parents = BTreeMap::from([
                ("Child".to_string(), "Parent".to_string()),
                ("Grandchild".to_string(), "Child".to_string()),
                ("Other".to_string(), "Elsewhere".to_string()),
            ]);

            assert!(is_group_descendant(&parents, "Parent", "Child"));
            assert!(is_group_descendant(&parents, "Parent", "Grandchild"));
            assert!(!is_group_descendant(&parents, "Parent", "Other"));
            assert!(!is_group_descendant(&parents, "Parent", "Parent"));
        }

        #[test]
        fn hierarchy_card_targets_keep_root_and_nested_group_rules_consistent() {
            let parents = BTreeMap::from([("Child".to_string(), "Parent".to_string()), ("Grandchild".to_string(), "Child".to_string())]);
            let root_drag = AssetDrag::Group { scope: "panel".into(), name: "Parent".into(), path: Vec::new() };
            let nested_drag = AssetDrag::Group { scope: "panel".into(), name: "Parent".into(), path: vec!["Folder".into()] };

            assert!(!hierarchy_drop_accepts(&root_drag, "panel", "Parent", &[], &parents, true));
            assert!(!hierarchy_drop_accepts(&root_drag, "panel", "Child", &[], &parents, true));
            assert!(hierarchy_drop_accepts(&root_drag, "panel", "Grandchild", &[], &parents, false));
            assert!(!hierarchy_drop_accepts(&nested_drag, "panel", "Parent", &["Folder".into(), "Inner".into()], &parents, true));
            assert!(hierarchy_drop_accepts(&nested_drag, "panel", "Parent", &["Other".into()], &parents, true));
            assert!(!hierarchy_drop_accepts(&nested_drag, "other-panel", "Parent", &["Other".into()], &parents, true));
        }
    }
    #[cfg(test)]
    mod selected_delete_tests {
        use super::{
            BrowserAction, BrowserDeleteContext, PickerNodeIdentity, full_hierarchy_selection_order, replace_selected_delete_actions, selected_delete_actions,
        };
        use crate::pickers::picker_browser_model::{HierarchyNode as HNode, PickerBrowserHierarchy, PickerId};
        use crate::pickers::picker_state::PresetUi;
        use std::collections::{BTreeMap, BTreeSet};

        fn fixture() -> (PickerBrowserHierarchy, BTreeMap<PickerId, PickerNodeIdentity>, Vec<PickerId>) {
            let a = PickerId::group("A");
            let a_card = PickerId::item("A", "a1");
            let b = PickerId::group("B");
            let b_card = PickerId::item("B", "b1");
            let c = PickerId::group("C");
            let hierarchy = PickerBrowserHierarchy::new(vec![
                HNode::group(a.clone(), vec![HNode::item(a_card.clone())]),
                HNode::group(b.clone(), vec![HNode::item(b_card.clone())]),
                HNode::group(c.clone(), vec![]),
            ])
            .unwrap();
            let identities = BTreeMap::from([
                (a.clone(), PickerNodeIdentity::Group("A".into())),
                (a_card.clone(), PickerNodeIdentity::Item { group: "A".into(), key: "a1".into() }),
                (b.clone(), PickerNodeIdentity::Group("B".into())),
                (b_card.clone(), PickerNodeIdentity::Item { group: "B".into(), key: "b1".into() }),
                (c.clone(), PickerNodeIdentity::Group("C".into())),
            ]);
            let order = full_hierarchy_selection_order(&hierarchy);
            (hierarchy, identities, order)
        }

        #[test]
        fn mixed_selection_deletes_groups_and_cards_in_visual_order() {
            let (hierarchy, identities, visible) = fixture();
            let selected = BTreeSet::from([PickerId::group("A"), PickerId::item("B", "b1"), PickerId::group("C")]);
            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            let actions = selected_delete_actions(&selected, &PickerId::group("A"), &context);
            assert!(matches!(actions.as_deref(), Some([
        BrowserAction::DeleteGroup(a),
        BrowserAction::DeleteItem { group, key, .. },
        BrowserAction::DeleteGroup(c),
    ]) if a == "A" && group == "B" && key == "b1" && c == "C"));
        }

        #[test]
        fn selected_group_subsumes_its_selected_descendant_and_unselected_click_falls_back() {
            let (hierarchy, identities, visible) = fixture();
            let selected = BTreeSet::from([PickerId::group("A"), PickerId::item("A", "a1")]);
            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            let actions = selected_delete_actions(&selected, &PickerId::item("A", "a1"), &context);
            assert!(matches!(actions.as_deref(), Some([BrowserAction::DeleteGroup(group)]) if group == "A"));
            assert!(selected_delete_actions(&selected, &PickerId::group("B"), &context).is_none());
        }

        #[test]
        fn nested_folder_deletion_subsumes_selected_nested_folders_and_folder_items() {
            let root = PickerId::group("A");
            let folder = PickerId::group_path(["A"], "Folder");
            let nested = PickerId::group_path(["A", "Folder"], "Nested");
            let folder_item = PickerId::item_in_path(["A", "Folder", "Nested"], "nested-card");
            let root_item = PickerId::item("A", "root-card");
            let hierarchy = PickerBrowserHierarchy::new(vec![HNode::group(
                root.clone(),
                vec![HNode::group(folder.clone(), vec![HNode::group(nested.clone(), vec![HNode::item(folder_item.clone())])]), HNode::item(root_item.clone())],
            )])
            .unwrap();
            let identities = BTreeMap::from([
                (root.clone(), PickerNodeIdentity::Group("A".into())),
                (folder.clone(), PickerNodeIdentity::Folder { group: "A".into(), path: vec!["Folder".into()] }),
                (nested.clone(), PickerNodeIdentity::Folder { group: "A".into(), path: vec!["Folder".into(), "Nested".into()] }),
                (
                    folder_item.clone(),
                    PickerNodeIdentity::FolderItem { group: "A".into(), path: vec!["Folder".into(), "Nested".into()], key: "nested-card".into() },
                ),
                (root_item.clone(), PickerNodeIdentity::Item { group: "A".into(), key: "root-card".into() }),
            ]);
            let visible = vec![root, folder.clone(), nested.clone(), folder_item.clone(), root_item.clone()];

            let selected = BTreeSet::from([folder.clone(), nested, folder_item.clone()]);
            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            let actions = selected_delete_actions(&selected, &folder, &context);
            assert!(matches!(actions.as_deref(), Some([BrowserAction::DeleteNestedGroup { group, group_path }]) if group == "A" && group_path == &["Folder"]));

            let selected = BTreeSet::from([root_item]);
            let actions = selected_delete_actions(&selected, &visible[4], &context);
            assert!(
                matches!(actions.as_deref(), Some([BrowserAction::DeleteItem { group, group_path, key }]) if group == "A" && group_path.is_empty() && key == "root-card")
            );

            let selected = BTreeSet::from([folder_item]);
            let actions = selected_delete_actions(&selected, &visible[3], &context);
            assert!(
                matches!(actions.as_deref(), Some([BrowserAction::DeleteItem { group, group_path, key }]) if group == "A" && group_path == &["Folder", "Nested"] && key == "nested-card")
            );
        }

        #[test]
        fn nested_domain_groups_subsume_selected_child_groups_and_cards() {
            let root = PickerId::group("A");
            let child = PickerId::group("B");
            let card = PickerId::item("B", "b1");
            let hierarchy =
                PickerBrowserHierarchy::new(vec![HNode::group(root.clone(), vec![HNode::group(child.clone(), vec![HNode::item(card.clone())])])]).unwrap();
            let identities = BTreeMap::from([
                (root.clone(), PickerNodeIdentity::Group("A".into())),
                (child.clone(), PickerNodeIdentity::Group("B".into())),
                (card.clone(), PickerNodeIdentity::Item { group: "B".into(), key: "b1".into() }),
            ]);
            let visible = vec![root.clone(), child.clone(), card.clone()];
            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };

            let selected = BTreeSet::from([root, child.clone(), card]);
            let actions = selected_delete_actions(&selected, &child, &context);
            assert!(matches!(actions.as_deref(), Some([BrowserAction::DeleteGroup(group)]) if group == "A"));
        }

        #[test]
        fn group_context_menu_delete_expands_to_the_mixed_selection() {
            let (hierarchy, identities, visible) = fixture();
            let selected = BTreeSet::from([PickerId::group("A"), PickerId::item("B", "b1")]);
            let mut state = PresetUi::default();
            state.browser_selection.insert("panel".into(), selected);
            let mut events = vec![BrowserAction::DeleteGroup("A".into())];

            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            replace_selected_delete_actions(&mut events, 0, &state, "panel", &PickerId::group("A"), &context);

            assert!(
                matches!(events.as_slice(), [BrowserAction::DeleteGroup(group), BrowserAction::DeleteItem { group: item_group, key, .. }] if group == "A" && item_group == "B" && key == "b1")
            );
        }

        #[test]
        fn card_context_menu_delete_expands_to_the_mixed_selection() {
            let (hierarchy, identities, visible) = fixture();
            let clicked = PickerId::item("B", "b1");
            let selected = BTreeSet::from([PickerId::group("A"), clicked.clone()]);
            let mut state = PresetUi::default();
            state.browser_selection.insert("panel".into(), selected);
            let mut events = vec![BrowserAction::DeleteItem { group: "B".into(), group_path: Vec::new(), key: "b1".into() }];

            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            replace_selected_delete_actions(&mut events, 0, &state, "panel", &clicked, &context);

            assert!(
                matches!(events.as_slice(), [BrowserAction::DeleteGroup(group), BrowserAction::DeleteItem { group: item_group, key, .. }] if group == "A" && item_group == "B" && key == "b1")
            );
        }

        #[test]
        fn unselected_context_menu_target_keeps_its_original_delete_action() {
            let (hierarchy, identities, visible) = fixture();
            let mut state = PresetUi::default();
            state.browser_selection.insert("panel".into(), BTreeSet::from([PickerId::group("A")]));
            let original = BrowserAction::DeleteGroup("B".into());
            let mut events = vec![original.clone()];

            let context = BrowserDeleteContext { selection_order: &visible, hierarchy: &hierarchy, identities: &identities };
            replace_selected_delete_actions(&mut events, 0, &state, "panel", &PickerId::group("B"), &context);

            assert!(matches!(events.as_slice(), [BrowserAction::DeleteGroup(group)] if group == "B"));

            let clicked_card = PickerId::item("B", "b1");
            let mut events = vec![BrowserAction::DeleteItem { group: "B".into(), group_path: Vec::new(), key: "b1".into() }];
            replace_selected_delete_actions(&mut events, 0, &state, "panel", &clicked_card, &context);
            assert!(matches!(events.as_slice(), [BrowserAction::DeleteItem { group, key, .. }] if group == "B" && key == "b1"));
        }
    }
    #[cfg(test)]
    mod display_layout_tests {
        use super::{canvas_drop_target, item_grid_position, list_layout_geometry};
        use crate::pickers::picker_browser_model::PickerId;
        use crate::pickers::picker_state::{PickerDisplayMode, PresetUi};

        #[test]
        fn list_width_is_fixed_only_when_wrap_is_enabled() {
            let (unwrapped, unwrapped_cols, unwrapped_rows) = list_layout_geometry(900.0, 0.0, 20.0, false, Some(198.0), 7);
            assert_eq!(unwrapped.x, 900.0);
            assert_eq!((unwrapped_cols, unwrapped_rows), (1, 7));
            let (unwrapped_large, large_cols, large_rows) = list_layout_geometry(900.0, 0.0, 34.0, false, Some(247.0), 7);
            assert_eq!(unwrapped_large.x, 900.0);
            assert_eq!((large_cols, large_rows), (1, 7));
            let (indented_unwrapped, indented_cols, indented_rows) = list_layout_geometry(900.0, 41.0, 34.0, false, Some(247.0), 7);
            assert_eq!(indented_unwrapped.x, 859.0, "the left hierarchy inset is also subtracted from row width");
            assert_eq!((indented_cols, indented_rows), (1, 7));
            let (narrow, narrow_cols, _) = list_layout_geometry(140.0, 0.0, 20.0, true, Some(198.0), 3);
            assert_eq!(narrow.x, 140.0);
            assert_eq!(narrow_cols, 1);
            let (narrow_large, narrow_large_cols, _) = list_layout_geometry(180.0, 0.0, 34.0, true, Some(247.0), 3);
            assert_eq!(narrow_large.x, 180.0);
            assert_eq!(narrow_large_cols, 1);

            let (wrapped, wrapped_cols, wrapped_rows) = list_layout_geometry(900.0, 0.0, 20.0, true, Some(198.0), 7);
            assert_eq!(wrapped.x, 198.0);
            assert_eq!((wrapped_cols, wrapped_rows), (4, 2));
            let (wrapped_text_only, text_cols, text_rows) = list_layout_geometry(900.0, 0.0, 20.0, true, PickerDisplayMode::TextOnly.list_width(), 7);
            assert_eq!(wrapped_text_only.x, 198.0, "Text Only shares Small Thumbnail's wrap width");
            assert_eq!((text_cols, text_rows), (4, 2));
            let (wrapped_large, wrapped_large_cols, wrapped_large_rows) = list_layout_geometry(1000.0, 0.0, 34.0, true, Some(247.0), 7);
            assert_eq!(wrapped_large.x, 247.0);
            assert_eq!((wrapped_large_cols, wrapped_large_rows), (4, 2));
        }

        #[test]
        fn wrapped_list_flows_down_each_column_before_moving_right_and_shift_selection_follows_it() {
            let (_, cols, rows) = list_layout_geometry(600.0, 0.0, 20.0, true, Some(198.0), 5);
            assert_eq!((cols, rows), (3, 2));
            assert_eq!(item_grid_position(0, rows, cols, true), (0.0, 0.0));
            assert_eq!(item_grid_position(1, rows, cols, true), (0.0, 1.0));
            assert_eq!(item_grid_position(2, rows, cols, true), (1.0, 0.0));
            assert_eq!(item_grid_position(3, rows, cols, true), (1.0, 1.0));
            assert_eq!(item_grid_position(4, rows, cols, true), (2.0, 0.0));

            let visible = (0..5).map(|index| PickerId::item("group", index.to_string())).collect::<Vec<_>>();
            let mut state = PresetUi::default();
            state.select_browser_node("panel", visible[1].clone(), false);
            state.select_browser_range("panel", &visible, &visible[2]);
            assert_eq!(state.browser_selection["panel"], [visible[1].clone(), visible[2].clone()].into_iter().collect());
        }

        #[test]
        fn canvas_drop_does_not_capture_a_release_inside_the_browser_viewport() {
            let canvas = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 600.0));
            let browser = egui::Rect::from_min_max(egui::pos2(500.0, 100.0), egui::pos2(800.0, 500.0));
            let item = egui::Rect::from_min_max(egui::pos2(520.0, 120.0), egui::pos2(620.0, 160.0));

            assert!(!canvas_drop_target(egui::pos2(700.0, 300.0), item, canvas, browser), "empty browser space is still a browser drop target");
            assert!(!canvas_drop_target(egui::pos2(550.0, 140.0), item, canvas, browser), "the source card is not a canvas drop target");
            assert!(canvas_drop_target(egui::pos2(300.0, 300.0), item, canvas, browser), "a release over the actual canvas remains an application drop");
        }
    }
    #[cfg(test)]
    mod filter_projection_tests {
        use super::{
            GroupView, HIT_DEBUG_LABEL_HEIGHT, HierarchyOrderEntry, ItemView, best_drop_target_index, best_indent_drop_target_index, filter_groups,
            group_render_segments, hit_debug_label_position, item_hierarchy_insertion_anchor, item_occurrence_position, test_projection_keys,
        };

        fn group(name: &str, items: Vec<ItemView>) -> GroupView {
            GroupView { name: name.into(), root_only: false, items }
        }

        fn item(key: &str, name: &str, group_path: &[&str]) -> ItemView {
            ItemView { key: key.into(), name: name.into(), group_path: group_path.iter().map(|part| (*part).into()).collect() }
        }

        #[test]
        fn filtering_keeps_identity_order_and_matches_root_or_nested_group_names() {
            let groups = vec![
                group("Warm", vec![item("sunset-id", "Sunset", &["Favorites", "Sky"]), item("ember-id", "Ember", &[])]),
                group("Cool", vec![item("ocean-id", "Ocean", &[])]),
            ];

            let by_nested_group = filter_groups(&groups, "sky");
            assert_eq!(by_nested_group.len(), 1);
            assert_eq!(by_nested_group[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["sunset-id"]);

            let by_item = filter_groups(&groups, "ember");
            assert_eq!(by_item.len(), 1);
            assert_eq!(by_item[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["ember-id"]);

            let by_group = filter_groups(&groups, "WARM");
            assert_eq!(by_group[0].items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["sunset-id", "ember-id"]);

            let all = filter_groups(&groups, "  ");
            assert_eq!(all.iter().map(|group| group.name.as_str()).collect::<Vec<_>>(), ["Warm", "Cool"]);
        }

        #[test]
        fn occurrence_lookup_distinguishes_items_with_the_same_display_name() {
            let groups = vec![group("Brushes", vec![item("brush-occurrence-a", "Soft Round", &[]), item("brush-occurrence-b", "Soft Round", &[])])];

            assert_eq!(item_occurrence_position(&groups, "brush-occurrence-a"), Some((0, 0)));
            assert_eq!(item_occurrence_position(&groups, "brush-occurrence-b"), Some((0, 1)));
        }

        #[test]
        fn adjacent_root_cards_share_an_unindented_grid_segment() {
            let groups = vec![GroupView { name: "Presets".into(), root_only: true, items: vec![item("first", "First", &[]), item("second", "Second", &[])] }];
            let orders = std::collections::BTreeMap::from([(
                "".into(),
                vec![
                    HierarchyOrderEntry::RootItem { group: "Presets".into(), key: "first".into() },
                    HierarchyOrderEntry::RootItem { group: "Presets".into(), key: "second".into() },
                ],
            )]);

            let segments = group_render_segments(&groups, &[], &Default::default(), &orders);

            assert_eq!(segments.len(), 1);
            assert!(segments[0].root_items);
            assert_eq!(segments[0].depth, 0);
            assert_eq!(segments[0].item_indices, [(0, 0), (0, 1)]);
        }

        #[test]
        fn root_card_runs_split_at_groups_and_retain_each_source_group() {
            let groups = vec![
                group("First", vec![item("first-a", "First A", &[]), item("first-b", "First B", &[])]),
                group("Middle", vec![item("middle", "Middle", &[])]),
                group("Last", vec![item("last", "Last", &[])]),
            ];
            let orders = std::collections::BTreeMap::from([(
                "".into(),
                vec![
                    HierarchyOrderEntry::Group("First".into()),
                    HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-a".into() },
                    HierarchyOrderEntry::Group("Middle".into()),
                    HierarchyOrderEntry::RootItem { group: "Last".into(), key: "last".into() },
                    HierarchyOrderEntry::Group("Last".into()),
                    HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-b".into() },
                ],
            )]);

            let segments = group_render_segments(&groups, &[(0, 0), (1, 0), (2, 0)], &Default::default(), &orders);

            assert_eq!(
                segment_summary(&segments, &groups),
                [
                    ("First".into(), true, vec![]),
                    ("First".into(), false, vec!["first-a".into()]),
                    ("Middle".into(), true, vec!["middle".into()]),
                    ("Last".into(), false, vec!["last".into()]),
                    ("Last".into(), true, vec![]),
                    ("First".into(), false, vec!["first-b".into()]),
                ]
            );
        }

        #[test]
        fn consecutive_root_cards_from_different_backing_groups_share_one_segment() {
            let groups = vec![
                group("First", vec![item("first-a", "First A", &[]), item("first-after", "First after", &[])]),
                group("Second", vec![item("second-a", "Second A", &[])]),
                group("Middle", vec![]),
            ];
            let orders = std::collections::BTreeMap::from([(
                "".into(),
                vec![
                    HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-a".into() },
                    HierarchyOrderEntry::RootItem { group: "Second".into(), key: "second-a".into() },
                    HierarchyOrderEntry::Group("Middle".into()),
                    HierarchyOrderEntry::RootItem { group: "First".into(), key: "first-after".into() },
                ],
            )]);

            let segments = group_render_segments(&groups, &[(0, 0), (1, 0), (2, 0)], &Default::default(), &orders);

            assert_eq!(segments.len(), 5, "each root card bunch must remain a single segment, with the intervening backing groups preserved");
            assert!(segments[0].root_items);
            assert_eq!(segments[0].item_indices, [(0, 0), (1, 0)]);
            assert!(segments[1].show_header);
            assert_eq!(segments[1].group_index, 2);
            assert!(segments[2].root_items);
            assert_eq!(segments[2].item_indices, [(0, 1)]);
            assert!(segments[3].show_header);
            assert_eq!(segments[3].group_index, 0);
            assert!(segments[4].show_header);
            assert_eq!(segments[4].group_index, 1);
        }

        #[test]
        fn best_drop_target_is_the_last_eligible_candidate() {
            let candidates = [false, true, false, true, false];

            assert_eq!(best_drop_target_index(&candidates, |eligible| *eligible), Some(3));
            assert_eq!(best_drop_target_index(&[false, false], |eligible| *eligible), None);
        }

        #[test]
        fn card_drop_target_wins_over_an_overlapping_group_indent_column() {
            let columns = [false, true, true];

            assert_eq!(best_indent_drop_target_index(&columns, true, |eligible| *eligible), None);
            assert_eq!(best_indent_drop_target_index(&columns, false, |eligible| *eligible), Some(2));
        }

        #[test]
        fn card_drop_anchor_tracks_the_next_item_or_nested_group_at_the_same_level() {
            let items = vec![item("a", "A", &[]), item("b", "B", &[]), item("nested", "Nested item", &["Nested"]), item("c", "C", &[])];

            assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], false, None), (None, Some("b".into())));
            assert_eq!(item_hierarchy_insertion_anchor(&items, 0, &[], true, None), (None, Some("b".into())));
            assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], true, None), (Some("Nested".into()), None));
            assert_eq!(item_hierarchy_insertion_anchor(&items, 3, &[], true, None), (None, None));
        }

        #[test]
        fn card_drop_after_skips_the_group_being_moved_when_it_is_the_next_sibling() {
            let items = vec![item("a", "A", &[]), item("f1", "F1", &["F"]), item("f2", "F2", &["F"]), item("b", "B", &[])];

            // F already follows A. It cannot be its own `before` anchor after the engine
            // partitions F out of the source group; B is the stable next sibling instead.
            assert_eq!(item_hierarchy_insertion_anchor(&items, 0, &[], true, Some(&["F".into()])), (None, Some("b".into())));
        }

        #[test]
        fn card_drop_after_skips_a_moving_group_before_the_target_card() {
            let items = vec![item("f", "F", &["F"]), item("a", "A", &[]), item("b", "B", &[])];

            assert_eq!(item_hierarchy_insertion_anchor(&items, 1, &[], true, Some(&["F".into()])), (None, Some("b".into())));
        }

        #[test]
        fn upper_and_lower_gap_debug_labels_do_not_overwrite_each_other() {
            let gap = egui::Rect::from_min_max(egui::pos2(10.0, 100.0), egui::pos2(210.0, 106.0));
            let hits = crate::pickers::asset_browser_interaction::group_gap_drop_hits(crate::pickers::asset_browser_interaction::GroupGapDropSpec {
                rect: gap,
                upper: crate::pickers::picker_browser_model::PickerId::from("upper"),
                lower: crate::pickers::picker_browser_model::PickerId::from("lower"),
                upper_bounds: egui::Rect::from_min_max(egui::pos2(20.0, 60.0), egui::pos2(120.0, 90.0)),
                lower_bounds: egui::Rect::from_min_max(egui::pos2(20.0, 106.0), egui::pos2(210.0, 135.0)),
                upper_depth: 0,
                lower_depth: 1,
                order: 0,
            });
            let upper_label = hit_debug_label_position(&hits[0]);
            let lower_label = hit_debug_label_position(&hits[1]);

            assert!(upper_label.y >= gap.bottom(), "upper label starts above the visible gap: y={}", upper_label.y);
            assert!(upper_label.y + HIT_DEBUG_LABEL_HEIGHT <= lower_label.y, "upper label at y={} overlaps lower label at y={}", upper_label.y, lower_label.y,);
        }

        #[test]
        fn nested_card_drop_after_skips_the_moving_group_and_anchors_to_the_next_sibling_group() {
            let items = vec![
                item("a", "A", &["Parent"]),
                item("f1", "F1", &["Parent", "Moving"]),
                item("f2", "F2", &["Parent", "Moving"]),
                item("g1", "G1", &["Parent", "Next"]),
            ];

            assert_eq!(
                item_hierarchy_insertion_anchor(&items, 0, &["Parent".into()], true, Some(&["Parent".into(), "Moving".into()])),
                (Some("Next".into()), None)
            );
        }

        fn segment_summary(
            segments: &[crate::pickers::picker_browser_projection::GroupRenderSegment],
            groups: &[GroupView],
        ) -> Vec<(String, bool, Vec<String>)> {
            segments
                .iter()
                .map(|segment| {
                    let group = &groups[segment.group_index];
                    let items = segment
                        .item_indices
                        .iter()
                        .filter_map(|(group_index, item_index)| groups.get(*group_index)?.items.get(*item_index))
                        .map(|item| item.key.clone())
                        .collect();
                    (group.name.clone(), segment.show_header, items)
                })
                .collect()
        }

        #[test]
        fn unified_sibling_order_splits_card_runs_at_group_positions() {
            let groups = vec![
                group("Parent", vec![item("a", "A", &[]), item("b", "B", &[]), item("c", "C", &[])]),
                group("Before A", vec![item("x", "X", &[])]),
                group("Before C", vec![item("y", "Y", &[])]),
            ];
            // The persisted sequence is the sole source of sibling positioning, regardless of the
            // groups' library/display order.
            let display = [(0, 0), (2, 1), (1, 1)];
            let parents = [("Before A".into(), "Parent".into()), ("Before C".into(), "Parent".into())].into();
            let orders = [(
                "Parent".into(),
                vec![
                    HierarchyOrderEntry::Group("Before A".into()),
                    HierarchyOrderEntry::Item("a".into()),
                    HierarchyOrderEntry::Item("b".into()),
                    HierarchyOrderEntry::Group("Before C".into()),
                    HierarchyOrderEntry::Item("c".into()),
                ],
            )]
            .into();

            let segments = group_render_segments(&groups, &display, &parents, &orders);

            assert_eq!(
                segment_summary(&segments, &groups),
                [
                    ("Parent".into(), true, vec![]),
                    ("Before A".into(), true, vec!["x".into()]),
                    ("Parent".into(), false, vec!["a".into(), "b".into()]),
                    ("Before C".into(), true, vec!["y".into()]),
                    ("Parent".into(), false, vec!["c".into()]),
                ]
            );
            assert_eq!(segments.iter().filter(|segment| segment.show_header).count(), groups.len());
        }

        #[test]
        fn sibling_sequence_keeps_empty_groups_and_appends_unlisted_entries() {
            let groups = vec![
                group("Parent", vec![item("a", "A", &[])]),
                group("First", vec![]),
                group("Second", vec![]),
                group("Stale", vec![]),
                group("Append", vec![]),
            ];
            let display = [(0, 0), (1, 1), (2, 1), (3, 1), (4, 1)];
            let parents =
                [("First".into(), "Parent".into()), ("Second".into(), "Parent".into()), ("Stale".into(), "Parent".into()), ("Append".into(), "Parent".into())]
                    .into();
            let orders = [(
                "Parent".into(),
                vec![HierarchyOrderEntry::Group("Second".into()), HierarchyOrderEntry::Item("a".into()), HierarchyOrderEntry::Group("First".into())],
            )]
            .into();

            let segments = group_render_segments(&groups, &display, &parents, &orders);

            assert_eq!(
                segment_summary(&segments, &groups),
                [
                    ("Parent".into(), true, vec![]),
                    ("Second".into(), true, vec![]),
                    ("Parent".into(), false, vec!["a".into()]),
                    ("First".into(), true, vec![]),
                    ("Stale".into(), true, vec![]),
                    ("Append".into(), true, vec![]),
                ]
            );
        }

        #[test]
        fn typed_root_items_distinguish_duplicate_keys_from_different_groups() {
            use super::build_hierarchy_projection;
            use crate::pickers::picker_browser_identity::PickerNodeIdentity;
            use crate::pickers::picker_browser_model::{HierarchyNode, PickerIdRegistry};

            let groups = vec![group("First", vec![item("same", "Same", &[])]), group("Second", vec![item("same", "Same", &[])])];
            let display = [(0, 0), (1, 0)];
            let orders = [
                (
                    "".into(),
                    vec![
                        HierarchyOrderEntry::RootItem { group: "First".into(), key: "same".into() },
                        HierarchyOrderEntry::RootItem { group: "Second".into(), key: "same".into() },
                    ],
                ),
                ("First".into(), vec![HierarchyOrderEntry::Item("same".into())]),
                ("Second".into(), vec![HierarchyOrderEntry::Item("same".into())]),
            ]
            .into();
            let (_, hierarchy, identities) =
                build_hierarchy_projection(&groups, &display, &Default::default(), &orders, &test_projection_keys(&groups), &mut PickerIdRegistry::default())
                    .unwrap();
            let root_items: Vec<_> = hierarchy.roots().iter().filter(|node| matches!(node, HierarchyNode::Item(_))).collect();
            assert_eq!(root_items.len(), 2);
            assert_eq!(identities[root_items[0].id()], PickerNodeIdentity::Item { group: "First".into(), key: "same".into() });
            assert_eq!(identities[root_items[1].id()], PickerNodeIdentity::Item { group: "Second".into(), key: "same".into() });
            assert!(hierarchy.roots().iter().all(|node| !node.is_group() || node.children().is_empty()));
        }

        #[test]
        fn projection_keeps_group_folder_and_card_ids_stable_when_sibling_order_changes() {
            use super::build_hierarchy_projection;
            use crate::pickers::picker_browser_identity::PickerNodeIdentity;
            use crate::pickers::picker_browser_model::{PickerId, PickerIdRegistry};

            let groups = vec![
                group("A", vec![item("a-root", "A root", &[]), item("a-nested", "A nested", &["Folder", "Inner"])]),
                group("B", vec![item("b-card", "B card", &[])]),
            ];
            let first_orders = std::collections::BTreeMap::from([
                ("".into(), vec![HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Group("B".into())]),
                ("A".into(), vec![HierarchyOrderEntry::Item("a-root".into()), HierarchyOrderEntry::Item("a-nested".into())]),
                ("B".into(), vec![HierarchyOrderEntry::Item("b-card".into())]),
            ]);
            let second_orders = std::collections::BTreeMap::from([
                ("".into(), vec![HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Group("A".into())]),
                ("A".into(), vec![HierarchyOrderEntry::Item("a-nested".into()), HierarchyOrderEntry::Item("a-root".into())]),
                ("B".into(), vec![HierarchyOrderEntry::Item("b-card".into())]),
            ]);
            let mut registry = PickerIdRegistry::with_scope(77);
            let (_, first_tree, first) =
                build_hierarchy_projection(&groups, &[(0, 0), (1, 0)], &Default::default(), &first_orders, &test_projection_keys(&groups), &mut registry)
                    .unwrap();
            let (_, second_tree, second) =
                build_hierarchy_projection(&groups, &[(1, 0), (0, 0)], &Default::default(), &second_orders, &test_projection_keys(&groups), &mut registry)
                    .unwrap();

            let by_identity = |identities: std::collections::BTreeMap<PickerId, PickerNodeIdentity>| {
                identities.into_iter().map(|(id, identity)| (identity, id)).collect::<std::collections::BTreeMap<_, _>>()
            };
            let first_ids = by_identity(first);
            let second_ids = by_identity(second);

            assert_eq!(first_ids, second_ids, "reordering groups, cards and nested folders must not allocate new runtime IDs");
            assert_ne!(first_tree, second_tree, "the projected order should actually change in this fixture");
            assert!(first_ids.contains_key(&PickerNodeIdentity::Folder { group: "A".into(), path: vec!["Folder".into()] }));
            assert!(first_ids.contains_key(&PickerNodeIdentity::Folder { group: "A".into(), path: vec!["Folder".into(), "Inner".into()] }));
            assert!(first_ids.contains_key(&PickerNodeIdentity::FolderItem {
                group: "A".into(),
                path: vec!["Folder".into(), "Inner".into()],
                key: "a-nested".into()
            }));
        }

        #[test]
        fn unified_order_supports_empty_parent_and_nested_group_sequences() {
            let groups = vec![group("Parent", vec![]), group("Child", vec![item("b", "B", &[])]), group("Grandchild", vec![])];
            let display = [(0, 0), (1, 1), (2, 2)];
            let parents = [("Child".into(), "Parent".into()), ("Grandchild".into(), "Child".into())].into();
            let orders = [
                ("Parent".into(), vec![HierarchyOrderEntry::Group("Child".into())]),
                ("Child".into(), vec![HierarchyOrderEntry::Group("Grandchild".into()), HierarchyOrderEntry::Item("b".into())]),
            ]
            .into();

            let segments = group_render_segments(&groups, &display, &parents, &orders);

            assert_eq!(
                segment_summary(&segments, &groups),
                [
                    ("Parent".into(), true, vec![]),
                    ("Child".into(), true, vec![]),
                    ("Grandchild".into(), true, vec![]),
                    ("Child".into(), false, vec!["b".into()]),
                ]
            );
            assert_eq!(segments.iter().filter(|segment| segment.show_header).count(), groups.len());
        }

        #[test]
        fn malformed_group_parent_cycles_are_emitted_once_without_recursing_forever() {
            let groups = vec![group("A", vec![]), group("B", vec![])];
            let display = [(0, 0), (1, 0)];
            let parents = [("A".into(), "B".into()), ("B".into(), "A".into())].into();

            let segments = group_render_segments(&groups, &display, &parents, &Default::default());

            assert_eq!(segment_summary(&segments, &groups).iter().map(|(name, _, _)| name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        }
    }
}
