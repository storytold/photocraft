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
use crate::pickers::preset_commands::{generic, run};
use crate::pickers::preset_preview::c32;

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
    let groups: Vec<GroupView> = library
        .iter()
        .map(|group| GroupView {
            name: group.name.clone(),
            items: group.items.iter().map(|item| ItemView { key: item.name.clone(), name: item.name.clone(), folder: Vec::new() }).collect(),
        })
        .collect();
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let canvas = canvas_drop_rect(app);
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
        },
        capabilities: BrowserCapabilities::default(),
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        folder_context_menu: None,
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
    let events = picker_shell(ui, &mut state, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new gradient"))), ..Default::default() });
    app.ui.presets_ui = state;

    for event in events {
        match &event {
            BrowserAction::Select(key) => {
                app.ui.presets_ui.selected.insert("gradients".into(), key.clone());
                run(app, "gradient.presets.select", json!({"preset": key}));
            }
            BrowserAction::Activate(key) | BrowserAction::Drop(key, _) => {
                app.ui.presets_ui.selected.insert("gradients".into(), key.clone());
                run(app, "gradient.presets.apply", json!({"preset": key}));
            }
            // Only the brush browser supports nested-folder context actions.
            BrowserAction::RenameFolder { .. } | BrowserAction::DeleteFolder { .. } => {}
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
