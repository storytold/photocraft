//! Window › Timeline: the panel that creates and drives the document timeline. The registry shows `body` in the
//! dock. All actions run the engine `timeline.*` commands, so the control channel drives it the same way.

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::timeline::PlaybackClock;
use craft::timeline::time::{FrameRate, Tick, TimeRange};
#[cfg(test)]
use craft::timeline::ui::TransportWords;
use craft::timeline::ui::{TransportBar, TransportButton};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::runtime::frame::request_repaint_after;
use craft::ui::style::TypeRole;
use craft::ui::value_drag::Slider;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Timeline panel view state: the playback clock. Whether the panel is open is the workspace's.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimelineUi {
    /// Drives playback; the playhead follows its position while it runs.
    #[serde(skip)]
    clock: PlaybackClock,
    /// Frame animation: playing back, each frame for its own delay.
    #[serde(skip)]
    pub(crate) frame_playing: bool,
    /// egui time (seconds) of the last frame change of a frame animation's playback.
    #[serde(skip)]
    pub(crate) last_time: f64,
    /// Frame animation: passes through the frames played so far, for a finite loop count.
    #[serde(skip)]
    pub(crate) plays_done: u32,
}

/// `PlaybackClock` has no equality; two states are equal when both play or both are stopped.
impl PartialEq for TimelineUi {
    fn eq(&self, other: &Self) -> bool {
        self.playing() == other.playing()
    }
}

impl TimelineUi {
    /// Whether playback runs, of a video timeline or of a frame animation.
    pub fn playing(&self) -> bool {
        self.clock.is_playing() || self.frame_playing
    }
}

/// Stops playback with the playhead at the frame the active document shows. For a stop that comes from outside
/// the body (the panel closing), where no frame time is at hand.
pub(crate) fn stop(app: &mut PhotocraftApp) {
    let at = app.session.active().and_then(|d| d.doc.timeline.as_ref()).map_or(Tick(0), |tl| FrameRate::from_f64(f64::from(tl.fps)).tick_of(tl.current as i64));
    app.ui.timeline.clock.halt(at);
    app.ui.timeline.frame_playing = false;
}

/// The transport row: Play/Pause, Stop, then the frame steps.
fn buttons() -> [TransportButton; 4] {
    [TransportButton::PlayStop, TransportButton::custom("stop", IconId::new("square"), "Stop"), TransportButton::StepBack, TransportButton::StepForward]
}

/// The `timeline` panel body.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let now = ui.ctx().input(|i| i.time);
    if app.session.active().is_none() {
        app.ui.timeline.clock.stop(now);
        app.ui.timeline.frame_playing = false;
        let _ = Notice::new(Tone::Neutral, tl!("Open a document to use the timeline.")).layout(NoticeLayout::Centred).show(ui);
        return;
    }
    // A document with a frame animation shows its strip of frames, not a video timeline's transport.
    if app.session.active().is_some_and(|d| d.doc.frame_animation.is_some()) {
        app.ui.timeline.clock.stop(now);
        frame_body(app, ui);
        return;
    }
    app.ui.timeline.frame_playing = false;
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());
    let mut act: Option<(&'static str, Value)> = None;
    let Some(tl) = tl else {
        app.ui.timeline.clock.stop(now);
        // Nothing yet: make a frame animation (of nothing, or of the layers) or a video timeline.
        let ctx = ui.ctx().clone();
        let mut acts = Vec::new();
        crate::frame_strip_ui::empty(ui, &mut acts);
        apply(app, &ctx, acts, false);
        return;
    };

    let rate = FrameRate::from_f64(f64::from(tl.fps));
    let duration = tl.duration as i64;
    let current = tl.current as i64;
    let clock = &mut app.ui.timeline.clock;
    clock.set_loop(Some(TimeRange::new(Tick(0), rate.tick_of(duration.max(1)))));

    // Playback: the playhead follows the clock; the next pass is asked for at the next frame boundary.
    let mut frame_from_clock = None;
    if clock.is_playing() {
        let at = clock.position(now);
        let frame = rate.frame_at(at);
        if frame != current {
            frame_from_clock = Some(frame);
        }
        request_repaint_after(ui.ctx(), rate.tick_of(frame + 1).saturating_sub(at).seconds());
    }
    if let Some(frame) = frame_from_clock {
        let _ = app.run("timeline.setFrame", json!({ "frame": frame as u64 }));
    }
    let current = frame_from_clock.unwrap_or(current);

    let playing = app.ui.timeline.playing();
    let mut stop = false;
    let mut toggle = false;
    let buttons = buttons();
    Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let bar = TransportBar::new(tl!("Transport"), &buttons).playing(playing).editable(false).show(ui);
            match bar.pressed {
                Some(TransportButton::PlayStop) => toggle = true,
                Some(TransportButton::Custom { id, .. }) if id == "stop" => {
                    if !playing {
                        act = Some(("timeline.setFrame", json!({ "frame": 0 })));
                    }
                    stop = true;
                }
                Some(TransportButton::StepBack) => {
                    stop = true;
                    act = Some(("timeline.previousFrame", json!({})));
                }
                Some(TransportButton::StepForward) => {
                    stop = true;
                    act = Some(("timeline.nextFrame", json!({})));
                }
                _ => {}
            }
            let shown =
                crate::i18n::fmt(crate::i18n::t("Frame {current} / {total}"), &[("current", &(current + 1).to_string()), ("total", &tl.duration.to_string())]);
            Text::new(shown).role(TypeRole::Mono).show(ui);
            Text::new(format!("{:.2}s", tl.time())).role(TypeRole::Mono).shade(Shade::Muted).show(ui);
        });
        Row::new().show(ui, |ui, _| {
            if clicked(&IconButton::new(IconId::new("trash")).label(tl!("Delete Timeline")).tooltip(tl!("Delete Timeline")).show(ui)) {
                stop = true;
                act = Some(("timeline.delete", json!({})));
            }
            Text::new(format!("{:.0} fps", tl.fps)).shade(Shade::Muted).show(ui);
        });
    });

    // Scrubber: the playhead across the duration.
    if duration > 1 {
        let slid = Slider::new(Bind::given(current as f64), 0.0..=(duration - 1) as f64).step(1.0).label(tl!("Playhead")).show(ui);
        if slid.events.began() {
            stop = true;
        }
        if let Some(value) = slid.value().copied() {
            stop = true;
            act = Some(("timeline.setFrame", json!({ "frame": value.round() as u64 })));
        }
    }

    if stop {
        app.ui.timeline.clock.stop(now);
    } else if toggle {
        app.ui.timeline.clock.toggle(now, rate.tick_of(current));
    }
    if let Some((cmd, p)) = act {
        let _ = app.run(cmd, p);
    }
}

/// The `timeline` panel body of a document with a frame animation: the strip of its frames, with its bar.
fn frame_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some((doc, revision, no_damage)) = app.session.active().map(|d| (d.doc.clone(), d.revision, d.last_damage.is_some_and(|r| r.is_empty()))) else {
        return;
    };
    let Some(animation) = doc.frame_animation.as_ref() else { return };
    let mut acts = Vec::new();
    let mut toggle_play = false;
    crate::frame_strip_ui::body(
        ui,
        &ctx,
        crate::frame_strip_ui::Strip {
            doc: &doc,
            revision,
            no_damage,
            animation,
            playing: app.ui.timeline.frame_playing,
            acts: &mut acts,
            toggle_play: &mut toggle_play,
        },
    );
    apply(app, &ctx, acts, toggle_play);
    playback(app, &ctx);
}

/// Carries out what the strip (or the empty panel) asked for.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, acts: Vec<crate::frame_strip_ui::Act>, toggle_play: bool) {
    if toggle_play {
        let starting = !app.ui.timeline.frame_playing;
        app.ui.timeline.frame_playing = starting;
        app.ui.timeline.last_time = ctx.input(|i| i.time);
        app.ui.timeline.plays_done = 0;
        if starting {
            restart_if_finished(app, ctx);
        }
    }
    for (cmd, p) in acts {
        let _ = app.run(cmd, p);
        crate::frame_strip_ui::ran(app, ctx, cmd);
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

/// Frame animation playback: each frame shows for its own delay, then the loop count decides.
fn playback(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.timeline.frame_playing {
        return;
    }
    // Reads only what it needs, not a copy of the whole animation.
    let Some((current, len, loops, delay)) = app
        .session
        .active()
        .and_then(|d| d.doc.frame_animation.as_ref())
        .map(|a| (a.current, a.frames.len(), a.loops, a.frames.get(a.current).map_or(0, |f| f.delay_ms)))
    else {
        app.ui.timeline.frame_playing = false;
        return;
    };
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
            None => app.ui.timeline.frame_playing = false,
        }
    }
    // Wake up when the shown frame's time is up, not on every UI frame: a long delay needs no redraws.
    let shown = app.session.active().and_then(|d| d.doc.frame_animation.as_ref()).map_or(0, |a| a.frames.get(a.current).map_or(0, |f| f.delay_ms));
    let left = (app.ui.timeline.last_time + crate::frame_strip_ui::playback_period(shown) - now).max(0.0);
    // The shell's own request: it also runs this body again, which a plain egui repaint of a retained panel doesn't.
    request_repaint_after(ctx, left);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};

    fn frame(h: &Harness<'static, PhotocraftApp>) -> u64 {
        h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current as u64
    }

    /// Starts playback with the Play button, so the clock starts from the playhead's frame.
    fn play(h: &mut Harness<'static, PhotocraftApp>) {
        h.get_by_label(TransportWords::default().play_stop.as_str()).click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.playing());
    }

    #[test]
    fn stop_step_and_scrub_pause_playback_and_second_stop_rewinds() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("timeline.create", json!({"duration": 10, "fps": 30.0})).unwrap();
        let mut h =
            Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 700.0)).with_step_dt(0.001).build_ui_state(|ui, app| body(app, ui), app);
        h.run_steps(2);

        // The Play button starts playback.
        play(&mut h);

        // The first Stop pauses; a second one rewinds to frame 0.
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing(), "first Stop pauses playback");
        h.state_mut().run("timeline.setFrame", json!({"frame": 5})).unwrap();
        h.run_steps(1);
        assert_eq!(frame(&h), 5, "a stopped clock leaves the playhead alone");
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing());
        assert_eq!(frame(&h), 0);

        // The frame steps pause playback.
        let words = TransportWords::default();
        play(&mut h);
        h.get_by_label(words.step_forward.as_str()).click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing(), "Next frame pauses playback");
        assert_eq!(frame(&h), 1);
        play(&mut h);
        h.get_by_label(words.step_back.as_str()).click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing(), "Previous frame pauses playback");
        assert_eq!(frame(&h), 0);

        // Scrubbing the playhead pauses playback and moves the frame.
        play(&mut h);
        let slider = h.get_by_role(egui::accesskit::Role::Slider).rect();
        let target = egui::pos2(slider.right() - 8.0, slider.center().y);
        h.event(egui::Event::PointerMoved(target));
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing(), "scrubbing pauses playback");
        assert!(frame(&h) > 0);
    }
}
