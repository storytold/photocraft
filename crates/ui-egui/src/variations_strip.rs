//! The Variations strip (ShotoPhopEX, plan Phase 4): the candidates of the active document's
//! recent generative runs as tiles at the canvas edge, newest run first. Click a tile to show
//! that variation (one undo step, `generate.variation`), × to delete it
//! (`generate.dropVariation`), ✓ on the shown one to keep it and delete the others
//! (`generate.keepVariation`), ✦ in a run's header for more like the shown one
//! (`generate.similar`, which adds a run). A run still going shows placeholder tiles with its
//! percent; a failed one a red tile with the reason, Retry (`generate.recreate`) and × (dismiss).
//!
//! The runs come from the engine's Generations library (`Session::generations`): every
//! generative command on this document, whoever started it (the panel, the task bar, a dialog, a
//! script), with its result layers while they exist. Nothing here is saved; undo, delete and
//! close simply change what the library's layers still point at. Placement:
//! `integrations.variationsStrip` (`right`, `bottom`, `off`).

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::LayerId;
use photocraft_engine::generate_jobs::{GenFailureKind, GenStatus};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;

/// Runs shown per document at most.
pub const MAX_BUNDLES: usize = 5;
/// A tile's side (points).
pub const TILE: f32 = 56.0;
/// Thumbnails built per frame at most (each samples its layer once).
const BUILDS_PER_FRAME: usize = 4;
/// The commands whose results are variations of one request (a split or a matte is not).
const COMMANDS: [&str; 6] = ["generate.fill", "generate.edit", "generate.expand", "generate.image", "generate.similar", "generate.angles"];

/// One run of the active document.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    /// The Generations library row.
    pub record: u64,
    pub status: GenStatus,
    /// What made the layers (`generate.fill`, …; a Generate Similar's layers say what it ran).
    pub command: String,
    pub prompt: String,
    /// Its result layers that still exist, in order.
    pub layers: Vec<u64>,
    /// The visible one (an index into `layers`).
    pub shown: Option<usize>,
    /// Variations asked for (the placeholders of a running job).
    pub count: u32,
    pub progress: f32,
    pub failure: Option<(GenFailureKind, String)>,
    /// The area the run generated (document pixels), for the thumbnails.
    pub rect: Option<[i32; 4]>,
}

/// The strip's state in `UiState::variations`: caches and what the last frame drew. Never saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariationsUi {
    thumbs: TileThumbs,
    /// What the last frame drew, for `ui.inspect.variations`: (record, what, index, rect).
    drawn: Vec<(u64, &'static str, usize, Rect)>,
    /// The frame the strip was last drawn in.
    frame: Option<u64>,
}

/// Tile textures by layer id with the fingerprint they were built from. A cache: never compared.
#[derive(Clone, Default)]
struct TileThumbs(HashMap<u64, (u64, egui::TextureHandle)>);

impl PartialEq for TileThumbs {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for TileThumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TileThumbs({})", self.0.len())
    }
}

/// A click on the strip, applied after drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Show(u64, usize),
    Drop(u64, usize),
    Keep(u64, usize),
    Similar(u64),
    Retry(u64),
    Dismiss(u64),
    Cancel(u64),
}

/// The active document's runs, newest first (at most [`MAX_BUNDLES`]): running ones, finished
/// ones with at least one result layer left, failed ones not dismissed.
pub fn bundles(app: &PhotocraftApp) -> Vec<Bundle> {
    let Some(d) = app.session.active() else { return Vec::new() };
    let doc = d.doc.id.0;
    app.session
        .generations
        .records()
        .iter()
        .rev()
        .filter(|r| r.doc == Some(doc) && !r.dismissed)
        .filter_map(|r| {
            let command = r.info.as_ref().map(|i| i.command.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| r.command.clone());
            let recreating = r.command == photocraft_engine::generate_jobs::RECREATE && r.status == GenStatus::Running;
            if !COMMANDS.contains(&command.as_str()) && !recreating {
                return None;
            }
            let layers: Vec<u64> = match r.status {
                GenStatus::Done => {
                    let res = r.result.as_ref().filter(|x| !x.selection)?;
                    let left: Vec<u64> = res.layers.iter().copied().filter(|id| d.doc.layer(LayerId(*id)).is_some()).collect();
                    if left.is_empty() {
                        return None;
                    }
                    left
                }
                GenStatus::Cancelled => return None,
                GenStatus::Running | GenStatus::Failed => Vec::new(),
            };
            let shown = layers.iter().position(|id| d.doc.layer(LayerId(*id)).is_some_and(|l| l.visible));
            Some(Bundle {
                record: r.id,
                status: r.status,
                command,
                prompt: if r.prompt.is_empty() { r.label.clone() } else { r.prompt.clone() },
                layers,
                shown,
                count: r.count.clamp(1, 16),
                progress: r.progress,
                failure: r.failure.as_ref().map(|f| (f.kind, f.message.clone())),
                rect: r.info.as_ref().map(|i| i.rect),
            })
        })
        .take(MAX_BUNDLES)
        .collect()
}

fn place(app: &PhotocraftApp) -> &str {
    match app.session.prefs().integrations.variations_strip.as_str() {
        "bottom" => "bottom",
        "off" => "off",
        _ => "right",
    }
}

/// Draw the strip over the canvas (after the canvas, before dialogs).
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let side = place(app).to_string();
    let busy = app.ui.transform.is_some() || app.ui.text_edit.is_some();
    let list = if side == "off" || busy { Vec::new() } else { bundles(app) };
    let canvas = crate::rulers::content_rect(app, app.last_canvas_rect);
    if list.is_empty() || canvas.width() < 240.0 || canvas.height() < 160.0 {
        app.ui.variations.drawn.clear();
        return;
    }
    let t = Tokens::get(ctx);
    let vertical = side != "bottom";
    let margin = 8.0;
    // Right: a column inside the canvas's right edge (clear of its scroll bar); bottom: a row
    // along its bottom edge.
    let strip_w = TILE + 12.0;
    let strip_h = TILE + 34.0;
    let pos =
        if vertical { pos2(canvas.right() - strip_w - 18.0, canvas.top() + margin) } else { pos2(canvas.left() + margin, canvas.bottom() - strip_h - 18.0) };
    let mut action = None;
    let mut drawn = Vec::new();
    let mut builds = 0usize;
    egui::Area::new(egui::Id::unique("variations-strip")).order(egui::Order::Middle).fixed_pos(pos).constrain(false).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.card_border))
            .corner_radius(t.radius_lg)
            .shadow(ui.style().visuals.popup_shadow)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                if vertical {
                    egui::ScrollArea::vertical().max_height((canvas.height() - 2.0 * margin - 30.0).max(TILE)).show(ui, |ui| {
                        ui.set_width(TILE);
                        for (n, b) in list.iter().enumerate() {
                            if n > 0 {
                                ui.add_space(4.0);
                            }
                            ui.vertical(|ui| bundle_ui(app, ui, b, true, &t, &mut builds, &mut drawn, &mut action));
                        }
                    });
                } else {
                    egui::ScrollArea::horizontal().max_width((canvas.width() - 2.0 * margin - 30.0).max(TILE)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (n, b) in list.iter().enumerate() {
                                if n > 0 {
                                    ui.add_space(8.0);
                                }
                                ui.vertical(|ui| bundle_ui(app, ui, b, false, &t, &mut builds, &mut drawn, &mut action));
                            }
                        });
                    });
                }
            });
    });
    app.ui.variations.drawn = drawn;
    app.ui.variations.frame = Some(ctx.cumulative_frame_nr());
    if list.iter().any(|b| b.status == GenStatus::Running) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if let Some(a) = action {
        apply(app, a);
    }
}

/// One run: a header (which one is shown of how many, ✦ Similar / × Cancel) and its tiles.
#[allow(clippy::too_many_arguments)]
fn bundle_ui(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    b: &Bundle,
    vertical: bool,
    t: &Tokens,
    builds: &mut usize,
    drawn: &mut Vec<(u64, &'static str, usize, Rect)>,
    action: &mut Option<Action>,
) {
    // Header.
    let (hr, _) = ui.allocate_exact_size(vec2(TILE, 16.0), Sense::hover());
    let label = match b.status {
        GenStatus::Running => format!("{:.0}%", b.progress.clamp(0.0, 1.0) * 100.0),
        GenStatus::Failed => tl!("Failed").to_string(),
        _ => match b.shown {
            Some(i) => format!("{}/{}", i + 1, b.layers.len()),
            None => format!("–/{}", b.layers.len()),
        },
    };
    let color = if b.status == GenStatus::Failed { t.danger } else { t.text_dim };
    ui.painter().text(pos2(hr.left() + 1.0, hr.center().y), Align2::LEFT_CENTER, label, FontId::monospace(10.5), color);
    let br = Rect::from_center_size(pos2(hr.right() - 8.0, hr.center().y), vec2(16.0, 16.0));
    match b.status {
        GenStatus::Done => {
            let resp = ui.interact(br, ui.scope_id().with(("var-similar", b.record)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(br, t.radius_sm, t.hover);
            }
            icons::paint(ui, br, "sparkles", 11.0, if resp.hovered() { t.accent } else { t.text_dim });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Generate Similar")));
            if resp.on_hover_text(tl!("Generate Similar")).clicked() {
                *action = Some(Action::Similar(b.record));
            }
            drawn.push((b.record, "similar", 0, br));
        }
        GenStatus::Running => {
            let resp = ui.interact(br, ui.scope_id().with(("var-cancel", b.record)), Sense::click());
            icons::paint(ui, br, "x", 10.0, if resp.hovered() { t.text } else { t.text_dim });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Cancel")));
            if resp.on_hover_text(tl!("Cancel")).clicked() {
                *action = Some(Action::Cancel(b.record));
            }
        }
        _ => {}
    }
    let tiles = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
        if vertical {
            ui.vertical(|ui| f(ui));
        } else {
            ui.horizontal(|ui| f(ui));
        }
    };
    match b.status {
        GenStatus::Running => tiles(ui, &mut |ui| {
            for i in 0..b.count.min(4) as usize {
                let (r, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::hover());
                pending_tile(ui, r, b.progress, t);
                resp.on_hover_text(format!("{}\n{}", b.prompt, tl!("Generating…")));
                drawn.push((b.record, "pending", i, r));
            }
        }),
        GenStatus::Failed => tiles(ui, &mut |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::click());
            let p = ui.painter();
            p.rect_filled(r, t.radius_sm, t.danger.gamma_multiply(if resp.hovered() { 0.28 } else { 0.18 }));
            p.rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.danger.gamma_multiply(0.7)), StrokeKind::Inside);
            icons::paint(ui, Rect::from_center_size(r.center() - vec2(0.0, 6.0), vec2(18.0, 18.0)), "triangle-alert", 16.0, t.danger);
            icons::paint(ui, Rect::from_center_size(pos2(r.center().x, r.bottom() - 11.0), vec2(14.0, 14.0)), "rotate-cw", 11.0, t.text);
            let (kind, why) = match &b.failure {
                Some((k, m)) => (crate::generations_panel::failure_label(*k), m.clone()),
                None => (tl!("Failed"), String::new()),
            };
            let tip = if why.is_empty() {
                format!("{kind}\n{}\n{}", b.prompt, tl!("Click to retry"))
            } else {
                format!("{kind}: {why}\n{}\n{}", b.prompt, tl!("Click to retry"))
            };
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Retry")));
            if resp.on_hover_text(tip).clicked() {
                *action = Some(Action::Retry(b.record));
            }
            drawn.push((b.record, "retry", 0, r));
            let xr = Rect::from_min_size(pos2(r.right() - 16.0, r.top() + 2.0), vec2(14.0, 14.0));
            let xresp = ui.interact(xr, ui.scope_id().with(("var-dismiss", b.record)), Sense::click());
            icons::paint(ui, xr, "x", 10.0, if xresp.hovered() { t.text } else { t.text_dim });
            xresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Dismiss")));
            if xresp.on_hover_text(tl!("Dismiss")).clicked() {
                *action = Some(Action::Dismiss(b.record));
            }
            drawn.push((b.record, "dismiss", 0, xr));
        }),
        _ => tiles(ui, &mut |ui| {
            for (i, id) in b.layers.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::click());
                let shown = b.shown == Some(i);
                let p = ui.painter();
                p.rect_filled(r, t.radius_sm, t.field);
                if let Some(tex) = tile_texture(app, ui.ctx(), *id, b.rect, builds) {
                    let size = tex.size_vec2();
                    let s = ((TILE - 4.0) / size.x.max(1.0)).min((TILE - 4.0) / size.y.max(1.0));
                    let ir = Rect::from_center_size(r.center(), size * s);
                    ui.painter().image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                let ring = if shown {
                    Stroke::new(2.0, t.accent)
                } else if resp.hovered() {
                    Stroke::new(1.0, t.text_dim)
                } else {
                    Stroke::new(1.0, t.card_border)
                };
                ui.painter().rect_stroke(r, t.radius_sm, ring, StrokeKind::Inside);
                let name = crate::i18n::fmt(tl!("Variation {n} of {count}"), &[("n", &(i + 1).to_string()), ("count", &b.layers.len().to_string())]);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, shown, &name));
                let hovered = resp.hovered();
                if resp.on_hover_text(format!("{name}\n{}", b.prompt)).clicked() && !shown {
                    *action = Some(Action::Show(b.record, i));
                }
                drawn.push((b.record, "tile", i, r));
                // × (delete this one): on hover; always clickable so scripts can reach it.
                let xr = Rect::from_min_size(pos2(r.right() - 17.0, r.top() + 3.0), vec2(14.0, 14.0));
                let xresp = ui.interact(xr, ui.scope_id().with(("var-drop", b.record, i)), Sense::click());
                if hovered || xresp.hovered() {
                    ui.painter().circle_filled(xr.center(), 7.0, Color32::from_black_alpha(170));
                    icons::paint(ui, xr, "x", 9.0, Color32::WHITE);
                }
                xresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Delete this variation")));
                if xresp.on_hover_text(tl!("Delete this variation")).clicked() {
                    *action = Some(Action::Drop(b.record, i));
                }
                drawn.push((b.record, "drop", i, xr));
                // ✓ (keep this one, delete the others): on the shown one when there are others.
                if shown && b.layers.len() > 1 {
                    let kr = Rect::from_min_size(pos2(r.right() - 17.0, r.bottom() - 17.0), vec2(14.0, 14.0));
                    let kresp = ui.interact(kr, ui.scope_id().with(("var-keep", b.record)), Sense::click());
                    ui.painter().circle_filled(kr.center(), 7.0, if kresp.hovered() { t.accent } else { t.accent.gamma_multiply(0.85) });
                    icons::paint(ui, kr, "check", 9.0, Color32::WHITE);
                    kresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Keep this one")));
                    if kresp.on_hover_text(tl!("Keep this one and delete the other variations")).clicked() {
                        *action = Some(Action::Keep(b.record, i));
                    }
                    drawn.push((b.record, "keep", i, kr));
                }
            }
        }),
    }
}

/// A placeholder for a result on its way: a moving sheen and the percent.
fn pending_tile(ui: &egui::Ui, r: Rect, progress: f32, t: &Tokens) {
    let p = ui.painter();
    p.rect_filled(r, t.radius_sm, t.field);
    let phase = (ui.input(|i| i.time) * 0.7).fract() as f32;
    let band = Rect::from_min_size(pos2(r.left() + (phase * 1.6 - 0.6) * r.width(), r.top()), vec2(r.width() * 0.45, r.height()));
    p.with_clip_rect(r).rect_filled(band, 0.0, t.text.gamma_multiply(0.07));
    p.rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), StrokeKind::Inside);
    let f = progress.clamp(0.0, 1.0);
    let bar = Rect::from_min_size(pos2(r.left() + 5.0, r.bottom() - 8.0), vec2(r.width() - 10.0, 3.0));
    p.rect_filled(bar, 1.5, t.separator);
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 1.5, t.accent);
}

/// The tile picture of a result layer: its pixels over the run's area, built once per change of
/// its pixels and at most [`BUILDS_PER_FRAME`] per frame.
fn tile_texture(app: &mut PhotocraftApp, ctx: &egui::Context, id: u64, area: Option<[i32; 4]>, builds: &mut usize) -> Option<egui::TextureHandle> {
    let st = app.session.active()?;
    let layer = st.doc.layer(LayerId(id))?;
    let surface = layer.surface()?;
    let bounds = [0, 0, st.doc.size.width as i32, st.doc.size.height as i32];
    let [x, y, w, h] = area.filter(|a| a[2] > 0 && a[3] > 0).unwrap_or(bounds);
    // Inside the canvas.
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = (x.saturating_add(w).min(bounds[2]), y.saturating_add(h).min(bounds[3]));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let rev = crate::surface_fingerprint(surface) ^ ((x0 as u64) << 48) ^ ((y0 as u64) << 32) ^ ((x1 as u64) << 16) ^ (y1 as u64);
    if let Some((r, tex)) = app.ui.variations.thumbs.0.get(&id)
        && *r == rev
    {
        return Some(tex.clone());
    }
    if *builds >= BUILDS_PER_FRAME {
        ctx.request_repaint();
        return app.ui.variations.thumbs.0.get(&id).map(|(_, t)| t.clone());
    }
    *builds += 1;
    let (aw, ah) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let long = 2.0 * TILE;
    let scale = aw.max(ah) / long;
    let (tw, th) = ((aw / scale).round().max(1.0) as usize, (ah / scale).round().max(1.0) as usize);
    let n = surface.channels();
    let format = surface.format();
    let mut px = [0.0f32; 8];
    let mut out = vec![Color32::TRANSPARENT; tw * th];
    for ty in 0..th {
        for tx in 0..tw {
            let dx = x0 + ((tx as f32 + 0.5) * scale) as i32;
            let dy = y0 + ((ty as f32 + 0.5) * scale) as i32;
            surface.read_pixel(dx, dy, &mut px[..n]);
            let c = photocraft_raster::to_rgba(&format, &px[..n]);
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if let Some(o) = out.get_mut(ty * tw + tx) {
                *o = Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]));
            }
        }
    }
    let img = egui::ColorImage::new([tw, th], out);
    let tex = ctx.load_texture(format!("variation-{id}"), img, egui::TextureOptions::LINEAR);
    app.ui.variations.thumbs.0.insert(id, (rev, tex.clone()));
    // Forget layers no run points at any more.
    if app.ui.variations.thumbs.0.len() > 64 {
        let live: Vec<u64> = bundles(app).iter().flat_map(|b| b.layers.clone()).collect();
        app.ui.variations.thumbs.0.retain(|k, _| live.contains(k));
    }
    Some(tex)
}

/// Is the strip drawing job `id`'s placeholders (its percent and Cancel), so the modal progress
/// dialog is not needed?
pub fn shows_job(app: &PhotocraftApp, ctx: &egui::Context, id: photocraft_engine::jobs::JobId) -> bool {
    let v = &app.ui.variations;
    let recent = v.frame.is_some_and(|f| ctx.cumulative_frame_nr().saturating_sub(f) <= 2);
    recent && app.session.generations.record_for_job(id).is_some_and(|r| v.drawn.iter().any(|(rec, what, _, _)| *rec == r && *what == "pending"))
}

fn find(app: &PhotocraftApp, record: u64) -> Option<Bundle> {
    bundles(app).into_iter().find(|b| b.record == record)
}

/// Show variation `index` (0-based) of run `record`: one undo step.
pub fn choose(app: &mut PhotocraftApp, record: u64, index: usize) -> Result<Value, String> {
    let b = find(app, record).ok_or("that run has no variations left")?;
    let index = index.min(b.layers.len().saturating_sub(1));
    app.run("generate.variation", json!({"layers": b.layers, "index": index + 1}))
}

/// Delete variation `index` (0-based) of run `record`: one undo step.
pub fn drop_variation(app: &mut PhotocraftApp, record: u64, index: usize) -> Result<Value, String> {
    let b = find(app, record).ok_or("that run has no variations left")?;
    if index >= b.layers.len() {
        return Err("no such variation".into());
    }
    app.run("generate.dropVariation", json!({"layers": b.layers, "index": index + 1}))
}

/// Keep variation `index` (0-based) of run `record` and delete the others: one undo step.
pub fn keep(app: &mut PhotocraftApp, record: u64, index: usize) -> Result<Value, String> {
    let b = find(app, record).ok_or("that run has no variations left")?;
    if index >= b.layers.len() {
        return Err("no such variation".into());
    }
    app.run("generate.keepVariation", json!({"layers": b.layers, "index": index + 1}))
}

/// More like the shown variation of run `record` (as many as the run has): a new run.
pub fn similar(app: &mut PhotocraftApp, record: u64) -> Result<Value, String> {
    let b = find(app, record).ok_or("that run has no variations left")?;
    let layer = b.shown.and_then(|i| b.layers.get(i)).or(b.layers.first()).copied().ok_or("that run has no variations left")?;
    // Generate Similar works on the active layer.
    if app.session.active().and_then(|d| d.active_layer) != Some(LayerId(layer)) {
        app.run("layer.select", json!({"layer": layer}))?;
    }
    app.run("generate.similar", json!({"layer": layer, "variations": b.layers.len().clamp(1, 4)}))
}

fn apply(app: &mut PhotocraftApp, a: Action) {
    let r = match a {
        Action::Show(rec, i) => choose(app, rec, i),
        Action::Drop(rec, i) => drop_variation(app, rec, i),
        Action::Keep(rec, i) => keep(app, rec, i),
        Action::Similar(rec) => similar(app, rec),
        Action::Retry(rec) => app.run("generate.recreate", json!({"id": rec})),
        Action::Dismiss(rec) => app.run("generate.jobs.dismiss", json!({"id": rec})),
        Action::Cancel(rec) => {
            if let Some(job) = app.session.generations.job_for_record(rec) {
                crate::jobs_ui::cancel(app, job);
            }
            Ok(Value::Null)
        }
    };
    if let Err(e) = r {
        crate::notices::error(app, e);
    }
}

/// `ui.inspect.variations`: the placement, whether the strip is on screen, and the active
/// document's runs with what the last frame drew of them (rects in points, for `ui.click`).
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let v = &app.ui.variations;
    let visible = v.frame.is_some_and(|f| ctx.cumulative_frame_nr().saturating_sub(f) <= 2) && !v.drawn.is_empty();
    let runs: Vec<Value> = bundles(app)
        .into_iter()
        .map(|b| {
            let rects = |what: &str| -> Vec<Value> {
                v.drawn
                    .iter()
                    .filter(|(r, w, _, _)| *r == b.record && *w == what)
                    .map(|(_, _, i, r)| json!({"index": i, "rect": [r.left(), r.top(), r.width(), r.height()]}))
                    .collect()
            };
            json!({
                "record": b.record, "status": b.status.as_str(), "command": b.command, "prompt": b.prompt,
                "layers": b.layers, "shown": b.shown.map(|i| i + 1), "count": b.count, "progress": b.progress,
                "failure": b.failure.as_ref().map(|(k, m)| json!({"kind": k, "message": m})),
                "tiles": rects("tile"), "drop": rects("drop"), "keep": rects("keep"), "similar": rects("similar"),
                "pending": rects("pending"), "retry": rects("retry"), "dismiss": rects("dismiss"),
            })
        })
        .collect();
    json!({"place": place(app), "visible": visible, "runs": runs})
}

#[cfg(test)]
#[path = "variations_strip_tests.rs"]
mod tests;
