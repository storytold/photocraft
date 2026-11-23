//! Window › Timeline: the panel that creates and drives the document timeline. The registry shows `body` in the
//! dock. All actions run the engine `timeline.*` commands, so the control channel drives it the same way.

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::timeline::PlaybackClock;
use craft::timeline::time::{FrameRate, Tick, TimeRange};
#[cfg(test)]
use craft::timeline::ui::TransportWords;
use craft::timeline::ui::{TransportBar, TransportButton};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::{Row, Stack};
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
}

/// `PlaybackClock` has no equality; two states are equal when both play or both are stopped.
impl PartialEq for TimelineUi {
    fn eq(&self, other: &Self) -> bool {
        self.playing() == other.playing()
    }
}

impl TimelineUi {
    /// Whether playback runs.
    pub fn playing(&self) -> bool {
        self.clock.is_playing()
    }
}

/// Stops playback with the playhead at the frame the active document shows. For a stop that comes from outside
/// the body (the panel closing), where no frame time is at hand.
pub(crate) fn stop(app: &mut PhotocraftApp) {
    let at = app.session.active().and_then(|d| d.doc.timeline.as_ref()).map_or(Tick(0), |tl| FrameRate::from_f64(f64::from(tl.fps)).tick_of(tl.current as i64));
    app.ui.timeline.clock.halt(at);
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
        let _ = Notice::new(Tone::Neutral, tl!("Open a document to use the timeline.")).layout(NoticeLayout::Centred).show(ui);
        return;
    }
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());
    let mut act: Option<(&'static str, Value)> = None;
    let Some(tl) = tl else {
        app.ui.timeline.clock.stop(now);
        Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                if clicked(&Button::new(tl!("Create Video Timeline")).show(ui)) {
                    act = Some(("timeline.create", json!({ "duration": 30, "fps": 30.0 })));
                }
                Text::new(tl!("30 frames @ 30 fps")).shade(Shade::Muted).show(ui);
            });
        });
        if let Some((cmd, p)) = act {
            let _ = app.run(cmd, p);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};

    fn frame(h: &Harness<'static, Box<PhotocraftApp>>) -> u64 {
        h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current as u64
    }

    /// Starts playback with the Play button, so the clock starts from the playhead's frame.
    fn play(h: &mut Harness<'static, Box<PhotocraftApp>>) {
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
