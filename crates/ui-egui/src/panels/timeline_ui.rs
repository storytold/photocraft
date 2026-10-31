//! Window › Timeline: a floating panel that creates and drives the document timeline. All actions
//! run the engine `timeline.*` commands, so the control channel drives it the same way.

use egui::{RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Timeline panel view state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineUi {
    pub open: bool,
    /// Playing back (auto-advancing the playhead at the frame rate).
    #[serde(default)]
    pub playing: bool,
    /// egui time (seconds) of the last auto-advance.
    #[serde(default, skip)]
    pub last_time: f64,
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

/// [`crate::icons::button`] twin whose glyph never mirrors: the frame-stepping chevrons are
/// media transport on the LTR time axis, which keeps its arrows in a right-to-left UI
/// (craft-libs `Unmirrored::MediaTransport`). Same chrome as the shared button; only the glyph
/// differs, via [`crate::icons::paint_ltr`].
fn button_ltr(ui: &mut egui::Ui, name: &str, box_size: f32, selected: bool, tooltip: &str) -> egui::Response {
    use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
    let t = Tokens::get(ui.ctx());
    let style = IconButtonStyle {
        normal: ButtonVisuals { fill: egui::Color32::TRANSPARENT, stroke: egui::Stroke::NONE, icon: t.icon },
        hovered: ButtonVisuals { fill: t.hover, stroke: egui::Stroke::NONE, icon: t.text },
        pressed: None,
        selected: ButtonVisuals { fill: t.accent_soft, stroke: egui::Stroke::new(1.0, t.accent_border), icon: t.accent_text },
        corner_radius: t.radius_sm.into(),
        focus_stroke: egui::Stroke::new(1.0, t.accent),
    };
    let label = if tooltip.is_empty() { name } else { tl!(tooltip) };
    let resp = IconButton::new(label, vec2(box_size, box_size), &style)
        .selected(selected)
        .show(ui, |ui, rect, tint| crate::icons::paint_ltr(ui, rect, name, (box_size * 0.52).round(), tint));
    if tooltip.is_empty() { resp } else { resp.on_hover_text(label) }
}

/// One control of the physical transport strip, in left-to-right screen order: the frame axis
/// runs left to right like the canvas, so the order holds in a right-to-left UI too.
#[derive(Clone, Copy)]
enum Transport {
    Play,
    Stop,
    Prev,
    Frame,
    Next,
    Gap,
    Time,
}

const TRANSPORT: [Transport; 7] = [Transport::Play, Transport::Stop, Transport::Prev, Transport::Frame, Transport::Next, Transport::Gap, Transport::Time];

/// The transport strip inside a row already laid out in its screen direction: emitted in table
/// order under a left-to-right layout, in reverse under a right-to-left one, so the drawn order
/// is always the physical left-to-right transport order ([`TRANSPORT`]). The reversed emission
/// also reverses the strip's registration (tab/focus) order under RTL; accepted because the
/// strip is an unmirrored physical island and mirroring focus would fight the mirrored row.
fn transport(
    ui: &mut egui::Ui,
    playing: bool,
    rtl: bool,
    act: &mut Option<(&'static str, Value)>,
    stop_play: &mut bool,
    toggle_play: &mut bool,
    tl: &photocraft_doc::Timeline,
) {
    for i in 0..TRANSPORT.len() {
        let item = if rtl { TRANSPORT[TRANSPORT.len() - 1 - i] } else { TRANSPORT[i] };
        match item {
            Transport::Play => {
                let play_icon = if playing { "pause" } else { "play" };
                if crate::icons::button(ui, play_icon, 22.0, playing, if playing { tl!("Pause") } else { tl!("Play") }).clicked() {
                    *toggle_play = true;
                }
            }
            Transport::Stop => {
                if crate::icons::button(ui, "square", 22.0, false, tl!("Stop")).clicked() {
                    if !playing {
                        *act = Some(("timeline.setFrame", json!({ "frame": 0 })));
                    }
                    *stop_play = true;
                }
            }
            Transport::Prev => {
                if button_ltr(ui, "chevron-left", 22.0, false, tl!("Previous frame")).clicked() {
                    *stop_play = true;
                    *act = Some(("timeline.previousFrame", json!({})));
                }
            }
            Transport::Frame => {
                ui.label(
                    RichText::new(crate::i18n::fmt(
                        crate::i18n::t("Frame {current} / {total}"),
                        &[("current", &(tl.current + 1).to_string()), ("total", &tl.duration.to_string())],
                    ))
                    .color(Tokens::get(ui.ctx()).text)
                    .monospace(),
                );
            }
            Transport::Next => {
                if button_ltr(ui, "chevron-right", 22.0, false, tl!("Next frame")).clicked() {
                    *stop_play = true;
                    *act = Some(("timeline.nextFrame", json!({})));
                }
            }
            Transport::Gap => {
                ui.separator();
            }
            Transport::Time => {
                ui.label(RichText::new(format!("{:.2}s", tl.time())).color(Tokens::get(ui.ctx()).text_dim).monospace());
            }
        }
    }
}

/// Render the Timeline panel.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.timeline.open {
        return;
    }
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());
    let have_doc = app.session.active().is_some();
    let mut act: Option<(&'static str, Value)> = None;
    let mut close = false;
    let mut toggle_play = false;
    let mut stop_play = false;
    let playing = app.ui.timeline.playing;
    let t = Tokens::get(ctx);

    crate::analysis_ui::panel_window(app, ctx, "timeline", tl!("Timeline"), vec2(0.0, 520.0), 660.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Timeline")).strong().color(t.text));
            ui.with_layout(crate::direction::trailing(), |ui| {
                if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        ui.separator();

        if !have_doc {
            ui.weak(tl!("Open a document to use the timeline."));
            return;
        }
        match &tl {
            None => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    if crate::widgets::primary_button(ui, tl!("Create Video Timeline"), 200.0).clicked() {
                        act = Some(("timeline.create", json!({ "duration": 30, "fps": 30.0 })));
                    }
                    ui.add_space(4.0);
                    ui.weak(tl!("30 frames @ 30 fps"));
                });
            }
            Some(tl) => {
                ui.horizontal(|ui| {
                    // Media transport is physical: the frame axis runs LTR like the canvas,
                    // so the controls keep their physical order and hug the reading start,
                    // while the delete cluster takes the reading end.
                    let rtl = crate::direction::is_rtl();
                    ui.with_layout(crate::direction::row(), |ui| {
                        transport(ui, playing, rtl, &mut act, &mut stop_play, &mut toggle_play, tl);
                    });
                    ui.with_layout(crate::direction::trailing(), |ui| {
                        if crate::icons::button(ui, "trash", 20.0, false, tl!("Delete Timeline")).clicked() {
                            stop_play = true;
                            act = Some(("timeline.delete", json!({})));
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
                        stop_play = true;
                        act = Some(("timeline.setFrame", json!({ "frame": frame as u64 })));
                    }
                }
            }
        }
    });

    if close {
        app.ui.timeline.open = false;
        app.ui.timeline.playing = false;
    }
    if stop_play {
        app.ui.timeline.playing = false;
    } else if toggle_play {
        app.ui.timeline.playing = !app.ui.timeline.playing;
        app.ui.timeline.last_time = ctx.input(|i| i.time);
    }
    if let Some((cmd, p)) = act {
        let _ = app.run(cmd, p);
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn stop_step_and_scrub_pause_playback_and_second_stop_rewinds() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("timeline.create", json!({"duration": 10, "fps": 30.0})).unwrap();
        app.ui.timeline.open = true;
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0));

        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.001).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);

        // Start playback via Play button.
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.playing);

        // Move to frame 5 while playing; first Stop pauses at the current frame without rewinding.
        h.state_mut().run("timeline.setFrame", json!({"frame": 5})).unwrap();
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "first Stop pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 5);

        // Second Stop while already paused rewinds to frame 0.
        h.get_by_label("Stop").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);

        // Next frame and Previous frame pause playback.
        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        h.get_by_label("Next frame").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "Next frame pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 1);

        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        h.get_by_label("Previous frame").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "Previous frame pauses playback");
        assert_eq!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);

        // Scrubbing the playhead slider pauses playback and updates the current frame.
        h.state_mut().ui.timeline.playing = true;
        h.run_steps(1);
        let slider = h.get_by_role(egui::accesskit::Role::Slider).rect();
        let target = egui::pos2(slider.right() - 8.0, slider.center().y);
        h.event(egui::Event::PointerMoved(target));
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing, "scrubbing pauses playback");
        assert!(h.state().session.active().unwrap().doc.timeline.as_ref().unwrap().current > 0);
    }

    /// RTL twin of the transport test: the window chrome mirrors — the close button takes the
    /// reading end (the window's left) and the delete cluster follows it there — while the
    /// media transport stays physical, in its left-to-right order on the LTR frame axis.
    #[test]
    fn rtl_the_chrome_mirrors_and_the_transport_stays_physical() {
        let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("timeline.create", json!({"duration": 10, "fps": 30.0})).unwrap();
        app.ui.timeline.open = true;
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0));

        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.001).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);

        let window = h.get_by_role(egui::accesskit::Role::Window).rect();
        let close = h.get_by_label("Close").rect();
        assert!(close.left() - window.left() < 24.0, "the close button takes the reading end: {close:?} in {window:?}");
        let play = h.get_by_label("Play").rect();
        let prev = h.get_by_label("Previous frame").rect();
        let next = h.get_by_label("Next frame").rect();
        assert!(play.right() <= prev.left() && prev.right() <= next.left(), "the transport keeps its LTR order: {play:?} {prev:?} {next:?}");
        let trash = h.get_by_label("Delete Timeline").rect();
        assert!(trash.right() <= play.left(), "the delete cluster takes the reading end: {trash:?} vs {play:?}");
    }
}
