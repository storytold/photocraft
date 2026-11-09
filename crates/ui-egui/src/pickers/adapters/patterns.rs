//! Pattern-specific previews and compact pattern-browser behavior.

use std::collections::BTreeSet;

use egui::{Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::pickers::browser::asset_browser::{
    BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, begin_item_rename, browser, delete_item_actions,
    item_context_targets, move_item_actions,
};
use crate::pickers::browser::browser_actions::{generic, run};
use crate::pickers::browser::browser_interaction::canvas_drop_rect;
use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity, PickerProjectionKeys, ProjectionNodeRef};
use crate::pickers::footer::FooterOptions;
use crate::pickers::preset_previews::{c32, cached_texture, sample_fingerprint};
use crate::pickers::shell::{ShellOptions, picker_shell};
use crate::pickers::state::PresetUi;
use crate::theme::Tokens;

fn pattern_projection_keys(groups: &[GroupView]) -> PickerProjectionKeys {
    let mut keys = PickerProjectionKeys::default();
    for (group_index, group) in groups.iter().enumerate() {
        keys.insert(
            ProjectionNodeRef::Group(group_index),
            PickerIdentityKey::from_parts("pattern-group", [group.name.as_str()]),
            PickerNodeIdentity::Group(group.name.clone()),
        );
        for (item_index, item) in group.items.iter().enumerate() {
            keys.insert(
                ProjectionNodeRef::Item { group_index, item_index },
                PickerIdentityKey::from_parts("pattern-id", [item.key.as_str()]),
                PickerNodeIdentity::Item { group: group.name.clone(), key: item.key.clone() },
            );
        }
    }
    keys
}

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
            // Preserve six swatches across; picker actions now live in the owning panel menu.
            ui.set_width(6.0 * (SWATCH + 4.0));
            compact_pattern_browser(ui, pats, &groups, selected, "compact-pattern-picker", 240.0, &mut app.ui.presets_ui)
        })
        .and_then(|r| r.inner);
    apply_compact_pattern_actions(app, "compact-pattern-picker", take_compact_pattern_actions(&ctx, "compact-pattern-picker"));
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
            root_only: false,
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
            views.push(GroupView { name: "Patterns".into(), root_only: false, items: loose });
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
    let projection_keys = pattern_projection_keys(&groups);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let canvas = canvas_drop_rect(app);
    let ctx = ui.ctx().clone();
    let events = {
        let mut commit_item_move = |source_group: &str, key: &str, destination_group: &str, index: usize| {
            run(app, "pattern.presets.edit", json!({"action": "move", "preset": key, "group": source_group, "to": destination_group, "index": index}))
                .map(|_| ())
                .ok_or_else(|| "The Pattern library rejected the item move.".to_owned())
        };
        let mut adapter = BrowserAdapter {
            panel: "patterns",
            groups: &groups,
            projection_keys: Some(&projection_keys),
            options: BrowserOptions {
                canvas,
                flush: false,
                center_grid: false,
                all_groups_open: false,
                compact_context_menus: false,
                domain_hierarchy_actions: false,
                root_item_group: None,
            },
            // Cross-group membership uses the Pattern domain command; sibling/group order is saved
            // in the canonical picker layout state.
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(crate::pickers::browser::asset_browser::standard_item_context_menu),
            group_context_menu: Some(crate::pickers::browser::asset_browser::standard_group_context_menu),
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            rekey_hierarchy_after_move: None,
            hierarchy_item_move: Some(&mut commit_item_move),
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
        picker_shell(
            ui,
            &mut st,
            &mut adapter,
            ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new pattern from the selection"))), ..Default::default() },
        )
    };
    app.ui.presets_ui = st;
    for event in events {
        match &event {
            BrowserAction::SelectItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("patterns", group, group_path, key);
                run(app, "pattern.presets.select", json!({"pattern": key}));
            }
            BrowserAction::Drop(key, _) => {
                app.ui.presets_ui.set_selected("patterns", key);
                run(app, "pattern.presets.apply", json!({"pattern": key}));
            }
            BrowserAction::ActivateItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("patterns", group, group_path, key);
                run(app, "pattern.presets.apply", json!({"pattern": key}));
            }
            BrowserAction::Import => crate::preset_files_ui::import_patterns_dialog(app),
            BrowserAction::ExportSelected(names) => export_selected_patterns_dialog(app, &names),
            _ => {
                generic(app, "patterns", "pattern.presets", &event, json!({}));
            }
        }
    }
}

/// Contribute the Patterns-specific actions to the shared docking header menu.
pub(crate) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if crate::menu_keys::menu_item_button(ui, tl!("Import"), true).clicked() {
        crate::preset_files_ui::import_patterns_dialog(app);
        ui.close();
    }
    let can_export = !app.session.patterns.items.is_empty() && !cfg!(target_arch = "wasm32");
    if crate::menu_keys::menu_item_button(ui, tl!("Export…"), can_export).clicked() {
        ui.close();
        export_patterns_dialog(app);
    }
}

/// Export the complete pattern library through the native `.PAT` command and save dialog.
/// The engine's path-based exporter is unavailable on the web, so its menu entry is disabled there.
fn export_patterns_dialog(app: &mut PhotocraftApp) {
    export_patterns(app, None);
}

fn export_selected_patterns_dialog(app: &mut PhotocraftApp, names: &[String]) {
    export_patterns(app, Some(names.to_vec()));
}

fn export_patterns(app: &mut PhotocraftApp, selected: Option<Vec<String>>) {
    let params = selected.map_or_else(|| json!({}), |patterns| json!({"patterns": patterns}));
    let result = app.pick_save("Patterns.pat", |app, path| {
        let mut params = params;
        params["path"] = json!(path);
        let result = app.run("pattern.export", params)?;
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
    args: crate::pickers::browser::asset_browser::ItemContextMenuArgs<'_>,
    ev: &mut Vec<BrowserAction>,
) {
    let crate::pickers::browser::asset_browser::ItemContextMenuArgs { panel, item, group, group_names, selected_items } = args;
    let targets = item_context_targets(item, group, selected_items);
    let count = targets.len();
    if crate::menu_keys::menu_item_button(ui, tl!("Rename…"), true).clicked() {
        begin_item_rename(st, panel, &targets);
        compact_context_menu_close_later(ui, panel);
    } else if crate::menu_keys::menu_item_button(ui, &if count > 1 { format!("{} ({count})", tl!("Delete")) } else { tl!("Delete").to_string() }, true)
        .clicked()
    {
        ev.extend(delete_item_actions(&targets));
        compact_context_menu_close_later(ui, panel);
    } else {
        let mut move_to = None;
        ui.menu_button(tl!("Move to"), |ui| {
            for name in group_names {
                let already_in_target = targets.iter().all(|(source_group, _)| source_group == name);
                if !already_in_target && crate::menu_keys::menu_item_button(ui, name, true).clicked() {
                    move_to = Some(name.clone());
                }
            }
        });
        if let Some(name) = move_to {
            ev.extend(move_item_actions(&targets, &name));
            compact_context_menu_close_later(ui, panel);
        }
    }
}

pub(crate) fn compact_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, ev: &mut Vec<BrowserAction>) {
    if crate::menu_keys::menu_item_button(ui, tl!("Rename Group…"), true).clicked() {
        st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone()));
        compact_context_menu_close_later(ui, panel);
    } else if crate::menu_keys::menu_item_button(ui, tl!("Delete Group"), true).clicked() {
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
    state: &mut PresetUi,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    // Popup contents are rebuilt every frame; use the app's persisted picker preferences rather
    // than a temporary popup-local state so display mode and Wrap List survive a restart.
    if state.initialized_browsers.insert(panel.to_owned()) {
        // Compact pickers replace the old ungrouped swatch grid. Keep every group's items
        // immediately reachable on first open, while allowing users to collapse groups afterward.
        state.toggled.extend(groups.iter().skip(1).map(|g| format!("{panel}/{}", crate::pickers::state::escape_hierarchy_key_part(&g.name))));
    }
    state.set_selected(panel, selected);
    let browser_h = max_h.min(ui.available_height());
    let projection_keys = pattern_projection_keys(groups);
    let mut adapter = BrowserAdapter {
        panel,
        groups,
        projection_keys: Some(&projection_keys),
        options: BrowserOptions {
            canvas: Rect::NOTHING,
            flush: false,
            center_grid: false,
            all_groups_open: false,
            compact_context_menus: true,
            domain_hierarchy_actions: false,
            root_item_group: None,
        },
        // Compact pickers have no domain-command context for committing item or group moves.
        // Keep selection/activation available, but do not expose drag semantics that could only
        // mutate this popup's transient PresetUi projection.
        capabilities: BrowserCapabilities { item_drag: false, item_drop: false, group_drag: false, selection: true, activation: true },
        item_context_menu: Some(compact_item_context_menu),
        group_context_menu: Some(compact_group_context_menu),
        nested_group_context_menu: None,
        canonical_hierarchy_move: None,
        rekey_hierarchy_after_move: None,
        hierarchy_item_move: None,
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
    for event in browser(ui, state, &mut adapter, browser_h) {
        match event {
            BrowserAction::SelectItem { group, group_path, key } | BrowserAction::ActivateItem { group, group_path, key } => {
                state.set_selected_item(panel, &group, &group_path, &key);
                if key != selected {
                    picked = Some(key);
                }
                ui.close();
            }
            BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
            action => action_events.push(action),
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("compact-pattern-actions", panel)), action_events));
    picked
}

/// Take generic actions emitted by the most recently drawn compact pattern browser instance.
pub(crate) fn take_compact_pattern_actions(ctx: &egui::Context, panel: &'static str) -> Vec<BrowserAction> {
    ctx.data_mut(|data| data.remove_temp(egui::Id::new(("compact-pattern-actions", panel))).unwrap_or_default())
}

/// Dispatch compact-browser actions through the same pattern command handler as the full Patterns panel.
pub(crate) fn apply_compact_pattern_actions(app: &mut PhotocraftApp, panel: &'static str, events: Vec<BrowserAction>) {
    for event in events {
        // The shared preset command translator clears the panel selection after every delete.
        // Compact consumers keep their own selection key, so deleting an unrelated pattern must
        // not clear the Brush Texture picker's current pattern.
        let deleted_key = match &event {
            BrowserAction::DeleteItem { key, .. } => Some(key.as_str()),
            _ => None,
        };
        let selected_before = deleted_key.and_then(|_| app.ui.presets_ui.selected.get(panel).cloned());
        if generic(app, panel, "pattern.presets", &event, json!({})) {
            if let (Some(key), Some(selected)) = (deleted_key, selected_before)
                && selected != key
            {
                app.ui.presets_ui.selected.insert(panel.into(), selected);
            }
        }
    }
}
