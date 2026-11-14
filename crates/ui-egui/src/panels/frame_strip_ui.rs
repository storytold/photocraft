//! Window › Timeline in frame-animation mode: Photoshop's frame strip. Each frame is a thumbnail
//! with its delay under it; the bar below holds the loop count, transport, and duplicate/delete.
//! Everything runs the engine `animation.*` commands, so the control channel drives it the same way.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_doc::{AnimFrame, Document, FrameAnimation, LayerId, LoopCount};
use serde_json::{Value, json};

use crate::theme::Tokens;

/// Longest side of a frame thumbnail, in pixels.
const THUMB_SIDE: u32 = 96;
/// Width of one frame cell in the strip.
const CELL_W: f32 = 84.0;
/// Delays offered by a frame's menu, in milliseconds (Photoshop's list).
pub const DELAY_PRESETS_MS: [u32; 7] = [100, 200, 500, 1000, 2000, 5000, 10000];

/// One command for the panel to run.
pub type Act = (&'static str, Value);

struct Thumb {
    /// The layers the frame hid when this was drawn.
    key: (Vec<LayerId>, Vec<(LayerId, f32)>),
    /// Still shows what the document looks like now.
    fresh: bool,
    tex: TextureHandle,
    size: [usize; 2],
}

#[derive(Default)]
struct StripCache {
    doc: Option<photocraft_doc::DocId>,
    rev: u64,
    /// The revision a frame-selecting command left behind: it moves the playhead, not the pixels.
    nav_rev: Option<u64>,
    thumbs: Vec<Option<Thumb>>,
}

fn cache_id() -> egui::Id {
    egui::Id::unique("frame-strip-thumbs")
}

fn with_cache<R>(ctx: &egui::Context, f: impl FnOnce(&mut StripCache) -> R) -> R {
    let cell = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<std::sync::Mutex<StripCache>>>(cache_id()));
    let cell = match cell {
        Some(c) => c,
        None => {
            let c = std::sync::Arc::new(std::sync::Mutex::new(StripCache::default()));
            ctx.data_mut(|d| d.insert_temp(cache_id(), c.clone()));
            c
        }
    };
    let mut g = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut g)
}

/// The revision a frame-selecting command left behind (see `ran`), if the last change was only that.
pub fn nav_rev(ctx: &egui::Context) -> Option<u64> {
    with_cache(ctx, |c| c.nav_rev)
}

/// Records that `cmd` just ran: a command that only moves the selected frame keeps the thumbnails.
pub fn ran(app: &crate::PhotocraftApp, ctx: &egui::Context, cmd: &str) {
    if matches!(cmd, "animation.selectFrame" | "animation.nextFrame" | "animation.previousFrame")
        && let Some(st) = app.session.active()
    {
        let (rev, doc) = (st.revision, st.doc.id);
        with_cache(ctx, |c| {
            if c.doc == Some(doc) && continues_navigation(c.rev, c.nav_rev, rev) {
                c.nav_rev = Some(rev);
            }
        });
    }
}

/// Did the one revision since the thumbnails were last checked leave the pixels alone (it reports no
/// damage)? Only a single step can be told apart this way: with several, any of them might have drawn.
fn leaves_pixels_alone(seen_rev: u64, rev: u64, no_damage: bool) -> bool {
    no_damage && seen_rev.checked_add(1) == Some(rev)
}

/// How many thumbnails to composite in one UI frame: a composite costs about in proportion to the
/// document's pixels, so a big document gets one a frame and a small one several.
fn thumbs_per_frame(pixels: u64) -> usize {
    ((4_000_000 / pixels.max(1)) as usize).clamp(1, 8)
}

/// Is revision `rev`, left by a frame-selecting command, nothing but navigation since the revision
/// `seen_rev` the thumbnails were last checked at? An edit in between (a canvas stroke in the same UI
/// frame as a playback step) makes the thumbnails stale, so it must not be hidden by the selection.
pub(crate) fn continues_navigation(seen_rev: u64, nav_rev: Option<u64>, rev: u64) -> bool {
    rev.checked_sub(1).is_some_and(|prev| prev == seen_rev || nav_rev == Some(prev))
}

/// Whether a thumbnail made with `key` still shows `frame`: tweened frames differ only in their opacities.
fn drawn_from(key: &(Vec<LayerId>, Vec<(LayerId, f32)>), frame: &AnimFrame) -> bool {
    key.0 == frame.hidden && key.1 == frame.opacities
}

/// Redraws a few thumbnails (see `thumbs_per_frame`) that are missing or stale.
fn refresh_thumbs(ctx: &egui::Context, doc: &Arc<Document>, revision: u64, no_damage: bool, a: &FrameAnimation) {
    with_cache(ctx, |c| {
        if c.doc != Some(doc.id) {
            *c = StripCache { doc: Some(doc.id), rev: revision, ..Default::default() };
        }
        if c.rev != revision {
            if c.nav_rev != Some(revision) && !leaves_pixels_alone(c.rev, revision, no_damage) {
                for t in c.thumbs.iter_mut().flatten() {
                    t.fresh = false;
                }
            }
            c.rev = revision;
        }
        c.thumbs.resize_with(a.frames.len(), || None);
        let mut budget = thumbs_per_frame(u64::from(doc.size.width) * u64::from(doc.size.height));
        for (i, frame) in a.frames.iter().enumerate() {
            let stale = match c.thumbs.get(i).and_then(Option::as_ref) {
                Some(t) => !t.fresh || !drawn_from(&t.key, frame),
                None => true,
            };
            if !stale {
                continue;
            }
            if budget == 0 {
                ctx.request_repaint();
                break;
            }
            budget -= 1;
            let mut d = (**doc).clone();
            a.apply(i, &mut d.layers);
            let img = photocraft_compose::thumbnail(&d, THUMB_SIDE);
            let size = [img.width as usize, img.height as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, &img.pixels);
            let slot = &mut c.thumbs[i];
            match slot {
                Some(t) => {
                    t.tex.set(color, TextureOptions::LINEAR);
                    t.key = (frame.hidden.clone(), frame.opacities.clone());
                    t.fresh = true;
                    t.size = size;
                }
                None => {
                    let tex = ctx.load_texture(format!("frame-thumb-{i}"), color, TextureOptions::LINEAR);
                    *slot = Some(Thumb { key: (frame.hidden.clone(), frame.opacities.clone()), fresh: true, tex, size });
                }
            }
        }
    });
}

/// The text on a frame's delay button: "0 sec.", "0.1 sec.", "2 sec.".
pub fn delay_label(ms: u32) -> String {
    crate::i18n::fmt(crate::i18n::t("{seconds} sec."), &[("seconds", &seconds_text(ms))])
}

/// `ms` in seconds without trailing zeros: 100 → "0.1", 2000 → "2", 0 → "0".
pub fn seconds_text(ms: u32) -> String {
    let s = format!("{:.2}", f64::from(ms) / 1000.0);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Seconds typed into the "Other…" field; None unless a number from 0 to the longest delay.
pub fn parse_seconds(text: &str) -> Option<f64> {
    let v: f64 = text.trim().replace(',', ".").parse().ok()?;
    (v.is_finite() && (0.0..=f64::from(photocraft_doc::frame_animation::MAX_DELAY_MS) / 1000.0).contains(&v)).then_some(v)
}

/// What the loop menu shows for `loops`.
fn loop_label(loops: LoopCount) -> String {
    match loops {
        LoopCount::Forever => tl!("Forever").to_string(),
        LoopCount::Once => tl!("Once").to_string(),
        LoopCount::Times(n) => crate::i18n::fmt(crate::i18n::t("{count} times"), &[("count", &n.to_string())]),
    }
}

/// The frame to show after `current` while playing, or None to stop. `plays_done` counts the
/// completed passes through the frames before this one.
pub fn next_playback_frame(current: usize, len: usize, loops: LoopCount, plays_done: u32) -> Option<(usize, u32)> {
    if current + 1 < len {
        return Some((current + 1, plays_done));
    }
    match loops {
        LoopCount::Forever => Some((0, plays_done)),
        LoopCount::Once => None,
        LoopCount::Times(n) => (plays_done + 1 < n).then_some((0, plays_done + 1)),
    }
}

/// How long frame `ms` shows during playback: a 0 delay still advances at a screen's pace.
pub fn playback_period(ms: u32) -> f64 {
    f64::from(ms.max(20)) / 1000.0
}

/// Where a dragged frame would be dropped: the gap (0 = before the first frame, `n` = after the
/// last) nearest the pointer, given the horizontal centre of each cell.
pub fn drop_slot(centers: &[f32], x: f32) -> usize {
    centers.iter().take_while(|c| **c < x).count()
}

/// How a click on a frame changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Selects just that frame.
    Plain,
    /// ⌘/Ctrl: adds the frame to the selection, or takes it out.
    Toggle,
    /// ⇧: selects every frame from the anchor to that frame.
    Range,
}

/// The selection after `click` on frame `i`: the selected frames, the selected (shown) frame and the
/// anchor a later range starts from.
pub fn click_selection(sel: &BTreeSet<usize>, current: usize, anchor: usize, i: usize, click: Click) -> (BTreeSet<usize>, usize, usize) {
    match click {
        Click::Plain => (BTreeSet::from([i]), i, i),
        Click::Range => ((anchor.min(i)..=anchor.max(i)).collect(), i, anchor),
        Click::Toggle if sel.contains(&i) && sel.len() > 1 => {
            let mut rest = sel.clone();
            rest.remove(&i);
            let now = if i == current { rest.iter().next_back().copied().unwrap_or(i) } else { current };
            (rest, now, now)
        }
        Click::Toggle => {
            let mut more = sel.clone();
            more.insert(i);
            (more, i, i)
        }
    }
}

/// The frames a command on frame `i` covers: the whole selection when `i` is in it, else just `i`.
pub fn targets(sel: &BTreeSet<usize>, i: usize) -> Vec<usize> {
    if sel.contains(&i) { sel.iter().copied().collect() } else { vec![i] }
}

/// Would dropping `group` into gap `slot` leave the frames as they are?
pub fn is_noop_drop(group: &[usize], slot: usize) -> bool {
    let (Some(first), Some(last)) = (group.first(), group.last()) else { return true };
    let contiguous = last - first + 1 == group.len();
    contiguous && (*first..=*last + 1).contains(&slot)
}

/// Id of the frame being dragged, kept in egui memory between frames.
fn drag_id() -> egui::Id {
    egui::Id::unique("frame-strip-drag")
}

/// How far the pointer may stray above or below the strip before a drop cancels the drag.
const DRAG_SLACK: f32 = 60.0;
/// Pointer distance from the strip's edge at which a drag scrolls the strip.
const EDGE_SCROLL_ZONE: f32 = 28.0;

/// What the strip shows and what it hands back.
pub struct Strip<'a> {
    pub doc: &'a Arc<Document>,
    /// The session revision of `doc`, which says when thumbnails go stale.
    pub revision: u64,
    /// The latest revision changed no pixels (its damage is empty): a rename, a selection, a lock.
    pub no_damage: bool,
    pub animation: &'a FrameAnimation,
    /// The selected frames (always including the shown one).
    pub selection: &'a BTreeSet<usize>,
    pub playing: bool,
    /// Commands the user asked for.
    pub acts: &'a mut Vec<Act>,
    /// The onion-skin settings, and where changes to them go.
    pub onion: crate::onion_skin::OnionSkin,
    /// Whether the Onion Skin menu is shown (Options menu in the panel header).
    pub show_onion: bool,
    /// Whether the Tween menu is shown.
    pub show_tween: bool,
    pub onion_out: &'a mut Option<crate::onion_skin::OnionSkin>,
    /// A click on a frame, to turn into a selection.
    pub clicked: &'a mut Option<(usize, Click)>,
    /// Set when Play or Pause was pressed.
    pub toggle_play: &'a mut bool,
}

/// Draws the strip and its bar.
pub fn body(ui: &mut egui::Ui, ctx: &egui::Context, strip: Strip<'_>) {
    let Strip { doc, revision, no_damage, animation: a, selection, playing, acts, onion, show_onion, show_tween, onion_out, clicked, toggle_play } = strip;
    let t = Tokens::get(ctx);
    refresh_thumbs(ctx, doc, revision, no_damage, a);

    let dragging: Option<(usize, Vec<usize>)> = ctx.data(|d| d.get_temp(drag_id()));
    egui::ScrollArea::horizontal().id_salt("frame-strip").auto_shrink([false, true]).show(ui, |ui| {
        let mut cells: Vec<Rect> = Vec::with_capacity(a.frames.len());
        ui.horizontal(|ui| {
            for (i, frame) in a.frames.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(CELL_W);
                    let shown = i == a.current;
                    let selected = selection.contains(&i);
                    let (rect, resp) = ui.allocate_exact_size(vec2(CELL_W, 66.0), Sense::click_and_drag());
                    cells.push(rect);
                    let fill = if selected { t.accent_soft } else { t.field };
                    ui.painter().rect_filled(rect, t.radius_sm, fill);
                    let inner = rect.shrink(3.0);
                    let drawn = with_cache(ctx, |c| c.thumbs.get(i).and_then(Option::as_ref).map(|th| (th.tex.id(), th.size)));
                    if let Some((tex, [w, h])) = drawn {
                        let scale = (inner.width() / w.max(1) as f32).min(inner.height() / h.max(1) as f32);
                        let r = Rect::from_center_size(inner.center(), vec2(w as f32 * scale, h as f32 * scale));
                        ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                        ui.painter().image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.painter().text(rect.left_top() + vec2(4.0, 2.0), Align2::LEFT_TOP, (i + 1).to_string(), FontId::monospace(11.0), t.text);
                    if selected {
                        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(if shown { 2.0 } else { 1.0 }, t.accent), StrokeKind::Inside);
                    }
                    if resp.clicked() || (resp.drag_started() && !selected) {
                        let mods = ui.input(|inp| inp.modifiers);
                        let kind = if mods.shift {
                            Click::Range
                        } else if mods.command || mods.ctrl {
                            Click::Toggle
                        } else {
                            Click::Plain
                        };
                        *clicked = Some((i, if resp.drag_started() { Click::Plain } else { kind }));
                    }
                    if resp.drag_started() {
                        // Dragging a selected frame carries the whole selection; any other frame alone.
                        ctx.data_mut(|d| d.insert_temp(drag_id(), (i, targets(selection, i))));
                    }
                    if dragging.as_ref().is_some_and(|(_, g)| g.contains(&i)) {
                        // The frame being carried stays put, dimmed, until it is dropped.
                        ui.painter().rect_filled(rect, t.radius_sm, t.chrome.gamma_multiply(0.5));
                    }
                    delay_menu(ui, targets(selection, i), frame.delay_ms, acts);
                });
                ui.add_space(2.0);
            }
        });
        if let Some((from, group)) = dragging {
            drag_frame(ui, ctx, &cells, from, &group, acts);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        loop_menu(ui, a.loops, acts);
        if show_tween {
            tween_menu(ui, acts);
        }
        if show_onion {
            onion_menu(ui, onion, onion_out);
        }
        ui.separator();
        if crate::icons::button(ui, "chevrons-left", 22.0, false, tl!("First frame")).clicked() {
            acts.push(("animation.selectFrame", json!({ "frame": 0 })));
        }
        if crate::icons::button(ui, "chevron-left", 22.0, false, tl!("Previous frame")).clicked() {
            acts.push(("animation.previousFrame", json!({})));
        }
        let play_icon = if playing { "pause" } else { "play" };
        let play_tip = format!("{} ({})", if playing { tl!("Pause") } else { tl!("Play") }, crate::shortcuts::pretty("Space"));
        if crate::icons::button(ui, play_icon, 22.0, playing, &play_tip).clicked() {
            *toggle_play = true;
        }
        if crate::icons::button(ui, "chevron-right", 22.0, false, tl!("Next frame")).clicked() {
            acts.push(("animation.nextFrame", json!({})));
        }
        ui.separator();
        ui.label(
            RichText::new(crate::i18n::fmt(
                crate::i18n::t("Frame {current} / {total}"),
                &[("current", &(a.current + 1).to_string()), ("total", &a.frames.len().to_string())],
            ))
            .color(t.text_dim)
            .monospace(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "trash", 22.0, false, tl!("Delete Selected Frame")).clicked() {
                // The last frame can't go: deleting it removes the animation, as Photoshop's does.
                if a.frames.len() == 1 {
                    acts.push(("animation.delete", json!({})));
                } else if selection.len() < a.frames.len() {
                    acts.push(("animation.deleteFrame", json!({ "frames": targets(selection, a.current) })));
                }
            }
            if crate::icons::button(ui, "copy", 22.0, false, tl!("Duplicate Selected Frame")).clicked() {
                acts.push(("animation.newFrame", json!({ "frames": targets(selection, a.current) })));
            }
        });
    });
}

/// The drag of frame `from`: scrolls the strip near its edges, marks the gap the frame would drop
/// into, and moves it on release. Esc or a release away from the strip cancels.
fn drag_frame(ui: &mut egui::Ui, ctx: &egui::Context, cells: &[Rect], from: usize, group: &[usize], acts: &mut Vec<Act>) {
    let t = Tokens::get(ctx);
    let end = |ctx: &egui::Context| ctx.data_mut(|d| d.remove::<(usize, Vec<usize>)>(drag_id()));
    let Some(pos) = ctx.pointer_interact_pos() else {
        end(ctx);
        return;
    };
    let view = ui.clip_rect();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        end(ctx);
        return;
    }
    if pos.x < view.left() + EDGE_SCROLL_ZONE {
        ui.scroll_with_delta(vec2(12.0, 0.0));
    } else if pos.x > view.right() - EDGE_SCROLL_ZONE {
        ui.scroll_with_delta(vec2(-12.0, 0.0));
    }
    let centers: Vec<f32> = cells.iter().map(|r| r.center().x).collect();
    let slot = drop_slot(&centers, pos.x);
    let inside = pos.y > view.top() - DRAG_SLACK && pos.y < view.bottom() + DRAG_SLACK;
    if inside {
        // The gap marker, a vertical bar between the cells.
        let x = match (cells.get(slot), slot.checked_sub(1).and_then(|p| cells.get(p))) {
            (Some(next), Some(prev)) => (prev.right() + next.left()) / 2.0,
            (Some(next), None) => next.left() - 1.0,
            (None, Some(prev)) => prev.right() + 1.0,
            (None, None) => view.left(),
        };
        let top = cells.first().map_or(view.top(), |r| r.top());
        let bottom = cells.first().map_or(view.bottom(), |r| r.bottom());
        ui.painter().line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(3.0, t.accent));
    }
    // A ghost of the carried frame follows the pointer.
    if let Some(src) = cells.get(from) {
        let ghost = Rect::from_center_size(pos, src.size());
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("frame-strip-ghost"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(ghost, t.radius_sm, t.accent_soft.gamma_multiply(0.8));
        let label = if group.len() > 1 { format!("{} ×{}", from + 1, group.len()) } else { (from + 1).to_string() };
        p.text(ghost.center(), Align2::CENTER_CENTER, label, FontId::monospace(14.0), t.text);
    }
    ctx.request_repaint();
    if ctx.input(|i| i.pointer.any_released()) {
        end(ctx);
        if inside && !is_noop_drop(group, slot) {
            acts.push(("animation.moveFrames", json!({ "frames": group, "slot": slot })));
        }
    }
}

/// A menu button with a drop-down arrow drawn after its label (the font has no ▾ to type).
fn drop_menu(ui: &mut egui::Ui, label: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    // Trailing spaces leave room for the arrow.
    // Only a click outside, or a choice, closes it: a click in the "Other…" field must not.
    let config = egui::containers::menu::MenuConfig::find(ui).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let (response, _) = egui::containers::menu::MenuButton::new(RichText::new(format!("{label}   ")).small()).config(config).ui(ui, add_contents);
    let r = egui::InnerResponse::new((), response);
    let c = pos2(r.response.rect.right() - 9.0, r.response.rect.center().y + 0.5);
    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -2.0), c + vec2(3.5, -2.0), c + vec2(0.0, 2.5)], t.text_dim, Stroke::NONE));
}

/// A frame's delay button and its menu: the presets, then a field for any other value.
fn delay_menu(ui: &mut egui::Ui, frames: Vec<usize>, delay_ms: u32, acts: &mut Vec<Act>) {
    drop_menu(ui, &delay_label(delay_ms), |ui| {
        let mut set = |ui: &mut egui::Ui, ms: u32| {
            acts.push(("animation.setDelay", json!({ "delayMs": ms, "frames": frames })));
            ui.close();
        };
        if ui.selectable_label(delay_ms == 0, tl!("No delay")).clicked() {
            set(ui, 0);
        }
        for ms in DELAY_PRESETS_MS {
            if ui.selectable_label(delay_ms == ms, delay_label(ms)).clicked() {
                set(ui, ms);
            }
        }
        ui.separator();
        other_field(ui, tl!("Other…"), &seconds_text(delay_ms), |ui, text| {
            if let Some(v) = parse_seconds(text) {
                set(ui, (v * 1000.0).round() as u32);
            }
        });
    });
}

/// The loop menu: Once, Forever, or a number of plays.
fn loop_menu(ui: &mut egui::Ui, loops: LoopCount, acts: &mut Vec<Act>) {
    drop_menu(ui, &loop_label(loops), |ui| {
        if ui.selectable_label(loops == LoopCount::Once, tl!("Once")).clicked() {
            acts.push(("animation.setLoop", json!({ "loops": "once" })));
            ui.close();
        }
        if ui.selectable_label(loops == LoopCount::Forever, tl!("Forever")).clicked() {
            acts.push(("animation.setLoop", json!({ "loops": "forever" })));
            ui.close();
        }
        ui.separator();
        let current = if matches!(loops, LoopCount::Times(_)) { loops.plays().to_string() } else { "3".to_string() };
        other_field(ui, tl!("Other…"), &current, |ui, text| {
            if let Ok(n) = text.trim().parse::<u32>()
                && n >= 1
            {
                acts.push(("animation.setLoop", json!({ "loops": n })));
                ui.close();
            }
        });
    });
}

/// The Tween menu: how many frames to add, then which frame to tween the selected one with.
fn tween_menu(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    drop_menu(ui, tl!("Tween"), |ui| {
        let id = ui.scope_id().with("tween-count");
        let mut count: u32 = ui.data_mut(|d| d.get_temp(id)).unwrap_or(3);
        ui.horizontal(|ui| {
            ui.label(tl!("Frames to add"));
            ui.add(egui::DragValue::new(&mut count).range(1..=photocraft_engine::MAX_TWEEN as u32));
        });
        ui.data_mut(|d| d.insert_temp(id, count));
        ui.separator();
        for (with, label) in [("previous", tl!("Previous frame")), ("next", tl!("Next frame")), ("first", tl!("First frame")), ("last", tl!("Last frame"))] {
            if ui.button(label).clicked() {
                acts.push(("animation.tween", json!({ "with": with, "frames": count })));
                ui.close();
            }
        }
    });
}

/// The onion-skin menu: on or off, how many neighbouring frames to show and how strongly.
fn onion_menu(ui: &mut egui::Ui, onion: crate::onion_skin::OnionSkin, out: &mut Option<crate::onion_skin::OnionSkin>) {
    drop_menu(ui, tl!("Onion Skin"), |ui| {
        let mut now = onion;
        ui.checkbox(&mut now.enabled, tl!("Onion Skin"));
        ui.separator();
        let max = crate::onion_skin::MAX_NEIGHBOURS;
        egui::Grid::new("onion-settings").num_columns(2).show(ui, |ui| {
            ui.label(tl!("Frames before"));
            ui.add(egui::DragValue::new(&mut now.before).range(0..=max));
            ui.end_row();
            ui.label(tl!("Frames after"));
            ui.add(egui::DragValue::new(&mut now.after).range(0..=max));
            ui.end_row();
            ui.label(tl!("Opacity"));
            ui.add(egui::DragValue::new(&mut now.opacity).range(1..=100).suffix("%"));
            ui.end_row();
        });
        if now != onion {
            *out = Some(now.clamped());
        }
    });
}

/// "Other…" with a text field inside a menu; Enter hands the text to `apply`.
fn other_field(ui: &mut egui::Ui, label: &str, initial: &str, apply: impl FnOnce(&mut egui::Ui, &str)) {
    let id = ui.scope_id().with(("other-field", label));
    let mut text: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_else(|| initial.to_string());
    ui.horizontal(|ui| {
        ui.label(label);
        let mut out = egui::TextEdit::singleline(&mut text).desired_width(52.0).show(ui);
        // Clicking in selects the old value, so typing replaces it.
        if out.response.gained_focus() {
            let end = egui::text::CCursor::new(out.galley.text().chars().count());
            out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
            out.state.store(ui.ctx(), out.response.id);
        }
        let r = out.response;
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            apply(ui, &text);
        }
    });
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// Shown when the document has neither a frame animation nor a video timeline.
pub fn empty(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        if crate::widgets::primary_button(ui, tl!("Create Frame Animation"), 220.0).clicked() {
            acts.push(("animation.create", json!({})));
        }
        ui.add_space(4.0);
        if crate::widgets::secondary_button(ui, tl!("Make Frames From Layers"), 220.0).clicked() {
            acts.push(("animation.makeFramesFromLayers", json!({})));
        }
        ui.add_space(4.0);
        if crate::widgets::secondary_button(ui, tl!("Create Video Timeline"), 220.0).clicked() {
            acts.push(("timeline.create", json!({ "duration": 30, "fps": 30.0 })));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_is_stale_when_only_the_fade_differs() {
        let frame = |op: f32| AnimFrame { delay_ms: 0, hidden: vec![], opacities: vec![(LayerId(1), op)] };
        let key = (vec![], vec![(LayerId(1), 0.5)]);
        assert!(drawn_from(&key, &frame(0.5)));
        assert!(!drawn_from(&key, &frame(0.6)), "a reorder of tweened frames leaves each index a different fade");
    }

    #[test]
    fn delays_read_like_photoshops() {
        assert_eq!(seconds_text(0), "0");
        assert_eq!(seconds_text(100), "0.1");
        assert_eq!(seconds_text(2000), "2");
        assert_eq!(seconds_text(250), "0.25");
        assert_eq!(seconds_text(10_000), "10");
    }

    #[test]
    fn typed_seconds_are_checked() {
        assert_eq!(parse_seconds("0.5"), Some(0.5));
        assert_eq!(parse_seconds(" 1,5 "), Some(1.5));
        assert_eq!(parse_seconds("0"), Some(0.0));
        for bad in ["", "abc", "-1", "NaN", "inf", "1e9"] {
            assert_eq!(parse_seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn playback_loops_as_the_count_says() {
        assert_eq!(next_playback_frame(0, 3, LoopCount::Once, 0), Some((1, 0)));
        assert_eq!(next_playback_frame(2, 3, LoopCount::Once, 0), None);
        assert_eq!(next_playback_frame(2, 3, LoopCount::Forever, 99), Some((0, 99)));
        assert_eq!(next_playback_frame(2, 3, LoopCount::Times(2), 0), Some((0, 1)));
        assert_eq!(next_playback_frame(2, 3, LoopCount::Times(2), 1), None);
        assert_eq!(next_playback_frame(0, 1, LoopCount::Once, 0), None, "one frame has nothing to play");
    }

    #[test]
    fn a_dragged_frame_drops_into_the_nearest_gap() {
        let centers = [10.0, 30.0, 50.0];
        assert_eq!(drop_slot(&centers, 0.0), 0);
        assert_eq!(drop_slot(&centers, 25.0), 1);
        assert_eq!(drop_slot(&centers, 500.0), 3);
        assert_eq!(drop_slot(&[], 5.0), 0);
        // Dropping a group beside itself changes nothing; elsewhere it moves.
        assert!(is_noop_drop(&[1], 1) && is_noop_drop(&[1], 2));
        assert!(!is_noop_drop(&[1], 3) && !is_noop_drop(&[1], 0));
        assert!(is_noop_drop(&[1, 2], 3) && !is_noop_drop(&[1, 3], 2), "a split group always moves");
    }

    #[test]
    fn clicks_select_one_toggle_or_range() {
        let none = BTreeSet::new();
        assert_eq!(click_selection(&none, 0, 0, 2, Click::Plain), (BTreeSet::from([2]), 2, 2));
        let sel = BTreeSet::from([1]);
        assert_eq!(click_selection(&sel, 1, 1, 4, Click::Range), (BTreeSet::from([1, 2, 3, 4]), 4, 1));
        assert_eq!(click_selection(&sel, 1, 1, 0, Click::Range), (BTreeSet::from([0, 1]), 0, 1), "a range can run backwards");
        assert_eq!(click_selection(&sel, 1, 1, 3, Click::Toggle), (BTreeSet::from([1, 3]), 3, 3));
        let two = BTreeSet::from([1, 3]);
        assert_eq!(click_selection(&two, 3, 3, 3, Click::Toggle), (BTreeSet::from([1]), 1, 1), "taking out the shown frame shows another");
        assert_eq!(click_selection(&two, 3, 3, 1, Click::Toggle), (BTreeSet::from([3]), 3, 3));
        assert_eq!(click_selection(&sel, 1, 1, 1, Click::Toggle), (sel.clone(), 1, 1), "the last selected frame stays");
        assert_eq!(targets(&two, 3), [1, 3]);
        assert_eq!(targets(&two, 2), [2], "a frame outside the selection is alone");
    }

    #[test]
    fn a_zero_delay_still_advances() {
        assert!(playback_period(0) > 0.0);
        assert_eq!(playback_period(500), 0.5);
    }
}
