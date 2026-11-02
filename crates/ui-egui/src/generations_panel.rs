//! Window › Generations (ShotoPhopEX): every generative job of this and earlier sessions, as a
//! feed you can walk away from. Running jobs show their progress and a Cancel button, failed ones
//! a reason the user can act on with Recreate, completed ones a thumbnail with Place, Similar,
//! Save and Copy prompt on hover. Clicking a result selects its layer while its document is open
//! and opens it as a new document otherwise.
//!
//! The rows come straight from the engine's library (`Session::generations`, see
//! `photocraft_engine::generate_jobs`); every action is an engine command (`generate.place`,
//! `generate.recreate`, `generate.export`, `generate.jobs.dismiss` / `.pin` / `.clear`,
//! `layer.select`), so automation sees the same thing through `generate.jobs`, `ui.inspect`
//! (`generations`) and `ui.set` (`generationsView`).

use std::collections::{BTreeSet, HashMap};

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_engine::generate_jobs::{GenFailureKind, GenStatus, GenerationRecord};
use photocraft_engine::jobs::{JobEvent, JobOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;
use crate::widgets;

/// Rows drawn at most (the library keeps up to 500 results; older ones stay reachable through
/// `generate.jobs`).
const MAX_ROWS: usize = 200;
/// Thumbnails decoded per frame at most, so a long list never stalls a frame.
const DECODES_PER_FRAME: usize = 6;
/// How often the panel looks for rows another process (the CLI) added (seconds).
const REFRESH_EVERY: f64 = 1.0;
/// Below this content height (points) the header, a section label and one grid row don't fit:
/// the panel shows its compact strip instead.
const COMPACT_BELOW: f32 = 140.0;

/// Grid of thumbnails or a list of rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenView {
    #[default]
    Grid,
    List,
}

impl GenView {
    fn key(self) -> &'static str {
        match self {
            GenView::Grid => "grid",
            GenView::List => "list",
        }
    }
}

/// The panel's state in `UiState::generations`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationsUi {
    pub view: GenView,
    /// The result opened last (badged "Last viewed").
    pub last_viewed: Option<u64>,
    /// Results that finished while the panel was hidden (the rail badge counts them).
    #[serde(skip)]
    pub unread: BTreeSet<u64>,
    /// A "Clear …" waiting for confirmation: completed, failed or all.
    #[serde(skip)]
    pub confirm: Option<String>,
    #[serde(skip)]
    pub thumbs: Thumbs,
    /// When the panel last looked for another process's rows (egui time, seconds).
    #[serde(skip)]
    pub last_refresh: f64,
    /// The panel was drawn this frame or the last (finished results are not "unread" then).
    #[serde(skip)]
    pub visible_frame: u64,
    /// Drawn as the compact strip last frame (the group was too short for the feed).
    #[serde(skip)]
    pub compact: bool,
    /// Select mode: clicks pick results for Save selected instead of opening them.
    #[serde(skip)]
    pub selecting: bool,
    #[serde(skip)]
    pub selected: BTreeSet<u64>,
    /// The last completion alert this session played (`done` / `failed`), for inspect and tests.
    #[serde(skip)]
    pub last_alert: Option<&'static str>,
}

/// Thumbnail textures by row id (`None` = the file could not be read). A cache: never compared,
/// never saved.
#[derive(Clone, Default)]
pub struct Thumbs(HashMap<u64, Option<egui::TextureHandle>>);

impl PartialEq for Thumbs {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Thumbs({})", self.0.len())
    }
}

impl Thumbs {
    pub fn len(&self) -> usize {
        self.0.values().filter(|t| t.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What a row needs to be drawn (copied out of the library each frame).
#[derive(Clone, Debug)]
struct Row {
    id: u64,
    status: GenStatus,
    title: String,
    template: String,
    progress: f32,
    message: String,
    failure: Option<(GenFailureKind, String)>,
    size: (u32, u32),
    results: usize,
    thumbnail: Option<std::path::PathBuf>,
    selection: bool,
    pinned: bool,
}

/// A click in the panel, applied after drawing.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Open(u64),
    Place(u64, &'static str),
    Recreate(u64),
    Save(u64),
    CopyPrompt(u64),
    Dismiss(u64),
    Pin(u64, bool),
    Cancel(u64),
    DismissFailed,
    AskClear(&'static str),
    View(GenView),
    /// Select mode on (with this result picked) or off.
    StartSelect(Option<u64>),
    EndSelect,
    Toggle(u64),
    SaveSelected,
}

fn rows(app: &PhotocraftApp) -> Vec<Row> {
    let store = &app.session.generations;
    store
        .records()
        .iter()
        .rev()
        .filter(|r| !r.dismissed)
        .take(MAX_ROWS)
        .map(|r| {
            let res = r.result.as_ref();
            Row {
                id: r.id,
                status: r.status,
                title: title_of(r),
                template: r.template.clone(),
                progress: r.progress,
                message: r.message.clone(),
                failure: r.failure.as_ref().map(|f| (f.kind, f.message.clone())),
                size: res.map_or((0, 0), |x| (x.width, x.height)),
                results: res.map_or(0, |x| x.files.len().max(x.layers.len())),
                thumbnail: res.filter(|x| !x.thumbnail.is_empty()).and_then(|x| store.dir().map(|d| d.join(&x.thumbnail))),
                selection: res.is_some_and(|x| x.selection),
                pinned: r.pinned,
            }
        })
        .collect()
}

/// The document index and first result layer of row `id`, while that document is open.
fn open_layer(app: &PhotocraftApp, id: u64) -> Option<(usize, u64)> {
    let r = app.session.generations.get(id)?;
    let docs = app.session.documents();
    let i = docs.iter().position(|st| Some(st.doc.id.0) == r.doc)?;
    let l = r.result.as_ref()?.layers.first().copied()?;
    docs.get(i)?.doc.layer(photocraft_doc::LayerId(l)).map(|_| (i, l))
}

/// The prompt, else the job's label.
fn title_of(r: &GenerationRecord) -> String {
    if r.prompt.is_empty() { r.label.clone() } else { r.prompt.clone() }
}

/// The translated label of a failure kind.
pub fn failure_label(kind: GenFailureKind) -> &'static str {
    match kind {
        GenFailureKind::ServerUnreachable => tl!("Server unreachable"),
        GenFailureKind::ModelFilesMissing => tl!("Model files missing"),
        GenFailureKind::OutOfMemory => tl!("Out of memory on the server"),
        GenFailureKind::ServerError => tl!("Server error"),
        GenFailureKind::Timeout => tl!("Timed out"),
        GenFailureKind::BadInput => tl!("Invalid request"),
        GenFailureKind::Discarded => tl!("Result discarded"),
        GenFailureKind::Interrupted => tl!("Interrupted"),
        GenFailureKind::Other => tl!("Failed"),
    }
}

/// The dock group's body.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let now = ui.ctx().input(|i| i.time);
    if now - app.ui.generations.last_refresh >= REFRESH_EVERY || now < app.ui.generations.last_refresh {
        app.session.generations.refresh();
        app.ui.generations.last_refresh = now;
    }
    if app.session.generations.is_dirty() {
        // The index is written on a one-second debounce from `poll_jobs`: keep a frame coming.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1100));
    }
    app.ui.generations.visible_frame = app.frame;
    app.ui.generations.unread.clear();
    let rows = rows(app);
    prune_thumbs(app, &rows);
    for r in rows.iter().filter(|r| r.status != GenStatus::Running) {
        crate::gen_preview::forget_row(ui.ctx(), r.id);
    }
    let mut action: Option<Action> = None;
    let mut decodes = DECODES_PER_FRAME;
    // A group squeezed to its compact height (a short window) gets one strip of small tiles
    // instead of a header and sections that would not fit.
    if ui.available_height() < COMPACT_BELOW && !rows.is_empty() {
        app.ui.generations.compact = true;
        strip(app, ui, &rows, &t, &mut decodes, &mut action);
        finish(app, ui.ctx(), decodes, action);
        return;
    }
    app.ui.generations.compact = false;

    header(app, ui, &rows, &t, &mut action);
    widgets::hairline(ui);
    let running: Vec<&Row> = rows.iter().filter(|r| r.status == GenStatus::Running).collect();
    let failed: Vec<&Row> = rows.iter().filter(|r| matches!(r.status, GenStatus::Failed | GenStatus::Cancelled)).collect();
    let done: Vec<&Row> = rows.iter().filter(|r| r.status == GenStatus::Done).collect();
    egui::ScrollArea::vertical().id_salt("generations-rows").auto_shrink([false, false]).show(ui, |ui| {
        if rows.is_empty() {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                let (icon, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                icons::paint(ui, icon, "grid-2x2", 22.0, t.text_faint);
                ui.label(egui::RichText::new(tl!("No generations yet")).color(t.text_dim));
                ui.label(egui::RichText::new(tl!("Generative Fill, Edit and Generate Image results appear here.")).small().color(t.text_faint));
            });
            return;
        }
        if !running.is_empty() {
            widgets::section_label(ui, tl!("In progress"));
            for r in &running {
                let preview = crate::gen_preview::row_texture(app, ui.ctx(), r.id);
                running_row(ui, r, &t, preview.as_ref(), &mut action);
            }
            ui.add_space(4.0);
        }
        if !failed.is_empty() {
            widgets::section_label(ui, tl!("Failed"));
            for r in &failed {
                failed_row(ui, r, &t, &mut action);
            }
            ui.add_space(4.0);
        }
        if !done.is_empty() {
            widgets::section_label(ui, tl!("Completed"));
            match app.ui.generations.view {
                GenView::Grid => grid(app, ui, &done, &t, &mut decodes, &mut action),
                GenView::List => list(app, ui, &done, &t, &mut decodes, &mut action),
            }
        }
    });
    finish(app, ui.ctx(), decodes, action);
}

/// After drawing: keep decoding thumbnails, show the Clear confirmation, apply the click.
fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, decodes: usize, action: Option<Action>) {
    if decodes == 0 {
        ctx.request_repaint();
    }
    confirm_clear(app, ctx);
    if let Some(a) = action {
        apply(app, ctx, a);
    }
}

/// The overflow menu (dismiss failed, clear …).
fn more_menu(ui: &mut egui::Ui, action: &mut Option<Action>) {
    let more = icons::button(ui, "ellipsis", 22.0, false, tl!("More"));
    egui::Popup::menu(&more).show(|ui| {
        if ui.button(tl!("Dismiss failed")).clicked() {
            *action = Some(Action::DismissFailed);
        }
        ui.separator();
        if ui.button(tl!("Clear completed…")).clicked() {
            *action = Some(Action::AskClear("completed"));
        }
        if ui.button(tl!("Clear failed…")).clicked() {
            *action = Some(Action::AskClear("failed"));
        }
        if ui.button(tl!("Remove all…")).clicked() {
            *action = Some(Action::AskClear("all"));
        }
    });
}

/// The compact form: every row as a small square tile in one horizontal strip, newest first
/// (running ones with their progress, failed ones with a warning sign, results as thumbnails),
/// the overflow menu at the end. Tooltips and context menus carry the rest.
fn strip(app: &mut PhotocraftApp, ui: &mut egui::Ui, rows: &[Row], t: &Tokens, decodes: &mut usize, action: &mut Option<Action>) {
    let h = ui.available_height().clamp(24.0, COMPACT_BELOW);
    let side = (h - 4.0).max(20.0);
    let step = side + 4.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let w = (ui.available_width() - 26.0).max(step);
        ui.allocate_ui(vec2(w, h), |ui| {
            egui::ScrollArea::horizontal().id_salt("generations-strip").auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let left = ui.cursor().left();
                    let clip = ui.clip_rect();
                    let first = (((clip.left() - left) / step).floor().max(0.0) as usize).min(rows.len());
                    let last = ((((clip.right() - left) / step).ceil().max(0.0) as usize).saturating_add(1)).min(rows.len());
                    ui.add_space(first as f32 * step);
                    for r in rows.iter().skip(first).take(last.saturating_sub(first)) {
                        let (slot, _) = ui.allocate_exact_size(vec2(step, side), Sense::hover());
                        let cell = Rect::from_min_size(slot.min, vec2(side, side));
                        match r.status {
                            GenStatus::Done => card(app, ui, cell, r, t, decodes, action, false),
                            _ => status_tile(ui, cell, r, t, action),
                        }
                    }
                    ui.add_space(rows.len().saturating_sub(last) as f32 * step);
                });
            });
        });
        more_menu(ui, action);
    });
}

/// A running or failed row as a strip tile.
fn status_tile(ui: &mut egui::Ui, cell: Rect, r: &Row, t: &Tokens, action: &mut Option<Action>) {
    let resp = ui.interact(cell, ui.scope_id().with(("gen-tile", r.id)), Sense::click());
    let p = ui.painter();
    p.rect_filled(cell, 3.0, t.hover.gamma_multiply(if resp.hovered() { 1.0 } else { 0.6 }));
    let id = r.id;
    if r.status == GenStatus::Running {
        let f = r.progress.clamp(0.0, 1.0);
        let bar = Rect::from_min_size(pos2(cell.left() + 3.0, cell.bottom() - 6.0), vec2(cell.width() - 6.0, 3.0));
        p.rect_filled(bar, 1.5, t.separator);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 1.5, t.accent);
        p.text(cell.center() - vec2(0.0, 3.0), Align2::CENTER_CENTER, format!("{:.0}%", f * 100.0), FontId::proportional(10.0), t.text);
        resp.on_hover_text(format!("{}\n{}", r.title, r.message)).context_menu(|ui| {
            if ui.button(tl!("Cancel")).clicked() {
                *action = Some(Action::Cancel(id));
            }
        });
        return;
    }
    let (label, why) = match (&r.failure, r.status) {
        (_, GenStatus::Cancelled) => (tl!("Cancelled"), String::new()),
        (Some((k, m)), _) => (failure_label(*k), m.clone()),
        (None, _) => (tl!("Failed"), r.message.clone()),
    };
    let color = if r.status == GenStatus::Cancelled { t.text_faint } else { t.danger };
    p.rect_stroke(cell, 3.0, Stroke::new(1.0, color.gamma_multiply(0.7)), StrokeKind::Inside);
    icons::paint(ui, cell, "triangle-alert", (cell.width() * 0.4).clamp(10.0, 20.0), color);
    let tip = if why.is_empty() { format!("{label}\n{}", r.title) } else { format!("{label}: {why}\n{}", r.title) };
    let resp = resp.on_hover_text(tip);
    if resp.double_clicked() {
        *action = Some(Action::Recreate(id));
    }
    resp.context_menu(|ui| {
        if ui.button(tl!("Recreate")).clicked() {
            *action = Some(Action::Recreate(id));
        }
        if ui.button(tl!("Dismiss")).clicked() {
            *action = Some(Action::Dismiss(id));
        }
    });
}

fn header(app: &mut PhotocraftApp, ui: &mut egui::Ui, rows: &[Row], t: &Tokens, action: &mut Option<Action>) {
    let running = rows.iter().filter(|r| r.status == GenStatus::Running).count();
    let done = rows.iter().filter(|r| r.status == GenStatus::Done).count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if app.ui.generations.selecting {
            let n = app.ui.generations.selected.len();
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("{n} selected"), &[("n", &n.to_string())])).small().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if labeled(icons::button(ui, "x", 22.0, false, tl!("Done")), tl!("Done")).clicked() {
                    *action = Some(Action::EndSelect);
                }
                let save =
                    ui.add_enabled_ui(n > 0, |ui| labeled(icons::button(ui, "folder-open", 22.0, false, tl!("Save selected")), tl!("Save selected"))).inner;
                if save.clicked() {
                    *action = Some(Action::SaveSelected);
                }
            });
            return;
        }
        let summary = if running > 0 {
            crate::i18n::fmt(tl!("{running} running · {done} results"), &[("running", &running.to_string()), ("done", &done.to_string())])
        } else {
            crate::i18n::fmt(tl!("{done} results"), &[("done", &done.to_string())])
        };
        ui.label(egui::RichText::new(summary).small().color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            more_menu(ui, action);
            if done > 0 && labeled(icons::button(ui, "check", 22.0, false, tl!("Select")), tl!("Select")).clicked() {
                *action = Some(Action::StartSelect(None));
            }
            let view = app.ui.generations.view;
            if icons::button(ui, "align-justify", 22.0, view == GenView::List, tl!("List")).clicked() {
                *action = Some(Action::View(GenView::List));
            }
            if icons::button(ui, "grid-2x2", 22.0, view == GenView::Grid, tl!("Grid")).clicked() {
                *action = Some(Action::View(GenView::Grid));
            }
        });
    });
}

/// An icon button with its name for screen readers (and tests).
fn labeled(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, resp.enabled(), label));
    resp
}

/// A running generation: title, progress, Cancel and, while the run shows pictures of itself
/// (`integrations.previewWhileGenerating`), the latest one as a small thumbnail at the left.
fn running_row(ui: &mut egui::Ui, r: &Row, t: &Tokens, preview: Option<&egui::TextureHandle>, action: &mut Option<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect.shrink2(vec2(0.0, 2.0)), t.radius_sm, t.hover.gamma_multiply(0.6));
    // The thumbnail fits a 34-px square, keeping the picture's aspect.
    let left = match preview {
        Some(tex) => {
            let [w, h] = tex.size().map(|v| v.max(1) as f32);
            let s = 34.0 / w.max(h);
            let cell = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 6.0), vec2(34.0, 34.0));
            p.image(tex.id(), Rect::from_center_size(cell.center(), vec2(w * s, h * s)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            rect.left() + 46.0
        }
        None => rect.left() + 8.0,
    };
    // Real progress, as a band under the text.
    let bar = Rect::from_min_size(pos2(left, rect.bottom() - 9.0), vec2((rect.right() - 36.0 - left).max(1.0), 3.0));
    p.rect_filled(bar, 1.5, t.separator);
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * r.progress.clamp(0.0, 1.0), bar.height())), 1.5, t.accent);
    let pct = format!("{:.0} %", r.progress.clamp(0.0, 1.0) * 100.0);
    let chars = if preview.is_some() { 40 } else { 46 };
    p.text(pos2(left, rect.top() + 12.0), Align2::LEFT_CENTER, clip(&r.title, chars), FontId::proportional(12.0), t.text);
    let sub = if r.message.is_empty() { pct } else { format!("{pct} · {}", r.message) };
    p.text(pos2(left, rect.top() + 27.0), Align2::LEFT_CENTER, clip(&sub, chars + 6), FontId::proportional(10.5), t.text_faint);
    let cancel = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y - 2.0), vec2(22.0, 22.0));
    let c = ui.interact(cancel, ui.scope_id().with(("gen-cancel", r.id)), Sense::click());
    let tint = icons::button_chrome(ui, cancel, false, c.hovered());
    icons::paint(ui, cancel, "x", 12.0, tint);
    if c.on_hover_text(tl!("Cancel")).clicked() {
        *action = Some(Action::Cancel(r.id));
    }
    resp.on_hover_text(&r.title);
}

fn failed_row(ui: &mut egui::Ui, r: &Row, t: &Tokens, action: &mut Option<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
    }
    let (label, why) = match (&r.failure, r.status) {
        (_, GenStatus::Cancelled) => (tl!("Cancelled"), String::new()),
        (Some((k, m)), _) => (failure_label(*k), m.clone()),
        (None, _) => (tl!("Failed"), r.message.clone()),
    };
    let color = if r.status == GenStatus::Cancelled { t.text_faint } else { t.danger };
    icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(16.0, 16.0)), "triangle-alert", 13.0, color);
    p.text(pos2(rect.left() + 28.0, rect.top() + 12.0), Align2::LEFT_CENTER, label, FontId::proportional(12.0), color);
    p.text(pos2(rect.left() + 28.0, rect.top() + 27.0), Align2::LEFT_CENTER, clip(&r.title, 40), FontId::proportional(10.5), t.text_faint);
    let mut x = rect.right() - 14.0;
    for (icon, tip, a) in [("x", tl!("Dismiss"), Action::Dismiss(r.id)), ("rotate-cw", tl!("Recreate"), Action::Recreate(r.id))] {
        let b = Rect::from_center_size(pos2(x, rect.center().y), vec2(22.0, 22.0));
        let c = ui.interact(b, ui.scope_id().with(("gen-failed", r.id, icon)), Sense::click());
        let tint = icons::button_chrome(ui, b, false, c.hovered());
        icons::paint(ui, b, icon, 12.0, tint);
        if c.on_hover_text(tip).clicked() {
            *action = Some(a);
        }
        x -= 24.0;
    }
    if !why.is_empty() {
        resp.on_hover_text(why);
    }
}

/// The lines (of `line_h` points each, starting at the cursor) that intersect the visible part
/// of the scroll area: only those are laid out, the rest is empty space (a 500-row library costs
/// what a screenful does).
fn visible_lines(ui: &egui::Ui, line_h: f32, lines: usize) -> std::ops::Range<usize> {
    let top = ui.cursor().top();
    let clip = ui.clip_rect();
    let first = ((clip.top() - top) / line_h).floor().max(0.0) as usize;
    let last = (((clip.bottom() - top) / line_h).ceil().max(0.0) as usize).saturating_add(1);
    first.min(lines)..last.min(lines)
}

fn grid(app: &mut PhotocraftApp, ui: &mut egui::Ui, done: &[&Row], t: &Tokens, decodes: &mut usize, action: &mut Option<Action>) {
    let gap = 4.0;
    let w = ui.available_width().max(60.0);
    let cols = ((w + gap) / (92.0 + gap)).floor().max(1.0) as usize;
    let side = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let lines = done.len().div_ceil(cols);
    let line_h = side + gap;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let shown = visible_lines(ui, line_h, lines);
        ui.add_space(shown.start as f32 * line_h);
        for line in shown.clone() {
            let (rect, _) = ui.allocate_exact_size(vec2(w, line_h), Sense::hover());
            for (k, r) in done.iter().skip(line * cols).take(cols).enumerate() {
                let cell = Rect::from_min_size(pos2(rect.left() + k as f32 * (side + gap), rect.top()), vec2(side, side));
                card(app, ui, cell, r, t, decodes, action, true);
            }
        }
        ui.add_space((lines - shown.end) as f32 * line_h);
    });
}

fn list(app: &mut PhotocraftApp, ui: &mut egui::Ui, done: &[&Row], t: &Tokens, decodes: &mut usize, action: &mut Option<Action>) {
    let line_h = 56.0;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let shown = visible_lines(ui, line_h, done.len());
        ui.add_space(shown.start as f32 * line_h);
        for r in done.iter().skip(shown.start).take(shown.len()) {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), line_h), Sense::hover());
            let thumb = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 4.0), vec2(48.0, 48.0));
            card(app, ui, thumb, r, t, decodes, action, false);
            let p = ui.painter();
            let x = thumb.right() + 8.0;
            p.text(pos2(x, rect.top() + 16.0), Align2::LEFT_CENTER, clip(&r.title, 34), FontId::proportional(12.0), t.text);
            let mut sub = r.template.clone();
            if r.size.0 > 0 {
                sub = format!("{sub} · {}×{}", r.size.0, r.size.1);
            }
            if r.results > 1 {
                sub = format!("{sub} · {}", r.results);
            }
            p.text(pos2(x, rect.top() + 34.0), Align2::LEFT_CENTER, clip(&sub, 40), FontId::proportional(10.5), t.text_faint);
        }
        ui.add_space((done.len() - shown.end) as f32 * line_h);
    });
}

/// A completed result's tile at `cell`: the thumbnail, badges, and (on hover, in the grid) the
/// action buttons. Click opens it; right-click shows every action.
#[allow(clippy::too_many_arguments)]
fn card(app: &mut PhotocraftApp, ui: &mut egui::Ui, cell: Rect, r: &Row, t: &Tokens, decodes: &mut usize, action: &mut Option<Action>, overlay: bool) {
    let resp = ui.interact(cell, ui.scope_id().with(("gen-card", r.id)), Sense::click());
    if !ui.is_rect_visible(cell) {
        return;
    }
    let p = ui.painter().clone();
    widgets::checker(&p, cell, 6.0);
    match thumb(app, ui.ctx(), r, decodes) {
        Some(tex) => {
            let s = tex.size_vec2();
            let k = (cell.width() / s.x.max(1.0)).min(cell.height() / s.y.max(1.0));
            let img = Rect::from_center_size(cell.center(), s * k);
            p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let icon = if r.selection { "square-dashed" } else { "image" };
            icons::paint(ui, cell, icon, (cell.width() * 0.3).clamp(12.0, 28.0), t.text_faint);
        }
    }
    let selecting = app.ui.generations.selecting;
    let picked = selecting && app.ui.generations.selected.contains(&r.id);
    let selected = picked || (!selecting && app.ui.generations.last_viewed == Some(r.id));
    let border = if selected { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.separator) };
    p.rect_stroke(cell, 2.0, border, StrokeKind::Inside);
    if selecting {
        // The pick mark: a filled check when picked, an empty ring otherwise.
        let c = pos2(cell.left() + 10.0, cell.top() + 10.0);
        if picked {
            p.circle_filled(c, 7.0, t.accent);
            icons::paint(ui, Rect::from_center_size(c, vec2(12.0, 12.0)), "check", 9.0, Color32::WHITE);
        } else {
            p.circle_filled(c, 7.0, Color32::from_black_alpha(120));
            p.circle_stroke(c, 7.0, Stroke::new(1.5, Color32::WHITE));
        }
    }
    if r.results > 1 {
        let badge = Rect::from_min_size(pos2(cell.right() - 20.0, cell.top() + 4.0), vec2(16.0, 14.0));
        p.rect_filled(badge, 3.0, t.chrome.gamma_multiply(0.85));
        p.text(badge.center(), Align2::CENTER_CENTER, r.results.to_string(), FontId::proportional(9.5), t.text);
    }
    if r.pinned {
        p.circle_filled(pos2(cell.left() + 8.0, cell.top() + 8.0), 3.5, t.accent);
    }
    let hovered = resp.hovered() || ui.rect_contains_pointer(cell);
    if overlay && hovered && cell.width() >= 70.0 && !r.selection && !selecting {
        let bar = Rect::from_min_max(pos2(cell.left(), cell.bottom() - 24.0), cell.max);
        p.rect_filled(bar, 0.0, t.chrome.gamma_multiply(0.82));
        let buttons: [(&str, &str, Action); 4] = [
            ("layers", tl!("Place"), Action::Place(r.id, "layer")),
            ("sparkles", tl!("Generate Similar"), Action::Recreate(r.id)),
            ("folder-open", tl!("Save…"), Action::Save(r.id)),
            ("copy", tl!("Copy prompt"), Action::CopyPrompt(r.id)),
        ];
        let step = bar.width() / buttons.len() as f32;
        for (k, (icon, tip, a)) in buttons.into_iter().enumerate() {
            let b = Rect::from_center_size(pos2(bar.left() + step * (k as f32 + 0.5), bar.center().y), vec2(20.0, 20.0));
            let c = ui.interact(b, ui.scope_id().with(("gen-act", r.id, k)), Sense::click());
            let tint = icons::button_chrome(ui, b, false, c.hovered());
            icons::paint(ui, b, icon, 12.0, tint);
            if c.on_hover_text(tip).clicked() {
                *action = Some(a);
            }
        }
    }
    let tip = if r.size.0 > 0 { format!("{}\n{} · {}×{}", r.title, r.template, r.size.0, r.size.1) } else { r.title.clone() };
    let resp = resp.on_hover_text(tip);
    if resp.clicked() && action.is_none() {
        *action = Some(if selecting { Action::Toggle(r.id) } else { Action::Open(r.id) });
    }
    let id = r.id;
    let can_place = !r.selection;
    let pinned = r.pinned;
    resp.context_menu(|ui| {
        if can_place {
            if ui.button(tl!("Place as Layer")).clicked() {
                *action = Some(Action::Place(id, "layer"));
            }
            if ui.button(tl!("Open as Document")).clicked() {
                *action = Some(Action::Place(id, "document"));
            }
            if ui.button(tl!("Generate Similar")).clicked() {
                *action = Some(Action::Recreate(id));
            }
            if ui.button(tl!("Save…")).clicked() {
                *action = Some(Action::Save(id));
            }
            if !selecting && ui.button(tl!("Select")).clicked() {
                *action = Some(Action::StartSelect(Some(id)));
            }
        }
        if ui.button(tl!("Copy prompt")).clicked() {
            *action = Some(Action::CopyPrompt(id));
        }
        ui.separator();
        if ui.button(if pinned { tl!("Unpin") } else { tl!("Pin") }).clicked() {
            *action = Some(Action::Pin(id, !pinned));
        }
        if ui.button(tl!("Dismiss")).clicked() {
            *action = Some(Action::Dismiss(id));
        }
    });
}

/// Generation `id`'s thumbnail (the Generate panel's reference deck shares the cache).
pub(crate) fn thumbnail(app: &mut PhotocraftApp, ctx: &egui::Context, id: u64) -> Option<egui::TextureHandle> {
    let path = app.session.generations.thumbnail_path(id)?;
    let row = Row {
        id,
        status: GenStatus::Done,
        title: String::new(),
        template: String::new(),
        progress: 1.0,
        message: String::new(),
        failure: None,
        size: (0, 0),
        results: 1,
        thumbnail: Some(path),
        selection: false,
        pinned: false,
    };
    let mut decodes = 1;
    thumb(app, ctx, &row, &mut decodes)
}

/// The row's thumbnail texture, decoded on first sight (at most `decodes` this frame).
fn thumb(app: &mut PhotocraftApp, ctx: &egui::Context, r: &Row, decodes: &mut usize) -> Option<egui::TextureHandle> {
    if let Some(t) = app.ui.generations.thumbs.0.get(&r.id) {
        return t.clone();
    }
    let path = r.thumbnail.as_ref()?;
    if *decodes == 0 {
        return None;
    }
    // Not written yet (a background job's writer is still on it): try again next frame.
    if !path.is_file() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        return None;
    }
    *decodes -= 1;
    let tex = std::fs::read(path)
        .ok()
        .and_then(|bytes| decode_png(&bytes))
        .map(|img| ctx.load_texture(format!("generation-{}", r.id), img, egui::TextureOptions::LINEAR));
    app.ui.generations.thumbs.0.insert(r.id, tex.clone());
    tex
}

fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    use photocraft_codecs::{ChannelLayout, SampleType};
    let img = photocraft_codecs::decode(bytes).ok()?.convert(ChannelLayout::Rgba, SampleType::U8);
    (img.data().len() == img.width() as usize * img.height() as usize * 4)
        .then(|| egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data()))
}

/// Drop textures of rows no longer listed.
fn prune_thumbs(app: &mut PhotocraftApp, rows: &[Row]) {
    let thumbs = &mut app.ui.generations.thumbs.0;
    if thumbs.len() > rows.len() {
        let keep: BTreeSet<u64> = rows.iter().map(|r| r.id).collect();
        thumbs.retain(|id, _| keep.contains(id));
    }
}

fn clip(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max { one_line } else { format!("{}…", one_line.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// The "Clear …" confirmation.
fn confirm_clear(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(which) = app.ui.generations.confirm.clone() else { return };
    let text = match which.as_str() {
        "completed" => tl!("Delete every completed result and its files?"),
        "failed" => tl!("Remove every failed and cancelled row?"),
        _ => tl!("Delete every generation and its files? Running jobs stay."),
    };
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("generations-clear")).show(ctx, |ui| {
        ui.set_max_width(320.0);
        ui.label(text);
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            answer = widgets::dialog_buttons(
                ui,
                &[
                    widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("Delete"), 80.0),
                    widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 80.0),
                ],
            );
        });
    });
    match answer {
        Some(widgets::ButtonRole::Default) => {
            app.ui.generations.confirm = None;
            run_report(app, "generate.jobs.clear", json!({"which": which}));
        }
        Some(_) => app.ui.generations.confirm = None,
        None => {}
    }
}

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Run a command; an error goes to the status bar.
fn run_report(app: &mut PhotocraftApp, id: &str, params: Value) {
    let r = app.run(id, params);
    report(app, r);
}

fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, a: Action) {
    match a {
        Action::Open(id) => open(app, id),
        Action::Place(id, target) => {
            app.ui.generations.last_viewed = Some(id);
            run_report(app, "generate.place", json!({"id": id, "target": target}));
        }
        Action::Recreate(id) => run_report(app, "generate.recreate", json!({"id": id})),
        Action::Save(id) => save(app, id),
        Action::CopyPrompt(id) => {
            if let Some(r) = app.session.generations.get(id) {
                ctx.copy_text(title_of(r));
                app.ui.status = tl!("Prompt copied").to_string();
                app.ui.status_error = false;
            }
        }
        Action::Dismiss(id) => run_report(app, "generate.jobs.dismiss", json!({"id": id})),
        Action::Pin(id, on) => run_report(app, "generate.jobs.pin", json!({"id": id, "pinned": on})),
        Action::Cancel(id) => {
            if let Some(job) = app.session.generations.job_for_record(id) {
                crate::jobs_ui::cancel(app, job);
            }
        }
        Action::DismissFailed => run_report(app, "generate.jobs.dismiss", json!({"which": "failed"})),
        Action::AskClear(which) => app.ui.generations.confirm = Some(which.to_string()),
        Action::View(v) => app.ui.generations.view = v,
        Action::StartSelect(id) => {
            let g = &mut app.ui.generations;
            g.selecting = true;
            g.selected.clear();
            g.selected.extend(id);
        }
        Action::EndSelect => {
            app.ui.generations.selecting = false;
            app.ui.generations.selected.clear();
        }
        Action::Toggle(id) => {
            let s = &mut app.ui.generations.selected;
            if !s.remove(&id) {
                s.insert(id);
            }
        }
        Action::SaveSelected => {
            // Newest first, as the panel shows them.
            let picked = &app.ui.generations.selected;
            let ids: Vec<u64> = app.session.generations.records().iter().rev().map(|r| r.id).filter(|id| picked.contains(id)).collect();
            if save_many(app, &ids) {
                app.ui.generations.selecting = false;
                app.ui.generations.selected.clear();
            }
        }
    }
}

/// Click on a result: select its layer while its document is open, else open it as a document.
pub fn open(app: &mut PhotocraftApp, id: u64) {
    app.ui.generations.last_viewed = Some(id);
    let target = open_layer(app, id);
    match target {
        Some((doc, layer)) => {
            app.session.set_active(doc);
            run_report(app, "layer.select", json!({"layer": layer}));
        }
        None => run_report(app, "generate.place", json!({"id": id, "target": "document"})),
    }
}

/// Save one result (Save… on a card).
fn save(app: &mut PhotocraftApp, id: u64) {
    save_many(app, &[id]);
}

/// Save results with `generate.export`: into Preferences › AI Integrations › Save results dir
/// under its naming pattern, or, with no folder set, where the save dialog says (one result:
/// under the name typed; several: the pattern in that folder). True when something was saved,
/// or will be once the dialog (which answers on a later frame) is confirmed.
fn save_many(app: &mut PhotocraftApp, ids: &[u64]) -> bool {
    let Some(first) = ids.first().and_then(|id| app.session.generations.get(*id)) else { return false };
    let prefs = &app.session.prefs().integrations;
    let naming = prefs.save_results_naming.trim();
    let pattern = if naming.is_empty() { photocraft_engine::generate_jobs::DEFAULT_EXPORT_NAME.to_string() } else { naming.to_string() };
    let folder = prefs.save_results_dir.trim().to_string();
    if !folder.is_empty() {
        return export_many(app, ids, &folder, None) > 0;
    }
    let suggested = format!("{}.png", photocraft_engine::generate_jobs::export_name(&pattern, first, 1, false));
    let ids = ids.to_vec();
    let r = app.pick_save(&suggested, move |app, path| {
        let path = std::path::PathBuf::from(path);
        let dir = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let name = (ids.len() == 1).then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()).filter(|n| !n.is_empty());
        Ok(json!({"saved": export_many(app, &ids, &dir, name)}))
    });
    match r {
        Ok(_) => true,
        Err(e) => {
            if e != crate::file_dialog::CANCELLED {
                app.ui.status = e;
                app.ui.status_error = true;
            } else if app.services.file_dialog.is_none() {
                app.ui.status = tl!("Saving files is not available here").to_string();
                app.ui.status_error = true;
            }
            false
        }
    }
}

/// Exports `ids` into `dir` (one result under `name` when given), reporting the first failure
/// or the count in the status bar; the number of files written.
fn export_many(app: &mut PhotocraftApp, ids: &[u64], dir: &str, name: Option<String>) -> usize {
    let mut saved = 0usize;
    let mut failed = Vec::new();
    for id in ids {
        let mut params = json!({"id": id, "dir": dir});
        if let Some(n) = &name {
            params["name"] = json!(n);
        }
        match app.run("generate.export", params) {
            Ok(v) => saved += v["files"].as_array().map_or(0, Vec::len),
            Err(e) => failed.push(e),
        }
    }
    match failed.first() {
        Some(e) => report(app, Err(e.clone())),
        None => {
            app.ui.status = crate::i18n::fmt(tl!("Saved {n} file(s)"), &[("n", &saved.to_string())]);
            app.ui.status_error = false;
        }
    }
    saved
}

/// The rail button's badge: running jobs, else results that finished while the panel was hidden.
pub fn rail_badge(app: &PhotocraftApp, ui: &egui::Ui, rect: Rect, t: &Tokens) {
    let running = app.session.generations.running();
    let unread = app.ui.generations.unread.len();
    let (n, fill) = if running > 0 { (running, t.accent) } else { (unread, t.danger) };
    if n == 0 {
        return;
    }
    let c = pos2(rect.right() - 6.0, rect.top() + 6.0);
    ui.painter().circle_filled(c, 6.5, fill);
    let text = if n > 9 { "9+".to_string() } else { n.to_string() };
    ui.painter().text(c, Align2::CENTER_CENTER, text, FontId::proportional(8.5), Color32::WHITE);
}

/// A job ended (`jobs_ui::on_event`): a result that finished while the panel was hidden counts
/// as unread; a thumbnail cached for the row is dropped so the new one loads; the completion
/// alert plays as Preferences › AI Integrations › Alerts says.
pub fn on_event(app: &mut PhotocraftApp, e: &JobEvent) {
    let Some(id) = app.session.generations.record_for_job(e.id) else { return };
    app.ui.generations.thumbs.0.remove(&id);
    let pref = app.session.prefs().integrations.alerts.clone();
    if let Some(a) = crate::alerts::for_outcome(&pref, matches!(e.outcome, JobOutcome::Done(_)), matches!(e.outcome, JobOutcome::Failed(_))) {
        crate::alerts::play(a);
        app.ui.generations.last_alert = Some(a.key());
    }
    let hidden = app.frame > app.ui.generations.visible_frame + 1;
    if hidden && matches!(e.outcome, JobOutcome::Done(_)) {
        app.ui.generations.unread.insert(id);
    }
}

/// `ui.inspect.generations`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let rows = rows(app);
    json!({
        "view": app.ui.generations.view.key(),
        "compact": app.ui.generations.compact,
        "running": rows.iter().filter(|r| r.status == GenStatus::Running).count(),
        "failed": rows.iter().filter(|r| matches!(r.status, GenStatus::Failed | GenStatus::Cancelled)).count(),
        "completed": rows.iter().filter(|r| r.status == GenStatus::Done).count(),
        "ids": rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        "lastViewed": app.ui.generations.last_viewed,
        "unread": app.ui.generations.unread.len(),
        "thumbnails": app.ui.generations.thumbs.len(),
        "selecting": app.ui.generations.selecting,
        "selected": app.ui.generations.selected,
        "lastAlert": app.ui.generations.last_alert,
    })
}

/// `ui.set {generationsView: "grid"|"list"}`.
pub fn set(app: &mut PhotocraftApp, p: &Value) -> Result<(), String> {
    let Some(v) = p.get("generationsView") else { return Ok(()) };
    app.ui.generations.view = match v.as_str() {
        Some("grid") => GenView::Grid,
        Some("list") => GenView::List,
        _ => return Err(format!("generationsView must be \"grid\" or \"list\", got {v}")),
    };
    Ok(())
}

#[cfg(test)]
#[path = "generations_panel_tests.rs"]
mod tests;
