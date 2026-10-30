//! Pattern-specific previews and compact pattern-browser behavior.

use std::collections::BTreeSet;

use egui::{Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::pickers::asset_browser::{BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, browser};
use crate::pickers::asset_browser_interaction::canvas_drop_rect;
use crate::pickers::picker_footer::FooterOptions;
use crate::pickers::picker_shell::{MIN_THUMB, ShellOptions, picker_menu_button, picker_shell};
use crate::pickers::picker_state::PresetUi;
use crate::pickers::preset_commands::{generic, run};
use crate::pickers::preset_preview::{c32, cached_texture, sample_fingerprint};
use crate::theme::Tokens;

pub(crate) fn pattern_texture(ctx: &egui::Context, pat: &photocraft_doc::Pattern) -> TextureHandle {
    let (w, h) = (pat.width.max(1), pat.height.max(1));
    let fmt = pat.surface.format();
    // Fingerprint only the source samples the preview actually uses. Reading the full surface here
    // makes a tiny cached swatch proportional to the entire pattern's dimensions/edge count.
    const S: u32 = 64;
    let step = (w.max(h) as f32 / S as f32).max(1.0);
    let n = fmt.channels();
    let mut samples = Vec::with_capacity((S * S) as usize * n);
    for y in 0..S {
        for x in 0..S {
            let sx = ((x as f32 * step) as u32) % w;
            let sy = ((y as f32 * step) as u32) % h;
            for channel in 0..n {
                samples.push(pat.surface.sample_channel(sx as i32, sy as i32, channel));
            }
        }
    }
    let fingerprint = sample_fingerprint(pat.width, pat.height, fmt, &samples);
    cached_texture(ctx, ("pattern", pat.id.clone()), fingerprint, || {
        // Tiles smaller than the swatch repeat; larger ones are shown shrunk.
        let mut img = ColorImage::new([S as usize, S as usize], vec![Color32::TRANSPARENT; (S * S) as usize]);
        for (pixel, source) in img.pixels.iter_mut().zip(samples.chunks_exact(n)) {
            *pixel = c32(photocraft_raster::to_rgba(&fmt, source));
        }
        img
    })
}

/// Photoshop's pattern picker: a swatch of the library pattern `selected` (an id) that opens a grid
/// of every library pattern's swatch, each named in its tooltip. Returns the id of a newly picked
/// pattern.
pub(crate) fn pattern_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui, selected: &str) -> Option<String> {
    const SWATCH: f32 = 40.0;
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let pats = &app.session.patterns.items;
    let current = pats.iter().find(|p| p.id == selected);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let (r, resp) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    if let Some(p) = current {
        ui.painter().image(pattern_texture(&ctx, p).id(), r, uv, Color32::WHITE);
    }
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let name = current.map_or(String::new(), |p| p.display_name().to_string());
    let resp = resp.on_hover_text(&name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Patterns")));
    let groups = pattern_group_views(pats, &app.session.presets.pattern_groups);
    let picked = egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            // Preserve six swatches across, with a dedicated settings strip beside the browser.
            ui.set_width(6.0 * (SWATCH + 4.0) + 28.0);
            compact_pattern_browser(ui, pats, &groups, selected, "compact-pattern-picker", 240.0)
        })
        .and_then(|r| r.inner);
    apply_compact_pattern_actions(app, &ctx, "compact-pattern-picker", take_compact_pattern_actions(&ctx, "compact-pattern-picker"));
    picked.filter(|id| id != selected)
}

/// Build browser groups from the current library, including patterns not yet reconciled into the
/// saved group projection. Newly defined/imported patterns and reconstructed library ids must be
/// visible in compact pickers even if the main Patterns panel has not rendered first.
pub(crate) fn pattern_group_views(patterns: &[photocraft_doc::Pattern], groups: &[photocraft_engine::presets::Group<String>]) -> Vec<GroupView> {
    let mut assigned = BTreeSet::new();
    let mut views: Vec<GroupView> = groups
        .iter()
        .map(|g| GroupView {
            name: g.name.clone(),
            items: g
                .items
                .iter()
                .filter_map(|id| patterns.iter().find(|p| &p.id == id))
                .filter(|p| assigned.insert(p.id.clone()))
                .map(|p| ItemView { key: p.id.clone(), name: p.display_name().to_string(), group_path: Vec::new() })
                .collect(),
        })
        .collect();
    let loose: Vec<ItemView> = patterns
        .iter()
        .filter(|p| !assigned.contains(&p.id))
        .map(|p| ItemView { key: p.id.clone(), name: p.display_name().to_string(), group_path: Vec::new() })
        .collect();
    if !loose.is_empty() {
        if let Some(group) = views.iter_mut().find(|g| g.name == "Patterns") {
            group.items.extend(loose);
        } else {
            views.push(GroupView { name: "Patterns".into(), items: loose });
        }
    }
    views
}

/// Standard Patterns panel: pattern-specific projection, previews, and command translation around
/// the shared picker shell and browser.
pub(crate) fn patterns_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    photocraft_engine::presets::patterns::sync(&mut app.session);
    let lib = app.session.patterns.items.clone();
    let groups = pattern_group_views(&lib, &app.session.presets.pattern_groups);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let canvas = canvas_drop_rect(app);
    let ctx = ui.ctx().clone();
    let mut adapter = BrowserAdapter {
        panel: "patterns",
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
        item_context_menu: Some(crate::pickers::asset_browser::standard_item_context_menu),
        group_context_menu: Some(crate::pickers::asset_browser::standard_group_context_menu),
        nested_group_context_menu: None,
        paint_group: None,
        paint_item: &mut |ui, rect, group_index, item_index, _paint_state: ItemPaintState| {
            if let Some(pattern) =
                groups.get(group_index).and_then(|group| group.items.get(item_index)).and_then(|item| lib.iter().find(|pattern| pattern.id == item.key))
            {
                let texture = pattern_texture(&ctx, pattern);
                ui.painter().image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
        },
        paint_list_item: None,
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new pattern from the selection"))), ..Default::default() },
    );
    app.ui.presets_ui = st;
    for event in events {
        match &event {
            BrowserAction::Select(key) => {
                app.ui.presets_ui.selected.insert("patterns".into(), key.clone());
                run(app, "pattern.presets.select", json!({"pattern": key}));
            }
            BrowserAction::Activate(key) | BrowserAction::Drop(key, _) => {
                app.ui.presets_ui.selected.insert("patterns".into(), key.clone());
                run(app, "pattern.presets.apply", json!({"pattern": key}));
            }
            // Only the brush browser supports nested-group context actions.
            BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
            _ => {
                generic(app, "patterns", "pattern.presets", &event, json!({}));
            }
        }
    }
}

/// Contribute the Patterns-specific actions to the shared docking header menu.
pub(crate) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::pickers::picker_shell::search_toggle(ui, &mut app.ui.presets_ui, "patterns");
    ui.separator();
    if ui.button(tl!("Import")).clicked() {
        crate::preset_files_ui::import_patterns_dialog(app);
        ui.close();
    }
    let can_export = !app.session.patterns.items.is_empty() && !cfg!(target_arch = "wasm32");
    if ui.add_enabled(can_export, egui::Button::new(tl!("Export…"))).clicked() {
        ui.close();
        export_patterns_dialog(app);
    }
    ui.separator();
}

/// Export the complete pattern library through the native `.PAT` command and save dialog.
/// The engine's path-based exporter is unavailable on the web, so its menu entry is disabled there.
fn export_patterns_dialog(app: &mut PhotocraftApp) {
    let result = app.pick_save("Patterns.pat", |app, path| {
        let result = app.run("pattern.export", json!({"path": path}))?;
        let count = result.get("count").and_then(serde_json::Value::as_u64).unwrap_or(0);
        app.ui.status_error = false;
        app.ui.status = crate::i18n::fmt(
            &crate::i18n::trn(crate::i18n::current(), count, "Exported {n} pattern to {file}", "Exported {n} patterns to {file}"),
            &[("n", &count.to_string()), ("file", &crate::file_open::display_name(&path))],
        );
        Ok(result)
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

fn compact_context_menu_close_later(ui: &egui::Ui, panel: &str) {
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("compact-pattern-submenu-close", panel)), true));
}

pub(crate) fn compact_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    group_names: &[String],
    ev: &mut Vec<BrowserAction>,
) {
    if ui.button(tl!("Rename…")).clicked() {
        st.renaming = Some((panel.to_string(), item.key.clone(), item.name.clone()));
        compact_context_menu_close_later(ui, panel);
    } else if ui.button(tl!("Delete")).clicked() {
        ev.push(BrowserAction::Delete(item.key.clone()));
        compact_context_menu_close_later(ui, panel);
    } else {
        let mut move_to = None;
        ui.menu_button(tl!("Move to"), |ui| {
            for name in group_names.iter().filter(|name| name.as_str() != group.name) {
                if ui.button(name).clicked() {
                    move_to = Some(name.clone());
                }
            }
        });
        if let Some(name) = move_to {
            ev.push(BrowserAction::Move(item.key.clone(), name));
            compact_context_menu_close_later(ui, panel);
        }
    }
}

pub(crate) fn compact_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, ev: &mut Vec<BrowserAction>) {
    if ui.button(tl!("Rename Group…")).clicked() {
        st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone()));
        compact_context_menu_close_later(ui, panel);
    } else if ui.button(tl!("Delete Group")).clicked() {
        ev.push(BrowserAction::DeleteGroup(group.name.clone()));
        compact_context_menu_close_later(ui, panel);
    }
}

/// The Patterns library rendered by the shared browser without the standard panel shell. Used by
/// compact pickers (Brush Settings and fill dialogs) that need selection and context actions.
pub(crate) fn compact_pattern_browser(
    ui: &mut egui::Ui,
    pats: &[photocraft_doc::Pattern],
    groups: &[GroupView],
    selected: &str,
    panel: &'static str,
    max_h: f32,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    // Popup contents are rebuilt every frame. Keep the browser's expansion state in egui's
    // temporary data store, scoped to this popup instance and consumer.
    let state_id = egui::Id::new(("compact-pattern-browser", panel));
    let mut state = ui.data(|d| d.get_temp::<PresetUi>(state_id)).unwrap_or_else(|| {
        let mut state = PresetUi::default();
        // Compact pickers replace the old ungrouped swatch grid. Keep every group's items
        // immediately reachable on first open, while allowing users to collapse groups afterward.
        state.toggled.extend(groups.iter().skip(1).map(|g| format!("{panel}/{}", crate::pickers::picker_state::escape_hierarchy_key_part(&g.name))));
        state
    });
    state.selected.insert(panel.into(), selected.to_string());
    state.sizes.entry(panel.into()).or_insert(40.0);
    let browser_h = max_h.min(ui.available_height());
    let parent_menu_id = egui::containers::menu::MenuState::from_ui(ui, |_, stack| stack.id);
    let close_pending_id = egui::Id::new(("compact-pattern-submenu-close", panel));
    if ui.ctx().data(|data| data.get_temp::<bool>(close_pending_id).unwrap_or(false)) {
        // Defer this until the next frame: egui checks whether a submenu is open after
        // rendering the parent popup. Closing it in the selection frame makes that same
        // click look like an outside click to the parent popup.
        egui::containers::menu::MenuState::from_id(ui.ctx(), parent_menu_id, |state| state.open_item = None);
        ui.ctx().data_mut(|data| data.insert_temp(close_pending_id, false));
    }
    let mut adapter = BrowserAdapter {
        panel,
        groups,
        options: BrowserOptions {
            canvas: Rect::NOTHING,
            flush: false,
            center_grid: false,
            grid_indent: 8.0,
            all_groups_open: false,
            compact_context_menus: true,
            item_size: None,
            list_item_height: None,
            domain_hierarchy_actions: false,
        },
        // Compact pickers use the same scoped drag contract as full panels. Selection closes the
        // popup; dragging items between groups or reordering/nesting group headers keeps it open.
        capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
        item_context_menu: Some(compact_item_context_menu),
        group_context_menu: Some(compact_group_context_menu),
        nested_group_context_menu: None,
        paint_group: None,
        paint_item: &mut |ui, rect, group_index, item_index, paint_state: ItemPaintState| {
            if let Some(pattern) =
                groups.get(group_index).and_then(|group| group.items.get(item_index)).and_then(|item| pats.iter().find(|pattern| pattern.id == item.key))
            {
                ui.painter().image(pattern_texture(&ctx, pattern).id(), rect, uv, Color32::WHITE);
                if let ItemPaintState::Item { selected } = paint_state {
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.field_border }),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        },
        paint_list_item: None,
    };
    let mut picked = None;
    let mut action_events = Vec::new();
    ui.horizontal(|ui| {
        let menu_width = 24.0;
        let browser_width = (ui.available_width() - menu_width - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(vec2(browser_width, browser_h), egui::Layout::top_down(egui::Align::Min), |browser_ui| {
            for event in browser(browser_ui, &mut state, &mut adapter, browser_h) {
                match event {
                    BrowserAction::Select(id) | BrowserAction::Activate(id) => {
                        if id != selected {
                            picked = Some(id);
                        }
                        browser_ui.close();
                    }
                    BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
                    action => action_events.push(action),
                }
            }
        });
        ui.allocate_ui_with_layout(vec2(menu_width, browser_h), egui::Layout::top_down(egui::Align::Min), |menu_ui| {
            let mut menu = picker_menu_button(menu_ui, "Panel menu");
            // Nudge the submenu anchor into the browser by a few pixels instead of leaving a gap.
            menu.interact_rect = menu.interact_rect.translate(egui::vec2(-3.0, 0.0));
            // Keep this as a real submenu so clicks inside it don't dismiss the picker popup.
            // egui's default submenu opens on hover, so drive its open state from the gear click.
            let submenu_id = egui::containers::menu::SubMenu::id_from_widget_id(menu.id);
            let mut submenu_open = egui::containers::menu::MenuState::from_ui(menu_ui, |state, _| state.open_item == Some(submenu_id));
            if menu.clicked() {
                submenu_open = !submenu_open;
                egui::containers::menu::MenuState::from_ui(menu_ui, |state, _| {
                    state.open_item = submenu_open.then_some(submenu_id);
                });
            }
            // `SubMenu::show` normally closes when the pointer moves to another entry in its
            // parent menu. This is a click-toggled settings panel, not a hover submenu: keep it
            // open while the pointer moves around, and dismiss it only on a fresh click outside
            // the submenu (or another click on the gear button).
            let submenu_rect = menu_ui.ctx().memory(|memory| memory.area_rect(submenu_id));
            let outside_press = submenu_open
                .then(|| {
                    menu_ui.ctx().input(|input| input.pointer.button_pressed(egui::PointerButton::Primary).then(|| input.pointer.interact_pos()).flatten())
                })
                .flatten();
            let dismiss_on_outside_press = outside_press.is_some_and(|pos| !submenu_rect.is_some_and(|rect| rect.contains(pos)) && !menu.rect.contains(pos));
            if dismiss_on_outside_press {
                submenu_open = false;
                egui::containers::menu::MenuState::from_ui(menu_ui, |state, _| {
                    state.open_item = None;
                });
            }
            if submenu_open {
                egui::containers::menu::SubMenu::new().show(menu_ui, &menu, |ui| {
                    let selected_size = *state.sizes.get(panel).unwrap_or(&40.0);
                    for (label, size) in [(tl!("Small Thumbnail"), 28.0), (tl!("Large Thumbnail"), 40.0), (tl!("List"), MIN_THUMB)] {
                        let selected = (selected_size - size).abs() < 0.1;
                        let row_label = format!("{}  {label}", if selected { "✓" } else { " " });
                        let clicked = ui.button(row_label).clicked();
                        if clicked {
                            state.sizes.insert(panel.into(), size);
                            // Close only this submenu on the next frame; `ui.close()` would also
                            // close the picker, and clearing MenuState now makes egui misclassify
                            // this selection click as outside the parent popup.
                            ui.ctx().data_mut(|data| data.insert_temp(close_pending_id, true));
                        }
                    }
                });
                // `SubMenu::show` clears `open_item` on hover-leave. Restore it unless this
                // frame explicitly requested a close, so pointer movement alone is harmless.
                let close_pending = menu_ui.ctx().data(|data| data.get_temp::<bool>(close_pending_id).unwrap_or(false));
                if !close_pending {
                    egui::containers::menu::MenuState::from_ui(menu_ui, |state, _| {
                        state.open_item = Some(submenu_id);
                    });
                }
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(state_id, state));
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("compact-pattern-actions", panel)), action_events));
    picked
}

/// Take generic actions emitted by the most recently drawn compact pattern browser instance.
pub(crate) fn take_compact_pattern_actions(ctx: &egui::Context, panel: &'static str) -> Vec<BrowserAction> {
    ctx.data_mut(|data| data.remove_temp(egui::Id::new(("compact-pattern-actions", panel))).unwrap_or_default())
}

/// Dispatch compact-browser actions through the same pattern command handler as the full Patterns panel.
pub(crate) fn apply_compact_pattern_actions(app: &mut PhotocraftApp, ctx: &egui::Context, panel: &'static str, events: Vec<BrowserAction>) {
    for event in events {
        // The shared preset command translator clears the panel selection after every delete.
        // Compact consumers keep their own selection key, so deleting an unrelated pattern must
        // not clear the Brush Texture picker's current pattern.
        let selected_before = if matches!(event, BrowserAction::Delete(_)) { app.ui.presets_ui.selected.get(panel).cloned() } else { None };
        if generic(app, panel, "pattern.presets", &event, json!({})) {
            if let (BrowserAction::Delete(key), Some(selected)) = (&event, selected_before)
                && selected != *key
            {
                app.ui.presets_ui.selected.insert(panel.into(), selected);
            }
            let state_id = egui::Id::new(("compact-pattern-browser", panel));
            ctx.data_mut(|data| {
                if let Some(mut state) = data.get_temp::<PresetUi>(state_id) {
                    match &event {
                        BrowserAction::RenameGroup(from, to) => state.rename_group(panel, from, to),
                        BrowserAction::DeleteGroup(name) => state.delete_group(panel, name),
                        BrowserAction::Delete(key) if state.selected.get(panel) == Some(key) => {
                            state.selected.remove(panel);
                        }
                        _ => {}
                    }
                    data.insert_temp(state_id, state);
                }
            });
        }
    }
}
