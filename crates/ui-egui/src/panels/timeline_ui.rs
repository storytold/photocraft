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
    /// Selected Aseprite tag (None plays the entire sprite).
    #[serde(default)]
    pub sprite_tag: Option<usize>,
    #[serde(skip)]
    pub sprite_start: f64,
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

fn delay_control(ui: &mut egui::Ui, document: u64, frame: usize, ms: u16) -> Option<Value> {
    ui.push_id(("sprite-frame-delay", document, frame), |ui| {
        let mut delay = f32::from(ms);
        let response = crate::widgets::value_field_prec(ui, &mut delay, 0.0..=65535.0, "ms", 72.0, 0).accessible_name(tl!("Frame Delay"));
        let key = response.id.with("edit-gesture");
        let stamp = ui.ctx().cumulative_pass_nr();
        if response.drag_started() || response.gained_focus() {
            ui.data_mut(|d| d.insert_temp(key, stamp));
        }
        let edit = response.changed().then(|| {
            let gesture = ui.data_mut(|d| *d.get_temp_mut_or(key, stamp));
            // A new drag/focus session gets its own key, scoped to this document/frame.
            json!({"frame":frame,"durationMs":delay.round() as u16,"coalesce":format!("sprite-delay:{document}:{frame}:{gesture}")})
        });
        if response.drag_stopped() || response.lost_focus() {
            ui.data_mut(|d| d.remove::<u64>(key));
        }
        edit
    })
    .inner
}

/// Render the Timeline panel.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A canvas tab can activate a document after the frame-start view sync.
    app.sync_document_state();
    if !app.ui.timeline.open {
        return;
    }
    let tl = app.session.active().and_then(|d| d.doc.timeline.clone());
    let sprite = app.session.active().and_then(|d| d.doc.sprite.as_ref());
    let sprite_duration = sprite.map(|s| s.frame_duration_ms());
    let delay_target = app.session.active().and_then(|d| d.doc.sprite.as_ref().map(|s| (d.doc.id.0, s.current)));
    let sprite_time = sprite.map(|s| s.frames.iter().take(s.current).map(|f| u64::from(f.duration_ms.max(1))).sum::<u64>() as f64 / 1000.0);
    let sprite_tags = sprite.map(|s| s.tags.iter().map(|t| t.name.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let mut sprite_tag = app.ui.timeline.sprite_tag;
    if sprite_tag.is_some_and(|i| i >= sprite_tags.len()) {
        sprite_tag = None;
    }
    let have_doc = app.session.active().is_some();
    let mut act: Option<(&str, Value)> = None;
    let mut close = false;
    let mut toggle_play = false;
    let mut stop_play = false;
    let playing = app.ui.timeline.playing;
    let t = Tokens::get(ctx);

    crate::analysis_ui::panel_window(app, ctx, "timeline", tl!("Timeline"), vec2(0.0, 520.0), 660.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Timeline")).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    let play_icon = if playing { "pause" } else { "play" };
                    if crate::icons::button(ui, play_icon, 22.0, playing, if playing { tl!("Pause") } else { tl!("Play") }).clicked() {
                        toggle_play = true;
                    }
                    if crate::icons::button(ui, "square", 22.0, false, tl!("Stop")).clicked() {
                        if !playing {
                            act = Some(("timeline.setFrame", json!({ "frame": 0 })));
                        }
                        stop_play = true;
                    }
                    if crate::icons::button(ui, "chevron-left", 22.0, false, tl!("Previous frame")).clicked() {
                        stop_play = true;
                        act = Some(("timeline.previousFrame", json!({})));
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
                        stop_play = true;
                        act = Some(("timeline.nextFrame", json!({})));
                    }
                    ui.separator();
                    ui.label(RichText::new(format!("{:.3}s", sprite_time.unwrap_or(f64::from(tl.time())))).color(t.text_dim).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if sprite_duration.is_none() && crate::icons::button(ui, "trash", 20.0, false, tl!("Delete Timeline")).clicked() {
                            stop_play = true;
                            act = Some(("timeline.delete", json!({})));
                        }
                        if let (Some(ms), Some((document, frame))) = (sprite_duration, delay_target) {
                            if let Some(params) = delay_control(ui, document, frame, ms) {
                                stop_play = true;
                                act = Some(("sprite.setDuration", params));
                            }
                        } else {
                            ui.label(RichText::new(format!("{:.0} fps", tl.fps)).color(t.text_dim));
                        }
                    });
                });
                ui.add_space(6.0);
                if sprite_duration.is_some() && !sprite_tags.is_empty() {
                    let mut selected = sprite_tag;
                    if selected.is_some_and(|i| i >= sprite_tags.len()) {
                        selected = None;
                    }
                    let _ = craft_ui::ComboBox::from_id_salt("sprite-animation-tag")
                        .selected_text(selected.and_then(|i| sprite_tags.get(i)).cloned().unwrap_or_else(|| tl!("All").to_owned()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut selected, None, tl!("All"));
                            for (i, name) in sprite_tags.iter().enumerate() {
                                ui.selectable_value(&mut selected, Some(i), name);
                            }
                        })
                        .response
                        .accessible_name(tl!("Animation"));
                    if selected != sprite_tag {
                        sprite_tag = selected;
                        stop_play = true;
                    }
                }
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

    app.ui.timeline.sprite_tag = sprite_tag;
    if close {
        app.ui.timeline.open = false;
        app.ui.timeline.playing = false;
    }
    if stop_play {
        app.ui.timeline.playing = false;
    } else if toggle_play {
        app.ui.timeline.playing = !app.ui.timeline.playing;
        app.ui.timeline.last_time = ctx.input(|i| i.time);
        app.ui.timeline.sprite_start = app.ui.timeline.last_time - sprite_time.unwrap_or(0.0);
        if app.ui.timeline.playing
            && let Some(tag) = app.ui.timeline.sprite_tag
            && let Some(start) = app.session.active().and_then(|d| d.doc.sprite.as_ref()).and_then(|s| s.playback_frame(0, Some(tag)).ok()).map(|v| v.0)
        {
            let _ = app.run("timeline.setFrame", json!({"frame":start}));
            app.ui.timeline.sprite_start = app.ui.timeline.last_time;
        }
    }
    if let Some((cmd, p)) = act {
        let _ = app.run(cmd, p);
    }
    // Playback: auto-advance the playhead at the frame rate (loops).
    if app.ui.timeline.playing {
        if app.session.active().is_some_and(|d| d.doc.sprite.is_some()) {
            let elapsed = ((ctx.input(|i| i.time) - app.ui.timeline.sprite_start).max(0.0) * 1000.0) as u64;
            if let Ok(result) = app.run("sprite.playback", json!({"elapsedMs":elapsed,"tag":app.ui.timeline.sprite_tag})) {
                if let Some(frame) = result.get("frame").and_then(Value::as_u64)
                    && app.session.active().and_then(|d| d.doc.timeline.as_ref()).is_some_and(|t| t.current != frame as usize)
                {
                    let _ = app.run("timeline.setFrame", json!({"frame":frame}));
                }
                if result.get("finished").and_then(Value::as_bool) == Some(true) {
                    app.ui.timeline.playing = false;
                }
            } else {
                app.ui.timeline.playing = false;
            }
            ctx.request_repaint();
            return;
        }
        if let Some(t) = app.session.active().and_then(|d| d.doc.timeline.clone()) {
            let now = ctx.input(|i| i.time);
            let period = app
                .session
                .active()
                .and_then(|d| d.doc.sprite.as_ref())
                .map_or_else(|| 1.0 / f64::from(t.fps.max(1.0)), |s| f64::from(s.frame_duration_ms().max(1)) / 1000.0);
            if now - app.ui.timeline.last_time >= period {
                app.ui.timeline.last_time += period;
                // A sleeping/hidden window resumes from now instead of spending minutes
                // catching up. Normal fractional periods accumulate without clock drift.
                if now - app.ui.timeline.last_time > 1.0 {
                    app.ui.timeline.last_time = now;
                }
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

    fn sprite_timeline() -> Harness<'static, PhotocraftApp> {
        let doc = photocraft_io::import("animation.aseprite", include_bytes!("../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc, None);
        app.sync_views();
        app.ui.timeline.open = true;
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(0.01).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);
        h
    }

    fn delay(h: &Harness<'_, PhotocraftApp>) -> u16 {
        h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().frame_duration_ms()
    }

    #[test]
    fn delay_drags_are_separate_single_undo_steps_with_redo() {
        let mut h = sprite_timeline();
        let original = delay(&h);
        let mut values = Vec::new();
        for gesture in 1..=2 {
            let start = h.get_by_role(egui::accesskit::Role::SpinButton).rect().center();
            h.hover_at(start);
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            let before = delay(&h);
            for dx in [12.0, 24.0, 40.0] {
                h.hover_at(start + egui::vec2(dx, 0.0));
                h.run_steps(1);
            }
            h.event(egui::Event::PointerButton {
                pos: start + egui::vec2(40.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(2);
            assert_ne!(delay(&h), before, "drag changes the delay");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), gesture, "one step per drag");
            values.push(delay(&h));
        }
        for (command, expected) in [("edit.undo", values[0]), ("edit.undo", original), ("edit.redo", values[0]), ("edit.redo", values[1])] {
            h.state_mut().run(command, json!({})).unwrap();
            h.run_steps(1);
            assert_eq!(delay(&h), expected);
        }
    }

    #[test]
    fn delay_typing_coalesces_per_focus_session() {
        let mut h = sprite_timeline();
        for (steps, text, ms) in [(1, "123", 123), (2, "456", 456)] {
            h.get_by_role(egui::accesskit::Role::SpinButton).click();
            h.run_steps(1);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run_steps(1);
            for ch in text.chars() {
                h.event(egui::Event::Text(ch.to_string()));
                h.run_steps(1);
            }
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
            assert_eq!(delay(&h), ms);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "one step per typed value");
        }
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(1);
        assert_eq!(delay(&h), 123);
    }

    #[test]
    fn aseprite_timeline_uses_millisecond_delays_and_stops_at_a_finite_tag_end() {
        let doc = photocraft_io::import("animation.aseprite", include_bytes!("../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc, None);
        app.sync_views();
        app.ui.timeline.open = true;
        app.ui.timeline.playing = true;
        app.ui.timeline.sprite_start = 0.0;
        let ctx = egui::Context::default();
        for (seconds, expected) in [(0.0, 0), (0.016, 0), (0.018, 1), (0.099, 1), (0.101, 2), (0.130, 0)] {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(seconds), ..Default::default() }, |ui| windows(&mut app, ui.ctx()));
            output.textures_delta.clear();
            assert_eq!(app.session.active().unwrap().doc.timeline.as_ref().unwrap().current, expected);
        }
        app.ui.timeline.sprite_tag = Some(0);
        let mut output = ctx.run_ui(egui::RawInput { time: Some(0.201), ..Default::default() }, |ui| windows(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.ui.timeline.playing, "two ping-pong traversals have completed");
        assert_eq!(app.session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);
        assert_eq!(app.session.active().unwrap().history.past_len(), 0, "playback adds no history");
    }

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
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::sync::Arc;

    fn app() -> PhotocraftApp {
        let mut doc = photocraft_io::import("animation.aseprite", include_bytes!("../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
        let sprite = doc.sprite.as_mut().unwrap();
        for frame in &mut sprite.frames {
            Arc::make_mut(frame).duration_ms = 1000;
        }
        sprite.tags[0].from = 1;
        sprite.tags[0].to = 2;
        sprite.tags[0].direction = 0;
        sprite.tags[0].repeat = 0;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc, None);
        app.sync_views();
        app.ui.timeline.open = true;
        app.ui.timeline.sprite_tag = Some(0);
        app
    }

    #[test]
    fn pausing_a_tag_keeps_the_displayed_frame_and_play_starts_at_the_tag_start() {
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(0.01).build_ui_state(|ui, app| windows(app, ui.ctx()), app());
        h.run_steps(2);
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 1);
        let now = h.ctx.input(|i| i.time);
        h.state_mut().ui.timeline.sprite_start = now - 1.5;
        h.run_steps(1);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 2);
        h.get_by_label("Pause").click();
        h.run_steps(2);
        assert!(!h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 2);
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 1);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    }

    #[test]
    fn selected_tags_and_playback_belong_to_the_active_document_and_survive_ordinary_edits() {
        let mut app = app();
        app.ui.timeline.playing = true;
        app.run("sprite.setDuration", json!({"durationMs": 900})).unwrap();
        assert_eq!(app.ui.timeline.sprite_tag, Some(0));
        assert!(app.ui.timeline.playing);
        let second = photocraft_io::import("second.aseprite", include_bytes!("../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
        app.session.add_document(second, None);
        app.sync_views();
        assert_eq!(app.ui.timeline.sprite_tag, None, "the same tag index in another sprite isn't a selection");
        assert!(!app.ui.timeline.playing);
        app.ui.timeline.sprite_tag = Some(0);
        app.run("document.activate", json!({"document": 0})).unwrap();
        assert_eq!(app.ui.timeline.sprite_tag, None);
        assert!(!app.ui.timeline.playing);
    }

    #[test]
    fn tag_reordering_or_removal_clears_selection_and_playback_but_duration_edits_do_not() {
        let mut app = app();
        app.session
            .edit("Tags", |doc, _| {
                let tags = &mut doc.sprite.as_mut().unwrap().tags;
                let mut other = tags[0].clone();
                other.name = "Other".into();
                tags.push(other);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        app.ui.timeline.sprite_tag = Some(0);
        app.ui.timeline.playing = true;
        app.session
            .edit("Reorder tags", |doc, _| {
                doc.sprite.as_mut().unwrap().tags.swap(0, 1);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        assert_eq!(app.ui.timeline.sprite_tag, None);
        assert!(!app.ui.timeline.playing);
        app.ui.timeline.sprite_tag = Some(0);
        app.ui.timeline.playing = true;
        app.session
            .edit("Remove tag", |doc, _| {
                doc.sprite.as_mut().unwrap().tags.remove(0);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        assert_eq!(app.ui.timeline.sprite_tag, None, "an in-range index must not silently select the next tag");
        assert!(!app.ui.timeline.playing);
    }

    #[test]
    fn timeline_checks_a_raw_tab_activation_before_advancing_the_new_document() {
        let mut app = app();
        let second = photocraft_io::import("second.aseprite", include_bytes!("../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
        app.session.add_document(second, None);
        app.sync_views();
        app.ui.timeline.sprite_tag = Some(0);
        app.ui.timeline.playing = true;
        app.ui.timeline.sprite_start = -10.0;
        app.session.set_active(0); // canvas tab activation, after frame-start sync
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(1);
        assert_eq!(h.state().ui.timeline.sprite_tag, None);
        assert!(!h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 0);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    }

    #[test]
    fn untagged_play_uses_the_playback_clock_for_preceding_zero_duration_frames() {
        let mut app = app();
        app.run("sprite.setDuration", json!({"frame":0,"durationMs":0})).unwrap();
        app.run("timeline.setFrame", json!({"frame":1})).unwrap();
        app.ui.timeline.sprite_tag = None;
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(0.0001).build_ui_state(|ui, app| windows(app, ui.ctx()), app);
        h.run_steps(2);
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert!(h.state().ui.timeline.playing);
        assert_eq!(h.state().session.active().unwrap().doc.sprite.as_ref().unwrap().current, 1);
    }
}
