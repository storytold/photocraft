//! Generative Fills that haven't landed. Nothing waits for them: while one generates
//! the user selects the next area and starts another. Each one shows
//! - on the canvas: its area under a moving shimmer, with a running outline and a tag (generating,
//!   waiting its turn or failed, how long it has taken, Cancel, and Retry once it failed);
//! - in the Layers panel: a locked "Generating…" row where its layer will land;
//! - in the status bar: how many are generating, waiting and failed, and a list of them.
//!
//! A fill's area flashes once when its layer lands.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use photocraft_doc::{DocId, Layer, LayerId};
use photocraft_engine::genai_cmds::{self as gc, FillInfo};
use photocraft_engine::jobs::{JobEvent, JobOutcome};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;

/// Grid cells across the longer side of a fill's shimmering veil.
const VEIL_CELLS: u32 = 40;
/// How long a landed fill's area flashes, in seconds.
const FLASH_S: f64 = 0.8;
/// Length of the running outline's dashes, in points.
const DASH: f32 = 5.0;

/// What the shell keeps for fills in progress.
#[derive(Default)]
pub struct Pending {
    /// Each fill's outline and veil, made once from its selection.
    shapes: HashMap<u64, Arc<Shape>>,
    /// Landed fills' areas, flashing until the time given.
    flashes: Vec<(Arc<Shape>, f64)>,
    /// This frame's fills (the Layers panel asks once per row): pass number and list.
    frame: Option<(u64, Arc<Vec<FillInfo>>)>,
}

/// The fills that haven't landed, read once per frame.
fn fills_now(app: &mut PhotocraftApp, ctx: &egui::Context) -> Arc<Vec<FillInfo>> {
    let pass = ctx.cumulative_pass_nr();
    if let Some((p, fills)) = &app.genai.pending.frame
        && *p == pass
    {
        return Arc::clone(fills);
    }
    let fills = Arc::new(app.session.generative_fills());
    app.genai.pending.frame = Some((pass, Arc::clone(&fills)));
    fills
}

/// A fill's area as drawn.
struct Shape {
    doc: DocId,
    /// Outline segments, in document pixels.
    outline: Vec<crate::outline::Segment>,
    /// The selection at the veil's grid points, `(cols + 1) × (rows + 1)`, `cell` pixels apart
    /// from the bounds' corner.
    veil: Vec<f32>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    cols: usize,
    rows: usize,
    cell: f32,
}

impl Shape {
    fn new(doc: DocId, mask: &Surface, b: photocraft_geom::Rect) -> Shape {
        let outline = crate::outline::outline_scaled_serial(mask, b, crate::outline::bounded_step(b, 1));
        let cell = (b.width().max(b.height()).max(1) as f32 / VEIL_CELLS as f32).max(1.0);
        let cols = ((b.width() as f32 / cell).ceil() as usize).max(1);
        let rows = ((b.height() as f32 / cell).ceil() as usize).max(1);
        let (x0, y0, x1, y1) = (b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32);
        let mut veil = Vec::with_capacity((cols + 1) * (rows + 1));
        for j in 0..=rows {
            for i in 0..=cols {
                // The pixel at (or, on the far edges, just inside) the grid point.
                let x = (x0 + i as f32 * cell).min(x1 - 1.0) as i32;
                let y = (y0 + j as f32 * cell).min(y1 - 1.0) as i32;
                veil.push(mask.sample_channel(x, y, 0).clamp(0.0, 1.0));
            }
        }
        Shape { doc, outline, veil, x0, y0, x1, y1, cols, rows, cell }
    }

    fn point(&self, i: usize, j: usize) -> (f32, f32) {
        ((self.x0 + i as f32 * self.cell).min(self.x1), (self.y0 + j as f32 * self.cell).min(self.y1))
    }
}

/// A colour at `alpha` (0..1) of its own.
fn faded(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Does job command `id` belong to Generative Fill (its progress shows here, not in the status
/// bar's job readout)?
pub fn is_generative(id: &str) -> bool {
    [gc::FILL, gc::RETRY, gc::MORE].contains(&id)
}

fn shape(app: &mut PhotocraftApp, f: &FillInfo) -> Option<Arc<Shape>> {
    if let Some(s) = app.genai.pending.shapes.get(&f.id) {
        return Some(Arc::clone(s));
    }
    let mask = app.session.generative_fill_mask(f.id)?;
    let [x0, y0, x1, y1] = f.bounds;
    let b = photocraft_geom::Rect::new(x0, y0, x1, y1);
    if b.is_empty() {
        return None;
    }
    let s = Arc::new(Shape::new(f.document, mask, b));
    app.genai.pending.shapes.insert(f.id, Arc::clone(&s));
    Some(s)
}

/// The veil: the area tinted `color`, `base` strong, with a bright band sweeping across it when
/// `sweep`.
fn paint_veil(painter: &egui::Painter, xf: &ViewXform, s: &Shape, color: Color32, base: f32, sweep: Option<f64>) {
    let mut mesh = egui::Mesh::default();
    // The band crosses the area diagonally, then pauses off its edge.
    let band = sweep.map(|t| (t * 0.5) % 1.6 - 0.3);
    for j in 0..=s.rows {
        for i in 0..=s.cols {
            let (x, y) = s.point(i, j);
            let cov = s.veil.get(j * (s.cols + 1) + i).copied().unwrap_or(0.0);
            let u = (i as f32 / s.cols as f32 + j as f32 / s.rows as f32) / 2.0;
            let glow = band.map_or(0.0, |b| (-((u - b as f32) / 0.14).powi(2)).exp());
            mesh.colored_vertex(xf.to_screen(x, y), faded(color, cov * (base + 0.3 * glow)));
        }
    }
    let w = s.cols + 1;
    for j in 0..s.rows {
        for i in 0..s.cols {
            let k = j * w + i;
            let corners = [k, k + 1, k + w, k + w + 1];
            if corners.iter().all(|c| s.veil.get(*c).is_none_or(|v| *v <= 0.0)) {
                continue;
            }
            let [a, b, c, d] = corners.map(|c| c as u32);
            mesh.add_triangle(a, b, d);
            mesh.add_triangle(a, d, c);
        }
    }
    painter.add(mesh);
}

/// A 1-point line from `a` to `b` as a quad in `mesh`.
fn line(mesh: &mut egui::Mesh, a: Pos2, b: Pos2, width: f32, color: Color32) {
    let d = b - a;
    let len = d.length();
    if len <= 0.0 {
        return;
    }
    let side = vec2(-d.y, d.x) / len * (width / 2.0);
    let i = mesh.vertices.len() as u32;
    for p in [a + side, a - side, b - side, b + side] {
        mesh.colored_vertex(p, color);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// The outline in `color`, with white dashes running along it when `time` is given.
fn paint_outline(painter: &egui::Painter, xf: &ViewXform, s: &Shape, color: Color32, time: Option<f64>) {
    let clip = painter.clip_rect().expand(2.0);
    let mut mesh = egui::Mesh::default();
    let phase = time.map(|t| ((t * 18.0) % f64::from(DASH * 2.0)) as f32);
    for (a, b) in &s.outline {
        let (pa, pb) = (xf.to_screen(a[0] as f32, a[1] as f32), xf.to_screen(b[0] as f32, b[1] as f32));
        if !clip.intersects(Rect::from_two_pos(pa, pb)) {
            continue;
        }
        line(&mut mesh, pa, pb, 1.5, color);
        let Some(phase) = phase else { continue };
        let len = (pb - pa).length();
        if len <= 0.0 {
            continue;
        }
        let dir = (pb - pa) / len;
        // Dashes keep their place along x + y, so they run on across segments.
        let mut at = -(pa.x + pa.y - phase).rem_euclid(DASH * 2.0);
        while at < len {
            let (from, to) = (at.max(0.0), (at + DASH).min(len));
            if to > from {
                line(&mut mesh, pa + dir * from, pa + dir * to, 1.5, Color32::from_white_alpha(230));
            }
            at += DASH * 2.0;
        }
    }
    painter.add(mesh);
}

/// Seconds as the tags show them: "12 s", "1:05".
fn elapsed(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    if s < 60 { format!("{s} s") } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// What a fill is called: its prompt (or, with none, what an empty prompt does).
fn title(f: &FillInfo) -> String {
    if f.prompt.trim().is_empty() { tl!("No prompt: removes what is selected").to_owned() } else { f.prompt.clone() }
}

/// A tag's or a row's tooltip: the prompt and, for a failed fill, why.
fn hover(f: &FillInfo) -> String {
    match &f.error {
        Some(e) => format!("{}\n{}", title(f), crate::i18n::fmt(tl!("Generative Fill failed: {error}"), &[("error", e)])),
        None => title(f),
    }
}

/// What a fill is doing, briefly ("Generating", "Waiting (2)", "Failed").
fn state_text(f: &FillInfo) -> String {
    match f.state {
        "failed" => tl!("Failed").to_owned(),
        "queued" => crate::i18n::fmt(tl!("Waiting ({n})"), &[("n", &f.queue_position.to_string())]),
        _ => tl!("Generating").to_owned(),
    }
}

/// The spinner, clock or warning sign before a fill's state.
fn state_icon(ui: &mut egui::Ui, f: &FillInfo, size: f32, t: &Tokens) {
    match f.state {
        "running" => {
            ui.add(egui::Spinner::new().size(size).color(t.accent));
        }
        state => {
            let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
            let (icon, tint) = if state == "failed" { ("triangle-alert", t.danger) } else { ("clock", t.text_dim) };
            crate::icons::paint(ui, r, icon, size, tint);
        }
    }
}

/// Retry and Cancel (or Discard, once it failed) for fill `f`, as icon buttons, Cancel last
/// (`reversed`: in a right-to-left layout, so it is added first).
fn buttons(ui: &mut egui::Ui, f: &FillInfo, size: f32, reversed: bool, actions: &mut Vec<(&'static str, Value)>) {
    let retry = |ui: &mut egui::Ui, actions: &mut Vec<(&'static str, Value)>| {
        if f.state == "failed" && crate::icons::button(ui, "rotate-cw", size, false, tl!("Retry")).clicked() {
            actions.push((gc::RETRY, json!({"fill": f.id})));
        }
    };
    if !reversed {
        retry(ui, actions);
    }
    let tip = if f.state == "failed" { tl!("Discard") } else { tl!("Cancel this Generative Fill") };
    if crate::icons::button(ui, "x", size, false, tip).clicked() {
        actions.push((gc::DISCARD, json!({"fill": f.id})));
    }
    if reversed {
        retry(ui, actions);
    }
}

fn run_actions(app: &mut PhotocraftApp, actions: Vec<(&'static str, Value)>) {
    for (id, params) in actions {
        if let Err(e) = app.run(id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Over the canvas of document `doc`: each fill's veil, outline and tag, and the flashes of those
/// that just landed.
pub fn paint_canvas(app: &mut PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, doc: DocId) {
    let all = app.session.generative_fills();
    let now = ctx.input(|i| i.time);
    app.genai.pending.flashes.retain(|(_, until)| *until > now);
    app.genai.pending.shapes.retain(|id, _| all.iter().any(|f| f.id == *id));
    let fills: Vec<FillInfo> = all.into_iter().filter(|f| f.document == doc).collect();
    if fills.is_empty() && app.genai.pending.flashes.iter().all(|(s, _)| s.doc != doc) {
        return;
    }
    let t = Tokens::get(ctx);
    for f in &fills {
        let Some(s) = shape(app, f) else { continue };
        match f.state {
            "running" => {
                paint_veil(painter, xf, &s, t.accent, 0.16, Some(now));
                paint_outline(painter, xf, &s, t.accent, Some(now));
            }
            "queued" => {
                paint_veil(painter, xf, &s, t.accent, 0.08, None);
                paint_outline(painter, xf, &s, t.text_dim, Some(now));
            }
            _ => {
                paint_veil(painter, xf, &s, t.danger, 0.14, None);
                paint_outline(painter, xf, &s, t.danger, None);
            }
        }
    }
    for (s, until) in app.genai.pending.flashes.iter().filter(|(s, _)| s.doc == doc) {
        let k = ((until - now) / FLASH_S).clamp(0.0, 1.0) as f32;
        paint_veil(painter, xf, s, Color32::WHITE, 0.45 * k, None);
    }
    // The tags would cover a dialog opened over the canvas (the next fill's prompt).
    if app.ui.dialogs.is_empty() {
        let mut actions = Vec::new();
        for f in &fills {
            tag(ctx, painter.clip_rect(), xf, f, &t, &mut actions);
        }
        run_actions(app, actions);
    }
    // The shimmer, the running outlines and the flashes move; failed fills alone don't.
    if fills.iter().any(|f| f.state != "failed") || !app.genai.pending.flashes.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// A fill's tag, just above its area's top-left corner (inside the canvas).
fn tag(ctx: &egui::Context, canvas: Rect, xf: &ViewXform, f: &FillInfo, t: &Tokens, actions: &mut Vec<(&'static str, Value)>) {
    let [x0, y0, x1, y1] = f.bounds.map(|v| v as f32);
    let corners = [xf.to_screen(x0, y0), xf.to_screen(x1, y0), xf.to_screen(x0, y1), xf.to_screen(x1, y1)];
    let mut area = Rect::NOTHING;
    for c in corners {
        area.extend_with(c);
    }
    if !canvas.intersects(area) {
        return;
    }
    const H: f32 = 24.0;
    let above = area.top() - H - 4.0;
    let y = if above >= canvas.top() + 2.0 { above } else { (area.top() + 4.0).max(canvas.top() + 2.0) };
    let x = area.left().clamp(canvas.left() + 4.0, (canvas.right() - 180.0).max(canvas.left() + 4.0));
    let border = if f.state == "failed" { t.danger } else { t.card_border };
    egui::Area::new(egui::Id::unique(("genai-fill-tag", f.id))).order(egui::Order::Middle).fixed_pos(pos2(x, y)).constrain(false).show(ctx, |ui| {
        let frame = egui::Frame::new().fill(t.card).stroke(Stroke::new(1.0, border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(6, 2));
        frame
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.horizontal(|ui| {
                    state_icon(ui, f, 12.0, t);
                    let color = if f.state == "failed" { t.danger } else { t.text };
                    ui.label(RichText::new(state_text(f)).size(11.5).color(color));
                    if f.state != "failed" {
                        ui.label(RichText::new(elapsed(f.elapsed_ms)).size(11.0).color(t.text_faint));
                    }
                    buttons(ui, f, 18.0, false, actions);
                });
            })
            .response
            .on_hover_text(hover(f));
    });
}

/// Show `f`'s area: its document comes forward and the view centres on it.
fn go_to(app: &mut PhotocraftApp, f: &FillInfo) {
    let Some(i) = app.session.documents().iter().position(|st| st.doc.id == f.document) else { return };
    if app.session.active_index() != Some(i) {
        let _ = app.run("document.activate", json!({"document": i}));
    }
    let [x0, y0, x1, y1] = f.bounds;
    if let Some(v) = app.ui.views.get_mut(i) {
        v.center = [(x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0];
    }
}

/// The Layers panel's rows for fills on `doc` whose layer lands right above layer `above`, drawn
/// just before its row: oldest first, since each lands right above it, under the earlier ones.
pub fn layer_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: DocId, above: LayerId, depth: usize) {
    let all = fills_now(app, ui.ctx());
    if all.is_empty() {
        return;
    }
    let fills: Vec<FillInfo> = all.iter().filter(|f| f.document == doc && f.anchor == Some(above)).cloned().collect();
    rows(app, ui, &fills, depth);
}

/// The rows of fills on `doc` whose layer isn't among the rows shown (`shown`): none was active,
/// it was deleted, or it is in a closed group. They go at the top.
pub fn layer_rows_unplaced(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: DocId, shown: &[(usize, &Layer)]) {
    let all = fills_now(app, ui.ctx());
    let fills: Vec<FillInfo> = all.iter().filter(|f| f.document == doc && f.anchor.is_none_or(|a| !shown.iter().any(|(_, l)| l.id == a))).cloned().collect();
    rows(app, ui, &fills, 0);
}

fn rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, fills: &[FillInfo], depth: usize) {
    if fills.is_empty() {
        return;
    }
    let mut actions = Vec::new();
    let mut go = None;
    for f in fills {
        if layer_row(ui, f, depth, &mut actions) {
            go = Some(f.clone());
        }
    }
    if let Some(f) = go {
        go_to(app, &f);
    }
    run_actions(app, actions);
}

/// A "Generating…" row: where the fill's layer will land, locked until it does. Clicking it shows
/// the area. True when clicked.
fn layer_row(ui: &mut egui::Ui, f: &FillInfo, depth: usize, actions: &mut Vec<(&'static str, Value)>) -> bool {
    let t = Tokens::get(ui.ctx());
    let row_h = if t.pro { 32.0 } else { 46.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let painter = ui.painter_at(rect.expand(1.0));
    let failed = f.state == "failed";
    let tint = if failed { t.danger } else { t.accent };
    painter.rect_filled(rect, 0.0, faded(tint, 0.08));
    if f.state == "running" {
        // A soft band runs along the row while it generates.
        let time = ui.input(|i| i.time);
        let cx = rect.left() + ((time * 0.4) % 1.0) as f32 * rect.width();
        let band = Rect::from_min_max(pos2((cx - 40.0).max(rect.left()), rect.top()), pos2((cx + 40.0).min(rect.right()), rect.bottom()));
        painter.rect_filled(band, 0.0, faded(tint, 0.08));
    }
    if t.pro {
        painter.line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    }
    let ts = if t.pro { 24.0 } else { 34.0 };
    let x = rect.left() + 34.0 + crate::layer_row_ui::indent(depth, rect.width(), 34.0 + ts + 90.0);
    let thumb = Rect::from_min_size(pos2(x, rect.center().y - ts / 2.0), vec2(ts, ts));
    painter.rect_filled(thumb, t.radius_sm, t.field);
    painter.rect_stroke(thumb, t.radius_sm, Stroke::new(1.0, faded(tint, 0.6)), egui::StrokeKind::Inside);
    let mut inner =
        ui.new_child(egui::UiBuilder::new().max_rect(thumb.shrink(ts * 0.22)).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)));
    state_icon(&mut inner, f, ts * 0.5, &t);
    // Lock, then the buttons, from the right.
    let mut right = rect.right() - crate::layer_row_ui::RIGHT_PAD;
    let mut button = |icon: &str, tip: &str| {
        let r = Rect::from_center_size(pos2(right - 9.0, rect.center().y), vec2(18.0, 18.0));
        right -= 22.0;
        let resp = ui.interact(r, ui.scope_id().with(("genai-row", f.id, icon.to_owned())), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, t.radius_sm, t.hover);
        }
        crate::icons::paint(ui, r, icon, 12.0, if resp.hovered() { t.text } else { t.text_dim });
        resp.on_hover_text(tip).clicked()
    };
    if button("x", if failed { tl!("Discard") } else { tl!("Cancel this Generative Fill") }) {
        actions.push((gc::DISCARD, json!({"fill": f.id})));
    }
    if failed && button("rotate-cw", tl!("Retry")) {
        actions.push((gc::RETRY, json!({"fill": f.id})));
    }
    let lock = Rect::from_center_size(pos2(right - 9.0, rect.center().y), vec2(14.0, 14.0));
    crate::icons::paint(ui, lock, "lock", 11.0, t.text_faint);
    right -= 20.0;
    let name = match f.state {
        "failed" => crate::i18n::fmt(tl!("Failed: {name}"), &[("name", &f.name)]),
        "queued" => crate::i18n::fmt(tl!("Waiting: {name}"), &[("name", &f.name)]),
        _ => crate::i18n::fmt(tl!("Generating: {name}"), &[("name", &f.name)]),
    };
    let name_rect = Rect::from_min_max(pos2(thumb.right() + 8.0, rect.top()), pos2(right, rect.bottom()));
    let font = egui::FontId::proportional(if t.pro { 12.0 } else { 13.0 });
    let name_label = name.clone();
    let galley = painter.layout(name, font, if failed { t.danger } else { t.text_dim }, name_rect.width().max(10.0));
    let y = rect.center().y - galley.size().y.min(row_h) / 2.0;
    ui.painter().with_clip_rect(name_rect).galley(pos2(name_rect.left(), y), galley, t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &name_label));
    resp.on_hover_text(format!("{}\n{}", hover(f), tl!("Locked until its layer lands"))).clicked()
}

/// The status bar's readout: how many fills are generating, waiting and failed; click for the
/// list.
pub fn status(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let fills = app.session.generative_fills();
    let more = app.session.jobs().iter().filter(|j| j.command == gc::MORE).count();
    if fills.is_empty() && more == 0 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let count = |state: &str| fills.iter().filter(|f| f.state == state).count() as u64;
    let (running, queued, failed) = (count("running") + more as u64, count("queued"), count("failed"));
    let lang = crate::i18n::current();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push(crate::i18n::trn(lang, running, "{n} generating", "{n} generating"));
    }
    if queued > 0 {
        parts.push(crate::i18n::trn(lang, queued, "{n} waiting", "{n} waiting"));
    }
    if failed > 0 {
        parts.push(crate::i18n::trn(lang, failed, "{n} failed", "{n} failed"));
    }
    ui.add_space(6.0);
    // The status bar reads from the far end: the readout, then what it is doing.
    let color = if failed > 0 && running + queued == 0 { t.danger } else { t.text_dim };
    let resp = ui
        .add(egui::Label::new(RichText::new(parts.join(" · ")).size(11.5).color(color)).sense(Sense::click()))
        .on_hover_text(tl!("Generative Fills in progress"));
    if running > 0 {
        ui.add(egui::Spinner::new().size(11.0).color(t.accent));
    } else if failed > 0 {
        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        crate::icons::paint(ui, r, "triangle-alert", 12.0, t.danger);
    }
    egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| list(app, ui, &fills));
}

/// The status bar's list: each fill with its state, Go to area, Retry and Cancel or Discard.
fn list(app: &mut PhotocraftApp, ui: &mut egui::Ui, fills: &[FillInfo]) {
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(380.0);
    ui.label(RichText::new(tl!("Generative Fill")).font(crate::theme::semibold(12.5)).color(t.text));
    let active = app.session.active().map(|st| st.doc.id);
    let mut actions = Vec::new();
    let mut go = None;
    for f in fills {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            state_icon(ui, f, 12.0, &t);
            let mut what = title(f);
            if what.chars().count() > 40 {
                what = format!("{}…", what.chars().take(39).collect::<String>());
            }
            if active != Some(f.document)
                && let Some(st) = app.session.documents().iter().find(|st| st.doc.id == f.document)
            {
                what = format!("{what} ({})", st.doc.name);
            }
            ui.label(RichText::new(what).size(12.0).color(t.text)).on_hover_text(hover(f));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                buttons(ui, f, 20.0, true, &mut actions);
                if crate::icons::button(ui, "search", 20.0, false, tl!("Go to area")).clicked() {
                    go = Some(f.clone());
                }
                let detail = if f.state == "running" { format!("{} · {}", state_text(f), elapsed(f.elapsed_ms)) } else { state_text(f) };
                ui.label(RichText::new(detail).size(11.0).color(if f.state == "failed" { t.danger } else { t.text_faint }));
            });
        });
    }
    let more = app.session.jobs().into_iter().filter(|j| j.command == gc::MORE).count();
    if more > 0 {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
            ui.label(RichText::new(tl!("Generate More")).size(12.0).color(t.text));
        });
    }
    if let Some(f) = go {
        go_to(app, &f);
    }
    run_actions(app, actions);
}

/// A Generative Fill job ended: its layer landed (the area flashes), or it failed (its tag says
/// so and offers Retry), or it was cancelled. True when the event was Generative Fill's.
pub fn on_job_event(app: &mut PhotocraftApp, ctx: &egui::Context, e: &JobEvent) -> bool {
    if !is_generative(&e.command) {
        return false;
    }
    app.session.tidy_generative_fills();
    let failed_and_kept = |app: &PhotocraftApp| app.session.generative_fills().iter().any(|f| f.job == Some(e.id) && f.state == "failed");
    match &e.outcome {
        JobOutcome::Done(v) => {
            app.sync_views();
            let fill = v.get("fill").and_then(Value::as_u64);
            if let Some(s) = fill.and_then(|id| app.genai.pending.shapes.remove(&id)) {
                let now = ctx.input(|i| i.time);
                app.genai.pending.flashes.push((s, now + FLASH_S));
            }
            let name = v
                .get("layer")
                .and_then(Value::as_u64)
                .and_then(|id| app.session.documents().iter().find_map(|st| st.doc.layer(LayerId(id))).map(|l| l.name.clone()));
            app.ui.status = match name {
                Some(n) => crate::i18n::fmt(tl!("Generated {name}"), &[("name", &n)]),
                None => tl!("Generative Fill").to_owned(),
            };
            app.ui.status_error = false;
        }
        JobOutcome::Failed(err) => {
            app.sync_views();
            let msg = crate::i18n::fmt(tl!("Generative Fill failed: {error}"), &[("error", err)]);
            if failed_and_kept(app) {
                // Its tag and row say so and offer Retry.
                app.ui.status = msg;
                app.ui.status_error = true;
            } else {
                crate::notices::error(app, msg);
            }
        }
        JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", tl!(&e.label))]);
            app.ui.status_error = false;
        }
    }
    true
}
