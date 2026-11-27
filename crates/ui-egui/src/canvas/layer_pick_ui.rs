//! The canvas layer context menu (#307): right-click with the Move tool, or ⌘/Ctrl+right-click
//! with any tool (and a plain right-click with the Frame tool, #13885), lists the layers with
//! visible pixels under the pointer, nested by group/frame/artboard/clipping hierarchy and with
//! live layer thumbnails. Choosing one selects it.
//!
//! The hit test is `layer.pickAt` with `"list": true` (non-transparent pixels, text bounds,
//! masks, clipping masks, frames, hidden layers and hidden groups), and choosing runs
//! `layer.select`, so the menu is the same two commands an agent would run. Painting tools keep
//! their plain right-click (the Brush Preset picker, #268); the ⌘/Ctrl variant reaches the layer
//! list from them too. The menu is `canvas_tool_menu::MenuKind::Layers`, shown by
//! `canvas_tool_menu::run` through [`show_menu`].

use std::collections::HashSet;

use craft::behavior::IconId;
use craft::keymap::Mods;
use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::ui::contract::overlay::{Anchor, Dismiss, Outside};
use craft::ui::contract::{Bind, Entry, ShowContainer as _};
use craft::ui::popover::Popover;
use craft::ui::primitives::textures::{Priority, ThumbRequest, ThumbnailOptions, Thumbnails, fit_rect};
use craft::ui::primitives::{IconSpec, paint_icon};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::Initial;
use craft::ui::style::Ink;
use egui::{
    Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, TextFormat, Vec2, pos2, text::LayoutJob, text::TextWrapping, vec2,
};
use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::panels::layers_tree::PhotoThumbs;
use crate::state::Tool;

/// The id prefix of a layer row: `layer.select.<layer id>`.
pub const ROW: &str = "layer.select.";

/// The row id of layer `layer`.
pub fn row_id(layer: u64) -> String {
    format!("{ROW}{layer}")
}

/// Does a right-click with `tool` and `mods` open the layer menu (rather than the tool's own)?
pub fn is_gesture(tool: Tool, mods: Mods) -> bool {
    tool == Tool::Move || tool == Tool::Frame || mods.cmd
}

/// The Frame tool's picker (#13885): a separator and Isolate Layers end it.
pub const ISOLATE: &str = "select.isolateLayers";

struct NestedRow {
    id: u64,
    name: String,
    depth: u8,
    clipped: bool,
}

fn collect_nested(list: &[Layer], hit_set: &HashSet<LayerId>, depth: u8, out: &mut Vec<NestedRow>) {
    // Partition sibling list bottom-to-top into (base, clipped_run) units.
    let mut units: Vec<(&Layer, &[Layer])> = Vec::new();
    let mut i = 0;
    while i < list.len() {
        let base = &list[i];
        let mut j = i + 1;
        while j < list.len() && list[j].clipped && !base.clipped {
            j += 1;
        }
        units.push((base, &list[i + 1..j]));
        i = j;
    }
    // Emit topmost unit first. For clipping groups, list the base above its clipped layers
    // indented underneath (matching parent -> nested-child order).
    for (base, clipped_slice) in units.into_iter().rev() {
        let mut clipped_rows = Vec::new();
        for cl in clipped_slice.iter().rev() {
            let mut cl_children = Vec::new();
            if let Some(children) = cl.children() {
                collect_nested(children, hit_set, depth.saturating_add(2), &mut cl_children);
            }
            if hit_set.contains(&cl.id) || !cl_children.is_empty() {
                clipped_rows.push(NestedRow { id: cl.id.0, name: cl.name.clone(), depth: depth.saturating_add(1), clipped: true });
                clipped_rows.extend(cl_children);
            }
        }
        let mut base_children = Vec::new();
        if let Some(children) = base.children() {
            collect_nested(children, hit_set, depth.saturating_add(1), &mut base_children);
        }
        if hit_set.contains(&base.id) || !clipped_rows.is_empty() || !base_children.is_empty() {
            out.push(NestedRow { id: base.id.0, name: base.name.clone(), depth, clipped: false });
            out.extend(clipped_rows);
            out.extend(base_children);
        }
    }
}

/// Arrange hit layers by their nesting hierarchy: enclosing artboards, groups and frames appear as
/// parent rows above their indented descendants, and clipping bases appear above their clipped
/// layers indented underneath (`clipped[i] == true`).
pub fn nested_rows(app: &PhotocraftApp, layers: Vec<(u64, String)>) -> (Vec<(u64, String)>, Vec<u8>, Vec<bool>) {
    let Some(st) = app.session.active() else {
        return (layers, Vec::new(), Vec::new());
    };
    let hit_set: HashSet<LayerId> = layers.iter().map(|(id, _)| LayerId(*id)).collect();
    let mut nested = Vec::new();
    collect_nested(&st.doc.layers, &hit_set, 0, &mut nested);
    if nested.is_empty() {
        return (layers, Vec::new(), Vec::new());
    }
    let rows = nested.iter().map(|r| (r.id, r.name.clone())).collect();
    let depth = nested.iter().map(|r| r.depth).collect();
    let clipped = nested.iter().map(|r| r.clipped).collect();
    (rows, depth, clipped)
}

/// The Frame tool's rows (#13885): returns the nested hierarchy rows and each row's depth.
pub fn frame_rows(app: &PhotocraftApp, layers: Vec<(u64, String)>) -> (Vec<(u64, String)>, Vec<u8>) {
    let (rows, depth, _) = nested_rows(app, layers);
    (rows, depth)
}

/// The layers at document point (x, y), topmost first, including text between glyphs.
pub fn layers_at(app: &mut PhotocraftApp, x: f64, y: f64) -> Vec<(u64, String)> {
    if !(x.is_finite() && y.is_finite()) {
        return Vec::new();
    }
    let Ok(v) = app.session.execute("layer.pickAt", json!({"x": x, "y": y, "list": true})) else { return Vec::new() };
    v.get("layers")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|l| Some((l.get("layer")?.as_u64()?, l.get("name").and_then(Value::as_str).unwrap_or_default().to_string()))).collect())
        .unwrap_or_default()
}

/// How far one level of `depth` indents a row's label: the menu rows have no indent of their own.
const INDENT: &str = "    ";

/// The menu's rows: one per layer, the active one checked, each indented by its level in `depth`
/// (none where `depth` is shorter); with `isolate`, a separator and Isolate Layers end it.
pub fn entries(layers: &[(u64, String)], depth: &[u8], isolate: bool, active: Option<u64>) -> Vec<Entry<String>> {
    let mut out: Vec<Entry<String>> = layers
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            let level = usize::from(depth.get(i).copied().unwrap_or(0));
            Entry::new(row_id(*id), format!("{}{name}", INDENT.repeat(level))).checked(Some(*id) == active)
        })
        .collect();
    if isolate {
        out.push(Entry::separator());
        out.push(Entry::new(ISOLATE.to_string(), tl!("Isolate Layers")));
    }
    out
}

/// Choose a layer from the menu: select it (one `layer.select`) and close the menu.
pub fn choose(app: &mut PhotocraftApp, layer: u64) {
    app.ui.canvas_tool_menu = None;
    app.hover_layer = None;
    if let Err(e) = app.run("layer.select", json!({"layer": layer, "mode": "replace"})) {
        app.ui.status = e;
    }
}

/// Isolate Layers from the Frame tool's picker: close the menu and isolate the selected layers.
pub fn isolate(app: &mut PhotocraftApp) {
    app.ui.canvas_tool_menu = None;
    app.hover_layer = None;
    if let Err(e) = app.run(ISOLATE, json!({})) {
        app.ui.status = e;
    }
}

fn paint_checkerboard(painter: &egui::Painter, rect: Rect) {
    let cell = 4.0;
    let light = Color32::from_gray(236);
    let dark = Color32::from_gray(196);
    painter.rect_filled(rect, CornerRadius::same(2), light);
    let cols = ((rect.width() / cell).ceil() as usize).min(32);
    let rows = ((rect.height() / cell).ceil() as usize).min(32);
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 1 {
                let x0 = rect.min.x + c as f32 * cell;
                let y0 = rect.min.y + r as f32 * cell;
                let x1 = (x0 + cell).min(rect.max.x);
                let y1 = (y0 + cell).min(rect.max.y);
                painter.rect_filled(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), CornerRadius::ZERO, dark);
            }
        }
    }
}

fn paint_clip_arrow(painter: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.25, color);
    let x_vert = rect.min.x + rect.width() * 0.35;
    let y_top = rect.min.y + rect.height() * 0.22;
    let y_mid = rect.center().y + 1.0;
    let x_tip = rect.max.x - 2.0;
    painter.line_segment([pos2(x_vert, y_top), pos2(x_vert, y_mid)], stroke);
    painter.line_segment([pos2(x_vert, y_mid), pos2(x_tip, y_mid)], stroke);
    painter.line_segment([pos2(x_tip - 3.0, y_mid - 2.8), pos2(x_tip, y_mid)], stroke);
    painter.line_segment([pos2(x_tip - 3.0, y_mid + 2.8), pos2(x_tip, y_mid)], stroke);
}

fn paint_check(painter: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let c = rect.center();
    painter.line_segment([pos2(c.x - 3.5, c.y + 0.2), pos2(c.x - 1.0, c.y + 3.0)], stroke);
    painter.line_segment([pos2(c.x - 1.0, c.y + 3.0), pos2(c.x + 4.0, c.y - 3.0)], stroke);
}

fn c32(c: craft::theme::Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

struct ThumbDrawCx<'a> {
    ctx: &'a egui::Context,
    env: &'a Env,
    thumbs: &'a Thumbnails<LayerThumb<LayerId>>,
    source: &'a PhotoThumbs<'a>,
    contents: ThumbContents,
}

fn paint_layer_thumb(painter: &egui::Painter, cx: &ThumbDrawCx<'_>, layer: Option<&Layer>, rect: Rect, highlighted: bool) {
    let tokens = cx.env.tokens.tokens();
    let frame_col = c32(tokens.color.line.frame);
    let sunken_col = c32(tokens.color.surface.sunken);
    let icon_ink = if highlighted { Ink::OnDark } else { Ink::default() };
    let ppp = cx.ctx.pixels_per_point();

    let Some(l) = layer else {
        painter.rect_filled(rect, CornerRadius::same(2), sunken_col);
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, frame_col), StrokeKind::Inside);
        return;
    };

    if l.frame.is_some() {
        let req = ThumbRequest::for_cell(rect.width(), ppp);
        let state = cx.thumbs.state(cx.ctx, cx.source, &LayerThumb::frame_content(l.id, cx.contents), &req, Priority::VISIBLE);
        if let Some(tex) = state.shown() {
            paint_checkerboard(painter, rect);
            let dst = fit_rect(rect.shrink(1.0), tex.size_vec2());
            painter.image(tex.id, dst, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.rect_filled(rect, CornerRadius::same(2), sunken_col);
            let icon_rect = rect.shrink(3.0);
            paint_icon(painter, &cx.env.tokens, cx.env.direction, icon_rect, &IconSpec::svg(IconId::new("image")).ink(icon_ink));
        }
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, frame_col), StrokeKind::Inside);
        return;
    }

    match &l.content {
        LayerContent::Group(g) if g.artboard.is_some() => {
            let icon_rect = rect.shrink(2.0);
            paint_icon(painter, &cx.env.tokens, cx.env.direction, icon_rect, &IconSpec::svg(IconId::new("layers")).ink(icon_ink));
        }
        LayerContent::Group(_) => {
            let icon_rect = rect.shrink(2.0);
            paint_icon(painter, &cx.env.tokens, cx.env.direction, icon_rect, &IconSpec::svg(IconId::new("folder")).ink(icon_ink));
        }
        LayerContent::Text(_) => {
            let paper = Color32::from_gray(244);
            let ink = Color32::from_gray(32);
            let glyph_size = (rect.height() * 0.58).max(11.0);
            painter.rect_filled(rect, CornerRadius::same(2), paper);
            painter.text(rect.center(), Align2::CENTER_CENTER, "T", FontId::proportional(glyph_size), ink);
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, frame_col), StrokeKind::Inside);
        }
        LayerContent::Adjustment(_) => {
            painter.rect_filled(rect, CornerRadius::same(2), sunken_col);
            let icon_rect = rect.shrink(3.0);
            paint_icon(painter, &cx.env.tokens, cx.env.direction, icon_rect, &IconSpec::svg(IconId::new("adjustments")).ink(icon_ink));
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, frame_col), StrokeKind::Inside);
        }
        _ => {
            let req = ThumbRequest::for_cell(rect.width(), ppp);
            let state = cx.thumbs.state(cx.ctx, cx.source, &LayerThumb::pixels(l.id, cx.contents), &req, Priority::VISIBLE);
            let is_fill = matches!(l.content, LayerContent::Fill(_));
            if !is_fill {
                paint_checkerboard(painter, rect);
            } else {
                painter.rect_filled(rect, CornerRadius::same(2), sunken_col);
            }
            if let Some(tex) = state.shown() {
                let dst = fit_rect(rect.shrink(1.0), tex.size_vec2());
                painter.image(tex.id, dst, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else if is_fill {
                let icon_rect = rect.shrink(3.0);
                paint_icon(painter, &cx.env.tokens, cx.env.direction, icon_rect, &IconSpec::svg(IconId::new("image")).ink(icon_ink));
            }
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, frame_col), StrokeKind::Inside);
        }
    }
}

/// Inputs for [`show_menu`].
pub(crate) struct LayerMenuData<'a> {
    pub pos: [f32; 2],
    pub layers: &'a [(u64, String)],
    pub depth: &'a [u8],
    pub clipped: &'a [bool],
    pub isolate: bool,
}

/// Shows the upgraded hierarchical right-click layer picker with live layer thumbnails, tree
/// indentation, clipping indicators, keyboard navigation and canvas rollover preview.
pub(crate) fn show_menu(ui: &mut egui::Ui, app: &mut PhotocraftApp, opener: &egui::Response, data: LayerMenuData<'_>) {
    let LayerMenuData { pos, layers, depth, clipped, isolate: isolate_row } = data;
    let ctx = ui.ctx().clone();
    let active = app.session.active().and_then(|s| s.active_layer).map(|l| l.0);
    let contents = crate::layers_panel_ui::thumbnail_contents(app);
    let doc = app.session.active().map(|st| st.doc.clone());
    let thumbs = Thumbnails::<LayerThumb<LayerId>>::in_context(&ctx, "photocraft-layer-thumbs", ThumbnailOptions::new());

    enum Picked {
        Layer(u64),
        Isolate,
        Cancel,
    }

    let mut picked: Option<Picked> = None;
    let mut hovered_layer: Option<LayerId> = None;
    let mut open = true;

    let shown = Popover::new(tl!("Canvas menu").to_string(), Bind::bound(&mut open))
        .opened_by(opener)
        .overlay(|overlay| overlay.opener_anchor(Anchor::Point(Pos2::new(pos[0], pos[1]))))
        .dismiss(Dismiss::new().outside(Outside::Absorb))
        .initial_focus(Initial::First)
        .escape(true)
        .show(ui, |ui, _| {
            let env = Env::of(ui);
            let tokens = env.tokens.tokens();
            let text_normal = c32(tokens.color.text.normal);
            let text_muted = c32(tokens.color.text.muted);
            let text_on_strong = c32(tokens.color.text.on_strong);
            let accent_strong = c32(tokens.color.accent.strong);
            let accent_solid = c32(tokens.color.accent.solid);
            let accent_soft = c32(tokens.color.accent.soft);
            let line_divider = c32(tokens.color.line.divider);
            let line_focus = c32(tokens.color.line.focus);

            let body_size = ui.style().text_styles.get(&egui::TextStyle::Body).map_or(12.5, |f| f.size);
            let row_h = (body_size * 2.15).max(28.0);
            let thumb_side = (row_h - 6.0).clamp(20.0, 32.0);
            let indent_step = 14.0;
            let clip_w = 14.0;
            let check_w = 14.0;
            let pad_x = 8.0;
            let gap = 7.0;
            let font = FontId::proportional(body_size);
            let group_font = FontId::new(body_size, egui::FontFamily::Name("medium".into()));

            let doc_ref = doc.as_deref();
            let source = doc_ref.map(|d| PhotoThumbs::new(app, d));

            // Compute a clean natural width so rows align neatly and long names elide cleanly.
            let mut width: f32 = 200.0;
            for (i, (id, name)) in layers.iter().enumerate() {
                let level = f32::from(depth.get(i).copied().unwrap_or(0));
                let is_clip = clipped.get(i).copied().unwrap_or(false);
                let is_group = doc_ref.and_then(|d| d.layer(LayerId(*id))).is_some_and(|l| l.is_group() && l.frame.is_none());
                let f = if is_group { group_font.clone() } else { font.clone() };
                let text_w = ui.painter().layout_no_wrap(name.clone(), f, text_normal).size().x;
                let row_w = pad_x + check_w + level * indent_step + if is_clip { clip_w } else { 0.0 } + thumb_side + gap + text_w + pad_x + 8.0;
                width = width.max(row_w);
            }
            if isolate_row {
                let text_w = ui.painter().layout_no_wrap(tl!("Isolate Layers").to_string(), font.clone(), text_normal).size().x;
                width = width.max(pad_x + check_w + text_w + pad_x + 12.0);
            }
            width = width.clamp(200.0, 340.0);

            let total_rows = layers.len() + usize::from(isolate_row);
            let base_id = ui.scope_id().with("layer_pick_rows");
            let row_ids: Vec<egui::Id> = (0..total_rows).map(|i| base_id.with(i)).collect();

            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                picked = Some(Picked::Cancel);
            }

            // Keyboard arrow navigation across rows using egui focus.
            let focused_idx = row_ids.iter().position(|id| ui.ctx().memory(|m| m.has_focus(*id)));
            let move_delta = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    Some(1isize)
                } else if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    Some(-1isize)
                } else if i.consume_key(egui::Modifiers::NONE, Key::Home) {
                    Some(isize::MIN)
                } else if i.consume_key(egui::Modifiers::NONE, Key::End) {
                    Some(isize::MAX)
                } else {
                    None
                }
            });
            if let Some(delta) = move_delta
                && total_rows > 0
            {
                let next = match (focused_idx, delta) {
                    (_, isize::MIN) => 0,
                    (_, isize::MAX) => total_rows - 1,
                    (None, d) if d > 0 => 0,
                    (None, _) => total_rows - 1,
                    (Some(cur), d) => (cur as isize + d).rem_euclid(total_rows as isize) as usize,
                };
                if let Some(&target_id) = row_ids.get(next) {
                    ui.ctx().memory_mut(|m| m.request_focus(target_id));
                }
            }

            let activate_key = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter) || i.consume_key(egui::Modifiers::NONE, Key::Space));
            let natural_h = total_rows as f32 * (row_h + 2.0) + if isolate_row { 11.0 } else { 0.0 };
            const MAX_MENU_H: f32 = 420.0;

            let mut render_rows = |ui: &mut egui::Ui| {
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
                for (i, (id, name)) in layers.iter().enumerate() {
                    let level = usize::from(depth.get(i).copied().unwrap_or(0));
                    let is_clip = clipped.get(i).copied().unwrap_or(false);
                    let is_active = Some(*id) == active;
                    let layer = doc_ref.and_then(|d| d.layer(LayerId(*id)));
                    let is_group = layer.is_some_and(|l| l.is_group() && l.frame.is_none());

                    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    let resp = ui.interact(rect, row_ids[i], Sense::click());
                    resp.widget_info(|| {
                        let mut info = egui::WidgetInfo::labeled(egui::Role::MenuItem, true, name.clone());
                        info.selected = Some(is_active);
                        info
                    });

                    let focused = resp.has_focus();
                    let hovered = resp.hovered();
                    let lit = hovered || focused;
                    if lit {
                        hovered_layer = Some(LayerId(*id));
                    }

                    let painter = ui.painter();
                    if lit {
                        painter.rect_filled(rect, CornerRadius::same(4), accent_strong);
                    } else if is_active {
                        painter.rect_filled(rect, CornerRadius::same(4), accent_soft);
                    }
                    if focused {
                        painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, line_focus), StrokeKind::Inside);
                    }

                    let mut x = rect.min.x + pad_x;
                    // Active check mark column
                    let check_rect = Rect::from_min_size(pos2(x, rect.min.y), vec2(check_w, row_h));
                    if is_active {
                        let check_col = if lit { text_on_strong } else { accent_solid };
                        paint_check(painter, check_rect, check_col);
                    }
                    x += check_w;

                    // Subtle vertical hierarchy guide lines for nested levels
                    let guide_col = if lit { text_on_strong.gamma_multiply(0.35) } else { line_divider };
                    for d in 0..level {
                        let gx = x + d as f32 * indent_step + indent_step * 0.45;
                        painter.line_segment([pos2(gx, rect.min.y + 1.0), pos2(gx, rect.max.y - 1.0)], Stroke::new(1.0, guide_col));
                    }
                    x += level as f32 * indent_step;

                    // Clipping mask arrow (↳) for layers clipped to the base above
                    if is_clip {
                        let clip_rect = Rect::from_min_size(pos2(x, rect.min.y), vec2(clip_w, row_h));
                        let arrow_col = if lit { text_on_strong } else { text_muted };
                        paint_clip_arrow(painter, clip_rect, arrow_col);
                        x += clip_w;
                    }

                    // Layer thumbnail / kind icon
                    let thumb_y = rect.center().y - thumb_side * 0.5;
                    let thumb_rect = Rect::from_min_size(pos2(x, thumb_y), vec2(thumb_side, thumb_side));
                    if let Some(src) = &source {
                        let thumb_cx = ThumbDrawCx { ctx: &ctx, env: &env, thumbs: &thumbs, source: src, contents };
                        paint_layer_thumb(painter, &thumb_cx, layer, thumb_rect, lit);
                    }
                    x += thumb_side + gap;

                    // Layer name: single line with ellipsis so long names never wrap or clip vertically.
                    let text_col = if lit { text_on_strong } else { text_normal };
                    let text_font = if is_group { group_font.clone() } else { font.clone() };
                    let avail = (rect.max.x - pad_x - x).max(12.0);
                    let mut job = LayoutJob::single_section(name.clone(), TextFormat::simple(text_font, text_col));
                    job.wrap = TextWrapping { max_width: avail, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                    let galley = painter.layout_job(job);
                    let label_rect = Rect::from_min_max(pos2(x, rect.min.y), pos2(rect.max.x - pad_x, rect.max.y));
                    let text_pos = pos2(x, rect.center().y - galley.size().y * 0.5);
                    painter.with_clip_rect(label_rect).galley(text_pos, galley, text_col);

                    if resp.clicked() || (focused && activate_key) {
                        picked = Some(Picked::Layer(*id));
                    }
                }

                if isolate_row {
                    let (sep_rect, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                    let sy = sep_rect.center().y;
                    ui.painter().line_segment([pos2(sep_rect.min.x + pad_x, sy), pos2(sep_rect.max.x - pad_x, sy)], Stroke::new(1.0, line_divider));

                    let idx = layers.len();
                    let label = tl!("Isolate Layers").to_string();
                    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    let resp = ui.interact(rect, row_ids[idx], Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, true, label.clone()));
                    let focused = resp.has_focus();
                    let lit = resp.hovered() || focused;
                    let painter = ui.painter();
                    if lit {
                        painter.rect_filled(rect, CornerRadius::same(4), accent_strong);
                    }
                    if focused {
                        painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, line_focus), StrokeKind::Inside);
                    }
                    let text_col = if lit { text_on_strong } else { text_normal };
                    let x = rect.min.x + pad_x + check_w;
                    painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font.clone(), text_col);
                    if resp.clicked() || (focused && activate_key) {
                        picked = Some(Picked::Isolate);
                    }
                }
            };

            if natural_h > MAX_MENU_H {
                egui::ScrollArea::vertical().min_scrolled_height(MAX_MENU_H).max_height(MAX_MENU_H).show(ui, render_rows);
            } else {
                render_rows(ui);
            }
        });

    if let Some(pick) = picked {
        match pick {
            Picked::Layer(id) => choose(app, id),
            Picked::Isolate => isolate(app),
            Picked::Cancel => {
                app.ui.canvas_tool_menu = None;
                app.hover_layer = None;
            }
        }
    } else if !open || shown.output.closed.is_some() {
        app.ui.canvas_tool_menu = None;
        app.hover_layer = None;
    } else if hovered_layer != app.hover_layer {
        app.hover_layer = hovered_layer;
        ctx.request_repaint();
    }
}

#[cfg(test)]
#[path = "layer_pick_ui/frame_picker_tests.rs"]
mod frame_picker_tests;

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::*;
    use crate::canvas_tool_menu::MenuKind;

    /// 200 × 200: a white Background, "Red" (a square in the middle), and "Hidden" (all over,
    /// hidden). Returns the app and the (background, red, hidden) ids.
    fn app() -> (Box<PhotocraftApp>, u64, u64, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let bg = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
        app.run("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.new.layer", json!({"name": "Hidden"})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let hidden = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.setProps", json!({"layer": hidden, "visible": false})).unwrap();
        (app, bg, red, hidden)
    }

    fn layer_ids(app: &PhotocraftApp) -> Option<Vec<u64>> {
        match app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind) {
            Some(MenuKind::Layers { layers, .. }) => Some(layers.iter().map(|l| l.0).collect()),
            _ => None,
        }
    }

    #[test]
    fn lists_visible_layers_with_pixels_topmost_first() {
        let (mut a, bg, red, _) = app();
        assert_eq!(layers_at(&mut a, 100.0, 100.0).iter().map(|l| l.0).collect::<Vec<_>>(), [red, bg]);
        assert_eq!(layers_at(&mut a, 10.0, 10.0).iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["Background"]);
        choose(&mut a, red);
        assert_eq!(a.session.active().unwrap().active_layer.unwrap().0, red);
        assert!(a.ui.canvas_tool_menu.is_none());
    }

    #[test]
    fn nothing_there_or_bad_input_opens_nothing() {
        let (mut a, _, _, _) = app();
        let open = |app: &mut PhotocraftApp, x: f64, y: f64| crate::canvas_tool_menu::open_at(app, Tool::Move, Mods::NONE, pos2(0.0, 0.0), [x, y], None);
        for (x, y) in [(-5.0, 10.0), (500.0, 500.0), (f64::NAN, 1.0), (1.0, f64::INFINITY), (1e300, -1e300)] {
            assert!(layers_at(&mut a, x, y).is_empty(), "({x}, {y})");
        }
        for (x, y) in [(f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            assert!(!open(&mut a, x, y), "({x}, {y})");
            assert!(a.ui.canvas_tool_menu.is_none());
        }
        // Off the canvas a right-click lists no layers: it opens the pasteboard menu.
        for (x, y) in [(-5.0, 10.0), (500.0, 500.0), (1e300, -1e300)] {
            assert!(open(&mut a, x, y), "({x}, {y})");
            assert!(a.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::Pasteboard)), "({x}, {y})");
            a.ui.canvas_tool_menu = None;
        }
        // A transparent document: nothing to list.
        let mut b = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!open(&mut b, 1.0, 1.0), "no document");
        b.run("file.new", json!({"width": 20, "height": 20, "background": "transparent"})).unwrap();
        assert!(!open(&mut b, 1.0, 1.0));
        // Choosing a layer that no longer exists reports it and keeps the app running.
        choose(&mut a, 987_654);
        assert!(!a.ui.status.is_empty());
    }

    fn harness(a: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
        let mut a = a;
        a.sync_views();
        let mut h = Harness::builder().with_context(a.ctx().clone()).with_size(vec2(1000.0, 700.0)).build_ui_state(
            |ui, app: &mut Box<PhotocraftApp>| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
        h.run_steps(4);
        h
    }

    fn right_click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
    }

    /// ⌘/Ctrl+right-click beats the Erase preference: it lists the layers and erases nothing.
    #[test]
    fn erase_preference_does_not_take_the_command_right_click() {
        let (mut a, _, red, _) = app();
        a.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        a.ui.tool = Tool::Brush;
        let mut h = harness(a);
        let c = h.state().last_canvas_rect.center();
        right_click(&mut h, c, Modifiers::COMMAND);
        assert_eq!(layer_ids(h.state()).and_then(|l| l.first().copied()), Some(red));
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing erased");
    }

    /// `ui.inspect` reports `layerMenu.layers` (docs/control-protocol.md). The injected press is
    /// delivered on the next pass, so the harness runs frames after `control::handle`.
    #[test]
    fn agents_open_it_with_a_right_pointer_press() {
        let (mut a, _, red, _) = app();
        a.ui.tool = Tool::Move;
        let mut h = harness(a);
        let ctx = h.ctx.clone();
        let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"tool": "move", "button": "right", "events": events}));
        let _ = crate::control::handle(h.state_mut(), &ctx, &req);
        h.run_steps(4);
        assert_eq!(layer_ids(h.state()).and_then(|l| l.first().copied()), Some(red));
        let (req, _rx) = crate::control::ControlRequest::new("ui.inspect", Value::Null);
        let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("inspect") };
        assert_eq!(v["result"]["layerMenu"]["layers"][0]["name"], "Red");
    }

    /// Groups and clipping masks are displayed hierarchically: enclosing folders appear as parent
    /// rows, and clipping bases appear above their clipped children indented underneath.
    #[test]
    fn nested_rows_show_groups_and_clipping_hierarchy() {
        let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        a.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let bg = a.session.active().unwrap().active_layer.unwrap().0;

        a.run("layer.new.layer", json!({"name": "RectBase"})).unwrap();
        a.run("select.rect", json!({"x": 40, "y": 40, "width": 60, "height": 60})).unwrap();
        a.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        a.run("select.deselect", json!({})).unwrap();
        let base = a.session.active().unwrap().active_layer.unwrap().0;

        a.run("layer.new.layer", json!({"name": "ClippedImg"})).unwrap();
        a.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let clipped_img = a.session.active().unwrap().active_layer.unwrap().0;
        a.run("layer.setProps", json!({"layer": clipped_img, "clipped": true})).unwrap();

        a.run("layer.select", json!({"layer": base, "mode": "replace"})).unwrap();
        a.run("layer.select", json!({"layer": clipped_img, "mode": "add"})).unwrap();
        let group = a.run("layer.new.groupFromLayers", json!({"name": "Card"})).unwrap()["layer"].as_u64().unwrap();

        // Inside the clipping rect: Card -> RectBase -> ClippedImg (clipped), then Background.
        let hit_inside = layers_at(&mut a, 60.0, 60.0);
        let (rows, depth, clipped_flags) = nested_rows(&a, hit_inside);
        let ids: Vec<u64> = rows.iter().map(|(id, _)| *id).collect();
        let names: Vec<&str> = rows.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(ids, vec![group, base, clipped_img, bg]);
        assert_eq!(names, vec!["Card", "RectBase", "ClippedImg", "Background"]);
        assert_eq!(depth, vec![0, 1, 2, 0]);
        assert_eq!(clipped_flags, vec![false, false, true, false]);

        // Outside the clipping rect: ClippedImg is not hit, so only Background is listed.
        let hit_outside = layers_at(&mut a, 150.0, 150.0);
        let (rows_out, depth_out, clipped_out) = nested_rows(&a, hit_outside);
        assert_eq!(rows_out.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![bg]);
        assert_eq!(depth_out, vec![0]);
        assert_eq!(clipped_out, vec![false]);
    }

    /// Keyboard navigation in the layer picker moves real egui focus across rows and Enter selects
    /// the focused row; Escape closes the menu without changing the active layer.
    #[test]
    fn keyboard_navigation_focuses_and_selects_layer_menu_rows() {
        use egui_kittest::kittest::Queryable as _;

        let (mut a, bg, red, _) = app();
        a.ui.tool = Tool::Move;
        a.run("layer.select", json!({"layer": bg, "mode": "replace"})).unwrap();
        let mut h = harness(a);
        let c = h.state().last_canvas_rect.center();
        right_click(&mut h, c, Modifiers::NONE);
        assert_eq!(layer_ids(h.state()), Some(vec![red, bg]));

        // Both menu items are exposed in the accessibility tree with their visible labels.
        assert!(h.query_by_role_and_label(egui::Role::MenuItem, "Red").is_some(), "Red menu item in a11y tree");
        assert!(h.query_by_role_and_label(egui::Role::MenuItem, "Background").is_some(), "Background menu item in a11y tree");

        // Escape closes without changing the active layer.
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.canvas_tool_menu.is_none());
        assert_eq!(h.state().session.active().unwrap().active_layer.map(|l| l.0), Some(bg));

        // Reopen, move focus with ArrowDown to the second row (or first row then Enter) and select Red.
        right_click(&mut h, c, Modifiers::NONE);
        assert_eq!(layer_ids(h.state()), Some(vec![red, bg]));
        h.key_press(Key::Home);
        h.run_steps(1);
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert!(h.state().ui.canvas_tool_menu.is_none(), "Enter closes the menu");
        assert_eq!(h.state().session.active().unwrap().active_layer.map(|l| l.0), Some(red), "Enter selects the focused row");
    }
}
