//! Window › Gradients, Patterns (tabs of the Color card, as in Photoshop's Essentials workspace),
//! Styles, Shapes, Tool Presets and Clone Source (floating panels).
//!
//! The preset panels share one browser: nested groups with disclosure triangles, a thumbnail grid (or a
//! list when the size slider is at its minimum), a rename field, a context menu (Rename, Delete,
//! Move to group) and the Photoshop footer (size slider · New Group · New · Delete). Clicking a
//! preset selects it, double-clicking or dragging it onto the canvas applies it. Every action is
//! an engine command (`gradient.presets.*`, `pattern.presets.*`, `style.presets.*`,
//! `shape.presets.*`, `tool.presets.*`, `cloneSource.*`); this module only draws and dispatches.

use egui::{Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, pos2, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
#[cfg(test)]
use crate::pickers::asset_browser::browser;
#[cfg(test)]
use crate::pickers::asset_browser::last_browser_rect;
#[cfg(test)]
use crate::pickers::asset_browser::last_group_headers;
use crate::pickers::asset_browser::{BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView};
use crate::pickers::asset_browser::{standard_group_context_menu, standard_item_context_menu};
#[cfg(test)]
use crate::pickers::asset_browser_hierarchy::hierarchy_tree;
pub use crate::pickers::asset_browser_hierarchy::{HierarchyGroup, HierarchyNode};
use crate::pickers::asset_browser_interaction::canvas_drop_rect;
#[cfg(test)]
use crate::pickers::asset_browser_interaction::{AssetDrag, asset_browser_header_payload, header_drop_before};
use crate::pickers::picker_footer::FooterOptions;
#[cfg(test)]
use crate::pickers::picker_shell::{CustomShellOptions, picker_shell_custom};
use crate::pickers::picker_shell::{ShellOptions, picker_shell};
use crate::state::Tool;
use crate::theme::Tokens;

pub use crate::pickers::picker_state::PresetUi;
use crate::pickers::preset_commands::{generic, run};
use crate::pickers::preset_preview::{cached_texture, content_fingerprint};

const PANELS: [&str; 7] = [
    "window.panel.gradients",
    "window.panel.patterns",
    "window.panel.swatches",
    "window.panel.styles",
    "window.panel.shapes",
    "window.panel.toolPresets",
    "window.panel.cloneSource",
];

/// Standard preset pickers are rendered by the shared docking surface.
pub fn shared_docking(id: &str) -> bool {
    PANELS.contains(&id)
}

// ------------------------------------------------------------------ textures

/// Draw a gradient swatch.
pub fn paint_gradient(ui: &egui::Ui, rect: Rect, stops: &[(f32, [f32; 4])]) {
    crate::pickers::gradients_browser::paint_gradient(ui, rect, stops);
}

pub(crate) fn style_texture(app: &PhotocraftApp, ctx: &egui::Context, st: &photocraft_engine::presets::styles::StylePreset) -> TextureHandle {
    // Effects carry the actual rendered appearance; counts alone are not a valid revision.
    let serialized = serde_json::to_string(st).unwrap_or_else(|_| format!("{:?}", st));
    let fingerprint = content_fingerprint(&serialized);
    cached_texture(ctx, ("style", st.name.clone()), fingerprint, || {
        const S: u32 = 64;
        let px = photocraft_engine::presets::styles::thumbnail(&app.session, st, S);
        ColorImage::from_rgba_unmultiplied([S as usize, S as usize], &px)
    })
}

fn shape_texture(ctx: &egui::Context, sh: &photocraft_engine::presets::shapes::ShapePreset) -> TextureHandle {
    let serialized = serde_json::to_string(&sh.path).unwrap_or_else(|_| format!("{:?}", sh.path));
    cached_texture(ctx, ("shape", sh.name.clone()), content_fingerprint(&serialized), || {
        const S: u32 = 64;
        let cov = photocraft_engine::presets::shapes::thumbnail(&sh.path, S);
        ColorImage::new([S as usize, S as usize], cov.iter().map(|a| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect())
    })
}

// ------------------------------------------------------------------ the browser

fn empty(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(s).color(t.text_faint).size(11.5));
}

/// Draw the standard Patterns panel.
pub fn patterns_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::pickers::pattern_browser::patterns_panel(app, ui);
}

// ------------------------------------------------------------------ Gradients

pub fn gradients_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::pickers::gradients_browser::gradients_panel(app, ui);
}

// ------------------------------------------------------------------ floating panels

// ------------------------------------------------------------------ Styles

pub fn styles_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let src = app.session.presets.styles.clone();
    let groups: Vec<GroupView> = src
        .iter()
        .map(|g| GroupView {
            name: g.name.clone(),
            root_only: false,
            items: g.items.iter().map(|i| ItemView { key: i.name.clone(), name: i.name.clone(), group_path: Vec::new() }).collect(),
        })
        .collect();
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut texes: Vec<Vec<TextureHandle>> = Vec::new();
    for g in &src {
        texes.push(g.items.iter().map(|s| style_texture(app, &ctx, s)).collect());
    }
    let mut commit_item_move = |source_group: &str, key: &str, destination_group: &str, index: usize| {
        run(app, "style.presets.edit", json!({"action": "move", "preset": key, "group": source_group, "to": destination_group, "index": index}))
            .map(|_| ())
            .ok_or_else(|| "The Style library rejected the item move.".to_owned())
    };
    let mut adapter = BrowserAdapter {
        panel: "styles",
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
            root_item_group: None,
        },
        // Cross-group membership uses the Style domain command; sibling/group order is saved
        // in the canonical picker layout state.
        capabilities: BrowserCapabilities { canonical_hierarchy_drag: true, ..BrowserCapabilities::default() },
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        nested_group_context_menu: None,
        canonical_hierarchy_move: None,
        hierarchy_item_move: Some(&mut commit_item_move),
        paint_group: None,
        paint_item: &mut |ui, rect, group_index, item_index, _paint_state: ItemPaintState| {
            ui.painter().rect_filled(rect, 2.0, t.field);
            ui.painter().image(texes[group_index][item_index].id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        },
        paint_list_item: None,
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new style from the selected layer"))), ..Default::default() },
    );
    drop(adapter);
    drop(commit_item_move);
    app.ui.presets_ui = st;
    let shift = ui.input(|i| i.modifiers.shift);
    for e in events {
        match &e {
            BrowserAction::Select(k) | BrowserAction::Activate(k) => {
                app.ui.presets_ui.set_selected("styles", k);
                // Clicking applies to the selected layers (⇧ adds to their effects).
                run(app, "style.presets.apply", json!({"preset": k, "add": shift}));
            }
            BrowserAction::SelectItem { group, group_path, key } | BrowserAction::ActivateItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("styles", group, group_path, key);
                run(app, "style.presets.apply", json!({"preset": key, "add": shift}));
            }
            BrowserAction::Drop(k, pos) => {
                // Dropping on the canvas styles the layer under the pointer's selection: the active layer.
                let _ = pos;
                run(app, "style.presets.apply", json!({"preset": k}));
            }
            // Only the brush browser supports nested-group context actions.
            BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
            _ => {
                generic(app, "styles", "style.presets", &e, json!({"name": "Style"}));
            }
        }
    }
}

// ------------------------------------------------------------------ Shapes

pub fn shapes_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let src = photocraft_engine::presets::shapes::all_groups(&app.session);
    let groups: Vec<GroupView> = src
        .iter()
        .map(|g| GroupView {
            name: g.name.clone(),
            root_only: false,
            items: g.items.iter().map(|i| ItemView { key: i.name.clone(), name: i.name.clone(), group_path: Vec::new() }).collect(),
        })
        .collect();
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut commit_item_move = |source_group: &str, key: &str, destination_group: &str, index: usize| {
        run(app, "shape.presets.edit", json!({"action": "move", "preset": key, "group": source_group, "to": destination_group, "index": index}))
            .map(|_| ())
            .ok_or_else(|| "The Shape library rejected the item move.".to_owned())
    };
    let mut adapter = BrowserAdapter {
        panel: "shapes",
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
            root_item_group: None,
        },
        // The projection also contains the derived `Custom Shapes` group, whose items live in
        // edit_state.custom_shapes and have no move command. Until that source supports the same
        // canonical move contract, keep drag semantics disabled for this mixed-source adapter.
        capabilities: BrowserCapabilities {
            item_drag: false,
            item_drop: false,
            group_drag: false,
            selection: true,
            activation: true,
            canonical_hierarchy_drag: false,
        },
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        nested_group_context_menu: None,
        canonical_hierarchy_move: None,
        hierarchy_item_move: Some(&mut commit_item_move),
        paint_group: None,
        paint_item: &mut |ui, rect, group_index, item_index, _paint_state: ItemPaintState| {
            let texture = shape_texture(&ctx, &src[group_index].items[item_index]);
            ui.painter().image(texture.id(), rect.shrink(2.0), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), t.text);
        },
        paint_list_item: None,
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new shape from the current path"))), ..Default::default() },
    );
    drop(adapter);
    drop(commit_item_move);
    app.ui.presets_ui = st;
    let fill = shape_fill(app);
    for e in events {
        match &e {
            BrowserAction::Select(k) => {
                // Selecting a shape picks it for the Custom Shape tool (and switches to it).
                app.ui.presets_ui.set_selected("shapes", k);
                app.ui.presets_ui.custom_shape = k.clone();
                app.ui.tool = Tool::CustomShape;
            }
            BrowserAction::SelectItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("shapes", group, group_path, key);
                app.ui.presets_ui.custom_shape = key.clone();
                app.ui.tool = Tool::CustomShape;
            }
            BrowserAction::Activate(k) => {
                app.ui.presets_ui.custom_shape = k.clone();
                run(app, "shape.presets.place", json!({"preset": k, "fill": fill}));
            }
            BrowserAction::ActivateItem { group, group_path, key } => {
                app.ui.presets_ui.set_selected_item("shapes", group, group_path, key);
                app.ui.presets_ui.custom_shape = key.clone();
                run(app, "shape.presets.place", json!({"preset": key, "fill": fill}));
            }
            BrowserAction::Drop(k, pos) => {
                app.ui.presets_ui.custom_shape = k.clone();
                let mut p = json!({"preset": k, "fill": fill});
                if let (Some(c), Some(d)) = (crate::canvas::doc_point_at(app, *pos), app.session.active()) {
                    let side = (d.doc.size.width.min(d.doc.size.height) as f64 * 0.3).max(8.0);
                    p["rect"] = json!([c[0] - side / 2.0, c[1] - side / 2.0, side, side]);
                }
                run(app, "shape.presets.place", p);
            }
            // Only the brush browser supports nested-group context actions.
            BrowserAction::RenameNestedGroup { .. } | BrowserAction::DeleteNestedGroup { .. } => {}
            _ => {
                generic(app, "shapes", "shape.presets", &e, json!({"name": "Shape"}));
            }
        }
    }
}

fn shape_fill(app: &PhotocraftApp) -> Value {
    let f = app.session.tools.foreground;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if app.ui.tool_options.shape_fill { json!(format!("#{:02x}{:02x}{:02x}", q(f[0]), q(f[1]), q(f[2]))) } else { Value::Null }
}

/// Options-bar shape picker for the Custom Shape tool.
pub fn shape_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Shape:")).color(t.text_dim).size(12.0));
    let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
    let cur = app.ui.presets_ui.shape().to_string();
    let ctx = ui.ctx().clone();
    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    ui.painter().rect_filled(r, 3.0, t.field);
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    if let Some(sh) = groups.iter().flat_map(|g| g.items.iter()).find(|s| s.name == cur) {
        let tex = shape_texture(&ctx, sh);
        ui.painter().image(
            tex.id(),
            Rect::from_center_size(r.center() - vec2(5.0, 0.0), vec2(18.0, 18.0)),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            t.text,
        );
    }
    crate::icons::paint(ui, Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_dim);
    let resp = resp.on_hover_text(&cur);
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_width(232.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for g in &groups {
                ui.label(egui::RichText::new(&g.name).color(t.text_dim).size(11.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                    for sh in &g.items {
                        let (cr, cresp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                        let on = sh.name == cur;
                        ui.painter().rect_filled(
                            cr,
                            2.0,
                            if on {
                                t.accent_soft
                            } else if cresp.hovered() {
                                t.hover
                            } else {
                                t.field
                            },
                        );
                        let tex = shape_texture(&ctx, sh);
                        ui.painter().image(tex.id(), cr.shrink(4.0), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), t.text);
                        if cresp.on_hover_text(&sh.name).clicked() {
                            app.ui.presets_ui.custom_shape = sh.name.clone();
                            ui.close();
                        }
                    }
                });
            }
        });
    });
}

/// Custom Shape tool drag: places the selected shape in the dragged rect (⇧ keeps proportions).
pub fn finish_custom_shape(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool, fill: Value, stroke: Value) {
    let name = app.ui.presets_ui.shape().to_string();
    let r = run(app, "shape.presets.place", json!({"preset": name, "rect": rect, "keepAspect": keep, "fill": fill, "stroke": stroke}));
    if r.is_none() && !photocraft_engine::presets::shapes::all_groups(&app.session).iter().any(|g| g.items.iter().any(|s| s.name == name)) {
        let template = "No custom shape \"{name}\": pick one in the options bar or Window › Shapes";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    }
}

// ------------------------------------------------------------------ Tool Presets

fn tool_id(t: Tool) -> String {
    let s = format!("{t:?}");
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

/// Apply a tool preset: the engine sets the brush/colour, the shell switches tool and options.
pub fn select_tool_preset(app: &mut PhotocraftApp, name: &str) {
    let Some(r) = run(app, "tool.presets.select", json!({"preset": name})) else { return };
    if let Some(tool) = r["tool"].as_str().and_then(Tool::from_name) {
        crate::tool_memory::select(app, tool);
        // Each tool keeps its own brush (#218), so switch it in before the preset's brush lands
        // on it, and re-apply the brush the command just set for the old tool.
        crate::paint_mouse::sync_tool_brush(app);
        if let Some(b) = r["options"].get("brush").filter(|b| b.is_object()) {
            let _ = app.run("tools.setBrush", json!({"brush": b}));
        }
    }
    if let Some(o) = r["options"].get("toolOptions").and_then(Value::as_object) {
        let mut cur = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
        if let Some(c) = cur.as_object_mut() {
            for (k, v) in o {
                c.insert(k.clone(), v.clone());
            }
        }
        if let Ok(opts) = serde_json::from_value(cur) {
            app.ui.tool_options = opts;
        }
    }
}

/// The `tool.presets.new` params for the + button: `tool`'s own options only, so choosing the
/// preset later leaves the other tools' settings alone.
fn new_tool_preset_params(app: &PhotocraftApp, tool: Tool) -> Value {
    let all = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
    let own: serde_json::Map<String, Value> =
        crate::tool_reset::option_keys(tool).iter().filter_map(|k| Some(((*k).to_string(), all.get(*k)?.clone()))).collect();
    json!({"name": tool.label(), "tool": tool_id(tool), "options": {"toolOptions": own}})
}

pub fn tool_presets_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let only = app.ui.presets_ui.current_tool_only;
    let cur = app.ui.tool;
    let items: Vec<(String, String)> =
        app.session.presets.tool_presets.iter().filter(|p| !only || Tool::from_name(&p.tool) == Some(cur)).map(|p| (p.name.clone(), p.tool.clone())).collect();
    let selected = app.ui.presets_ui.selected.get("toolPresets").cloned();
    let mut action: Option<(String, Value)> = None;
    let mut pick: Option<String> = None;
    // Rename field.
    if let Some((_, key, mut text)) = app.ui.presets_ui.renaming.clone().filter(|r| r.0 == "toolPresets") {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                app.ui.presets_ui.renaming = None;
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                    action = Some(("tool.presets.edit".into(), json!({"action": "rename", "preset": key, "name": text.trim()})));
                }
            } else {
                app.ui.presets_ui.renaming = Some(("toolPresets".into(), key.clone(), text.clone()));
            }
        });
    }
    egui::ScrollArea::vertical().id_salt("tool-presets").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if items.is_empty() {
            empty(ui, if only { tl!("No presets for the current tool.") } else { tl!("No tool presets.") });
        }
        for (name, tool) in &items {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            if selected.as_deref() == Some(name) {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
            }
            let icon = Tool::from_name(tool).map(crate::icons::tool_icon).unwrap_or("settings");
            crate::icons::paint(ui, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(16.0, 16.0)), icon, 13.0, t.icon);
            ui.painter().text(pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.5), t.text);
            if resp.clicked() {
                pick = Some(name.clone());
            }
            resp.context_menu(|ui| {
                if ui.button(tl!("Rename Tool Preset…")).clicked() {
                    app.ui.presets_ui.renaming = Some(("toolPresets".into(), name.clone(), name.clone()));
                    ui.close();
                }
                if ui.button(tl!("Delete Tool Preset")).clicked() {
                    action = Some(("tool.presets.edit".into(), json!({"action": "delete", "preset": name})));
                    ui.close();
                }
            });
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        crate::widgets::checkbox(ui, &mut app.ui.presets_ui.current_tool_only, tl!("Current Tool Only"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "trash", 24.0, false, tl!("Delete tool preset")).clicked()
                && let Some(n) = selected.clone()
            {
                action = Some(("tool.presets.edit".into(), json!({"action": "delete", "preset": n})));
            }
            if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new tool preset")).clicked() {
                action = Some(("tool.presets.new".into(), new_tool_preset_params(app, cur)));
            }
        });
    });
    if let Some(n) = pick {
        app.ui.presets_ui.selected.insert("toolPresets".into(), n.clone());
        select_tool_preset(app, &n);
    }
    if let Some((cmd, p)) = action {
        let created = cmd == "tool.presets.new";
        if let Some(r) = run(app, &cmd, p)
            && created
        {
            let n = r["name"].as_str().unwrap_or_default().to_string();
            app.ui.presets_ui.selected.insert("toolPresets".into(), n.clone());
            app.ui.presets_ui.renaming = Some(("toolPresets".into(), n.clone(), n));
        }
    }
}

// ------------------------------------------------------------------ Clone Source

pub fn clone_source_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let cs = app.session.presets.clone.clone();
    let slot = cs.active().clone();
    let mut action: Option<Value> = None;
    // The five source buttons.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for i in 0..photocraft_engine::presets::clone_source::SLOTS {
            let set = cs.slots[i].source.is_some();
            let tip = match cs.slots[i].source {
                Some(s) => format!("Clone source {}: {:.0}, {:.0}", i + 1, s[0], s[1]),
                None => crate::i18n::fmt(
                    tl!("Clone source {n} ({key}-click with Clone Stamp to set)"),
                    &[("n", &(i + 1).to_string()), ("key", &crate::shortcuts::pretty("Alt"))],
                ),
            };
            let r = crate::icons::button(ui, "stamp", 30.0, cs.active == i, &tip);
            if set {
                let d = r.rect.right_bottom() - vec2(6.0, 6.0);
                ui.painter().circle_filled(d, 2.5, t.accent);
            }
            if r.clicked() {
                run(app, "cloneSource.select", json!({"index": i}));
            }
        }
    });
    ui.add_space(4.0);
    let src_line =
        match (slot.source, slot.layer.and_then(|l| app.session.active().and_then(|d| d.doc.layer(photocraft_doc::LayerId(l)).map(|x| x.name.clone())))) {
            (Some(_), Some(l)) => format!("Source: {} : {l}", app.session.active().map(|d| d.doc.name.clone()).unwrap_or_default()),
            (Some(_), None) => "Source: set".to_string(),
            _ => crate::i18n::fmt(tl!("Source: not set ({key}-click with Clone Stamp)"), &[("key", &crate::shortcuts::pretty("Alt"))]),
        };
    ui.label(egui::RichText::new(src_line).color(t.text_dim).size(11.0));
    ui.add_space(4.0);
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.add_sized(vec2(18.0, 18.0), egui::Label::new(egui::RichText::new(s).color(t.text_dim).size(11.5)));
    };
    let off = slot.offset().unwrap_or([0.0, 0.0]);
    let (mut ox, mut oy) = (off[0] as f32, off[1] as f32);
    let (mut w, mut h) = (slot.scale[0] as f32, slot.scale[1] as f32);
    let mut rot = slot.rotation as f32;
    egui::Grid::new("clone-src-grid").num_columns(5).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        lbl(ui, "X:");
        if crate::widgets::value_field(ui, &mut ox, -30000.0..=30000.0, "px", 72.0).changed() && slot.source.is_some() {
            action = Some(json!({"offset": [ox, oy]}));
        }
        lbl(ui, "W:");
        if crate::widgets::value_field(ui, &mut w, -1000.0..=1000.0, "%", 64.0).changed() {
            action = Some(json!({"width": w}));
        }
        // The W/H chain: with it on, a new W or H scales the other by the same factor.
        let icon = if slot.linked { "link" } else { "unlink" };
        if crate::icons::button(ui, icon, 22.0, slot.linked, tl!("Maintain aspect ratio")).clicked() {
            action = Some(json!({"linked": !slot.linked}));
        }
        ui.end_row();
        lbl(ui, "Y:");
        if crate::widgets::value_field(ui, &mut oy, -30000.0..=30000.0, "px", 72.0).changed() && slot.source.is_some() {
            action = Some(json!({"offset": [ox, oy]}));
        }
        lbl(ui, "H:");
        if crate::widgets::value_field(ui, &mut h, -1000.0..=1000.0, "%", 64.0).changed() {
            action = Some(json!({"height": h}));
        }
        ui.end_row();
        lbl(ui, "A:");
        if crate::widgets::value_field(ui, &mut rot, -180.0..=180.0, "°", 72.0).changed() {
            action = Some(json!({"rotation": rot}));
        }
        ui.horizontal(|ui| {
            if crate::icons::button(ui, "arrow-left-right", 22.0, slot.flip_h, tl!("Flip Horizontal")).clicked() {
                action = Some(json!({"flipH": !slot.flip_h}));
            }
            if crate::icons::button(ui, "rotate-cw", 22.0, false, tl!("Reset Transform")).clicked() {
                run(app, "cloneSource.resetTransform", json!({}));
            }
        });
        ui.horizontal(|ui| {
            if crate::icons::button(ui, "scaling", 22.0, slot.flip_v, tl!("Flip Vertical")).clicked() {
                action = Some(json!({"flipV": !slot.flip_v}));
            }
        });
        ui.end_row();
    });
    if let Some(p) = action {
        run(app, "cloneSource.set", p);
    }
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
    // Overlay options.
    let mut o = cs.overlay.clone();
    let before = o.clone();
    crate::widgets::checkbox(ui, &mut o.show, tl!("Show Overlay"));
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tl!("Opacity:")).color(t.text_dim).size(11.5));
        crate::widgets::value_field(ui, &mut o.opacity, 0.0..=100.0, "%", 56.0);
        let opts = [
            ("normal".to_string(), tl!("Normal")),
            ("darken".to_string(), tl!("Darken")),
            ("lighten".to_string(), tl!("Lighten")),
            ("difference".to_string(), tl!("Difference")),
        ];
        crate::widgets::dropdown(ui, "clone-overlay-mode", &mut o.blend, &opts, 90.0);
    });
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        crate::widgets::checkbox(ui, &mut o.clipped, tl!("Clipped"));
        crate::widgets::checkbox(ui, &mut o.auto_hide, tl!("Auto Hide"));
        crate::widgets::checkbox(ui, &mut o.invert, tl!("Invert"));
    });
    if o != before {
        run(
            app,
            "cloneSource.overlay",
            json!({"show": o.show, "opacity": o.opacity, "clipped": o.clipped, "autoHide": o.auto_hide, "invert": o.invert, "blend": o.blend}),
        );
    }
}

/// Where the active clone source samples for document point `at` (for the canvas marker).
/// The first stroke anchors at its press, so the marker follows before the stroke commits.
pub fn clone_sample_point(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<[f64; 2]> {
    use photocraft_engine::presets::clone_source::{Mapping, transform_matrix};
    let s = app.session.presets.clone.active();
    let src = s.source?;
    let stroking = app.drag.as_ref().filter(|d| matches!(d.tool, Tool::CloneStamp | Tool::Healing)).map(|d| d.start);
    let anchor = if app.ui.tool_options.clone_aligned { s.anchor.or(stroking) } else { stroking };
    match (anchor, at) {
        (Some(a), Some(h)) => {
            let m = Mapping { source: (src[0], src[1]), anchor: (a[0], a[1]), m: transform_matrix(s.scale, s.rotation, s.flip_h, s.flip_v) };
            let (x, y) = m.map(h[0], h[1]);
            Some([x, y])
        }
        _ => Some(src),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::invoke;
    use crate::pickers::pattern_browser::{compact_pattern_browser, pattern_group_views};
    use crate::pickers::picker_state::{HierarchyOrderEntry, MAX_GROUP_DEPTH};
    use crate::state::ToolOptions;

    fn app() -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
        (app, ctx)
    }

    #[test]
    fn all_preset_windows_use_shared_docking_chrome() {
        let (mut app, ctx) = app();
        for id in PANELS {
            assert!(crate::menus::is_live(id), "{id}");
        }
        invoke(&mut app, &ctx, "window.panel.gradients", Value::Null).unwrap();
        assert!(app.ui.docking.as_ref().is_some_and(|layout| layout.floating.iter().any(|group| group.panels.iter().any(|panel| panel == "gradients"))));
        assert_eq!(crate::panel_docking::checked(&app, "window.panel.gradients"), Some(true));
        invoke(&mut app, &ctx, "window.panel.patterns", Value::Null).unwrap();
        assert!(app.ui.docking.as_ref().is_some_and(|layout| layout.floating.iter().any(|group| group.panels.iter().any(|panel| panel == "patterns"))));
        invoke(&mut app, &ctx, "window.panel.swatches", Value::Null).unwrap();
        assert!(app.ui.docking.as_ref().is_some_and(|layout| layout.floating.iter().any(|group| group.panels.iter().any(|panel| panel == "swatches"))));
        // Choosing a shared-docking picker again closes it.
        invoke(&mut app, &ctx, "window.panel.patterns", Value::Null).unwrap();
        assert!(!app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&"patterns".to_owned())));
        for id in ["window.panel.styles", "window.panel.shapes", "window.panel.toolPresets", "window.panel.cloneSource"] {
            invoke(&mut app, &ctx, id, Value::Null).unwrap();
            let panel = id.strip_prefix("window.panel.").unwrap();
            assert!(
                app.ui.docking.as_ref().is_some_and(|layout| layout.floating.iter().any(|group| group.panels.iter().any(|member| member == panel))),
                "{id} uses shared floating chrome"
            );
            assert_eq!(crate::panel_docking::checked(&app, id), Some(true));
            invoke(&mut app, &ctx, id, json!({"show": false})).unwrap();
            assert!(!app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&panel.to_owned())), "{id} closes through shared docking");
        }
    }

    #[test]
    fn tool_preset_switches_tool_and_options() {
        let (mut app, _) = app();
        app.ui.tool = Tool::Brush;
        select_tool_preset(&mut app, "Radial Gradient");
        assert_eq!(app.ui.tool, Tool::Gradient);
        assert_eq!(app.ui.tool_options.gradient_style, "radial");
        select_tool_preset(&mut app, "Soft Eraser 60 px");
        assert_eq!(app.ui.tool, Tool::Eraser);
        assert_eq!(app.session.tools.brush.size, 60.0);
        assert_eq!(tool_id(Tool::CloneStamp), "cloneStamp");
        assert_eq!(tool_id(Tool::PatternStamp), "patternStamp");
        assert_eq!(Tool::from_name(&tool_id(Tool::CustomShape)), Some(Tool::CustomShape));
    }

    /// #7356: a preset made with + keeps only its own tool's options, so choosing it doesn't
    /// reset the Crop tool's Delete Cropped Pixels or the Move tool's Auto-Select.
    #[test]
    fn new_tool_preset_stores_only_its_tools_options() {
        let (mut app, _) = app();
        app.ui.tool = Tool::RectMarquee;
        app.ui.tool_options.feather = 5.0;
        let p = new_tool_preset_params(&app, Tool::RectMarquee);
        let r = app.run("tool.presets.new", p).unwrap();
        let name = r["name"].as_str().unwrap().to_string();
        let stored = &app.session.presets.tool_presets.iter().find(|t| t.name == name).unwrap().options["toolOptions"];
        assert_eq!(stored["feather"], json!(5.0));
        assert!(stored.as_object().unwrap().keys().all(|k| !k.starts_with("crop_") && !k.starts_with("move_")), "{stored}");
        app.ui.tool_options.feather = 0.0;
        app.ui.tool_options.crop_delete = !ToolOptions::default().crop_delete;
        app.ui.tool_options.move_auto_select = !ToolOptions::default().move_auto_select;
        select_tool_preset(&mut app, &name);
        assert_eq!(app.ui.tool_options.feather, 5.0);
        assert_eq!(app.ui.tool_options.crop_delete, !ToolOptions::default().crop_delete);
        assert_eq!(app.ui.tool_options.move_auto_select, !ToolOptions::default().move_auto_select);
    }

    #[test]
    fn custom_shape_tool_places_the_picked_shape() {
        let (mut app, _) = app();
        app.ui.presets_ui.custom_shape = "Star".into();
        finish_custom_shape(&mut app, [4.0, 4.0, 20.0, 20.0], true, json!("#ff0000"), Value::Null);
        let d = app.session.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        assert_eq!(l.name, "Star");
        assert!(matches!(l.content, photocraft_doc::LayerContent::Shape(_)));
    }

    /// Root group moves use the canonical hierarchy sequence; legacy groupOrder is normalized
    /// into that sequence when old saved UI state is read.
    #[test]
    fn moving_a_nested_group_reorders_like_the_dock_tabs() {
        let names: Vec<String> = ["Grays", "Pastel", "Pure", "Dark"].iter().map(|s| s.to_string()).collect();
        let mut st = PresetUi::default();
        assert_eq!(st.display_order("swatches", &names), vec![0, 1, 2, 3]);
        st.move_group("swatches", &names, "Pastel", None);
        assert_eq!(st.hierarchy_order["swatches"][""], ["Grays", "Pure", "Dark", "Pastel"].map(|name| HierarchyOrderEntry::Group(name.into())));
        assert!(serde_json::to_value(&st).unwrap().get("groupOrder").is_none(), "new root order does not write the legacy field");
        st.move_group("swatches", &names, "Pastel", Some("Grays"));
        assert_eq!(st.hierarchy_order["swatches"][""], ["Pastel", "Grays", "Pure", "Dark"].map(|name| HierarchyOrderEntry::Group(name.into())));
        assert_eq!(st.display_order("swatches", &names), vec![1, 0, 2, 3]);
        // Back to the library's own order: no entry, as the dock stores its default tab order.
        st.move_group("swatches", &names, "Pastel", Some("Pure"));
        assert!(!st.hierarchy_order.get("swatches").is_some_and(|orders| orders.contains_key("")), "a no-op order is not stored");
        // Out-of-range names, moves onto themselves and unknown `before` names change nothing bad.
        for (from, before) in [("Nope", None), ("Grays", Some("Grays"))] {
            let mut m = PresetUi::default();
            m.move_group("swatches", &names, from, before);
            assert!(m.hierarchy_order.is_empty(), "{from} {before:?}");
            assert_eq!(m.display_order("swatches", &names), vec![0, 1, 2, 3]);
        }
        // A missing group in a saved order follows the rest in library order.
        let m: PresetUi = serde_json::from_value(json!({"groupOrder": {"swatches": ["Dark", "Grays"]}})).unwrap();
        assert_eq!(m.display_order("swatches", &names), vec![3, 0, 1, 2]);
    }

    #[test]
    fn nested_groups_keep_their_expansion_state_and_reject_cycles() {
        let names: Vec<String> = ["Parent", "Child", "Grandchild", "Sibling"].iter().map(|s| s.to_string()).collect();
        let mut st = PresetUi::default();
        st.toggled.insert("gradients/Child".into()); // Child is expanded (non-default toggle).
        st.nest_group("gradients", &names, "Child", Some("Parent"));
        st.nest_group("gradients", &names, "Grandchild", Some("Child"));
        st.nest_group("gradients", &names, "Parent", Some("Grandchild")); // would create a cycle

        assert_eq!(st.parent_of("gradients", "Child"), Some("Parent"));
        assert_eq!(st.parent_of("gradients", "Grandchild"), Some("Child"));
        assert_eq!(st.parent_of("gradients", "Parent"), None);
        assert!(st.toggled.contains("gradients/Child"), "moving a group must not reset its open state");
        assert_eq!(st.nested_display_order("gradients", &names), vec![(0, 0), (1, 1), (2, 2), (3, 0)]);
        let saved = serde_json::to_value(&st).unwrap();
        assert_eq!(saved["groupParent"]["gradients"]["Child"], "Parent");
        assert!(saved.get("group_parent").is_none(), "saved UI state uses the stable camelCase key");
        let legacy: PresetUi = serde_json::from_value(json!({"group_parent":{"gradients":{"Child":"Parent"}}})).unwrap();
        assert_eq!(legacy.parent_of("gradients", "Child"), Some("Parent"));

        st.set_hierarchy_order(
            "gradients",
            "",
            ["Parent", "Sibling"].map(|name| HierarchyOrderEntry::Group(name.into())).to_vec(),
        );
        st.toggled.insert("gradients/Parent/Inner".into());
        st.rename_group("gradients", "Parent", "Renamed");
        assert_eq!(st.parent_of("gradients", "Child"), Some("Renamed"));
        assert_eq!(st.hierarchy_order["gradients"][""][0], HierarchyOrderEntry::Group("Renamed".into()));
        assert!(st.toggled.contains("gradients/Renamed/Inner"));
        st.delete_group("gradients", "Renamed");
        assert_eq!(st.parent_of("gradients", "Child"), None, "children of a deleted group become roots");
        assert!(!st.hierarchy_order["gradients"][""].contains(&HierarchyOrderEntry::Group("Renamed".into())));
        assert!(!st.toggled.iter().any(|key| key.starts_with("gradients/Renamed")));

        // Slash-containing labels are encoded as one component so they cannot collide with a
        // group path; renaming and deleting them must preserve that distinction.
        let slash_group = format!("gradients/{}", crate::pickers::picker_state::escape_hierarchy_key_part("A/B"));
        st.toggled.insert(format!("{slash_group}/Inner"));
        st.toggled.insert("gradients/A/B/Inner".into());
        st.rename_group("gradients", "A/B", "C/D");
        assert!(st.toggled.contains(&format!("gradients/{}/Inner", crate::pickers::picker_state::escape_hierarchy_key_part("C/D"))));
        assert!(st.toggled.contains("gradients/A/B/Inner"), "a nested path with the same legacy spelling is independent");
        st.delete_group("gradients", "C/D");
        assert!(st.toggled.contains("gradients/A/B/Inner"));
        assert_ne!(
            format!(
                "brush-picker/{}/{}",
                crate::pickers::picker_state::escape_hierarchy_key_part("A"),
                crate::pickers::picker_state::escape_hierarchy_key_part("B/C")
            ),
            format!(
                "brush-picker/{}/{}/{}",
                crate::pickers::picker_state::escape_hierarchy_key_part("A"),
                crate::pickers::picker_state::escape_hierarchy_key_part("B"),
                crate::pickers::picker_state::escape_hierarchy_key_part("C")
            ),
            "a slash inside one group component is not a path separator",
        );

        st.nest_group("gradients", &names, "Child", None);
        assert_eq!(st.parent_of("gradients", "Child"), None);
    }

    #[test]
    fn nested_group_header_drop_matches_brush_group_drop_convention() {
        let names: Vec<String> = ["Parent", "Child", "Grandchild", "Sibling"].iter().map(|s| s.to_string()).collect();
        let mut st = PresetUi::default();
        st.nest_group("gradients", &names, "Child", Some("Parent"));
        st.nest_group("gradients", &names, "Grandchild", Some("Child"));

        // Dropping on the upper edge of a nested header inserts before it at the same level.
        st.drop_group_on_header("gradients", &names, "Sibling", "Grandchild", true);
        assert_eq!(st.parent_of("gradients", "Sibling"), Some("Child"));
        assert_eq!(st.nested_display_order("gradients", &names), vec![(0, 0), (1, 1), (3, 2), (2, 2)]);

        // Dropping on the body nests into the target, just like the Brushes panel.
        st.drop_group_on_header("gradients", &names, "Sibling", "Parent", false);
        assert_eq!(st.parent_of("gradients", "Sibling"), Some("Parent"));
        // A source cannot be dropped before its own descendant, which would make a cycle.
        st.drop_group_on_header("gradients", &names, "Parent", "Grandchild", true);
        assert_eq!(st.parent_of("gradients", "Parent"), None);
    }

    #[test]
    fn extremely_deep_group_trees_are_bounded_and_remain_visible() {
        let names: Vec<String> = (0..64).map(|i| format!("Group {i}")).collect();
        let mut st = PresetUi::default();
        for pair in names.windows(2) {
            st.nest_group("gradients", &names, &pair[1], Some(&pair[0]));
        }
        let order = st.nested_display_order("gradients", &names);
        assert_eq!(order.len(), names.len());
        assert!(order.iter().all(|(_, depth)| *depth <= MAX_GROUP_DEPTH));
    }

    #[test]
    fn every_point_on_a_shared_header_is_a_drop_onto_target() {
        let header = Rect::from_min_size(pos2(10.0, 20.0), vec2(120.0, 24.0));
        assert!(!header_drop_before(header, pos2(30.0, 29.0)));
        assert!(!header_drop_before(header, pos2(30.0, 25.0)));
        assert!(!header_drop_before(header, pos2(30.0, 40.0)));
    }

    #[test]
    fn shared_header_payload_keeps_root_and_nested_group_identity() {
        assert_eq!(
            asset_browser_header_payload("patterns", "Textures", &[]),
            AssetDrag::Group { scope: "patterns".into(), name: "Textures".into(), path: Vec::new() }
        );
        assert_eq!(
            asset_browser_header_payload("brushes", "Ink", &["Imported".into(), "Dry".into()]),
            AssetDrag::Group { scope: "brushes".into(), name: "Ink".into(), path: vec!["Imported".into(), "Dry".into()] }
        );
    }

    #[test]
    fn short_preset_library_keeps_the_allocated_browser_height() {
        use egui_kittest::Harness;

        let mut st = PresetUi::default();
        let groups = Vec::<GroupView>::new();
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut adapter = BrowserAdapter {
            panel: "short-library",
            groups: &groups,
            options: BrowserOptions {
                canvas: Rect::NOTHING,
                flush: false,
                center_grid: false,
                grid_indent: 8.0,
                all_groups_open: false,
                compact_context_menus: false,
                item_size: None,
                list_item_height: None,
                domain_hierarchy_actions: false,
                root_item_group: None,
            },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(standard_item_context_menu),
            group_context_menu: Some(standard_group_context_menu),
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: &mut |_, _, _, _, _: ItemPaintState| {},
            paint_list_item: None,
        };
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell(ui, &mut st, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard("Create preset")), ..Default::default() });
            },
            app,
        );
        h.run_steps(2);
        let rect = last_browser_rect(&h.ctx, "short-library");
        assert!(rect.is_some(), "browser viewport is recorded");
        assert!(rect.unwrap().height() >= 200.0, "short libraries should retain the remaining panel height: {rect:?}");
    }

    #[test]
    fn short_picker_keeps_the_footer_visible_while_renaming() {
        use std::{cell::Cell, rc::Rc};

        use egui_kittest::Harness;

        let mut st = PresetUi { renaming: Some(("short-rename".into(), "preset-key".into(), "Preset name".into())), ..Default::default() };
        let groups = Vec::<GroupView>::new();
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut adapter = BrowserAdapter {
            panel: "short-rename",
            groups: &groups,
            options: BrowserOptions {
                canvas: Rect::NOTHING,
                flush: false,
                center_grid: false,
                grid_indent: 8.0,
                all_groups_open: false,
                compact_context_menus: false,
                item_size: None,
                list_item_height: None,
                domain_hierarchy_actions: false,
                root_item_group: None,
            },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(standard_item_context_menu),
            group_context_menu: Some(standard_group_context_menu),
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: &mut |_, _, _, _, _: ItemPaintState| {},
            paint_list_item: None,
        };
        let content_bottom = Rc::new(Cell::new(0.0_f32));
        let measured_bottom = Rc::clone(&content_bottom);
        let mut h = Harness::builder().with_size(vec2(320.0, 180.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell(ui, &mut st, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard("Create preset")), ..Default::default() });
                measured_bottom.set(ui.min_rect().bottom());
            },
            app,
        );
        h.run_steps(3);
        assert!(content_bottom.get() <= 180.0, "footer should stay inside the short panel: content bottom {}", content_bottom.get());
    }

    #[test]
    fn custom_picker_shell_renders_search_and_passes_remaining_height_to_renderer() {
        use std::{cell::Cell, rc::Rc};

        use egui_kittest::Harness;

        let calls = Rc::new(Cell::new(0));
        let observed_height = Rc::new(Cell::new(0.0_f32));
        let observed_clip_height = Rc::new(Cell::new(0.0_f32));
        let observed_content_height = Rc::new(Cell::new(0.0_f32));
        let call_count = Rc::clone(&calls);
        let height = Rc::clone(&observed_height);
        let clip_height = Rc::clone(&observed_clip_height);
        let content_height = Rc::clone(&observed_content_height);
        let mut search = move |ui: &mut egui::Ui| {
            call_count.set(call_count.get() + 1);
            ui.label("Brush search");
        };
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell_custom(ui, 260.0, CustomShellOptions { search: Some(&mut search), ..Default::default() }, |ui, allocated_h| {
                    height.set(allocated_h);
                    clip_height.set(ui.clip_rect().height());
                    ui.allocate_exact_size(vec2(ui.available_width(), allocated_h + 80.0), Sense::hover());
                    content_height.set(ui.min_rect().height());
                });
            },
            app,
        );
        h.run_steps(2);
        assert!(calls.get() > 0, "the shared shell renders enabled search controls");
        assert!(observed_height.get() > 100.0, "the custom renderer receives the space remaining after shell controls");
        assert!(observed_height.get() <= 260.0, "the custom renderer respects its consumer height cap");
        assert!(observed_clip_height.get() <= observed_height.get(), "the renderer is clipped to its allocated browser viewport");
        assert!(observed_content_height.get() <= observed_height.get(), "an oversized renderer cannot grow beyond the browser viewport");
    }

    #[test]
    fn picker_shell_controls_and_browser_capabilities_are_optional() {
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };

        use egui_kittest::{Harness, kittest::Queryable};

        let groups =
            vec![GroupView { name: "Only group".into(), root_only: false, items: vec![ItemView { key: "only".into(), name: "Only item".into(), group_path: Vec::new() }] }];
        let mut st = PresetUi::default();
        let menu_calls = Rc::new(Cell::new(0));
        let search_calls = Rc::new(Cell::new(0));
        let recent_calls = Rc::new(Cell::new(0));
        let menu_count = Rc::clone(&menu_calls);
        let search_count = Rc::clone(&search_calls);
        let recent_count = Rc::clone(&recent_calls);
        let mut menu = move |ui: &mut egui::Ui| {
            menu_count.set(menu_count.get() + 1);
            ui.label("Panel menu");
        };
        let mut search = move |ui: &mut egui::Ui| {
            search_count.set(search_count.get() + 1);
            ui.label("Search");
        };
        let mut recents = move |ui: &mut egui::Ui| {
            recent_count.set(recent_count.get() + 1);
            ui.label("Recent");
        };
        let events = Rc::new(RefCell::new(Vec::<BrowserAction>::new()));
        let captured_events = Rc::clone(&events);
        let mut adapter = BrowserAdapter {
            panel: "compact-library",
            groups: &groups,
            options: BrowserOptions {
                canvas: Rect::NOTHING,
                flush: false,
                center_grid: false,
                grid_indent: 8.0,
                all_groups_open: false,
                compact_context_menus: false,
                item_size: None,
                list_item_height: None,
                domain_hierarchy_actions: false,
                root_item_group: None,
            },
            capabilities: BrowserCapabilities { item_drag: false, item_drop: false, group_drag: false, selection: false, activation: false, canonical_hierarchy_drag: false },
            item_context_menu: None,
            group_context_menu: None,
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: &mut |_, _, _, _, _: ItemPaintState| {},
            paint_list_item: None,
        };
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                let emitted = picker_shell(
                    ui,
                    &mut st,
                    &mut adapter,
                    ShellOptions { panel_menu: Some(&mut menu), search: Some(&mut search), recents: Some(&mut recents), footer: None, ..Default::default() },
                );
                captured_events.borrow_mut().extend(emitted);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        assert!(menu_calls.get() > 0 && search_calls.get() > 0 && recent_calls.get() > 0, "enabled shell slots are rendered");
        let rect = last_browser_rect(&h.ctx, "compact-library").expect("browser viewport is recorded without a footer");
        assert!(rect.height() > 200.0, "without a footer, the browser expands into the remaining panel space: {rect:?}");
        h.get_by_label("Only item").click();
        h.run_steps(2);
        assert!(events.borrow().is_empty(), "disabled browser capabilities do not emit consumer actions");
    }

    #[test]
    fn shared_hierarchy_preserves_order_and_bounds_nested_paths() {
        #[derive(Debug, PartialEq)]
        struct Item(&'static str);
        let a = Item("a");
        let b = Item("b");
        let c = Item("c");
        let d = Item("d");
        let path = |parts: &[&str]| parts.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let group_path = path(&["Nested"]);
        let nested_path = path(&["Nested", "Child"]);
        let tree = hierarchy_tree(vec![(&group_path, &a), (&[], &b), (&nested_path, &c), (&group_path, &d)], 4);
        assert!(matches!(tree.first(), Some(HierarchyNode::Group(f)) if f.path == ["Nested"] && f.count == 3));
        assert!(matches!(tree.get(1), Some(HierarchyNode::Preset(item)) if item.0 == "b"));
        let Some(HierarchyNode::Group(group)) = tree.first() else { panic!("group node") };
        assert!(matches!(group.children.first(), Some(HierarchyNode::Preset(item)) if item.0 == "a"));
        assert!(matches!(group.children.get(1), Some(HierarchyNode::Group(nested)) if nested.path == ["Nested", "Child"]));
        assert!(matches!(group.children.get(2), Some(HierarchyNode::Preset(item)) if item.0 == "d"));

        let deep_path = path(&["Nested", "Deep", "TooDeep"]);
        let bounded = hierarchy_tree(vec![(&nested_path, &c), (&deep_path, &d)], 1);
        assert!(matches!(bounded.first(), Some(HierarchyNode::Group(group)) if group.path == ["Nested"] && group.children.len() == 2));
    }

    #[test]
    fn asset_group_projects_legacy_folder_paths_into_the_shared_group_hierarchy() {
        let group = GroupView {
            name: "Brushes".into(),
            root_only: false,
            items: vec![
                ItemView { key: "a".into(), name: "A".into(), group_path: vec!["Imported".into()] },
                ItemView { key: "b".into(), name: "B".into(), group_path: Vec::new() },
                ItemView { key: "c".into(), name: "C".into(), group_path: vec!["Imported".into(), "Soft".into()] },
            ],
        };
        let tree = hierarchy_tree(group.items.iter().map(|item| (item.group_path.as_slice(), item)).collect(), 4);
        assert!(matches!(tree.first(), Some(HierarchyNode::Group(group)) if group.path == ["Imported"] && group.count == 2));
        assert!(matches!(tree.get(1), Some(HierarchyNode::Preset(item)) if item.key == "b"));
        assert!(
            matches!(&tree[0], HierarchyNode::Group(group) if matches!(group.children.get(1), Some(HierarchyNode::Group(nested)) if nested.path == ["Imported", "Soft"] && nested.count == 1 && matches!(nested.children.first(), Some(HierarchyNode::Preset(item)) if item.key == "c")))
        );
    }

    #[test]
    fn compact_patterns_browser_selects_a_library_identity() {
        use egui_kittest::{Harness, kittest::Queryable};

        let (app, _) = app();
        let patterns = app.session.patterns.items.clone();
        let groups = pattern_group_views(&patterns, &app.session.presets.pattern_groups);
        let first = groups.iter().find_map(|g| g.items.first()).expect("built-in Patterns group has an item");
        let (id, name) = (first.key.clone(), first.name.clone());
        let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
            move |ui, picked: &mut Option<String>| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                if let Some(id) = compact_pattern_browser(ui, &patterns, &groups, "", "compact-test", 180.0) {
                    *picked = Some(id);
                }
            },
            None::<String>,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        h.get_by_label(&name).click();
        h.run_steps(2);
        assert_eq!(h.state().as_deref(), Some(id.as_str()));
        assert!(last_browser_rect(&h.ctx, "compact-test").is_some(), "compact consumer uses the shared browser viewport");
    }

    #[test]
    fn compact_patterns_browser_preserves_group_disclosure_between_frames() {
        use egui_kittest::Harness;

        let (app, _) = app();
        let patterns = app.session.patterns.items.clone();
        let groups = pattern_group_views(&patterns, &app.session.presets.pattern_groups);
        assert!(groups.len() > 1, "the fixture needs a second group to toggle");
        let group = groups[1].name.clone();
        let panel = "compact-disclosure-test";
        let state_id = egui::Id::new(("compact-pattern-browser", panel));
        let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
            move |ui, _picked: &mut Option<String>| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                let _ = compact_pattern_browser(ui, &patterns, &groups, "", panel, 180.0);
            },
            None::<String>,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);

        h.run_steps(3);
        assert!(
            h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&format!("{panel}/{group}")))),
            "later groups start collapsed"
        );

        let header =
            last_group_headers(&h.ctx, panel).iter().find(|(name, _)| name == &group).map(|(_, rect)| rect.center()).expect("second group header is visible");
        h.event(egui::Event::PointerMoved(header));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);

        let key = format!("{panel}/{group}");
        assert!(!h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&key))), "clicking the group expands it");
        h.run_steps(2);
        assert!(!h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&key))), "the next frame must retain the expanded state");
    }

    #[test]
    fn patterns_panel_dispatches_selection_to_the_patterns_command() {
        use egui_kittest::{Harness, kittest::Queryable};

        let (app, _) = app();
        let first = pattern_group_views(&app.session.patterns.items, &app.session.presets.pattern_groups)
            .iter()
            .find_map(|g| g.items.first().map(|p| (p.key.clone(), p.name.clone())))
            .expect("a built-in Patterns group");
        let (id, name) = first;
        let mut h = Harness::builder().with_size(vec2(340.0, 360.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                // Harness construction draws one frame before the production context setup below.
                // Do not resolve the named theme font until that setup has installed it.
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                patterns_panel(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(4);
        h.get_by_label(&name).click();
        h.run_steps(3);
        assert_eq!(h.state().session.presets.pattern.as_deref(), Some(id.as_str()));
        assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("pattern.presets.select"));
    }

    #[test]
    fn patterns_panel_card_drag_updates_the_shared_card_group_sequence() {
        use egui_kittest::{Harness, kittest::Queryable};

        let (app, _) = app();
        let groups = pattern_group_views(&app.session.patterns.items, &app.session.presets.pattern_groups);
        let (group_name, first_key, first_name, second_key, second_name) = groups
            .iter()
            .find(|group| group.items.len() >= 2)
            .map(|group| (group.name.clone(), group.items[0].key.clone(), group.items[0].name.clone(), group.items[1].key.clone(), group.items[1].name.clone()))
            .expect("a built-in Patterns group has two cards for a reorder gesture");
        let mut h = Harness::builder().with_size(vec2(360.0, 420.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    patterns_panel(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(4);
        let from = h.get_by_label(&second_name).rect().center();
        let to = h.get_by_label(&first_name).rect().center() - vec2(4.0, 0.0);
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=6 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);

        assert_eq!(
            &h.state().ui.presets_ui.hierarchy_order["patterns"][&group_name][0..2],
            &[crate::pickers::picker_state::HierarchyOrderEntry::Item(second_key), crate::pickers::picker_state::HierarchyOrderEntry::Item(first_key),]
        );
    }

    fn consumer_item_menu(
        ui: &mut egui::Ui,
        _st: &mut PresetUi,
        _panel: &str,
        item: &ItemView,
        _group: &GroupView,
        _names: &[String],
        ev: &mut Vec<BrowserAction>,
    ) {
        if ui.button("Consumer item operation").clicked() {
            ev.push(BrowserAction::Delete(item.key.clone()));
            ui.close();
        }
    }

    fn consumer_group_menu(ui: &mut egui::Ui, _st: &mut PresetUi, _panel: &str, group: &GroupView, ev: &mut Vec<BrowserAction>) {
        if ui.button("Consumer group operation").clicked() {
            ev.push(BrowserAction::DeleteGroup(group.name.clone()));
            ui.close();
        }
    }

    #[test]
    fn browser_keeps_context_hit_targets_but_delegates_menu_actions() {
        use egui_kittest::{Harness, kittest::Queryable};

        let groups =
            vec![GroupView { name: "Only group".into(), root_only: false, items: vec![ItemView { key: "only".into(), name: "Only item".into(), group_path: Vec::new() }] }];
        let mut st = PresetUi::default();
        let mut adapter = BrowserAdapter {
            panel: "consumer-actions",
            groups: &groups,
            options: BrowserOptions {
                canvas: Rect::NOTHING,
                flush: false,
                center_grid: false,
                grid_indent: 8.0,
                all_groups_open: false,
                compact_context_menus: false,
                item_size: None,
                list_item_height: None,
                domain_hierarchy_actions: false,
                root_item_group: None,
            },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(consumer_item_menu),
            group_context_menu: Some(consumer_group_menu),
            nested_group_context_menu: None,
            canonical_hierarchy_move: None,
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: &mut |_, _, _, _, _: ItemPaintState| {},
            paint_list_item: None,
        };
        let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
            |ui, events: &mut Vec<BrowserAction>| {
                if !ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                events.extend(browser(ui, &mut st, &mut adapter, 180.0));
            },
            Vec::<BrowserAction>::new(),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        h.get_by_label("Only item").click_secondary();
        h.run_steps(2);
        h.get_by_label("Consumer item operation").click();
        h.run_steps(2);
        assert!(matches!(h.state().last(), Some(BrowserAction::Delete(key)) if key == "only"));

        let header = last_group_headers(&h.ctx, "consumer-actions").first().map(|(_, r)| r.center()).expect("group hit target recorded");
        h.event(egui::Event::PointerMoved(header));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        h.get_by_label("Consumer group operation").click();
        h.run_steps(2);
        assert!(matches!(h.state().last(), Some(BrowserAction::DeleteGroup(group)) if group == "Only group"));
    }

    /// #3658: legacy groupOrder migrates to canonical mixed sibling order and reads leniently.
    #[test]
    fn legacy_group_order_migrates_and_reads_leniently() {
        let old: PresetUi = serde_json::from_value(json!({"sizes": {"swatches": 22.0}})).unwrap();
        assert!(old.hierarchy_order.is_empty(), "an older state without the field still loads");
        let st: PresetUi = serde_json::from_value(json!({"groupOrder": {"swatches": ["Dark", "Grays"]}})).unwrap();
        assert_eq!(
            st.hierarchy_order["swatches"][""],
            ["Dark", "Grays"].map(|name| HierarchyOrderEntry::Group(name.into()))
        );
        let saved = serde_json::to_value(&st).unwrap();
        assert!(saved.get("groupOrder").is_none(), "new writes use only the canonical hierarchy sequence");
        let back: PresetUi = serde_json::from_value(saved).unwrap();
        assert_eq!(back.display_order("swatches", &["Grays".into(), "Dark".into()]), vec![1, 0]);
        for bad in [json!("x"), json!(7), json!({"swatches": "paths"}), json!({"swatches": [1, null, {"a": 1}]})] {
            let back: PresetUi = serde_json::from_value(json!({"groupOrder": bad})).unwrap();
            assert!(back.hierarchy_order.get("swatches").is_none_or(|orders| orders.get("").is_none_or(Vec::is_empty)), "{bad}");
        }
    }

    /// Regression: preset drops mapped the pointer through the raw widget rect, ignoring the
    /// rulers' 16 px inset — a dropped shape landed 8 screen px from the pointer, and a drop on
    /// the ruler strip counted as a canvas drop. Drops now map through, and target, the same
    /// content rect the canvas draws with.
    #[test]
    fn preset_drops_map_through_the_ruler_inset() {
        let (mut app, _) = app();
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        app.ui.views[0] = crate::state::View { zoom: 2.0, center: [32.0, 24.0], fit_pending: false, fill_pending: false, doc_size: [64, 48], rotation: 0.0 };
        let p = egui::pos2(400.0, 300.0);
        let off = crate::canvas::doc_point_at(&app, p).unwrap();
        assert_eq!(off, [32.0, 24.0], "without rulers the widget rect is the whole mapping");
        // With rulers the content rect shrinks by the inset on the left and top: the same screen
        // point is 8 screen px (4 document px at zoom 2) up-left of where it was.
        app.ui.extras.rulers = true;
        let on = crate::canvas::doc_point_at(&app, p).unwrap();
        assert!((on[0] - (off[0] - 4.0)).abs() < 1e-3, "{on:?} vs {off:?}");
        assert!((on[1] - (off[1] - 4.0)).abs() < 1e-3, "{on:?} vs {off:?}");
        // The ruler strip is no longer a drop target; the canvas proper still is.
        assert!(!canvas_drop_rect(&app).contains(egui::pos2(5.0, 300.0)));
        assert!(canvas_drop_rect(&app).contains(p));
    }

    #[test]
    fn clone_marker_follows_the_engine_slot() {
        let (mut app, _) = app();
        assert_eq!(clone_sample_point(&app, Some([5.0, 5.0])), None);
        app.run("cloneSource.set", json!({"source": [10, 10]})).unwrap();
        assert_eq!(clone_sample_point(&app, Some([30.0, 30.0])), Some([10.0, 10.0]));
        app.drag = Some(crate::canvas::Drag::new(Tool::CloneStamp, [40.0, 40.0], vec![[50.0, 30.0, 1.0]], egui::Modifiers::NONE, false));
        assert_eq!(clone_sample_point(&app, Some([50.0, 30.0])), Some([20.0, 0.0]));
        app.drag = None;
        app.session.presets.clone.active_mut().anchor = Some([20.0, 20.0]);
        assert_eq!(clone_sample_point(&app, Some([30.0, 30.0])), Some([20.0, 20.0]));
    }

    #[test]
    fn clone_marker_shows_only_while_painting() {
        // #668: ⌥-click used to leave a "+" on the canvas until the next stroke.
        let (mut app, _) = app();
        app.ui.tool = Tool::CloneStamp;
        crate::retouch_ui::set_source(&mut app, 10.0, 10.0);
        app.hover_doc = Some([40.0, 40.0]);
        assert_eq!(crate::retouch_ui::source_marker_point(&app), None, "hovering after ⌥-click shows nothing");
        app.drag = Some(crate::canvas::Drag::new(Tool::CloneStamp, [40.0, 40.0], vec![[40.0, 40.0, 1.0]], egui::Modifiers::NONE, false));
        assert!(crate::retouch_ui::source_marker_point(&app).is_some(), "painting shows where it samples");
        app.drag = None;
        app.ui.tool = Tool::Brush;
        assert_eq!(crate::retouch_ui::source_marker_point(&app), None);
    }
}
