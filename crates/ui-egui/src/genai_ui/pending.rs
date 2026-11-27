//! Generative Fills that haven't landed. Nothing waits for them: while one generates the user
//! selects the next area and starts another. Each one shows on the canvas: its area under a moving
//! shimmer, with a running outline and a tag (generating, waiting its turn, waiting for the
//! person's OK to send it, or failed; how long it has taken, Cancel, Retry once it failed, Review…
//! while it waits for consent). A fill's area flashes once when its layer lands.

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::IconId;
use craft::imaging::trace::{MaskRect, Segment};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _};
use craft::ui::runtime::env::Label;
use egui::{Color32, Pos2, Rect, RichText, Stroke, pos2, vec2};
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::genai::AiErrorKind;
use photocraft_engine::genai_cmds::{self as gc, FillInfo};
use photocraft_engine::jobs::{JobEvent, JobOutcome};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Grid cells across the longer side of a fill's shimmering veil.
const VEIL_CELLS: u32 = 40;
/// How long a landed fill's area flashes, in seconds.
const FLASH_S: f64 = 0.8;
/// Length of the running outline's dashes, in points.
const DASH: f32 = 5.0;
/// Most outline segments across the longer side of a fill's area: the trace steps over larger
/// areas so a big fill stays cheap to draw.
const OUTLINE_CELLS: u32 = 1024;

/// What the shell keeps for fills in progress.
#[derive(Default)]
pub struct Pending {
    /// Each fill's outline and veil, made once from its selection.
    shapes: HashMap<u64, Arc<Shape>>,
    /// Landed fills' areas, flashing until the time given.
    flashes: Vec<(Arc<Shape>, f64)>,
}

/// A fill's area as drawn.
struct Shape {
    doc: DocId,
    /// Outline segments, in document pixels.
    outline: Vec<Segment>,
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
    /// A fill's outline and veil grid, from its selection within bounds `b` of a document `size`.
    fn new(doc: DocId, mask: &Surface, b: photocraft_geom::Rect, size: [u32; 2]) -> Shape {
        let step = b.width().max(b.height()).max(1).div_ceil(OUTLINE_CELLS).max(1);
        let source = crate::canvas::outline::SurfaceMask::new(mask, mask.generation(), size);
        let outline = craft::imaging::trace::outline(&source, MaskRect::new(b.x0, b.y0, b.x1, b.y1), step);
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

/// Document pixels to screen points through the canvas's view.
fn to_screen(view: &CanvasView, x: f32, y: f32) -> Pos2 {
    view.to_screen(craft::behavior::kurbo::Point::new(f64::from(x), f64::from(y)))
}

/// A colour at `alpha` (0..1) of its own.
fn faded(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Does job command `id` belong to Generative Fill (its progress shows on the canvas, not in the
/// status bar's job readout)?
pub fn is_generative(id: &str) -> bool {
    [gc::FILL, gc::RETRY, gc::MORE, gc::AGREE].contains(&id)
}

/// Shell actions a tag asks for besides commands.
const REVIEW: &str = "genai.review";
const SETUP: &str = "genai.setup";

/// A failure as the person reads it: the shared layer's message, except where its guide says
/// to put it plainly (a busy service, an internal fault).
pub(super) fn error_text(f: &FillInfo) -> Option<String> {
    let e = f.error.as_deref()?;
    Some(match f.error_kind {
        Some(AiErrorKind::RateLimited) => tl!("The AI service is busy. Try again shortly.").to_owned(),
        Some(AiErrorKind::Internal) => crate::i18n::fmt(tl!("Something went wrong in the shared AI layer: {error}"), &[("error", e)]),
        _ => e.to_owned(),
    })
}

/// A failure that needs the setup (a refused key, a model this app can't run).
fn needs_setup(f: &FillInfo) -> bool {
    matches!(f.error_kind, Some(AiErrorKind::Auth | AiErrorKind::Unsupported))
}

/// Fill `f`'s shape as drawn, made once and kept.
fn shape(app: &mut PhotocraftApp, f: &FillInfo) -> Option<Arc<Shape>> {
    if let Some(s) = app.genai.pending.shapes.get(&f.id) {
        return Some(Arc::clone(s));
    }
    let size = app.session.documents().iter().find(|st| st.doc.id == f.document).map(|st| [st.doc.size.width, st.doc.size.height])?;
    let mask = app.session.generative_fill_mask(f.id)?;
    let [x0, y0, x1, y1] = f.bounds;
    let b = photocraft_geom::Rect::new(x0, y0, x1, y1);
    if b.is_empty() {
        return None;
    }
    let s = Arc::new(Shape::new(f.document, mask, b, size));
    app.genai.pending.shapes.insert(f.id, Arc::clone(&s));
    Some(s)
}

/// The veil: the area tinted `color`, `base` strong, with a bright band sweeping across it when
/// `sweep`.
fn paint_veil(painter: &egui::Painter, view: &CanvasView, s: &Shape, color: Color32, base: f32, sweep: Option<f64>) {
    let mut mesh = egui::Mesh::default();
    // The band crosses the area diagonally, then pauses off its edge.
    let band = sweep.map(|t| (t * 0.5) % 1.6 - 0.3);
    for j in 0..=s.rows {
        for i in 0..=s.cols {
            let (x, y) = s.point(i, j);
            let cov = s.veil.get(j * (s.cols + 1) + i).copied().unwrap_or(0.0);
            let u = (i as f32 / s.cols as f32 + j as f32 / s.rows as f32) / 2.0;
            let glow = band.map_or(0.0, |b| (-((u - b as f32) / 0.14).powi(2)).exp());
            mesh.colored_vertex(to_screen(view, x, y), faded(color, cov * (base + 0.3 * glow)));
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

/// A line `width` points wide from `a` to `b` as a quad in `mesh`.
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
fn paint_outline(painter: &egui::Painter, view: &CanvasView, s: &Shape, color: Color32, time: Option<f64>) {
    let clip = painter.clip_rect().expand(2.0);
    let mut mesh = egui::Mesh::default();
    let phase = time.map(|t| ((t * 18.0) % f64::from(DASH * 2.0)) as f32);
    for (a, b) in &s.outline {
        let (pa, pb) = (to_screen(view, a[0] as f32, a[1] as f32), to_screen(view, b[0] as f32, b[1] as f32));
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

/// A tag's tooltip: the prompt and, for a failed fill, why.
fn hover(f: &FillInfo) -> String {
    match error_text(f) {
        Some(e) => format!("{}\n{}", title(f), crate::i18n::fmt(tl!("Generative Fill failed: {error}"), &[("error", &e)])),
        None => title(f),
    }
}

/// What a fill is doing, briefly ("Generating", "Waiting (2)", "Failed").
fn state_text(f: &FillInfo) -> String {
    match f.state {
        "failed" => tl!("Failed").to_owned(),
        "consent" => tl!("Needs your OK").to_owned(),
        "queued" => crate::i18n::fmt(tl!("Waiting ({n})"), &[("n", &f.queue_position.to_string())]),
        _ => tl!("Generating").to_owned(),
    }
}

/// An icon button of a tag.
fn icon_button(ui: &mut egui::Ui, icon: &'static str, tip: &str) -> bool {
    IconButton::new(IconId::new(icon)).label(Label::from(tip.to_owned())).tooltip(tip.to_owned()).show(ui).clicked()
}

/// Set Up, Retry or Review…, then Cancel (or Discard, once it failed) for fill `f`.
fn buttons(ui: &mut egui::Ui, f: &FillInfo, actions: &mut Vec<(&'static str, Value)>) {
    if f.state == "failed" && needs_setup(f) && icon_button(ui, "settings", tl!("Set Up Generative AI…")) {
        actions.push((SETUP, Value::Null));
    }
    if f.state == "failed" && icon_button(ui, "rotate_cw", tl!("Retry")) {
        actions.push((gc::RETRY, json!({"fill": f.id})));
    }
    if f.state == "consent" && icon_button(ui, "hand", tl!("Review…")) {
        actions.push((REVIEW, Value::Null));
    }
    let tip = if matches!(f.state, "failed" | "consent") { tl!("Discard") } else { tl!("Cancel this Generative Fill") };
    if icon_button(ui, "x", tip) {
        actions.push((gc::DISCARD, json!({"fill": f.id})));
    }
}

/// Run what the tags asked for: commands, the consent prompt or the setup.
fn run_actions(app: &mut PhotocraftApp, actions: Vec<(&'static str, Value)>) {
    for (id, params) in actions {
        if id == REVIEW {
            super::consent::ask_if_waiting(app);
            continue;
        }
        if id == SETUP {
            super::open_setup(app);
            continue;
        }
        if let Err(e) = app.run(id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Over the canvas `rect` of the active document: each fill's veil, outline and tag, and the
/// flashes of those that just landed.
pub fn paint_canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, view: CanvasView) {
    let Some(doc) = app.session.active().map(|st| st.doc.id) else { return };
    let ctx = ui.ctx().clone();
    let all = app.session.generative_fills();
    let now = ctx.input(|i| i.time);
    app.genai.pending.flashes.retain(|(_, until)| *until > now);
    app.genai.pending.shapes.retain(|id, _| all.iter().any(|f| f.id == *id));
    let fills: Vec<FillInfo> = all.into_iter().filter(|f| f.document == doc).collect();
    if fills.is_empty() && app.genai.pending.flashes.iter().all(|(s, _)| s.doc != doc) {
        return;
    }
    let t = Tokens::get(&ctx);
    let painter = ui.painter().with_clip_rect(rect);
    for f in &fills {
        let Some(s) = shape(app, f) else { continue };
        match f.state {
            "running" => {
                paint_veil(&painter, &view, &s, t.accent, 0.16, Some(now));
                paint_outline(&painter, &view, &s, t.accent, Some(now));
            }
            "queued" => {
                paint_veil(&painter, &view, &s, t.accent, 0.08, None);
                paint_outline(&painter, &view, &s, t.text_dim, Some(now));
            }
            "consent" => {
                paint_veil(&painter, &view, &s, t.accent, 0.08, None);
                paint_outline(&painter, &view, &s, t.accent, None);
            }
            _ => {
                paint_veil(&painter, &view, &s, t.danger, 0.14, None);
                paint_outline(&painter, &view, &s, t.danger, None);
            }
        }
    }
    for (s, until) in app.genai.pending.flashes.iter().filter(|(s, _)| s.doc == doc) {
        let k = ((until - now) / FLASH_S).clamp(0.0, 1.0) as f32;
        paint_veil(&painter, &view, s, Color32::WHITE, 0.45 * k, None);
    }
    // The tags would cover a dialog opened over the canvas (the next fill's prompt).
    if app.framework.dialogs.stack().ids().next().is_none() {
        let mut actions = Vec::new();
        for f in &fills {
            tag(&ctx, rect, &view, f, &t, &mut actions);
        }
        run_actions(app, actions);
    }
    // The shimmer, the running outlines and the flashes move; failed and waiting fills don't.
    if fills.iter().any(|f| !matches!(f.state, "failed" | "consent")) || !app.genai.pending.flashes.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// A fill's tag, just above its area's top-left corner (inside the canvas).
fn tag(ctx: &egui::Context, canvas: Rect, view: &CanvasView, f: &FillInfo, t: &Tokens, actions: &mut Vec<(&'static str, Value)>) {
    let [x0, y0, x1, y1] = f.bounds.map(|v| v as f32);
    let corners = [to_screen(view, x0, y0), to_screen(view, x1, y0), to_screen(view, x0, y1), to_screen(view, x1, y1)];
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
                    if f.state == "running" {
                        ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                    }
                    let color = if f.state == "failed" { t.danger } else { t.text };
                    ui.label(RichText::new(state_text(f)).size(11.5).color(color));
                    if !matches!(f.state, "failed" | "consent") {
                        ui.label(RichText::new(elapsed(f.elapsed_ms)).size(11.0).color(t.text_faint));
                    }
                    buttons(ui, f, actions);
                });
            })
            .response
            .on_hover_text(hover(f));
    });
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
            let done = match name {
                Some(n) => crate::i18n::fmt(tl!("Generated {name}"), &[("name", &n)]),
                None => tl!("Generative Fill").to_owned(),
            };
            // Colour matching skipped is said, not left for the person to notice.
            app.ui.status = match super::skipped_note(v.get("matching")) {
                Some(note) => format!("{done} · {note}"),
                None => done,
            };
            app.ui.status_error = false;
        }
        JobOutcome::Failed(err) => {
            app.sync_views();
            // Waiting for the person's agreement is not a failure: ask.
            if app.session.generative_fills().iter().any(|f| f.job == Some(e.id) && f.state == "consent") {
                super::consent::ask_if_waiting(app);
                return true;
            }
            let shown = app.session.generative_fills().iter().find(|f| f.job == Some(e.id)).and_then(error_text).unwrap_or_else(|| err.clone());
            let msg = crate::i18n::fmt(tl!("Generative Fill failed: {error}"), &[("error", &shown)]);
            if failed_and_kept(app) {
                // Its tag says so and offers Retry.
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
