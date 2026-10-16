//! Window › Timeline: a panel that creates and drives the document timeline. It docks along the
//! bottom of the window, under the canvas, or floats over it. All actions run the engine
//! `timeline.*` and `animation.*` commands, so the control channel drives it the same way.

use egui::{RichText, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Timeline panel view state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineUi {
    pub open: bool,
    /// Floats over the canvas instead of docking along the bottom of the window.
    #[serde(default)]
    pub floating: bool,
    /// Playing back (auto-advancing the playhead at the frame rate).
    #[serde(default)]
    pub playing: bool,
    /// egui time (seconds) of the last auto-advance.
    #[serde(default, skip)]
    pub last_time: f64,
    /// Frame animation: passes through the frames played so far, for a finite loop count.
    #[serde(default, skip)]
    pub plays_done: u32,
    /// Frames copied with ⌘C, for ⌘V to paste after the selected frame.
    #[serde(default, skip)]
    pub clipboard: Vec<std::sync::Arc<photocraft_doc::AnimFrame>>,
    /// The selected frames of a frame animation (the shown frame is always among them).
    #[serde(default, skip)]
    pub sel: std::collections::BTreeSet<usize>,
    /// The document `sel` belongs to: another document starts with just its shown frame.
    #[serde(default, skip)]
    pub sel_doc: Option<photocraft_doc::DocId>,
    /// Where a ⇧-click range starts.
    #[serde(default, skip)]
    pub anchor: usize,
    /// Where the panel was drawn last frame: the timeline's keys work while the pointer is over it.
    #[serde(default, skip)]
    pub panel_rect: Option<egui::Rect>,
}

/// Menu checkmark for Window › Timeline.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == "window.panel.timeline").then_some(app.ui.timeline.open)
}

/// Ids this module owns (for parity / enablement).
pub fn handles(id: &str) -> bool {
    id == "window.panel.timeline"
}

/// Toggle the Timeline panel.
pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.timeline" {
        app.ui.timeline.open = !app.ui.timeline.open;
        return Some(Ok(json!({ "open": app.ui.timeline.open })));
    }
    None
}

/// Docked height of the panel, in points: room for the frame strip and its bar.
const DOCK_HEIGHT: f32 = 168.0;

/// What the panel's contents asked for this frame.
#[derive(Default)]
struct Out {
    acts: Vec<crate::frame_strip_ui::Act>,
    act: Option<(&'static str, Value)>,
    close: bool,
    toggle_play: bool,
    toggle_float: bool,
    clicked: Option<(usize, crate::frame_strip_ui::Click)>,
    rect: Option<egui::Rect>,
}

/// The panel's contents, the same whether it is docked or floating.
fn contents(app: &PhotocraftApp, ui: &mut egui::Ui, ctx: &egui::Context, out: &mut Out) {
    let t = Tokens::get(ctx);
    out.rect = Some(ui.max_rect());
    let playing = app.ui.timeline.playing;
    let floating = app.ui.timeline.floating;
    // Borrowed from the document's Arc: this runs on every UI frame, and a long animation is big.
    let doc = app.session.active().map(|d| (d.doc.clone(), d.revision, d.last_damage.is_some_and(|r| r.is_empty())));
    let tl = doc.as_ref().and_then(|(d, ..)| d.timeline.as_ref());
    let anim = doc.as_ref().and_then(|(d, ..)| d.frame_animation.as_ref());
    let selection = selection(app);

    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Timeline")).strong().color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                out.close = true;
            }
            let (icon, tip) = if floating { ("panels-top-left", tl!("Dock Panel")) } else { ("app-window", tl!("Float Panel")) };
            if crate::icons::button(ui, icon, 20.0, false, tip).clicked() {
                out.toggle_float = true;
            }
        });
    });
    ui.separator();

    if doc.is_none() {
        ui.weak(tl!("Open a document to use the timeline."));
        return;
    }
    if let (Some(a), Some((d, revision, unchanged))) = (anim, &doc) {
        crate::frame_strip_ui::body(
            ui,
            ctx,
            crate::frame_strip_ui::Strip {
                doc: d,
                revision: *revision,
                no_damage: *unchanged,
                animation: a,
                selection: &selection,
                playing,
                acts: &mut out.acts,
                clicked: &mut out.clicked,
                toggle_play: &mut out.toggle_play,
            },
        );
        return;
    }
    match tl {
        None => crate::frame_strip_ui::empty(ui, &mut out.acts),
        Some(tl) => {
            ui.horizontal(|ui| {
                let play_icon = if playing { "pause" } else { "play" };
                if crate::icons::button(ui, play_icon, 22.0, playing, if playing { tl!("Pause") } else { tl!("Play") }).clicked() {
                    out.toggle_play = true;
                }
                if crate::icons::button(ui, "chevron-left", 22.0, false, tl!("Previous frame")).clicked() {
                    out.act = Some(("timeline.previousFrame", json!({})));
                }
                ui.label(
                    RichText::new(crate::i18n::fmt(
                        crate::i18n::t("Frame {current} / {total}"),
                        &[("current", &(tl.current + 1).to_string()), ("total", &tl.duration.to_string())],
                    ))
                    .color(t.text)
                    .monospace(),
                );
                if crate::icons::button(ui, "chevron-right", 22.0, false, tl!("Next frame")).clicked() {
                    out.act = Some(("timeline.nextFrame", json!({})));
                }
                ui.separator();
                ui.label(RichText::new(format!("{:.2}s", tl.time())).color(t.text_dim).monospace());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "trash", 20.0, false, tl!("Delete Timeline")).clicked() {
                        out.act = Some(("timeline.delete", json!({})));
                    }
                    ui.label(RichText::new(format!("{:.0} fps", tl.fps)).color(t.text_dim));
                });
            });
            ui.add_space(6.0);
            // Scrubber: frame playhead across the duration.
            if tl.duration > 1 {
                let mut frame = tl.current as f64;
                let resp = ui.add(egui::Slider::new(&mut frame, 0.0..=(tl.duration - 1) as f64).integer().show_value(false).text("playhead"));
                if resp.changed() {
                    out.act = Some(("timeline.setFrame", json!({ "frame": frame as u64 })));
                }
            }
        }
    }
}

/// Carries out what the contents asked for.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, out: Out) {
    app.ui.timeline.panel_rect = out.rect;
    if out.close {
        app.ui.timeline.open = false;
        app.ui.timeline.playing = false;
    }
    if out.toggle_float {
        app.ui.timeline.floating = !app.ui.timeline.floating;
    }
    if out.toggle_play {
        toggle_play(app, ctx);
    }
    if let Some((i, click)) = out.clicked {
        click_frame(app, ctx, i, click);
    }
    if let Some((cmd, p)) = out.act {
        let _ = app.run(cmd, p);
    }
    for (cmd, p) in out.acts {
        act(app, ctx, cmd, p);
    }
}

/// Play pressed with the last frame shown and a finite loop count: start again from the first frame,
/// or the animation would stop at once.
fn restart_if_finished(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let at_end = app
        .session
        .active()
        .and_then(|d| d.doc.frame_animation.as_ref())
        .is_some_and(|a| a.loops != photocraft_doc::LoopCount::Forever && a.frames.len() > 1 && a.current + 1 == a.frames.len());
    if at_end {
        let _ = app.run("animation.selectFrame", json!({ "frame": 0 }));
        crate::frame_strip_ui::ran(app, ctx, "animation.selectFrame");
    }
}

/// Starts or stops playback.
fn toggle_play(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let starting = !app.ui.timeline.playing;
    app.ui.timeline.playing = starting;
    app.ui.timeline.last_time = ctx.input(|i| i.time);
    app.ui.timeline.plays_done = 0;
    if starting {
        restart_if_finished(app, ctx);
    }
}

/// The selected frames, kept valid: inside the animation and including the shown frame.
pub fn selection(app: &PhotocraftApp) -> std::collections::BTreeSet<usize> {
    let Some(a) = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()) else { return Default::default() };
    let mine = app.ui.timeline.sel_doc == app.session.active().map(|d| d.doc.id);
    let mut sel: std::collections::BTreeSet<usize> =
        if mine { app.ui.timeline.sel.iter().copied().filter(|i| *i < a.frames.len()).collect() } else { Default::default() };
    if !sel.contains(&a.current) {
        sel = std::collections::BTreeSet::from([a.current]);
    }
    sel
}

/// Selects `sel` (with `anchor` for a later ⇧-click) in the active document.
fn set_selection(app: &mut PhotocraftApp, sel: std::collections::BTreeSet<usize>, anchor: usize) {
    app.ui.timeline.sel_doc = app.session.active().map(|d| d.doc.id);
    app.ui.timeline.sel = sel;
    app.ui.timeline.anchor = anchor;
}

/// A click on frame `i` in the strip: changes the selection and shows that frame.
fn click_frame(app: &mut PhotocraftApp, ctx: &egui::Context, i: usize, click: crate::frame_strip_ui::Click) {
    let Some(current) = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()).map(|a| a.current) else { return };
    let sel = selection(app);
    // A ⇧-click range starts at the anchor of this document's selection: another document's means nothing here.
    let last = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()).map_or(0, |a| a.frames.len().saturating_sub(1));
    let from = if app.ui.timeline.sel_doc == app.session.active().map(|d| d.doc.id) { app.ui.timeline.anchor.min(last) } else { current };
    let (sel, now, anchor) = crate::frame_strip_ui::click_selection(&sel, current, from, i, click);
    set_selection(app, sel, anchor);
    if now != current {
        let _ = app.run("animation.selectFrame", json!({ "frame": now }));
        crate::frame_strip_ui::ran(app, ctx, "animation.selectFrame");
    }
}

/// Runs a frame-animation command, keeping the thumbnails and the frame selection in step.
pub(crate) fn act(app: &mut PhotocraftApp, ctx: &egui::Context, cmd: &'static str, params: Value) {
    let before = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()).map(|a| a.frames.len());
    // Reversing moves the shown frame, and the selection is only valid while it holds that frame: take it as
    // it was before the command, then flip it.
    let before_selection = (cmd == "animation.reverseFrames").then(|| (selection(app), app.ui.timeline.anchor));
    let result = app.run(cmd, params);
    crate::frame_strip_ui::ran(app, ctx, cmd);
    let Ok(v) = result else { return };
    if let Some(list) = v.get("selection").and_then(Value::as_array) {
        let sel: std::collections::BTreeSet<usize> = list.iter().filter_map(Value::as_u64).filter_map(|n| usize::try_from(n).ok()).collect();
        let anchor = sel.iter().next().copied().unwrap_or(0);
        set_selection(app, sel, anchor);
    } else if let Some((was, anchor)) = before_selection {
        let n = before.unwrap_or(0);
        let flipped = was.iter().map(|i| n.saturating_sub(1 + i)).collect();
        set_selection(app, flipped, n.saturating_sub(1 + anchor));
    }
}

/// Is the pointer over the Timeline panel? Its keys only work there, so Space, the arrows and ⌘C
/// keep their usual meaning on the canvas and in the other panels.
fn pointer_over(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    app.ui.timeline.open && app.ui.timeline.panel_rect.is_some_and(|r| ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| r.contains(p)))
}

/// Do the Timeline's keys apply now? Not while typing on the canvas, which owns Space and the arrows
/// even with the pointer resting over the panel.
pub(crate) fn keys_apply(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    app.ui.text_edit.is_none() && pointer_over(app, ctx)
}

/// Keys for the Timeline, while the pointer is over it (and no text field has the keyboard):
/// Space plays or pauses; ← → step the selected frame, Home / End jump to the first / last, and
/// with ⇧ they extend the selection; ⌘A selects every frame; ⌥← ⌥→ move the selected frames;
/// ⌘C copies them and ⌘V pastes after the last selected frame; Delete removes them. True when a
/// key was taken.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    if !keys_apply(app, ctx) {
        return false;
    }
    // Read only the numbers the keys need: this runs on every UI frame the pointer is over the panel, and
    // a long animation is big to copy.
    let Some((has_timeline, anim)) =
        app.session.active().map(|d| (d.doc.timeline.is_some(), d.doc.frame_animation.as_ref().map(|a| (a.current, a.frames.len()))))
    else {
        return false;
    };
    let press = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
    if (has_timeline || anim.is_some()) && press(Modifiers::NONE, Key::Space) {
        toggle_play(app, ctx);
        return true;
    }
    let Some((current, len)) = anim else { return false };
    let last = len.saturating_sub(1);
    let sel: Vec<usize> = selection(app).into_iter().collect();
    let (first_sel, last_sel) = (sel.first().copied().unwrap_or(current), sel.last().copied().unwrap_or(current));
    // Copy and Paste reach egui as clipboard events, not key presses; take both forms.
    let clip = |ctx: &egui::Context, copy: bool| {
        ctx.input_mut(|i| {
            let n = i.events.len();
            i.events.retain(|e| if copy { !matches!(e, egui::Event::Copy) } else { !matches!(e, egui::Event::Paste(_)) });
            n != i.events.len()
        })
    };
    if clip(ctx, true) | press(Modifiers::COMMAND, Key::C) {
        // Only the copied frames are cloned, and only now.
        app.ui.timeline.clipboard = app
            .session
            .active()
            .and_then(|d| d.doc.frame_animation.as_ref())
            .map(|a| sel.iter().filter_map(|i| a.frames.get(*i).cloned()).collect())
            .unwrap_or_default();
        return true;
    }
    // With no frames copied, ⌘V is left alone for Edit › Paste (an image on the system clipboard, say).
    if !app.ui.timeline.clipboard.is_empty() && (clip(ctx, false) | press(Modifiers::COMMAND, Key::V)) {
        let frames: Vec<Value> =
            app.ui.timeline.clipboard.iter().map(|f| json!({ "delayMs": f.delay_ms, "hidden": f.hidden.iter().map(|l| l.0).collect::<Vec<_>>() })).collect();
        act(app, ctx, "animation.pasteFrames", json!({ "frames": frames, "after": last_sel }));
        return true;
    }
    if press(Modifiers::COMMAND, Key::A) {
        set_selection(app, (0..len).collect(), 0);
        return true;
    }
    if press(Modifiers::ALT, Key::ArrowLeft) {
        if first_sel > 0 {
            act(app, ctx, "animation.moveFrames", json!({ "frames": sel, "slot": first_sel - 1 }));
        }
        return true;
    }
    if press(Modifiers::ALT, Key::ArrowRight) {
        if last_sel < last {
            act(app, ctx, "animation.moveFrames", json!({ "frames": sel, "slot": last_sel + 2 }));
        }
        return true;
    }
    // ⇧ with an arrow, Home or End extends the selection from its anchor.
    for (k, to) in [(Key::ArrowLeft, current.saturating_sub(1)), (Key::ArrowRight, (current + 1).min(last)), (Key::Home, 0), (Key::End, last)] {
        if press(Modifiers::SHIFT, k) {
            click_frame(app, ctx, to, crate::frame_strip_ui::Click::Range);
            return true;
        }
    }
    let target = if press(Modifiers::NONE, Key::ArrowLeft) {
        Some(current.saturating_sub(1))
    } else if press(Modifiers::NONE, Key::ArrowRight) {
        Some((current + 1).min(last))
    } else if press(Modifiers::NONE, Key::Home) {
        Some(0)
    } else if press(Modifiers::NONE, Key::End) {
        Some(last)
    } else {
        None
    };
    if let Some(frame) = target {
        click_frame(app, ctx, frame, crate::frame_strip_ui::Click::Plain);
        return true;
    }
    if press(Modifiers::NONE, Key::Delete) || press(Modifiers::NONE, Key::Backspace) {
        // Taken even when every frame is selected, so it can't reach Edit › Clear. At least one frame stays:
        // removing the animation is the trash button's job, not a stray key's.
        if sel.len() < len {
            act(app, ctx, "animation.deleteFrame", json!({ "frames": sel }));
        }
        return true;
    }
    false
}

/// The docked panel, along the bottom of the window under the canvas. Add it after the side
/// panels so it spans only the canvas column, and before the central panel.
pub fn dock(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !app.ui.timeline.open || app.ui.timeline.floating {
        return;
    }
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut out = Out::default();
    egui::Panel::bottom("timeline_dock")
        .resizable(true)
        .default_size(DOCK_HEIGHT)
        .size_range(110.0..=420.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(10, 6)))
        .show(ui, |ui| {
            // The separator is on the canvas side.
            let r = ui.max_rect();
            ui.painter().line_segment([pos2(r.left() - 10.0, r.top() - 6.0), pos2(r.right() + 10.0, r.top() - 6.0)], egui::Stroke::new(1.0, t.separator));
            contents(app, ui, &ctx, &mut out);
        });
    apply(app, &ctx, out);
}

/// The floating panel, when the Timeline is set to float, and playback in either mode.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.timeline.open {
        return;
    }
    if app.ui.timeline.floating {
        let mut out = Out::default();
        crate::analysis_ui::panel_window(app, ctx, "timeline", tl!("Timeline"), vec2(0.0, 520.0), 660.0, |ui| contents(app, ui, ctx, &mut out));
        apply(app, ctx, out);
    }
    playback(app, ctx);
}

/// Playback: a frame animation shows each frame for its own delay, then the loop count decides;
/// a video timeline advances at its frame rate and loops.
fn playback(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Frame animation playback: read only what it needs, not a copy of the whole animation.
    if app.ui.timeline.playing
        && let Some((current, len, loops, delay)) = app
            .session
            .active()
            .and_then(|d| d.doc.frame_animation.as_ref())
            .map(|a| (a.current, a.frames.len(), a.loops, a.frames.get(a.current).map_or(0, |f| f.delay_ms)))
    {
        let now = ctx.input(|i| i.time);
        let period = crate::frame_strip_ui::playback_period(delay);
        if now - app.ui.timeline.last_time >= period {
            app.ui.timeline.last_time = now;
            match crate::frame_strip_ui::next_playback_frame(current, len, loops, app.ui.timeline.plays_done) {
                // Counted only once the frame has changed: while a background job runs the engine refuses to
                // select one, and the same tick must not be counted again and again.
                Some((next, plays)) => {
                    if app.run("animation.selectFrame", json!({ "frame": next })).is_ok() {
                        app.ui.timeline.plays_done = plays;
                        crate::frame_strip_ui::ran(app, ctx, "animation.selectFrame");
                    }
                }
                None => app.ui.timeline.playing = false,
            }
        }
        // Wake up when the shown frame's time is up, not on every UI frame: a long delay needs no redraws.
        let shown = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()).map_or(0, |a| a.frames.get(a.current).map_or(0, |f| f.delay_ms));
        let left = (app.ui.timeline.last_time + crate::frame_strip_ui::playback_period(shown) - now).max(0.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        return;
    }
    // Playback: auto-advance the playhead at the frame rate (loops).
    if app.ui.timeline.playing {
        if let Some(t) = app.session.active().and_then(|d| d.doc.timeline.clone()) {
            let now = ctx.input(|i| i.time);
            let period = 1.0 / f64::from(t.fps.max(1.0));
            if now - app.ui.timeline.last_time >= period {
                app.ui.timeline.last_time = now;
                let next = (t.current + 1) % t.duration.max(1);
                let _ = app.run("timeline.setFrame", json!({ "frame": next as u64 }));
            }
            ctx.request_repaint();
        } else {
            app.ui.timeline.playing = false;
        }
    }
}
