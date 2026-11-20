use super::{
    PhotocraftApp, canvas, discard_ui, gpu_canvas, gpu_status, i18n, jobs_ui, last_brush, monitor_status, native_menu, notices, paint_mouse, panels, prefs_ui,
    screen_picker, shortcuts, smart_svg_ui, tool_memory, transform_tool, type_tool, updates_ui, view_cmds, work_area,
};
use craft::ui::contract::Bind;
use craft::ui::runtime::env::WindowKey;
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::{AppShell, slots};

impl PhotocraftApp {
    /// Close this frame's stage record (`frame_stages`): keep it if it was slow, and hand it to
    /// the stroke trace. `ui_ms` is this pass's UI build. A pass egui will run again
    /// (`request_discard`, such as a sizing pass) leaves the record open, so a frame is one
    /// record across all its passes.
    pub(crate) fn finish_frame(&mut self, ctx: &egui::Context, ui_ms: f64) {
        self.perf.stage.ui_ms += ui_ms;
        self.perf.stage.stroking |= self.live_stroke.is_some();
        if ctx.will_discard() {
            return;
        }
        let stages = std::mem::take(&mut self.perf.stage);
        let stages = self.slow_frames.finish(stages, gpu_canvas::now_ms(), self.perf.gpu, self.perf.gpu_ms);
        let info = &self.perf.gpu_info;
        self.stroke_trace.frame(&stages, stages.stroking, || {
            let gpu = format!("{} {}", info.backend, info.adapter).trim().to_string();
            if info.present.is_empty() { gpu } else { format!("{gpu}, {}", info.present) }
        });
    }

    /// Wait out the rest of [`Self::min_frame_ms`] since the last frame started.
    fn cap_frame_rate(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(min) = self.min_frame_ms.filter(|m| m.is_finite() && *m > 0.0) {
            let wait = min.min(100.0) - (gpu_canvas::now_ms() - self.last_frame_start_ms);
            if wait > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(wait / 1000.0));
            }
        }
        self.last_frame_start_ms = gpu_canvas::now_ms();
    }
}

impl PhotocraftApp {
    /// Whether the Home screen is what the canvas area shows this frame. Files opening in the
    /// background (#210) have tabs before they have documents, so they win over Home. The frame
    /// draws no tool strip, options bar, status bar or docks under Home, and the canvas area
    /// draws Home itself; both ask here so they cannot disagree.
    pub(crate) fn home_screen_up(&mut self) -> bool {
        let opening = !self.jobs.opens.is_empty();
        let n = self.session.documents().len();
        !opening && self.ui.chrome.shows_home(n, self.session.prefs().general.auto_show_home_screen)
    }

    /// Shared logic body. A wrapping host owns the final reset tail after its own work.
    pub fn logic_frame(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.cap_frame_rate();
        // Shortcuts run in this phase: a pass whose input carries an injected step acts for the
        // automation client here too.
        self.automation_input = WindowHost::of(ctx).injected_pass();
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        // Caps Lock once per frame: the state injected input holds (`ui.capsLock`), else the OS's
        // (`Services::caps_lock`); the canvas cursor block needs it before the frame renders.
        // `None` (Wayland/web) keeps `false`.
        let services = &mut self.services;
        self.caps_lock = WindowHost::of(ctx).injected_caps_lock().unwrap_or_else(|| services.caps_lock.as_mut().is_some_and(|f| f()));
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_frame_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_frame_time = now;
        self.sync_views();
        crate::memory_warning::tick(self, now);
        self.check_gpu(ctx);
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        let _ = self.live_tokens.poll(ctx, &self.ui.theme);
        #[cfg(not(target_arch = "wasm32"))]
        self.drain_control(ctx);
        view_cmds::leave_full_screen_without_document(self);
        // A box drag that ended without a release reaching the Type tool keeps its box, before
        // the commit below can end the edit.
        crate::type_box_preview::settle(self);
        // Switching tools commits, also between the Type and Type Mask tools.
        if self.ui.text_edit.as_ref().is_some_and(|ed| !self.ui.tool.is_type() || ed.mask != self.ui.tool.is_type_mask()) {
            type_tool::commit(self);
        }
        // So does selecting another layer or document by any route (#6483).
        type_tool::end_if_left(self);
        transform_tool::end_if_left(self);
        prefs_ui::tick(self, ctx, std::time::Instant::now());
        updates_ui::tick(self);
        monitor_status::poll_frame(self, ctx, frame);
        // Control requests and persisted preferences can change the language in this frame.
        i18n::sync_context(ctx, &self.session.prefs().interface.language);
        // A window bigger than its display (1440 × 900 on 1366 × 768) runs under the taskbar:
        // maximize it into the work area once (#315).
        work_area::fit_window(ctx);
        // The runtime's close path, which asks PhotoCraft when the window was asked to close.
        craft::ui::shell::embed::logic(ctx, || discard_ui::close_decision(self));
        // A component refused its input (a zoom field turning a Tab or Alt away, a dialog field):
        // the alert plays once per frame, whatever the number of refusals.
        if std::mem::take(&mut self.framework.refused)
            && let Some(play) = self.services.alert.or(self.services.beep)
        {
            play();
        }
        // Background jobs: apply finished ones, keep frames coming, Esc cancels (before the
        // shortcuts see Esc).
        if !screen_picker::tick(self, ctx) {
            jobs_ui::tick(self, ctx);
            #[cfg(not(target_arch = "wasm32"))]
            crate::dialogs::filter_preview_worker::discard_closed(self);
            // `fired` and `delivered` come only from the runtime's last pass, so a hidden logic
            // tick finds them taken.
            crate::shortcut_dispatch::record_view_commands(self);
            shortcuts::handle(self, ctx);
        }
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
        // Web drops onto the canvas, read since the last frame: placed below, one at a time.
        self.queue_web_places();
        // Finder double-click / Open With / Dock drops (macOS open-documents events).
        self.drain_os_events(ctx);
        // The main window's file drops reach `open_dropped` through the document area's drop
        // target, and the runtime repaints while files hover.
        crate::file_open::log_drag_over(ctx, ctx.input(|i| i.raw.hovered_files.len()));
        self.place_next_dropped(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let capture_unissued = self.poll_captures(ctx);
        #[cfg(target_arch = "wasm32")]
        let capture_unissued = false;
        // A hidden host runs no UI pass: a queued (or re-queued) capture asks for one. An issued
        // capture's own `Screenshot` command forces its paint; asking again would spin a hidden
        // window while a slow readback is in flight. Synthetic input queued during hidden ticks
        // needs the same wake: its steps wait for UI passes.
        if capture_unissued || !WindowHost::of(ctx).injected_idle() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
    }

    /// Shared UI body.
    pub fn ui_frame(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // A real UI pass, not a hidden `run_logic` tick, which leaves these input values stale:
        // only this phase counts toward settling.
        #[cfg(not(target_arch = "wasm32"))]
        self.note_capture_pass(&ctx);
        // A pass whose input carries an injected step acts for the automation client.
        self.automation_input = WindowHost::of(&ctx).injected_pass();
        self.hover_layer = None;
        // The contextual task bar reports itself again if this frame draws it (#10890).
        self.ui.task_bar_shown = None;
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        let t0 = gpu_canvas::now_ms();
        self.perf.stage.ui_start(t0, frame.info().cpu_usage);
        // A software adapter or CPU mode chosen at startup is every document's reason.
        crate::render_path::frame(self);
        // Every frame that started its stage record closes it, whichever way `ui_body` returns.
        self.ui_body(ui);
        // What an injected step changed is the target of the rest of its request: only a change
        // between passes withdraws queued input (`withdraw_stale_input`).
        #[cfg(not(target_arch = "wasm32"))]
        if self.input_queued_for.is_some() && WindowHost::of(&ctx).injected_pass() {
            self.input_queued_for = Some(self.input_target());
        }
        let ui_ms = gpu_canvas::now_ms() - t0;
        // A GPU measurement lands a frame or two after the refresh it belongs to. Refreshes
        // carry them out, but the last refresh before the app settles has none following it, so
        // drain the timer here too or its number is never reported. Drain it either way: a
        // measurement left sitting would surface later against an unrelated refresh. Report it
        // only while the canvas is still on the GPU, since a CPU refresh clears `gpu_ms`
        // precisely so a GPU number is never shown as that refresh's.
        if let Some(gpu) = &self.gpu
            && let Some(t) = gpu.take_gpu_timing()
            && self.perf.last_refresh.starts_with("gpu")
        {
            self.perf.record_gpu(t);
        }
        self.perf.frame(ui_ms);
        self.finish_frame(&ctx, ui_ms);
        // The window host injects one step per pass and asks for the next pass itself; once it has
        // run every step, the replies waiting on them are answered.
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_input(&ctx);
        // A menu or dialog can start a filter after `jobs_ui::tick` ran in this frame. Keep
        // polling it even when the platform would otherwise put the event loop to sleep.
        jobs_ui::keep_polling(self, &ctx);
        // Menus and buttons in this frame may have asked for a file dialog: the files service
        // answers first, then the dialog's reply is applied once for the pass. The framework's
        // dialog service delivers every reply, the web's async completion included.
        // PLAN: `pump_files` is written by top-w28 (and `Services::files` by dialogs-w59); it does not exist yet.
        #[cfg(not(target_arch = "wasm32"))]
        self.pump_files(frame);
        self.poll_file_dialog();
        // Issue any due `ui.screenshot` now that this pass's state is settled: a queued request
        // that has waited its passes and animation delay captures exactly this frame.
        #[cfg(not(target_arch = "wasm32"))]
        self.issue_captures(&ctx);
        // Last, after everything drawn this frame: a lost window device can't draw it (#3438).
        gpu_status::check_window(self, &ctx);
    }

    /// The frame's UI, between `ui_start` and `finish_frame`.
    fn ui_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Each painting tool keeps its own brush (#218), so switch the active tool's brush in
        // before anything this frame reads it (the cursor, the options bar, a stroke).
        paint_mouse::sync_tool_brush(self);
        // Step by Step and Pause For: play an action's next step once its wait is over (Esc ends it).
        crate::actions::playback::tick(self, &ctx);
        // View › Screen Mode › Full Screen Mode: only the image, on black (F or Esc returns).
        if screen_picker::busy(self) {
            ui.disable();
            ui.set_opacity(1.0);
        }
        // Home is only the menu bar and the Home content; the saved panel prefs are untouched.
        let home = self.home_screen_up();
        let panels_on = &self.ui.panels;
        let (menu_bar, options_bar, status_bar) = (panels_on.menu_bar, panels_on.options_bar, panels_on.status_bar);
        let window_title = panels::title_bar::window_title(self);
        let versions = (panels::region_versions::options_bar(self), panels::region_versions::status_bar(self), panels::region_versions::title_bar(self));
        // The shell binds a copy of the mode, since `show` takes the app too; the copy goes back
        // to the view after the pass.
        let mut mode = self.ui.view.screen_mode;
        // The body is the dock layout whose canvas panel draws the document area, or Home.
        AppShell::new(WindowKey::main())
            .window_title(window_title)
            .screen_mode(Bind::bound(&mut mode))
            .title_bar_if(menu_bar, |ui, _cx, app| panels::title_bar(app, ui))
            .content(slots::TITLE, versions.2)
            .options_bar(options_bar && !home, |ui, _cx, app| panels::options_bar(app, ui))
            .content(slots::OPTIONS, versions.0)
            .status(status_bar && !home, |ui, _cx, app| panels::status_bar(app, ui))
            .content(slots::STATUS, versions.1)
            .body(|ui, cx, app| {
                if home {
                    canvas::document_area(app, ui);
                } else {
                    // Shift+Tab hides the panels and keeps the tools: the toolbar is a panel of the dock.
                    let panels_hidden = cx.shell().is_some_and(|shell| shell.panels_hidden());
                    crate::panel_docking::show(app, ui, panels_hidden);
                }
                notices::show(app, ui);
                smart_svg_ui::host(app, &ui.ctx().clone());
            })
            .show(ui, self);
        self.ui.view.screen_mode = mode;
        // Over the window: the background job and update dialogs before the host runs, then every
        // hosted dialog, then the other OS windows' contents.
        jobs_ui::dialog(self, ui);
        updates_ui::show(self, &ctx);
        // The Keyboard Shortcuts dialog draws while its session is open.
        prefs_ui::show_shortcuts(self, ui);
        crate::dialogs::run(self, ui);
        canvas::extra_windows(self, ui);
        screen_picker::show(self, ui);
        smart_svg_ui::poll(self, &ctx);
        // Esc and the view keys read the window's intents last, once every UI above had its chance at Esc.
        crate::shortcuts::escape_keys(self, &ctx);
        // A device lost while drawing this frame: switch to the CPU canvas before the next one.
        gpu_status::check(self, &ctx);
        // The tool's brush (settings and tip references) is remembered the same way. Like the
        // picker's view, in the frame loop, so the control channel can set it with the panels
        // closed; the engine's revision counter keeps an unchanged brush free.
        last_brush::persist(self, &ctx);
        // Tool options are remembered between launches the same way (#2814).
        tool_memory::persist(self, &ctx);
        // So are View › Extras, Show and Snap To (#3762).
        view_cmds::persist(self, &ctx);
        self.automation_input = false;
        native_menu::sync(self, &ctx);
    }
}
