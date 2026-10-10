//! Mobile UI adaptations, touch controls, fullscreen toggle, and mobile file picker for PhotoCraft.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Stroke, vec2};
use photocraft_ui_egui::file_dialog::{FileDialogAnswer, FileDialogReply};
use photocraft_ui_egui::{PhotocraftApp, theme::Tokens};

/// Common image file extensions PhotoCraft opens.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "psd", "psb", "pcraft", "bmp", "gif", "tif", "tiff", "tga", "qoi", "hdr", "exr",
];

/// Shared state for the Android mobile shell.
#[derive(Default)]
pub struct MobileState {
    pub file_picker_open: bool,
    pub current_dir: PathBuf,
    pub selected_file: Option<PathBuf>,
    pub save_dialog_open: bool,
    pub save_filename: String,
    pub pending_open: Option<FileDialogReply>,
    pub pending_save: Option<FileDialogReply>,
    pub is_fullscreen: bool,
}

pub type SharedMobileState = Arc<Mutex<MobileState>>;

/// Configures egui style spacing and touch target sizing for phones.
pub fn configure_mobile_style(style: &mut egui::Style) {
    style.spacing.interact_size = vec2(44.0, 40.0);
    style.spacing.button_padding = vec2(12.0, 10.0);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.icon_width = 24.0;
}

/// Applies phone-first UI state defaults to maximize canvas working space.
pub fn apply_mobile_defaults(app: &mut PhotocraftApp) {
    // 1. Hide desktop title bar (10 desktop menus) by default;
    // mobile action bar is used instead. User can toggle it via "☰ Menu".
    app.ui.panels.menu_bar = false;

    // 2. Collapse the right dock and status bar by default for maximum screen space.
    app.ui.panels.dock = false;
    app.ui.panels.status_bar = false;
    app.ui.panels.toolbar = true;
    app.ui.panels.options_bar = true;

    // 3. Enable touch-friendly modifier keys (Shift, Ctrl, Alt).
    app.ui.shell.modifier_keys = true;

    // 4. Android handles the system status bar.
    app.custom_titlebar = false;
}

/// Top quick-action bar designed for thumb navigation on phones.
pub fn render_mobile_action_bar(app: &mut PhotocraftApp, state: &mut MobileState, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());

    egui::Panel::top("mobile_action_bar")
        .exact_size(44.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 4)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Brand logo mark and app name
                let (mark_rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                photocraft_ui_egui::brand::paint_mark(ui, mark_rect);
                ui.label(RichText::new("PhotoCraft").strong().color(t.text));

                ui.separator();

                // Open file button
                if ui.button(RichText::new("📂 Open").size(13.0)).clicked() {
                    state.file_picker_open = true;
                    state.current_dir = default_browse_dir();
                    state.selected_file = None;
                }

                // Quick Save button
                if ui.button(RichText::new("💾 Save").size(13.0)).clicked() {
                    let doc_name = app.session.active().map(|d| d.doc.name.clone()).unwrap_or_else(|| "artwork.png".into());
                    state.save_filename = doc_name;
                    state.save_dialog_open = true;
                }

                ui.separator();

                // Undo / Redo
                let can_undo = app.session.active().map(|d| d.history.can_undo()).unwrap_or(false);
                if ui.add_enabled(can_undo, egui::Button::new("↩")).clicked() {
                    let _ = app.session.undo();
                }

                let can_redo = app.session.active().map(|d| d.history.can_redo()).unwrap_or(false);
                if ui.add_enabled(can_redo, egui::Button::new("↪")).clicked() {
                    let _ = app.session.redo();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Fullscreen toggle button
                    let fs_label = if state.is_fullscreen { "⛶ Standard" } else { "⛶ Full" };
                    if ui.button(RichText::new(fs_label).size(13.0)).clicked() {
                        state.is_fullscreen = !state.is_fullscreen;
                        app.ui.view.screen_mode = if state.is_fullscreen {
                            "fullScreen".into()
                        } else {
                            "standard".into()
                        };
                    }

                    // Layers panel toggle
                    let layers_active = app.ui.panels.dock;
                    if ui.selectable_label(layers_active, "📑 Layers").clicked() {
                        app.ui.panels.dock = !app.ui.panels.dock;
                    }

                    // Tools panel toggle
                    let tools_active = app.ui.panels.toolbar;
                    if ui.selectable_label(tools_active, "🛠 Tools").clicked() {
                        app.ui.panels.toolbar = !app.ui.panels.toolbar;
                    }

                    // Desktop menu bar toggle
                    let menu_active = app.ui.panels.menu_bar;
                    if ui.selectable_label(menu_active, "☰ Menu").clicked() {
                        app.ui.panels.menu_bar = !app.ui.panels.menu_bar;
                    }
                });
            });
        });
}

/// Floating button to return from Fullscreen mode back to normal view.
pub fn render_fullscreen_exit_button(ctx: &egui::Context, state: &mut MobileState, app: &mut PhotocraftApp) {
    if !state.is_fullscreen {
        return;
    }

    let screen = ctx.content_rect();
    let btn_rect = egui::Rect::from_min_size(egui::pos2(screen.right() - 56.0, screen.top() + 16.0), vec2(44.0, 44.0));

    egui::Area::new(egui::Id::new("fullscreen_exit_btn"))
        .fixed_pos(btn_rect.min)
        .show(ctx, |ui| {
            ui.painter().rect_filled(btn_rect, 22.0, Color32::from_black_alpha(180));
            ui.painter().rect_stroke(btn_rect, 22.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);

            let resp = ui.allocate_rect(btn_rect, egui::Sense::click());
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                "✕",
                egui::FontId::proportional(20.0),
                Color32::WHITE,
            );

            if resp.clicked() {
                state.is_fullscreen = false;
                app.ui.view.screen_mode = "standard".into();
            }
        });
}

/// Mobile file picker dialog for opening images.
pub fn render_file_picker_dialog(ctx: &egui::Context, state: &mut MobileState) {
    if !state.file_picker_open {
        return;
    }

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 24.0).min(480.0);
    let max_h = (screen.height() - 48.0).min(560.0);

    let mut open = true;
    egui::Window::new("📂 Open Image")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(max_w, max_h))
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            // Quick directory shortcuts
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Quick:").strong());
                if ui.button("📸 Camera").clicked() {
                    state.current_dir = PathBuf::from("/sdcard/DCIM/Camera");
                    state.selected_file = None;
                }
                if ui.button("🖼 Pictures").clicked() {
                    state.current_dir = PathBuf::from("/sdcard/Pictures");
                    state.selected_file = None;
                }
                if ui.button("📥 Downloads").clicked() {
                    state.current_dir = PathBuf::from("/sdcard/Download");
                    state.selected_file = None;
                }
                if ui.button("📁 All Storage").clicked() {
                    state.current_dir = PathBuf::from("/sdcard");
                    state.selected_file = None;
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("⬆ Up").clicked()
                    && let Some(parent) = state.current_dir.parent()
                    && parent.exists()
                {
                    state.current_dir = parent.to_path_buf();
                    state.selected_file = None;
                }
                ui.label(RichText::new(format!("Location: {}", state.current_dir.display())).weak().small());
            });

            // List folders and files in current directory
            let (dirs, files) = list_entries_in_dir(&state.current_dir);

            egui::ScrollArea::vertical().max_height(max_h - 150.0).show(ui, |ui| {
                if dirs.is_empty() && files.is_empty() {
                    ui.label("No supported files or folders found here.");
                }

                // Subdirectories first
                for dir in dirs {
                    let dirname = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dir".into());
                    if ui.button(format!("📁 {dirname}")).clicked() {
                        state.current_dir = dir;
                        state.selected_file = None;
                    }
                }

                // Supported image files
                for file in files {
                    let filename = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
                    let is_selected = state.selected_file.as_ref() == Some(&file);

                    if ui.selectable_label(is_selected, format!("📄 {filename}")).clicked() {
                        state.selected_file = Some(file);
                    }
                }
            });

            ui.separator();

            // Bottom action buttons
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    state.file_picker_open = false;
                    if let Some(reply) = state.pending_open.take() {
                        reply.send(None);
                    }
                }

                let can_open = state.selected_file.is_some();
                if ui.add_enabled(can_open, egui::Button::new(RichText::new("Open File").strong())).clicked()
                    && let Some(path) = state.selected_file.take()
                {
                    state.file_picker_open = false;
                    if let Some(reply) = state.pending_open.take() {
                        reply.send(Some(FileDialogAnswer::Paths(vec![path.to_string_lossy().to_string()])));
                    }
                }
            });
        });

    if !open {
        state.file_picker_open = false;
        if let Some(reply) = state.pending_open.take() {
            reply.send(None);
        }
    }
}

/// Mobile save dialog for exporting or saving images.
pub fn render_save_dialog(ctx: &egui::Context, state: &mut MobileState) {
    if !state.save_dialog_open {
        return;
    }

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 24.0).min(440.0);

    let mut open = true;
    egui::Window::new("💾 Save Artwork")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(max_w, 220.0))
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("File Name:");
            ui.text_edit_singleline(&mut state.save_filename);

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Format:").weak());
                for ext in ["png", "jpg", "psd", "pcraft"] {
                    if ui.button(format!(".{ext}")).clicked() {
                        let stem = Path::new(&state.save_filename).file_stem().and_then(|s| s.to_str()).unwrap_or("artwork");
                        state.save_filename = format!("{stem}.{ext}");
                    }
                }
            });

            ui.add_space(6.0);
            let save_dir = PathBuf::from("/sdcard/Pictures/PhotoCraft");
            ui.label(RichText::new(format!("Destination: {}", save_dir.display())).weak().small());

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    state.save_dialog_open = false;
                    if let Some(reply) = state.pending_save.take() {
                        reply.send(None);
                    }
                }

                let can_save = !state.save_filename.trim().is_empty();
                if ui.add_enabled(can_save, egui::Button::new(RichText::new("Save Now").strong())).clicked() {
                    let _ = std::fs::create_dir_all(&save_dir);
                    let filename = if Path::new(&state.save_filename).extension().is_none() {
                        format!("{}.png", state.save_filename.trim())
                    } else {
                        state.save_filename.trim().to_string()
                    };
                    let target_path = save_dir.join(filename);
                    state.save_dialog_open = false;
                    if let Some(reply) = state.pending_save.take() {
                        reply.send(Some(FileDialogAnswer::SaveTo(target_path.to_string_lossy().to_string())));
                    }
                }
            });
        });

    if !open {
        state.save_dialog_open = false;
        if let Some(reply) = state.pending_save.take() {
            reply.send(None);
        }
    }
}

fn default_browse_dir() -> PathBuf {
    let pics = PathBuf::from("/sdcard/Pictures");
    if pics.exists() {
        pics
    } else {
        let dl = PathBuf::from("/sdcard/Download");
        if dl.exists() { dl } else { PathBuf::from("/sdcard") }
    }
}

fn list_entries_in_dir(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && !name.starts_with('.')
                {
                    dirs.push(path);
                }
            } else if path.is_file()
                && let Some(ext) = path.extension().and_then(|e| e.to_str())
            {
                let ext_lower = ext.to_ascii_lowercase();
                if IMAGE_EXTENSIONS.contains(&ext_lower.as_str()) {
                    files.push(path);
                }
            }
        }
    }
    dirs.sort();
    files.sort();
    (dirs, files)
}
