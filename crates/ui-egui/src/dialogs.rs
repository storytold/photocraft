//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};
use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};

/// Where this frame's dialogs are on screen (the canvas reads last frame's: it draws first).
const RECTS: &str = "pc-dialog-rects";

pub(crate) struct AboutUpdate {
    dialog_id: u64,
    status: AboutUpdateStatus,
    receiver: Option<crate::UpdateEvents>,
}

#[derive(Clone)]
enum AboutUpdateStatus {
    Checking,
    Current,
    Available { version: String, installable: bool },
    Downloading { version: String, downloaded: u64, total: Option<u64> },
    Ready { version: String, installer: std::path::PathBuf },
    Failed(String),
}

fn begin_update_check(app: &mut PhotocraftApp, dialog_id: u64) {
    let receiver = app.services.check_for_updates.as_mut().map(|check| check());
    let status = if receiver.is_some() { AboutUpdateStatus::Checking } else { return };
    app.about_update = Some(AboutUpdate { dialog_id, status, receiver });
}

fn start_update_download(app: &mut PhotocraftApp, version: String) {
    let receiver = app.services.download_update.as_mut().map(|download| download());
    let Some(receiver) = receiver else { return };
    if let Some(update) = app.about_update.as_mut() {
        update.status = AboutUpdateStatus::Downloading { version, downloaded: 0, total: None };
        update.receiver = Some(receiver);
    }
}

fn poll_update(app: &mut PhotocraftApp, dialog_id: u64, ctx: &egui::Context) {
    if app.about_update.as_ref().is_none_or(|update| update.dialog_id != dialog_id) {
        begin_update_check(app, dialog_id);
    }
    let mut events = Vec::new();
    let mut disconnected = false;
    if let Some(update) = app.about_update.as_ref().filter(|update| update.dialog_id == dialog_id)
        && let Some(receiver) = &update.receiver
    {
        loop {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    }
    if let Some(update) = app.about_update.as_mut().filter(|update| update.dialog_id == dialog_id) {
        for event in events {
            update.status = match event {
                crate::UpdateEvent::Current => AboutUpdateStatus::Current,
                crate::UpdateEvent::Available { version, installable } => AboutUpdateStatus::Available { version, installable },
                crate::UpdateEvent::Progress { downloaded, total } => match &update.status {
                    AboutUpdateStatus::Downloading { version, .. } => AboutUpdateStatus::Downloading { version: version.clone(), downloaded, total },
                    _ => continue,
                },
                crate::UpdateEvent::Ready { version, installer } => AboutUpdateStatus::Ready { version, installer },
                crate::UpdateEvent::Failed(error) => AboutUpdateStatus::Failed(error),
            };
        }
        if disconnected && matches!(&update.status, AboutUpdateStatus::Checking | AboutUpdateStatus::Downloading { .. }) {
            update.status = AboutUpdateStatus::Failed("The update worker stopped before it finished.".into());
            update.receiver = None;
        }
        if matches!(&update.status, AboutUpdateStatus::Checking | AboutUpdateStatus::Downloading { .. }) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

fn update_action(app: &mut PhotocraftApp, ctx: &egui::Context, dialog_id: u64) {
    let status = app.about_update.as_ref().filter(|update| update.dialog_id == dialog_id).map(|update| update.status.clone());
    match status {
        Some(AboutUpdateStatus::Current | AboutUpdateStatus::Failed(_)) => begin_update_check(app, dialog_id),
        Some(AboutUpdateStatus::Available { version, installable: true }) => start_update_download(app, version),
        Some(AboutUpdateStatus::Available { installable: false, .. }) => {
            crate::links::open(app, ctx, crate::links::RELEASES);
        }
        Some(AboutUpdateStatus::Ready { installer, .. }) => {
            let result = crate::menus::invoke(app, ctx, "app.installUpdate", json!({"installer": installer.to_string_lossy()}));
            if let Err(error) = result
                && let Some(update) = app.about_update.as_mut()
            {
                update.status = AboutUpdateStatus::Failed(error);
            }
        }
        _ => {}
    }
}

fn rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.data(|m| m.get_temp(egui::Id::new(RECTS))).unwrap_or_default()
}

/// With a dialog open the rest of the window is inert (egui's modal layer), but like Photoshop the
/// image can still be panned and zoomed under it. The pointer position when it is over `canvas`
/// and not over a dialog or one of its popups.
pub fn free_pointer_over(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let p = ctx.pointer_hover_pos()?;
    let rects = rects(ctx);
    (canvas.contains(p) && !rects.iter().any(|r| r.contains(p))).then_some(p)
}

/// Pan drag under an open dialog: Space-drag, middle-drag, or a drag with the Hand tool, started on
/// the free canvas. Returns this frame's pointer movement.
pub fn pan_delta(ctx: &egui::Context, canvas: egui::Rect, hand: bool) -> Option<egui::Vec2> {
    let rects = rects(ctx);
    let (origin, panning, delta) = ctx.input(|i| {
        let p = &i.pointer;
        (p.press_origin(), p.middle_down() || (p.primary_down() && (hand || i.key_down(egui::Key::Space))), p.delta())
    });
    let origin = origin?;
    (panning && canvas.contains(origin) && !rects.iter().any(|r| r.contains(origin))).then_some(delta)
}

/// A click or drag with the primary button started on the free canvas under an open dialog, Space
/// not held: where the pointer is this frame. The press itself counts even when it is released in
/// the same frame (a quick click, `ui.click`); the drag only while it stays on the free canvas.
pub fn free_press(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let rects = rects(ctx);
    let free = |p: egui::Pos2| canvas.contains(p) && !rects.iter().any(|r| r.contains(p));
    ctx.input(|i| {
        if i.key_down(egui::Key::Space) {
            return None;
        }
        let pressed = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        if let Some(p) = pressed {
            return free(p).then_some(p);
        }
        let held = i.pointer.primary_down() && i.pointer.press_origin().is_some_and(free);
        i.pointer.latest_pos().filter(|p| held && free(*p))
    })
}

/// Height of a dialog's title, rule, button row and frame margins (points), around a body that
/// scrolls.
const DIALOG_CHROME: f32 = 150.0;

/// Where a dialog's last position is remembered: per command for command dialogs (each filter has
/// its own), per kind for the rest.
fn place_id(d: &Dialog) -> egui::Id {
    match d.fields.get("__command").and_then(Value::as_str) {
        Some(cmd) => egui::Id::new(("dialog-place", cmd)),
        None => egui::Id::new(("dialog-place", format!("{:?}", d.kind))),
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::layer_style::color_picker::prune(app);
    crate::type_panels_ui::color_picker::prune(app);
    let dialogs = app.ui.dialogs.clone();
    let top = dialogs.last().map(|d| d.id);
    let mut shown = Vec::new();
    for d in dialogs {
        if d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true) {
            poll_update(app, d.id, ctx);
        }
        let lang = if crate::prefs_ui::is_preferences(&d.fields) {
            crate::i18n::Lang::from_pref(d.fields.get("values").and_then(|v| v.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("auto"))
        } else {
            crate::i18n::current()
        };
        let _language = crate::i18n::language_scope(lang);
        let interactive = top == Some(d.id);
        let mut fields = d.fields.clone();
        let mut outcome: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        let mut apply_requested = false;
        let title = display_title(&d);
        let id = egui::Id::new(("dialog", d.id));
        // Opens where the same dialog was last left (centred the first time), then its top-left
        // stays put (offset from the window's top-left, moved by dragging the title bar; view
        // state only, so egui memory): a dialog whose body grows, like Layer Style switching
        // effects, extends down and right instead of re-centring.
        let pinned: Option<egui::Vec2> = ctx.data(|m| m.get_temp(id));
        let place = place_id(&d);
        let start = pinned.or_else(|| ctx.data_mut(|m| m.get_persisted::<egui::Vec2>(place)));
        let mut drag = egui::Vec2::ZERO;
        let mut sizing = false;
        let area = match start {
            Some(offset) => egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, offset),
            None => egui::Modal::default_area(id),
        };
        let color_picker = crate::color_picker_ui::owns(&fields);
        let mut modal = egui::Modal::new(id).area(area).backdrop_color(egui::Color32::TRANSPARENT);
        if color_picker {
            modal = modal.frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16));
        }
        // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
        let modal = modal.show(ctx, |ui| {
            if !interactive {
                ui.disable();
            }
            sizing = ui.is_sizing_pass();
            ui.set_min_width(380.0);
            let wide = crate::prefs_ui::width(&d.fields, (ctx.content_rect().width() - 48.0).max(380.0));
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            // The About window: room for the contributor table, the same width on every tab.
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width(700.0);
            }
            ui.set_max_width(wide.unwrap_or(if d.kind == DialogKind::NewDocument {
                800.0
            } else if about_tabs {
                700.0
            } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
                600.0
            } else {
                440.0
            }));
            if let Some(w) = crate::file_ui::dialog_width(&d.fields) {
                ui.set_min_width(w);
                ui.set_max_width(w);
            }
            if color_picker {
                ui.set_width(crate::color_picker_ui::CONTENT_WIDTH);
            }
            let label = egui::Label::new(egui::RichText::new(&title).font(crate::theme::semibold(15.0))).selectable(false);
            let t = if crate::color_picker_ui::owns(&fields) { ui.add_sized(egui::vec2(ui.available_width(), 22.0), label).rect } else { ui.add(label).rect };
            // The whole title band drags (#1921): the full width plus a little of the popup's
            // padding around it and the gap above the hairline, but none of the controls below.
            let bar = egui::Rect::from_min_max(t.min - egui::vec2(7.0, 7.0), egui::pos2(ui.max_rect().right() + 7.0, t.bottom() + 4.0));
            drag = ui.interact(bar, id.with("title"), egui::Sense::drag()).drag_delta();
            ui.add_space(4.0);
            crate::widgets::hairline(ui);
            ui.add_space(8.0);
            match d.kind {
                DialogKind::NewDocument => crate::new_doc_ui::body(app, ui, &mut fields),
                DialogKind::About if fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => {
                    let lines = crate::gpu_status::system_info(app);
                    for l in &lines {
                        ui.add(egui::Label::new(egui::RichText::new(l).font(crate::theme::mono(12.0))).selectable(true));
                    }
                    ui.add_space(8.0);
                    if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
                        ui.ctx().copy_text(lines.join("\n"));
                    }
                }
                DialogKind::About => {
                    // Tabs About · Contributors · Models (craftrules standards/contributors.md). The
                    // tab is a dialog field, so automation can switch it with `ui.dialog.set`.
                    let tab = about_tab(&fields);
                    let mut chosen = tab;
                    ui.horizontal(|ui| {
                        for (key, label) in [("about", tl!("About")), ("contributors", tl!("Contributors")), ("models", tl!("Models"))] {
                            if crate::widgets::pill_tab(ui, label, tab == key).clicked() {
                                chosen = key;
                            }
                        }
                    });
                    if chosen != tab {
                        fields.insert("tab".into(), json!(chosen));
                    }
                    ui.add_space(8.0);
                    about_tab_body(app, ui, chosen, d.id);
                }
                DialogKind::Command if crate::fill_ui::owns(&fields) => crate::fill_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::stroke_ui::owns(&fields) => crate::stroke_ui::body(ui, &mut fields),
                DialogKind::Command if crate::shape_dialog::owns(&fields) => crate::shape_dialog::body(app, ui, &mut fields),
                DialogKind::Command if crate::delete_layer_prompt::owns(&fields) => crate::delete_layer_prompt::body(ui, &mut fields),
                DialogKind::Command if crate::rasterize_prompt::owns(&fields) => crate::rasterize_prompt::body(ui, &fields),
                DialogKind::Command if crate::variables_ui::owns(&fields) => crate::variables_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::file_ui::owns(&fields) => crate::file_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::color_picker_ui::owns(&fields) => {
                    outcome = crate::color_picker_ui::body(ui, &mut fields);
                    crate::color_picker_ui::take_add_swatch(app, &mut fields);
                }
                DialogKind::Command if crate::color_range_ui::owns(&fields) => crate::color_range_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::prefs_ui::owns(&fields) => crate::prefs_ui::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__export") => crate::export_dialog::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__sizing") => crate::sizing::body(ui, &mut fields),
                DialogKind::Command if crate::adjust_dialog::owns(&fields) => crate::adjust_dialog::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__filter") => {
                    // Color Settings: the monitor profile in use can change while it is open.
                    if fields.get("__command").and_then(Value::as_str) == Some("edit.colorSettings") {
                        fields.insert("__note".into(), Value::String(crate::monitor_status::note(app)));
                    }
                    // Long parameter lists (Flame, Lighting Effects) scroll, so the title and the
                    // OK / Cancel buttons stay inside a small window.
                    let room = (ctx.content_rect().height() - DIALOG_CHROME).max(120.0);
                    // Photoshop opens a value dialog on its first number, selected: typing
                    // replaces it and Enter applies it (#1757). Once, on the first laid-out frame.
                    let focused = id.with("first-field");
                    let first = !ui.is_sizing_pass() && !ctx.data(|m| m.get_temp::<bool>(focused).unwrap_or(false));
                    if first {
                        ctx.data_mut(|m| m.insert_temp(focused, true));
                    }
                    crate::widgets::focus_first_field(ctx, first);
                    egui::ScrollArea::vertical().id_salt(id.with("body")).max_height(room).show(ui, |ui| crate::filter_dialog::body(ui, &mut fields));
                    crate::widgets::focus_first_field(ctx, false);
                }
                DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, &mut fields),
                DialogKind::Command => {}
                DialogKind::LayerStyle => crate::layer_style::body(app, ui, &mut fields),
                DialogKind::Error => {
                    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
                }
            }
            if crate::color_picker_ui::owns(&fields) {
                if interactive && outcome.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome = Some(true);
                }
            } else {
                ui.add_space(8.0);
                // Align::Min, not Center: a centred row fills the height left over from last frame's
                // (larger) size, so a dialog whose body gets shorter would never shrink back.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if matches!(d.kind, DialogKind::About | DialogKind::Error) {
                        let mut buttons = vec![DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0)];
                        if d.kind == DialogKind::About && app.services.check_for_updates.is_some() {
                            let update = app.about_update.as_ref().filter(|update| update.dialog_id == d.id);
                            let (label, enabled) = match update.map(|update| &update.status) {
                                Some(AboutUpdateStatus::Checking) => ("Checking for updates…", false),
                                Some(AboutUpdateStatus::Current) => ("Check for Updates", true),
                                Some(AboutUpdateStatus::Available { installable: true, .. }) => ("Update", true),
                                Some(AboutUpdateStatus::Available { installable: false, .. }) => ("Download", true),
                                Some(AboutUpdateStatus::Downloading { .. }) => ("Downloading update…", false),
                                Some(AboutUpdateStatus::Ready { .. }) => ("Install Update", true),
                                Some(AboutUpdateStatus::Failed(_)) => ("Check for Updates", true),
                                None => ("Checking for updates…", false),
                            };
                            buttons.insert(0, DialogButton::new(ButtonRole::Alternate, tl!(label), 112.0).enabled(enabled));
                        }
                        match dialog_buttons(ui, &buttons) {
                            Some(ButtonRole::Alternate) => update_action(app, ctx, d.id),
                            Some(_) => outcome = Some(false),
                            None => {}
                        }
                    } else {
                        let ok_label = if d.kind == DialogKind::NewDocument {
                            tl!("Create")
                        } else if crate::delete_layer_prompt::owns(&d.fields) {
                            tl!("Delete")
                        } else if d.fields.contains_key("__export") {
                            tl!("Export")
                        } else {
                            crate::file_ui::ok_label(&d.fields).unwrap_or(tl!("OK"))
                        };
                        let ok = DialogButton::new(ButtonRole::Default, ok_label, 84.0);
                        // Photoshop: holding Alt turns Cancel into Reset (the dialog stays open).
                        let reset = ui.input(|i| i.modifiers.alt) && crate::adjust_dialog::resets(&fields);
                        let cancel_label = if reset {
                            tl!("Reset")
                        } else if d.kind == DialogKind::NewDocument {
                            tl!("Close")
                        } else {
                            tl!("Cancel")
                        };
                        let cancel = DialogButton::new(ButtonRole::Cancel, cancel_label, 84.0);
                        let clicked = if d.kind == DialogKind::Command && crate::prefs_ui::is_preferences(&fields) {
                            let changed = crate::prefs_ui::preferences_changed(app, &fields);
                            dialog_buttons(ui, &[ok, cancel, DialogButton::new(ButtonRole::Apply, tl!("Apply"), 84.0).enabled(changed)])
                        } else {
                            dialog_buttons(ui, &[ok, cancel])
                        };
                        match clicked {
                            Some(ButtonRole::Cancel) if reset => crate::adjust_dialog::reset(ui.ctx(), &mut fields),
                            Some(ButtonRole::Cancel) => outcome = Some(false),
                            Some(ButtonRole::Apply) => apply_requested = true,
                            Some(_) => outcome = Some(true),
                            None if interactive && ui.input(|i| i.key_pressed(egui::Key::Enter)) => outcome = Some(true),
                            None => {}
                        }
                    }
                });
            }
            // The frame around the content (Frame::popup's margin and stroke).
            ui.min_rect().expand(ui.spacing().menu_margin.sum().max_elem() + 2.0)
        });
        // The picker has a larger frame margin; include that whole frame in the canvas hit guard.
        shown.push(if color_picker { modal.response.rect } else { modal.inner });
        // Pin once laid out at its real size (the first frame is an invisible sizing pass).
        if !sizing && (pinned.is_none() || drag != egui::Vec2::ZERO) {
            let screen = ctx.content_rect();
            // Keep the whole dialog (and so its title bar) on screen.
            let room = (screen.size() - modal.response.rect.size()).max(egui::Vec2::ZERO);
            let offset = (pinned.unwrap_or(modal.response.rect.min - screen.min) + drag).clamp(egui::Vec2::ZERO, room);
            ctx.data_mut(|m| m.insert_temp(id, offset));
            // A dialog the user moved reopens there (Photoshop remembers each dialog's place).
            if drag != egui::Vec2::ZERO {
                ctx.data_mut(|m| m.insert_persisted(place, offset));
            }
        }
        // Esc cancels (topmost dialog, no popup open). A click outside does nothing: Photoshop keeps
        // the dialog, and the pointer may be panning or zooming the canvas under it.
        if interactive
            && outcome.is_none()
            && (modal.response.should_close()
                || (modal.is_top_modal && !modal.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
        {
            outcome = Some(false);
        }
        if let Some(dm) = app.ui.dialog_mut(d.id) {
            dm.fields = fields;
        }
        // A colour swatch (Edit › Fill, Edit › Stroke) opens the Color Picker over the dialog.
        if outcome.is_none() {
            crate::color_picker_ui::open_requested(app, d.id);
        }
        if apply_requested && outcome.is_none() {
            let _ = crate::prefs_ui::apply(app, d.id);
        }
        match outcome {
            Some(true) => {
                if let Err(error) = confirm(app, d.id) {
                    app.ui.status = error;
                }
            }
            Some(false) => {
                let _ = cancel(app, d.id);
            }
            None => {
                if let Err(error) = crate::layer_style::color_picker::take_request(app, d.id) {
                    app.ui.status = error;
                }
            }
        }
    }
    ctx.data_mut(|m| m.insert_temp(egui::Id::new(RECTS), shown));
}

/// The About window's tabs, as stored in its `tab` field.
pub const ABOUT_TABS: [&str; 3] = ["about", "contributors", "models"];

/// The About tab to show: the `tab` field when it names one, otherwise "about".
fn about_tab(fields: &serde_json::Map<String, Value>) -> &'static str {
    let want = fields.get("tab").and_then(Value::as_str).unwrap_or("");
    ABOUT_TABS.iter().copied().find(|t| *t == want).unwrap_or("about")
}

/// One About tab's contents: the credits lists, or the product blurb and links.
fn about_tab_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, tab: &str, dialog_id: u64) {
    match tab {
        "contributors" => crate::credits::contributors_ui(app, ui),
        "models" => crate::credits::models_ui(ui),
        _ => {
            ui.label(tl!("PhotoCraft — an open-source, native image editor written in Rust."));
            let update = app.about_update.as_ref().filter(|update| update.dialog_id == dialog_id).map(|update| update.status.clone());
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::fmt(tl!("Version {version}"), &[("version", &photocraft_engine::build_info::long_version())]));
                if let Some(version) = update.as_ref().and_then(|status| match status {
                    AboutUpdateStatus::Available { version, .. }
                    | AboutUpdateStatus::Downloading { version, .. }
                    | AboutUpdateStatus::Ready { version, .. } => Some(version.as_str()),
                    _ => None,
                }) {
                    ui.label(crate::i18n::fmt(tl!("New version {version}"), &[("version", version)]));
                }
            });
            match update {
                Some(AboutUpdateStatus::Checking) => {
                    ui.weak(tl!("Checking for updates…"));
                }
                Some(AboutUpdateStatus::Current) => {
                    ui.weak(tl!("You're up to date."));
                }
                Some(AboutUpdateStatus::Available { installable: false, .. }) => {
                    if ui.link(tl!("GitHub")).clicked() {
                        crate::links::open(app, ui.ctx(), crate::links::RELEASES);
                    }
                }
                Some(AboutUpdateStatus::Downloading { downloaded, total, .. }) => {
                    ui.weak(tl!("Downloading update…"));
                    if let Some(total) = total.filter(|total| *total > 0) {
                        ui.add(egui::ProgressBar::new((downloaded as f32 / total as f32).clamp(0.0, 1.0)).show_percentage());
                    } else {
                        ui.spinner();
                    }
                }
                Some(AboutUpdateStatus::Ready { .. }) => {}
                Some(AboutUpdateStatus::Failed(error)) => {
                    ui.weak(crate::i18n::fmt(tl!("Update failed: {error}"), &[("error", &error)]));
                    if ui.link(tl!("GitHub")).clicked() {
                        crate::links::open(app, ui.ctx(), crate::links::RELEASES);
                    }
                }
                None => {}
            }
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                crate::links::discord_button(app, ui, 220.0);
                ui.add_space(8.0);
                crate::links::link_row(app, ui);
            });
            ui.add_space(10.0);
            ui.weak("egui · wgpu · photocraft-engine");
        }
    }
}

/// The dialog title as shown: [`title`] in the UI language.
fn display_title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::Command => {
            let label = d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command");
            // Dialog labels are catalogued with their "…" ("Export As…"); some commands omit it.
            let with_dots = format!("{}…", label.trim_end_matches('…'));
            let shown = if crate::i18n::has(crate::i18n::current(), label) { tl!(label) } else { tl!(&with_dots) };
            shown.trim_end_matches('…').to_string()
        }
        _ => tl!(&title(d)).to_string(),
    }
}

pub fn title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::NewDocument => "New Document".into(),
        DialogKind::About if d.fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => "System Info".into(),
        DialogKind::About => "About PhotoCraft".into(),
        DialogKind::LayerStyle => "Layer Style".into(),
        DialogKind::Command => d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command").trim_end_matches('…').to_string(),
        DialogKind::Error => "Error".into(),
    }
}

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
pub fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    if crate::layer_style::color_picker::has_child(app, id) {
        return Err("confirm or cancel the Layer Style color picker first".into());
    }
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    match d.kind {
        DialogKind::NewDocument => {
            let r = app.run("file.new", crate::new_doc_ui::command_params(&d.fields));
            if let Some(i) = app.session.active_index() {
                app.ui.views[i].fit_pending = true;
            }
            if r.is_ok() {
                crate::remember_new_document(app, &d.fields);
            }
            r
        }
        DialogKind::Command if crate::fill_ui::owns(&d.fields) => crate::fill_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::stroke_ui::owns(&d.fields) => crate::stroke_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::shape_dialog::owns(&d.fields) => crate::shape_dialog::confirm(app, &d.fields),
        DialogKind::Command if crate::delete_layer_prompt::owns(&d.fields) => crate::delete_layer_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::rasterize_prompt::owns(&d.fields) => crate::rasterize_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_range_ui::owns(&d.fields) => crate::color_range_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        DialogKind::Command if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        DialogKind::Command => {
            let Some(cmd) = d.fields.get("__command").and_then(Value::as_str).map(str::to_string) else {
                app.filter_preview = None;
                return Err("dialog has no command".into());
            };
            let (cmd, params) = crate::smart_ui::confirm_command(&d.fields, cmd, crate::filter_dialog::params_of(&d.fields));
            let result = app.run(&cmd, params.clone());
            // A filter that runs as a background job keeps its preview on screen until the job
            // lands (`canvas::committed_filter_preview`) instead of flashing the unfiltered image.
            let running = app
                .session
                .active()
                .is_some_and(|st| app.filter_preview.as_ref().is_some_and(|p| p.key.doc == st.doc.id) && app.session.job_on(st.doc.id).is_some());
            match app.filter_preview.as_mut() {
                Some(p) if result.is_ok() && running => p.committing = true,
                _ => app.filter_preview = None,
            }
            if result.is_ok() && cmd == "view.newGuideLayout" {
                app.ui.view.guide_layout = params;
            }
            if result.is_ok() {
                crate::filter_dialog::remember(app, &cmd, &d.fields);
            }
            result
        }
        DialogKind::LayerStyle => crate::layer_style::confirm(app, &d.fields),
        DialogKind::About | DialogKind::Error => Ok(Value::Null),
    }
}

/// Cancel a dialog and its dependent Layer Style picker, without applying edits.
pub fn cancel(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    app.ui.dialogs.retain(|d| !crate::layer_style::color_picker::child_of(&d.fields, id));
    app.filter_preview = None;
    app.color_range = None;
    crate::type_panels_ui::color_picker::prune(app);
    Ok(Value::Null)
}

/// Open the parameter dialog of `command`: the adjustment editor for `image.adjustments.*`, the
/// schema dialog for filters, the Color Range dialog for `select.colorRange`, otherwise a bare
/// confirm dialog.
pub fn open_command_dialog(app: &mut PhotocraftApp, command: &str, label: &str) -> u64 {
    if command == crate::stroke_ui::COMMAND {
        return crate::stroke_ui::open(app);
    }
    if command == crate::color_range_ui::COMMAND {
        return crate::color_range_ui::open(app);
    }
    if let Some(id) = crate::adjust_dialog::open(app, command) {
        return id;
    }
    if crate::filter_dialog::has_dialog(command)
        && let Some(id) = crate::filter_dialog::open(app, command)
    {
        return id;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    app.ui.open_dialog(DialogKind::Command, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_events(event: crate::UpdateEvent) -> crate::UpdateEvents {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(event);
        receiver
    }

    #[test]
    fn about_shows_the_new_version_and_downloads_the_verified_update() {
        use egui_kittest::{Harness, kittest::Queryable};
        use std::sync::{Arc, Mutex};

        crate::i18n::set_current(crate::i18n::Lang::EN);
        let installer = std::env::temp_dir().join("photocraft-update-test.msi");
        let installed = Arc::new(Mutex::new(Vec::<std::path::PathBuf>::new()));
        let installed_by_service = installed.clone();
        let services = crate::Services {
            check_for_updates: Some(Box::new(|| update_events(crate::UpdateEvent::Available {
                version: "0.7.0".into(),
                installable: true,
            }))),
            download_update: Some(Box::new({
                let installer = installer.clone();
                move || update_events(crate::UpdateEvent::Ready { version: "0.7.0".into(), installer: installer.clone() })
            })),
            install_update: Some(Box::new(move |path| {
                installed_by_service.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(path);
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.open_dialog(DialogKind::About, serde_json::Map::new());
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.run_steps(4);
        assert!(harness.query_by_label("New version 0.7.0").is_some());
        harness.get_by_label("Update").click();
        harness.run_steps(3);
        harness.get_by_label("Install Update").click();
        harness.run_steps(2);
        assert_eq!(*installed.lock().unwrap_or_else(std::sync::PoisonError::into_inner), vec![installer]);
        assert!(harness.state().allow_close);
    }

    /// A filter dialog over a 256×256 layer, with the preview the canvas computed for it on screen.
    fn filter_dialog_with_preview(background: bool) -> (PhotocraftApp, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = background;
        app.run("file.new", serde_json::json!({"width": 256, "height": 256, "background": "#3366cc"})).unwrap();
        let id = open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur");
        let st = app.session.active().unwrap();
        let result = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "filter.blur.gaussianBlur", &serde_json::json!({"radius": 4.0}), 1);
        let key = crate::filter_dialog::FilterPreviewKey {
            doc: st.doc.id,
            revision: st.revision,
            dialog: id,
            active: st.active_layer,
            command: "filter.blur.gaussianBlur".into(),
            params: serde_json::json!({"radius": 4.0}),
            k: 1,
        };
        app.filter_preview = Some(crate::filter_dialog::FilterPreview { key, result: result.map(std::sync::Arc::new), committing: false });
        (app, id)
    }

    #[test]
    fn a_filters_preview_stays_on_screen_until_its_job_lands() {
        // OK dropped the preview and the filter ran as a background job, so the canvas flashed
        // the unfiltered image until the job finished.
        let (mut app, id) = filter_dialog_with_preview(true);
        let revision = app.session.active().unwrap().revision;
        confirm(&mut app, id).unwrap();
        let doc = app.session.active().unwrap().doc.id;
        let job = app.session.job_on(doc).expect("the blur runs as a background job").id;
        assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some((1, _, [256, 256])))), "the preview is shown");
        // The job lands: the document changes and the preview goes.
        app.session.wait_job(job).unwrap();
        assert_ne!(app.session.active().unwrap().revision, revision);
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
        assert!(app.filter_preview.is_none());

        // A cancelled job leaves the document as it was, so its preview goes too.
        let (mut app, id) = filter_dialog_with_preview(true);
        confirm(&mut app, id).unwrap();
        let doc = app.session.active().unwrap().doc.id;
        let job = app.session.job_on(doc).unwrap().id;
        assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some(_))));
        assert!(app.session.cancel_job(job));
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
        assert!(app.filter_preview.is_none());

        // Inline (no background jobs): the filter has already landed, nothing is held.
        let (mut app, id) = filter_dialog_with_preview(false);
        confirm(&mut app, id).unwrap();
        assert!(app.filter_preview.is_none());
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), None);
    }

    #[test]
    fn color_picker_has_inset_content_and_actions_at_the_right_edge() {
        use egui_kittest::{Harness, kittest::Queryable};

        for size in [egui::vec2(760.0, 480.0), egui::vec2(960.0, 640.0)] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            crate::color_picker_ui::open(h.state_mut(), "foreground");
            h.run_steps(4);
            let frame = rects(&h.ctx)[0];
            let title = h.get_by_label("Color Picker (Foreground Color)").rect();
            let ok = h.get_by_label("OK").rect();
            let cancel = h.get_by_label("Cancel").rect();
            let web = h.get_by_label("Only Web Colors").rect();
            assert!((ok.right() - title.right()).abs() < 1.0, "OK aligns with the right content edge: {ok:?} vs {title:?}");
            assert!((cancel.right() - ok.right()).abs() < 1.0);
            assert!(frame.right() - ok.right() >= 16.0, "buttons retain the window's padding");
            assert!(web.left() - frame.left() >= 16.0, "checkbox has comfortable left padding");
            assert!(frame.bottom() - web.bottom() >= 16.0, "checkbox has comfortable bottom padding");
            assert!(title.top() - frame.top() >= 16.0, "title has top padding");
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(frame), "padded picker fits {size:?}: {frame:?}");
            // A click anywhere inside the added margin must stay out of the canvas eyedropper.
            let margin = egui::pos2(frame.left() + 8.0, web.center().y);
            h.hover_at(margin);
            h.run_steps(1);
            assert!(free_pointer_over(&h.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)).is_none());
        }
    }

    #[test]
    fn layer_style_title_gutters_can_start_drags() {
        use egui_kittest::{Harness, kittest::Queryable};

        for point in 0..5 {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
            h.run_steps(3);
            let before = h.get_by_label("Layer Style").rect();
            let from = match point {
                0 => egui::pos2(before.left() - 5.0, before.bottom() - 4.0),
                1 => egui::pos2(before.right() + 5.0, before.bottom() - 4.0),
                2 => egui::pos2(before.center().x, before.bottom() + 3.0),
                3 => egui::pos2(before.center().x, before.top() - 5.0),
                _ => egui::pos2(before.center().x, before.bottom() - 3.0),
            };
            h.hover_at(from);
            h.drag_at(from);
            h.run_steps(2);
            for i in 1..=8 {
                h.hover_at(from + egui::vec2(64.0, -32.0) * (i as f32 / 8.0));
                h.run_steps(1);
            }
            h.drop_at(from + egui::vec2(64.0, -32.0));
            h.run_steps(3);
            let delta = h.get_by_label("Layer Style").rect().min - before.min;
            assert!((delta - egui::vec2(64.0, -32.0)).length() < 2.0, "point {point}: moved {delta:?}");
            assert_eq!(h.state().ui.dialogs.len(), 1);
        }
    }

    #[test]
    fn dragging_the_title_bar_moves_the_dialog() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
        harness.drag_at(from);
        harness.run_steps(2);
        for i in 1..=10 {
            harness.hover_at(from + egui::vec2(-12.0, 8.0) * i as f32);
            harness.run_steps(1);
        }
        harness.drop_at(from + egui::vec2(-120.0, 80.0));
        harness.run_steps(3);

        let moved = harness.get_by_label("Layer Style").rect().min - before.min;
        assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }

    fn dialog_harness(size: egui::Vec2, command: &str) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        crate::filter_dialog::open(h.state_mut(), command).unwrap();
        h.run_steps(4);
        h
    }

    #[test]
    fn a_tall_filter_dialog_keeps_its_title_and_buttons_in_a_small_window() {
        // Filter › Render › Flame was taller than a 1000×560 window: OK and Cancel were cut off.
        use egui_kittest::kittest::Queryable;
        let size = egui::vec2(1000.0, 560.0);
        let h = dialog_harness(size, "filter.render.flame");
        for label in ["Flame", "OK", "Cancel"] {
            let r = h.get_by_label(label).rect();
            assert!(r.top() >= 0.0 && r.bottom() <= size.y, "{label} at {r:?} is outside the {size:?} window");
        }
    }

    #[test]
    fn a_moved_dialog_reopens_where_it_was_left() {
        use egui_kittest::kittest::Queryable;
        let size = egui::vec2(1280.0, 800.0);
        let mut h = dialog_harness(size, "filter.blur.gaussianBlur");
        let centred = h.get_by_label("Gaussian Blur").rect();
        // Drag the title bar up and to the left.
        let (from, by) = (centred.center(), egui::vec2(-240.0, -150.0));
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(from + by * (k as f32 / 6.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: from + by, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        let moved = h.get_by_label("Gaussian Blur").rect();
        assert!((moved.min - (centred.min + by)).length() < 2.0, "dragged: {centred:?} → {moved:?}");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        // Reopened: where it was left.
        crate::filter_dialog::open(h.state_mut(), "filter.blur.gaussianBlur").unwrap();
        h.run_steps(4);
        let reopened = h.get_by_label("Gaussian Blur").rect();
        assert!((reopened.min - moved.min).length() < 2.0, "reopened at {reopened:?}, left at {moved:?}");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        // Another dialog keeps its own place: never moved, it opens centred.
        crate::filter_dialog::open(h.state_mut(), "filter.blur.motionBlur").unwrap();
        h.run_steps(4);
        let other = h.get_by_label("Motion Blur").rect();
        assert!(other.min.x > moved.min.x + 100.0, "Motion Blur opened centred: {other:?}");
    }

    /// #1757: like Photoshop, a value dialog opens on its first number with the text selected, so
    /// typing replaces it and Enter applies it; it reopens with the value applied last time, and a
    /// cancelled edit is not remembered.
    #[test]
    fn a_value_dialog_opens_on_its_first_number_selected_and_remembers_it() {
        use egui_kittest::kittest::Queryable;
        const BLUR: &str = "filter.blur.gaussianBlur";
        let mut h = dialog_harness(egui::vec2(1280.0, 800.0), BLUR);
        let radius = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.state().ui.dialogs.last().and_then(|d| d.fields["radius"].as_f64());
        assert_eq!(radius(&h), Some(1.0), "the default the first time");
        assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "the Radius field has focus");
        h.event(egui::Event::Text("5".into()));
        h.run_steps(1);
        assert_eq!(radius(&h), Some(5.0), "typing replaced the selected 1");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "Enter is OK");
        assert_eq!(h.state().session.journal.last(), Some(&(BLUR.to_string(), json!({"radius": 5.0}))));

        crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
        h.run_steps(4);
        assert_eq!(radius(&h), Some(5.0), "reopens with the last value");
        assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "focused again");
        h.event(egui::Event::Text("9".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "Esc is Cancel");
        crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
        h.run_steps(2);
        assert_eq!(radius(&h), Some(5.0), "a cancelled 9 is not remembered");
    }
}
