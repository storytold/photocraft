//! Gradient-library adapter for the shared preset browser.

use egui::{Rect, pos2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::pickers::asset_browser::{
    BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, standard_group_context_menu,
    standard_item_context_menu,
};
use crate::pickers::asset_browser_interaction::canvas_drop_rect;
use crate::pickers::picker_footer::FooterOptions;
use crate::pickers::picker_shell::{ShellOptions, picker_shell};
use crate::pickers::picker_state::{HierarchyOrderEntry, ItemRenameState};
use crate::pickers::preset_commands::{generic, run};
use crate::pickers::preset_preview::c32;

/// The engine's fallback owner for gradients created when the library has no groups. The shared
/// hierarchy treats it as an invisible backing group while its cards live at the root.
const ROOT_ITEMS_GROUP: &str = "Presets";

/// Draw a gradient swatch, showing transparency over the standard checkerboard.
pub(crate) fn paint_gradient(ui: &egui::Ui, rect: Rect, stops: &[(f32, [f32; 4])]) {
    let painter = ui.painter_at(rect);
    if stops.iter().any(|stop| stop.1[3] < 0.999) {
        crate::widgets::checker(&painter, rect, (rect.height() / 4.0).clamp(3.0, 8.0));
    }
    let x = |t: f32| rect.left() + rect.width() * t.clamp(0.0, 1.0);
    let mut points = stops.to_vec();
    if points.first().is_some_and(|stop| stop.0 > 0.0) {
        points.insert(0, (0.0, points[0].1));
    }
    if let Some(last) = points.last().copied().filter(|stop| stop.0 < 1.0) {
        points.push((1.0, last.1));
    }

    let mut mesh = egui::Mesh::default();
    for pair in points.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let index = mesh.vertices.len() as u32;
        let (start_color, end_color) = (c32(start.1), c32(end.1));
        mesh.colored_vertex(pos2(x(start.0), rect.top()), start_color);
        mesh.colored_vertex(pos2(x(end.0), rect.top()), end_color);
        mesh.colored_vertex(pos2(x(end.0), rect.bottom()), end_color);
        mesh.colored_vertex(pos2(x(start.0), rect.bottom()), start_color);
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index, index + 2, index + 3);
    }
    painter.add(mesh);
}

/// Draw the standard Gradients panel using the shared browser and shell.
pub(crate) fn gradients_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (foreground, background) = (app.session.tools.foreground, app.session.tools.background);
    let library = app.session.presets.gradients.clone();
    // The reserved fallback group is the durable owner for root cards. Unlike generic picker
    // layout state, the gradient library itself is saved across restarts, so membership in this
    // group is the source of truth for root-level placement.
    let groups: Vec<GroupView> = library
        .iter()
        .map(|group| GroupView {
            name: group.name.clone(),
            root_only: group.name == ROOT_ITEMS_GROUP,
            items: group.items.iter().map(|item| ItemView { key: item.name.clone(), name: item.name.clone(), group_path: Vec::new() }).collect(),
        })
        .collect();
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let root_items: std::collections::BTreeSet<String> =
        library.iter().find(|group| group.name == ROOT_ITEMS_GROUP).into_iter().flat_map(|group| group.items.iter().map(|item| item.name.clone())).collect();
    // Preserve the canonical mixed root sequence written by commit_hierarchy_move. Rebuilding
    // RootItems first on every frame silently discarded the requested insertion point, making a
    // successful drop between groups or at the bottom appear to have failed (the card jumped to
    // the beginning of the list, often outside the current viewport).
    let visible_groups: std::collections::BTreeSet<String> = groups.iter().filter(|group| !group.root_only).map(|group| group.name.clone()).collect();
    let mut root_order: Vec<HierarchyOrderEntry> = state
        .hierarchy_order
        .get("gradients")
        .and_then(|orders| orders.get(""))
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            // commit_hierarchy_move records the item's original library group in the UI token,
            // while the domain adapter has physically moved it into the reserved owner group.
            // Match by durable key and normalize the backing group without changing its position.
            HierarchyOrderEntry::RootItem { key, .. } if root_items.contains(key) => {
                Some(HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() })
            }
            HierarchyOrderEntry::Group(name) if visible_groups.contains(name) => Some(entry.clone()),
            _ => None,
        })
        .collect();
    let represented_root_items: std::collections::BTreeSet<String> = root_order
        .iter()
        .filter_map(|entry| match entry {
            HierarchyOrderEntry::RootItem { key, .. } if root_items.contains(key) => Some(key.clone()),
            _ => None,
        })
        .collect();
    // Newly discovered durable root cards (for example after an app restart) have no transient
    // mixed-order token. Seed them before the existing group sequence; subsequent drag commits
    // then preserve their explicitly chosen positions.
    let mut missing_root_items: Vec<HierarchyOrderEntry> =
        root_items.difference(&represented_root_items).map(|key| HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: key.clone() }).collect();
    missing_root_items.append(&mut root_order);
    let root_order = missing_root_items;
    state.set_hierarchy_order("gradients", "", root_order);
    let canvas = canvas_drop_rect(app);
    let mut commit_item_move = |source_group: &str, key: &str, destination_group: &str, index: usize| {
        run(app, "gradient.presets.edit", json!({"action": "move", "preset": key, "group": source_group, "to": destination_group, "index": index}))
            .map(|_| ())
            .ok_or_else(|| "The Gradient library rejected the item move.".to_owned())
    };
    let mut adapter = BrowserAdapter {
        panel: "gradients",
        groups: &groups,
        options: BrowserOptions {
            canvas,
            flush: false,
            center_grid: false,
            grid_indent: 8.0,
            all_groups_open: false,
            compact_context_menus: false,
            item_size: None,
            list_item_height: None,
            domain_hierarchy_actions: false,
            root_item_group: Some(ROOT_ITEMS_GROUP),
        },
        // Cross-group membership uses the Gradient domain command; sibling/group order is saved
        // in the canonical picker layout state.
        capabilities: BrowserCapabilities { canonical_hierarchy_drag: true, ..BrowserCapabilities::default() },
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        nested_group_context_menu: None,
        canonical_hierarchy_move: None,
        hierarchy_item_move: Some(&mut commit_item_move),
        paint_group: None,
        paint_item: &mut |ui, rect, group_index, item_index, _paint_state: ItemPaintState| {
            let group = &groups[group_index];
            let key = &group.items[item_index].key;
            let stops = library
                .iter()
                .find(|g| g.name == group.name)
                .and_then(|g| g.items.iter().find(|item| &item.name == key))
                .map(|item| item.resolve(foreground, background))
                .unwrap_or_default();
            paint_gradient(ui, rect, &stops);
        },
        paint_list_item: None,
    };
    let events =
        picker_shell(ui, &mut state, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new gradient"))), ..Default::default() });
    drop(adapter);
    drop(commit_item_move);
    app.ui.presets_ui = state;

    for event in events {
        match &event {
            BrowserAction::Select(key) => {
                app.ui.presets_ui.set_selected("gradients", key);
                run(app, "gradient.presets.select", json!({"preset": key}));
            }
            BrowserAction::SelectItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("gradients", group, group_path, key);
                run(app, "gradient.presets.select", json!({"preset": key}));
            }
            BrowserAction::Activate(key) | BrowserAction::Drop(key, _) => {
                app.ui.presets_ui.set_selected("gradients", key);
                run(app, "gradient.presets.apply", json!({"preset": key}));
            }
            BrowserAction::ActivateItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("gradients", group, group_path, key);
                run(app, "gradient.presets.apply", json!({"preset": key}));
            }
            BrowserAction::New => {
                let root_level = groups.iter().all(|group| group.root_only);
                let destination_group = if root_level {
                    ROOT_ITEMS_GROUP
                } else {
                    groups.iter().find(|group| !group.root_only).map(|group| group.name.as_str()).unwrap_or(ROOT_ITEMS_GROUP)
                };
                if let Some(result) = run(app, "gradient.presets.new", json!({"name": "Custom", "group": destination_group})) {
                    let name = result.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
                    if !name.is_empty() {
                        app.ui.presets_ui.set_selected_item("gradients", destination_group, &[], &name);
                        if root_level {
                            let mut root = app.ui.presets_ui.hierarchy_order.get("gradients").and_then(|orders| orders.get("")).cloned().unwrap_or_default();
                            root.push(HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: name.clone() });
                            app.ui.presets_ui.set_hierarchy_order("gradients", "", root);
                        }
                        app.ui.presets_ui.item_renaming = Some(ItemRenameState {
                            panel: "gradients".into(),
                            group: destination_group.into(),
                            group_path: Vec::new(),
                            key: name.clone(),
                            text: name,
                        });
                    }
                }
            }
            // Only the brush browser supports nested-group context actions.
            BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
            _ => {
                generic(app, "gradients", "gradient.presets", &event, json!({"name": "Custom"}));
            }
        }
    }
}

/// Contribute the Gradients-specific actions to the shared docking header menu.
pub(crate) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::pickers::picker_shell::search_toggle(ui, &mut app.ui.presets_ui, "gradients");
    ui.separator();
    if ui.button(tl!("Import .GRD")).clicked() {
        crate::preset_files_ui::import_gradients_dialog(app);
        ui.close();
    }
    if ui.button(tl!("Reset")).clicked() {
        run(app, "gradient.presets.reset", json!({}));
        ui.close();
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::{ROOT_ITEMS_GROUP, gradients_panel};
    use crate::PhotocraftApp;
    use crate::pickers::asset_browser::last_group_headers;
    use crate::pickers::picker_state::HierarchyOrderEntry;

    #[test]
    fn creating_a_gradient_with_no_groups_places_the_card_at_root_without_a_visible_backing_group() {
        let mut session = photocraft_engine::Session::new();
        for group in session.presets.gradients.clone() {
            session.execute("gradient.presets.edit", serde_json::json!({"action": "deleteGroup", "group": group.name})).unwrap();
        }
        assert!(session.presets.gradients.is_empty());
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let create = h.get_by_label("Create new gradient").rect().center();
        h.event(Event::PointerMoved(create));
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: create, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(4);

        let library = &h.state().session.presets.gradients;
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].name, ROOT_ITEMS_GROUP);
        assert_eq!(library[0].items.len(), 1);
        let root = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
        assert_eq!(root, &[HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: library[0].items[0].name.clone() }]);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "the domain backing group must not render as a folder");
        assert!(h.query_all_by_label("Custom").next().is_some(), "the new gradient card should be visible at root");

        // The UI mirror remains serde-safe, while durable root membership is stored in the
        // gradient library's reserved owner group rather than depending on this transient state.
        let restored_ui: crate::state::UiState = serde_json::from_value(serde_json::to_value(&h.state().ui).unwrap()).unwrap();
        h.state_mut().ui = restored_ui;
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "the backing folder must remain hidden after restoring UiState");
        assert!(h.query_all_by_label("Custom").next().is_some(), "the root card must remain visible after restoring UiState");
        h.state_mut().ui.presets_ui = Default::default();
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").is_empty(), "root placement must come from the saved gradient library, not transient UI state");
        assert!(h.query_all_by_label("Custom").next().is_some(), "the card must stay at root even when picker layout state is reset");
    }

    #[test]
    fn dropping_a_gradient_before_the_first_group_moves_it_into_the_persistent_root_owner() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let headers = last_group_headers(&h.ctx, "gradients");
        assert!(headers.len() >= 2);
        let (source_group, _) = headers.first().unwrap();
        let source_key =
            h.state().session.presets.gradients.iter().find(|group| &group.name == source_group).and_then(|group| group.items.first()).unwrap().name.clone();
        let source = h.get_by_label(&source_key).rect().center();
        let first_header = headers.first().unwrap().1;
        let target = egui::pos2(first_header.center().x, first_header.top() - 5.0);

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

        let root_owner =
            h.state().session.presets.gradients.iter().find(|group| group.name == ROOT_ITEMS_GROUP).expect("the domain root owner is created on demand");
        assert_eq!(root_owner.items.first().map(|item| item.name.as_str()), Some(source_key.as_str()));
        assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));

        // Clearing picker-only state simulates an application restart: domain membership still
        // reconstructs the root item and the backing group remains invisible.
        h.state_mut().ui.presets_ui = Default::default();
        h.run_steps(3);
        assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));
        assert!(h.query_all_by_label(&source_key).next().is_some());
    }

    #[test]
    fn dropping_a_gradient_between_groups_or_after_the_last_group_moves_it_to_root() {
        for drop_position in ["gap_upper", "gap_lower", "root_end"] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                        gradients_panel(app, ui);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
            h.run_steps(3);

            let initial_headers = last_group_headers(&h.ctx, "gradients");
            assert!(initial_headers.len() >= 2);
            let (source_group, _) = initial_headers.first().unwrap();
            let source_key = h
                .state()
                .session
                .presets
                .gradients
                .iter()
                .find(|group| &group.name == source_group)
                .and_then(|group| {
                    group.items.iter().find(|item| {
                        !h.state()
                            .session
                            .presets
                            .gradients
                            .iter()
                            .find(|owner| owner.name == ROOT_ITEMS_GROUP)
                            .is_some_and(|owner| owner.items.iter().any(|root_item| root_item.name == item.name))
                    })
                })
                .unwrap()
                .name
                .clone();
            if h.query_all_by_label(&source_key).next().is_none() {
                let header = initial_headers.first().unwrap().1.center();
                h.event(Event::PointerMoved(header));
                h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                h.run_steps(1);
                h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                h.run_steps(2);
            }
            // Expanding a collapsed source group changes every later row's position. Always use
            // the freshly rendered headers for the drop coordinate.
            let headers = last_group_headers(&h.ctx, "gradients");
            let source = h.get_by_label(&source_key).rect().center();
            let target = match drop_position {
                "gap_upper" | "gap_lower" => {
                    let second_header = headers[1].1;
                    let offset = if drop_position == "gap_upper" { -9.0 } else { -3.0 };
                    egui::pos2(second_header.center().x, second_header.top() + offset)
                }
                "root_end" => {
                    let (last_group, last_header) = headers.last().unwrap();
                    let last_key = h
                        .state()
                        .session
                        .presets
                        .gradients
                        .iter()
                        .find(|group| &group.name == last_group)
                        .and_then(|group| group.items.last())
                        .unwrap()
                        .name
                        .clone();
                    if h.query_all_by_label(&last_key).next().is_none() {
                        let header = last_header.center();
                        h.event(Event::PointerMoved(header));
                        h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                        h.run_steps(1);
                        h.event(Event::PointerButton { pos: header, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                        h.run_steps(2);
                    }
                    let last_card = h.get_by_label(&last_key).rect();
                    // Drop well into the blank trailing area, not merely the narrow gap after the card.
                    egui::pos2(last_card.center().x, last_card.bottom() + 48.0)
                }
                _ => unreachable!(),
            };
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

            let root_owner = h
                .state()
                .session
                .presets
                .gradients
                .iter()
                .find(|group| group.name == ROOT_ITEMS_GROUP)
                .unwrap_or_else(|| panic!("root owner should be created for drop_position={drop_position}"));
            assert!(
                root_owner.items.iter().any(|item| item.name == source_key),
                "drop_position={drop_position}: expected {source_key:?} in root owner, library={:?}",
                h.state().session.presets.gradients
            );
            let root_order = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
            let moved = HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: source_key.clone() };
            let moved_index = root_order.iter().position(|entry| entry == &moved).expect("the mixed root sequence retains the moved card");
            match drop_position {
                "gap_upper" | "gap_lower" => {
                    let first_group_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group(headers[0].0.clone())).unwrap();
                    let second_group_index = root_order.iter().position(|entry| *entry == HierarchyOrderEntry::Group(headers[1].0.clone())).unwrap();
                    assert!(
                        first_group_index < moved_index && moved_index < second_group_index,
                        "drop_position={drop_position}: card must remain between the first two groups: {root_order:?}"
                    );
                }
                "root_end" => assert_eq!(root_order.last(), Some(&moved), "the card must remain at the root sequence's end: {root_order:?}"),
                _ => unreachable!(),
            }
            assert!(last_group_headers(&h.ctx, "gradients").iter().all(|(name, _)| name != ROOT_ITEMS_GROUP));
        }
    }

    #[test]
    fn dropping_a_gradient_at_the_bottom_of_a_scrolled_browser_appends_to_root() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(520.0, 420.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let scroll_to_bottom = |h: &mut Harness<'static, PhotocraftApp>| {
            let viewport = crate::pickers::asset_browser::last_browser_rect(&h.ctx, "gradients").unwrap();
            h.event(Event::PointerMoved(viewport.center()));
            h.event(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -2400.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(3);
        };
        scroll_to_bottom(&mut h);

        let (last_group, last_header) = last_group_headers(&h.ctx, "gradients").last().cloned().unwrap();
        let last_key =
            h.state().session.presets.gradients.iter().find(|group| group.name == last_group).and_then(|group| group.items.last()).unwrap().name.clone();
        if h.query_all_by_label(&last_key).next().is_none() {
            let point = egui::pos2(last_header.left() + 9.0, last_header.center().y);
            h.event(Event::PointerMoved(point));
            h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            scroll_to_bottom(&mut h);
        }
        let source = h.get_by_label(&last_key).rect().center();
        let viewport = crate::pickers::asset_browser::last_browser_rect(&h.ctx, "gradients").unwrap();
        let target = egui::pos2(viewport.center().x, viewport.bottom() - 8.0);
        assert!(viewport.contains(target));
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

        let root_owner = h
            .state()
            .session
            .presets
            .gradients
            .iter()
            .find(|group| group.name == ROOT_ITEMS_GROUP)
            .expect("dropping at the root end creates the durable owner group");
        assert!(root_owner.items.iter().any(|item| item.name == last_key));
        let root_order = &h.state().ui.presets_ui.hierarchy_order["gradients"][""];
        assert_eq!(root_order.last(), Some(&HierarchyOrderEntry::RootItem { group: ROOT_ITEMS_GROUP.into(), key: last_key }));
    }

    #[test]
    fn gradients_group_drop_before_card_persists_the_unified_order() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(app.session.presets.gradients.len() >= 2, "the fixture needs two gradient groups");
        let mut h = Harness::builder().with_size(vec2(360.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    gradients_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let headers = last_group_headers(&h.ctx, "gradients");
        assert!(headers.len() >= 2, "gradient group headers are visible");
        let (target_group, target_header) = &headers[0];
        let (moving_group, moving_header) = headers.last().unwrap();
        let target_item =
            h.state().session.presets.gradients.iter().find(|group| &group.name == target_group).and_then(|group| group.items.first()).unwrap().name.clone();
        let size = *h.state().ui.presets_ui.sizes.get("gradients").unwrap_or(&40.0);
        let origin = h.ctx.input(|input| input.viewport_rect().min);
        let target = egui::pos2(origin.x + 8.0 + 20.0 + size * 0.5 - 5.0, target_header.bottom() + 2.0 + size * 0.5);

        h.event(egui::Event::PointerMoved(moving_header.center()));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: moving_header.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(egui::Event::PointerMoved(moving_header.center() + (target - moving_header.center()) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);

        let state = &h.state().ui.presets_ui;
        assert_eq!(state.parent_of("gradients", moving_group), Some(target_group.as_str()));
        assert_eq!(state.hierarchy_order["gradients"][target_group].get(0), Some(&HierarchyOrderEntry::Group(moving_group.clone())));
        assert_eq!(state.hierarchy_order["gradients"][target_group].get(1), Some(&HierarchyOrderEntry::Item(target_item)));
        let names: Vec<String> = headers.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(state.display_order("gradients", &names).len(), names.len());
    }
}
