use super::services::{ExportSettings, FormatChoices, SAVE_OPTIONS_OPEN, SaveKind, Services, plain_raster, saved_revision_after_write};
use super::{
    DocId, Document, HashMap, PhotocraftApp, Session, UiState, Value, actions, canvas, delete_layer_prompt, discard_ui, file_open, gpu_canvas, gpu_status,
    histogram_panel, i18n, jobs_ui, json, kys_import, last_brush, layers_panel_ui, mask_thumbs_ui, mask_trash_prompt, monitor_status, native_menu, notices,
    open_prompt, paint_mouse, panel_docking, panels, prefs_ui, preset_files_ui, screen_picker, shortcuts, smart_svg_ui, state, stroke_trace, stylus, theme,
    tool_memory, transform_tool, type_tool, type_transform, updates_ui, vector_ui, view_cmds,
};

mod clipboard;
mod commands;
#[cfg(not(target_arch = "wasm32"))]
mod control;
pub(crate) mod doc_queries;
mod files;
pub(crate) use files::ensure_memory_to_save;
mod frame;
pub(crate) mod framework;
mod gpu;
mod prefs;
mod targets;
pub(crate) mod thumbs;
mod view;

pub(crate) use targets::adjustment_layer_active;
pub(crate) use thumbs::layer_thumb_frame;

impl PhotocraftApp {
    /// Creates the application state without a window: it makes the one context it runs on,
    /// installs the framework on it ([`Self::setup_context`], the default theme) and keeps it. The
    /// desktop and web apps build theirs with [`Self::with_context`].
    pub fn new(session: Session, services: Services) -> Box<Self> {
        let ctx = egui::Context::default();
        Self::setup_context(&ctx, theme::AppTheme::default());
        Self::with_context(ctx, session, services)
    }

    /// Creates the application state on `ctx`, the window's context (eframe's `cc.egui_ctx`),
    /// managing the engine session and UI services. It installs nothing: the caller has run
    /// [`Self::setup_window`] on `ctx`. The app keeps `ctx` for its whole life. The
    /// session keeps the services' automation policy as the gate actions automation wrote play
    /// under.
    pub fn with_context(ctx: egui::Context, mut session: Session, services: Services) -> Box<Self> {
        // Actions automation wrote keep this gate when the user plays them. Batch and droplets
        // copy steps instead of the action, so they do not keep the gate yet.
        session.automation_gate = session.automation_gate.or(services.automation_authorize);
        // PLAN: dialogs-w59 adds `Services::files` (assumed `Option<Arc<dyn craft::host::Files>>`).
        let framework = framework::Framework::new(services.files.clone());
        let pen_feed = stylus::StylusFeed::of(&ctx);
        let mut app = Self {
            session,
            ui: UiState::default(),
            services,
            canvases: HashMap::new(),
            raster_logs: HashMap::new(),
            navigator_textures: HashMap::new(),
            display_memo: HashMap::new(),
            navigator_gpu: HashMap::new(),
            navigator_plain: std::collections::HashSet::new(),
            streams: HashMap::new(),
            info_samplers: None,
            monitors: Default::default(),
            thumbnail_cache: std::sync::Arc::new(std::sync::Mutex::new(crate::home_recent_ui::ThumbnailCache::default())),
            canvas_tools: Default::default(),
            live_stroke: None,
            committed_stroke: None,
            retouch_from_live: false,
            trail: None,
            remove_pending: Vec::new(),
            remove_pending_target: None,
            remove_trail: None,
            texture_serials: Default::default(),
            menu_model: None,
            toolbar_slots: Default::default(),
            dock_model: Default::default(),
            preset_art: Default::default(),
            move_preview: None,
            move_hover: None,
            move_distance: None,
            object_finder: None,
            blend_preview: None,
            reorder_preview: None,
            reorder_preview_hit: false,
            patch_preview: None,
            cam_preview: None,
            data_set_preview: Default::default(),
            held_erase: false,
            last_stroke_end: None,
            brush_resize: Default::default(),
            move_pick: None,
            caps_lock: false,
            opacity_keys: craft::behavior::DigitKeys::default(),
            #[cfg(not(target_arch = "wasm32"))]
            control: None,
            live_adjust: None,
            frame: 0,
            notices_live: notices::Live::default(),
            low_memory: crate::memory_warning::Watch::default(),
            last_canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            last_canvas_layer: egui::LayerId::background(),
            ppp: 1.0,
            drop_canvas_rect: None,
            dock_drop_rect: None,
            ctx,
            frame_preview: None,
            drop_places: Default::default(),
            view_docs: Vec::new(),
            path_document: None,
            fps: 0.0,
            last_frame_time: 0.0,
            bounds_cache: HashMap::new(),
            proxy: None,
            proxy_uploaded: None,
            guide_edit: None,
            filter_preview: None,
            filter_live: craft::imaging::ui::LivePreview::new(craft::imaging::ui::PreviewLane::Dialog),
            adjust_open: None,
            adjust_session: None,
            properties_hidden: 0,
            properties_tracked: false,
            dialog_dropper: None,
            layer_style_child: None,
            color_targets: HashMap::new(),
            color_answers: Vec::new(),
            screen_pick: Default::default(),
            caf: None,
            liquify: None,
            gallery: None,
            vanishing: None,
            layer_style_target: None,
            select_mask_target: None,
            guide_layout_doc: None,
            export_open: None,
            pending_reveal: None,
            pattern_fill: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            filter_preview_worker: Default::default(),
            redraws: Default::default(),
            kept: Default::default(),
            color_range: None,
            adjust_preview: None,
            live_adjust_preview: None,
            levels_clipping: None,
            #[cfg(not(target_arch = "wasm32"))]
            input_queued_for: None,
            automation_input: false,
            programmatic_playback: false,
            run_depth: 0,
            paused_playbacks: Default::default(),
            type_layout: None,
            type_path_preview: None,
            type_transform_preview: None,
            type_path_damage: None,
            type_box_preview: None,
            type_font_preview: None,
            color_picker_preview: None,
            layer_box: None,
            finished_pens: Vec::new(),
            hover_doc: None,
            commands_run: 0,
            hover_layer: None,
            layers_rename: None,
            smart_filter_picks: HashMap::new(),
            find_layers_focus: false,
            canvas_hover_layer: None,
            clone_preview: None,
            info_sample: None,
            os_clip_sig: None,
            os_clip_svg: None,
            clip_unmirrored: true,
            clip_external: false,
            clip_read_for_paste: false,
            smart_svg: Default::default(),
            transform_preview: None,
            transform_area: None,
            transform_preparing: None,
            move_mods: Default::default(),
            nudge_bundle: Default::default(),
            style_preview: None,
            text_style_preview: None,
            solid_fill_preview: None,
            shape_fill_preview: None,
            distort: Default::default(),
            gradient: Default::default(),
            raw: None,
            raw_reply: None,
            raw_shelf: Vec::new(),
            raw_redevelop: Vec::new(),
            wide_angle: None,
            color_picker_live: Vec::new(),
            after_transform_commit: None,
            tone_hist: None,
            doc_hist: None,
            doc_hist_preview: None,
            hist_job: None,
            hist_uncached: None,
            hist_sel_key: None,
            gpu: None,
            window_device: None,
            window_lost: false,
            window_recovery_retry: None,
            gpu_render_state: None,
            gpu_fault: None,
            gpu_oom_retries: 0,
            started: None,
            perf: Default::default(),
            stroke_trace: stroke_trace::StrokeTrace::from_env(),
            render_path: Default::default(),
            slow_frames: Default::default(),
            prefs_rt: Default::default(),
            updates_rt: Default::default(),
            discard: None,
            parked_save: None,
            save_dirs: HashMap::new(),
            disk_stamps: HashMap::new(),
            held_opens: Default::default(),
            held_memory_prompts: Default::default(),
            prompt_order: Default::default(),
            held_record_again: None,
            held_insert_menu: None,
            allow_close: false,
            framework,
            file_dialog: None,
            tiff_remembered: Default::default(),
            brush_tip: None,
            snap_cache: Default::default(),
            doc_queries: Default::default(),
            workspaces: Default::default(),
            // Off until `prefs_ui::load` sets the saved interval.
            autosave: craft::platform::autosave::Autosave::new(None),
            // The feed the runtime was installed with: each pointer the canvas takes carries the pen's reading.
            stylus: stylus::Stylus::over(pen_feed),
            background_jobs: false,
            min_frame_ms: None,
            last_frame_start_ms: 0.0,
            jobs: Default::default(),
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            live_tokens: theme::live::LiveTokens::from_env(),
        };
        #[cfg(all(test, not(target_arch = "wasm32")))]
        photocraft_raster::spill::use_test_volume_space();
        // Saved preferences are in place before the first frame; recovery starts in upkeep.
        prefs_ui::load(&mut app);
        // The workspace set reads the loaded preferences (and moves the earlier saves into it).
        crate::workspace_ui::load(&mut app);
        layers_panel_ui::restore_options(&mut app);
        histogram_panel::restore_options(&mut app);
        // The dialogs' last-used values the preferences still hold move to the framework's saved
        // memory before the first frame.
        app.framework.move_memories(&mut app.session);
        // After the saved preferences, which say whether the set was already offered.
        kys_import::offer(&mut app);
        // File › Scripts › Script Events Manager: "Start Application".
        photocraft_engine::automate_cmds::fire_event(&mut app.session, "startApplication");
        Box::new(app)
    }

    /// The window's egui context: the one every pass of the app runs on. SAVED-tier reads and
    /// writes and code that holds no context of its own use it.
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }
}

impl eframe::App for PhotocraftApp {
    fn on_exit(&mut self) {
        if let Err(err) = tool_memory::save_on_exit(self) {
            log::warn!("Preferences: could not save the final tool state: {err}");
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.close_control();
        if let Some(at_exit) = self.services.at_exit.take() {
            at_exit();
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.logic_frame(ctx, frame);
    }

    /// Drain the platform's optional WinTab queue before the frame's input is processed.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let frame_start = gpu_canvas::now_ms();
        self.framework_before_input(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        crate::text::installed_fonts::on_input(raw_input);
        if let Some(tablet_poll) = self.services.tablet_poll.as_mut() {
            tablet_poll(raw_input.focused);
        }
        // An undecorated window gets its caption back when the toolkit changes its flags
        // (maximize, restore, …): take it off again before the frame is laid out (#2246).
        if let Some(window_frame) = self.services.window_frame.as_mut() {
            window_frame();
        }
        // Native menu clicks become the commands they fire (see `native_menu`).
        native_menu::poll(self, ctx);
        shortcuts::clipboard_keys(ctx, ctx.text_edit_focused() || self.ui.text_edit.is_some(), raw_input);
        #[cfg(not(target_arch = "wasm32"))]
        self.withdraw_stale_input(ctx);
        // A pen held still is no touch screen's press-and-hold right click (stylus.rs).
        let pen_down = self.stylus.pen_contacts.observe(&raw_input.events);
        stylus::pen_hold_is_no_right_click(ctx, pen_down);
        // The OS's moves.
        let moves = raw_input.events.iter().filter(|e| matches!(e, egui::Event::PointerMoved(_))).count();
        self.perf.stage.start(frame_start, moves);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Hands the runtime the native window (on Windows it answers the window's hit test) and
        // maximizes a window bigger than its display (1440 × 900 on 1366 × 768) once (#315).
        #[cfg(not(target_arch = "wasm32"))]
        craft::ui::shell::embed::before_ui(ui.ctx(), frame);
        #[cfg(test)]
        if let Some(mut body) = self.framework.test_body.take() {
            body(ui, self);
            crate::shortcuts::escape_keys(self, &ui.ctx().clone());
            self.framework.test_body = Some(body);
            return;
        }
        photocraft_compose::discard_unreadable_pixel_caches();
        if let Err(error) = photocraft_raster::spill::check_integrity() {
            self.ui.status = error;
            self.ui.status_error = true;
        } else if let Some(error) = photocraft_raster::spill::last_error() {
            self.ui.status = error;
            self.ui.status_error = true;
        }
        self.ui_frame(ui, frame);
    }
}

/// The boxed app is what the test harness holds: the 85 KiB app lives in one heap allocation, not in
/// every frame between the test and the harness's first pass.
impl eframe::App for Box<PhotocraftApp> {
    fn on_exit(&mut self) {
        (**self).on_exit();
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        (**self).logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        (**self).raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        (**self).ui(ui, frame);
    }
}

#[cfg(test)]
mod clipboard_tests;

#[cfg(test)]
mod apply_data_set_alert_tests;

#[cfg(test)]
mod apply_data_set_missing_file_tests;

#[cfg(test)]
mod exit_hook_tests;
