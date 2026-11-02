//! The Generate hub (ShotoPhopEX, plan Phase 5): the generative features as cards, on the Home
//! screen under the recent files and in Help › Generative Features…. **Create** makes a picture
//! from nothing (the Generate panel in Image mode, a new document as the result); **Edit** works
//! on an open picture: the panel in Fill, Edit or Expand mode, or the command's dialog (Remove
//! Background, Split into Layers, Select by Text). With no document open an Edit card asks for a
//! file first and carries on once it is open. Every card ends in the panel or a menu command, so
//! nothing here does work of its own.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::icons;
use crate::theme::Tokens;

/// Where a card leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The Generate panel in this mode.
    Panel(&'static str),
    /// This command (its dialog when it has parameters).
    Command(&'static str),
    /// Not in this build yet (a SOON card).
    Soon,
}

/// One feature.
#[derive(Clone, Copy, Debug)]
pub struct Card {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub sentence: &'static str,
    /// Create (needs nothing) or Edit (needs an open picture).
    pub create: bool,
    pub target: Target,
    /// `NEW` or `SOON` (or none).
    pub badge: Option<&'static str>,
}

/// The cards, Create first.
pub const CARDS: [Card; 8] = [
    Card {
        id: "image",
        icon: "image",
        label: "Image",
        sentence: "A new picture from a prompt, with up to three references",
        create: true,
        target: Target::Panel("image"),
        badge: Some("NEW"),
    },
    Card {
        id: "fill",
        icon: "square-dashed",
        label: "Fill",
        sentence: "Paint the selection from a prompt",
        create: false,
        target: Target::Panel("fill"),
        badge: None,
    },
    Card {
        id: "edit",
        icon: "wand-sparkles",
        label: "Edit",
        sentence: "Change the picture by instruction",
        create: false,
        target: Target::Panel("edit"),
        badge: Some("NEW"),
    },
    Card {
        id: "expand",
        icon: "scaling",
        label: "Expand",
        sentence: "Grow the canvas and continue the scene",
        create: false,
        target: Target::Panel("expand"),
        badge: None,
    },
    Card {
        id: "removeBackground",
        icon: "scissors",
        label: "Remove Background",
        sentence: "Cut the subject out onto transparency",
        create: false,
        target: Target::Command("generate.removeBackground"),
        badge: None,
    },
    Card {
        id: "splitLayers",
        icon: "layers",
        label: "Split into Layers",
        sentence: "Separate the picture into editable layers",
        create: false,
        target: Target::Command("generate.splitLayers"),
        badge: Some("NEW"),
    },
    Card {
        id: "selectByText",
        icon: "text-cursor",
        label: "Select by Text",
        sentence: "Select what you name in a few words",
        create: false,
        target: Target::Command("select.byText"),
        badge: None,
    },
    Card {
        id: "angles",
        icon: "rotate-cw",
        label: "Angles",
        sentence: "See the subject from another camera angle",
        create: false,
        target: Target::Command("generate.angles"),
        badge: Some("NEW"),
    },
];

/// The hub's state in `UiState::hub`. Never saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HubUi {
    /// Help › Generative Features… is open.
    pub open: bool,
    /// An Edit card waiting for the file it asked for.
    pub pending: Option<&'static str>,
}

const CARD_H: f32 = 64.0;
const GAP: f32 = 6.0;

fn columns(width: f32) -> usize {
    (((width + GAP) / (140.0 + GAP)).floor() as usize).clamp(1, 4)
}

/// Height of [`section`] at `width`.
pub fn height(width: f32) -> f32 {
    let cols = columns(width);
    let rows = |create: bool| CARDS.iter().filter(|c| c.create == create).count().div_ceil(cols) as f32;
    22.0 + 2.0 * 18.0 + (rows(true) + rows(false)) * (CARD_H + GAP)
}

/// The cards in two captioned rows (the Home screen and the Help window); returns the card
/// clicked.
pub fn section(ui: &mut egui::Ui, width: f32) -> Option<Card> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let cols = columns(width);
    let w = ((width - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
    let (title, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    ui.painter().text(title.left_center(), Align2::LEFT_CENTER, tl!("Generate"), crate::theme::semibold(13.0), t.text);
    for create in [true, false] {
        let (cap, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        let caption = if create { tl!("CREATE") } else { tl!("EDIT") };
        ui.painter().text(cap.left_center(), Align2::LEFT_CENTER, caption, FontId::monospace(10.0), t.text_faint);
        let cards: Vec<Card> = CARDS.iter().copied().filter(|c| c.create == create).collect();
        for row in cards.chunks(cols) {
            let (line, _) = ui.allocate_exact_size(vec2(width, CARD_H + GAP), Sense::hover());
            for (k, c) in row.iter().enumerate() {
                let r = Rect::from_min_size(pos2(line.left() + k as f32 * (w + GAP), line.top()), vec2(w, CARD_H));
                if card_ui(ui, r, c, &t) {
                    clicked = Some(*c);
                }
                let frame = ui.ctx().cumulative_frame_nr();
                ui.ctx().data_mut(|d| {
                    let v = d.get_temp_mut_or_default::<Drawn>(egui::Id::unique("hub-cards"));
                    v.retain(|(_, _, f)| frame.saturating_sub(*f) <= 2);
                    v.push((c.id, r, frame));
                });
            }
        }
    }
    clicked
}

/// One card: a tinted icon tile, the uppercase label, one sentence, the badge.
fn card_ui(ui: &mut egui::Ui, r: Rect, c: &Card, t: &Tokens) -> bool {
    let live = c.target != Target::Soon;
    let resp = ui.interact(r, ui.scope_id().with(("hub-card", c.id)), if live { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    p.rect_filled(r, t.radius_sm, if resp.hovered() && live { t.hover } else { t.card });
    p.rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), StrokeKind::Inside);
    let dim = |col: Color32| if live { col } else { col.gamma_multiply(0.5) };
    let tile = Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 8.0), vec2(22.0, 22.0));
    p.rect_filled(tile, 5.0, dim(t.accent).gamma_multiply(0.22));
    icons::paint(ui, tile, c.icon, 13.0, dim(t.accent));
    let label = tl!(c.label).to_uppercase();
    let lg = ui.painter().layout(label, FontId::monospace(10.0), dim(t.text), r.width() - 44.0 - if c.badge.is_some() { 34.0 } else { 0.0 });
    ui.painter().galley(pos2(tile.right() + 7.0, tile.center().y - lg.size().y / 2.0), lg, dim(t.text));
    // The sentence: two lines at most, an ellipsis when a translation runs longer.
    let mut job = egui::text::LayoutJob::simple(tl!(c.sentence).to_string(), FontId::proportional(10.5), dim(t.text_faint), r.width() - 16.0);
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    let sg = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(r.shrink(2.0)).galley(pos2(r.left() + 8.0, tile.bottom() + 4.0), sg, dim(t.text_faint));
    if let Some(b) = c.badge {
        let color = if b == "SOON" { t.text_faint } else { t.accent };
        let bg = ui.painter().layout_no_wrap(tl!(b).to_string(), FontId::monospace(8.5), color);
        let br = Rect::from_min_size(pos2(r.right() - bg.size().x - 14.0, r.top() + 8.0), vec2(bg.size().x + 8.0, 13.0));
        ui.painter().rect_stroke(br, 6.0, Stroke::new(1.0, color.gamma_multiply(0.7)), StrokeKind::Inside);
        ui.painter().galley(pos2(br.left() + 4.0, br.center().y - bg.size().y / 2.0), bg, color);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, live, tl!(c.label)));
    let tip = if live { tl!(c.sentence).to_string() } else { format!("{}\n{}", tl!(c.sentence), tl!("Not in this build yet")) };
    live && resp.on_hover_text(tip).clicked()
}

/// Do what a card says: Create opens the panel at once; an Edit card needs an open picture and
/// asks for a file first when there is none (then carries on in [`tick`]).
pub fn activate(app: &mut PhotocraftApp, ctx: &egui::Context, card: &Card) -> Result<Value, String> {
    app.ui.hub.open = false;
    if card.target == Target::Soon {
        return Err(tl!("Not in this build yet").to_string());
    }
    if !card.create && app.session.active().is_none() {
        // The dialog answers on a later frame (`file_dialog`): the card carries on in `tick`
        // once the picture is open. No file dialog here: nothing will open, so nothing waits.
        if app.open_dialog_file().is_err() {
            app.ui.status = tl!("Open a picture first").to_string();
            app.ui.status_error = false;
            return Ok(json!({"waiting": false}));
        }
        app.ui.hub.pending = Some(card.id);
        return Ok(json!({"waiting": true}));
    }
    run(app, ctx, card)
}

fn run(app: &mut PhotocraftApp, ctx: &egui::Context, card: &Card) -> Result<Value, String> {
    match card.target {
        Target::Panel(mode) => {
            app.ui.panels.generate = true;
            if let Some(g) = crate::dock::Group::from_key("generate") {
                crate::dock::reveal(app, g);
            }
            crate::generate_panel::focus_mode(app, mode);
            if mode == "fill" || mode == "edit" {
                app.ui.generative_bar.mode = mode.to_string();
            }
            if mode == "image" && app.session.active().is_none() {
                app.ui.generate.into = "document".into();
            }
            Ok(json!({"panel": mode}))
        }
        Target::Command(id) => crate::menus::invoke(app, ctx, id, json!({})),
        Target::Soon => Err(tl!("Not in this build yet").to_string()),
    }
}

/// Each frame: an Edit card that asked for a file carries on once a picture is open (and gives
/// up when the dialog was cancelled or the opening failed).
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(id) = app.ui.hub.pending else { return };
    if app.session.active().is_some() {
        app.ui.hub.pending = None;
        if let Some(card) = CARDS.iter().find(|c| c.id == id)
            && let Err(e) = run(app, ctx, card)
        {
            crate::notices::error(app, e);
        }
    } else if app.jobs.opens.is_empty() && !app.file_dialog_open() {
        app.ui.hub.pending = None;
    }
}

/// Help › Generative Features…: the same cards in a window.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.hub.open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut chosen = None;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("generate-hub")).show(ctx, |ui| {
        let width = 3.0 * 150.0 + 2.0 * GAP;
        ui.set_width(width);
        ui.label(egui::RichText::new(tl!("Generative Features")).font(crate::theme::semibold(16.0)).color(t.text));
        ui.label(egui::RichText::new(tl!("Runs on the native worker with the models you installed.")).small().color(t.text_dim));
        ui.add_space(6.0);
        chosen = section(ui, width);
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::secondary_button(ui, tl!("Close"), 90.0).clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        app.ui.hub.open = false;
    }
    if let Some(c) = chosen
        && let Err(e) = activate(app, ctx, &c)
    {
        crate::notices::error(app, e);
    }
}

/// Cards drawn in the last frames: (id, rect, frame).
type Drawn = Vec<(&'static str, Rect, u64)>;

/// `ui.inspect.hub`: the window, a waiting card, and every card with the rects (points) the last
/// frame drew it at (the Home screen's and the window's), for `ui.click`.
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let now = ctx.cumulative_frame_nr();
    let drawn: Drawn = ctx.data_mut(|d| {
        let v = d.get_temp_mut_or_default::<Drawn>(egui::Id::unique("hub-cards"));
        v.retain(|(_, _, f)| now.saturating_sub(*f) <= 2);
        v.clone()
    });
    // The latest frame's cards (the Home screen's and the window's when both are on screen).
    let latest = drawn.iter().map(|(_, _, f)| *f).max();
    let rects = |id: &str| -> Vec<Value> {
        drawn.iter().filter(|(c, _, f)| *c == id && Some(*f) == latest).map(|(_, r, _)| json!([r.left(), r.top(), r.width(), r.height()])).collect()
    };
    json!({
        "open": app.ui.hub.open,
        "pending": app.ui.hub.pending,
        "cards": CARDS.iter().map(|c| json!({"id": c.id, "label": c.label, "create": c.create, "badge": c.badge, "live": c.target != Target::Soon, "rects": rects(c.id)})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
#[path = "generate_hub_tests.rs"]
mod tests;
