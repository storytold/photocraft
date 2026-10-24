//! PhotoCraft on Android — GameActivity + eframe/wgpu, reusing the desktop engine and egui UI.
//!
//! Nothing here is Android-specific beyond the entry point, the data directory, the touch
//! adjustments, the font fallback and the SAF bridge: the same `photocraft_engine::Session` and
//! `photocraft_ui_egui::PhotocraftApp` the desktop shell builds are wired up here. What the desktop
//! shell gets from the OS, this crate takes from the activity:
//!
//! | Desktop (`apps/photocraft`) | Android (this crate) |
//! |---|---|
//! | `std::env::args()` launch flags | [`winit::platform::android::activity::AndroidApp`] |
//! | config dir from `app_dirs` | [`AndroidApp::internal_data_path`] (see [`services::Paths`]) |
//! | `rfd` file dialogs | the SAF pickers in [`services`]; a picked document is copied into the app directory and opened through the app's own `os_events` seam (the Finder-double-click one) |
//! | `arboard` clipboard, `open::that` | not wired (the session clipboard and `ctx.open_url` still work) |
//! | desktop-sized UI | [`TouchShell`] enlarges touch targets and offers a finger-sized command palette |
//! | `crash_guard` around Open/Export | the same guard in [`services`] |
//!
//! The engine, the UI, the theme and the menus are untouched — File ▸ Open runs PhotoCraft's own
//! import, File ▸ Save its own export. The shell only adjusts egui's spacing *after* PhotoCraft
//! installs its own theme on the first frame.
#![cfg(target_os = "android")]

use std::sync::mpsc::Sender;

use photocraft_engine::Session;
use photocraft_ui_egui::{ControlRequest, PhotocraftApp};
use winit::platform::android::activity::AndroidApp;

mod saf;
mod selftest;
mod services;

/// How much to scale the desktop UI up on a phone before the layout reflows.
///
/// Measured on a 1080×2400 @ 420 dpi phone: egui's logical size is 411×914 pt at
/// `pixels_per_point = 2.625`, while PhotoCraft's chrome is designed for a 1440×900 desktop.
/// Scaling up (the FilmCraft pilot tried 1.35) only shrinks the logical viewport further and makes
/// the panels overlap more, so the shell does **not** scale: the fix is bigger hit targets (below)
/// and a touch command palette, not a zoom. Keep this knob at 1.0.
const TOUCH_SCALE: f32 = 1.0;

/// The name of the fallback font this shell appends to every family (see [`install_system_font`]).
const SYSTEM_FONT: &str = "system-cjk";

/// Touch shell: forward everything to [`PhotocraftApp`] and add the phone affordances once,
/// *after* PhotoCraft's own theme is in place.
struct TouchShell {
    inner: PhotocraftApp,
    /// The spacing/theme tuning has run (first `ui` pass, i.e. after `theme::install_fonts`).
    tuned: bool,
    /// The CJK fallback font has been appended (after the theme's fonts went live).
    font_done: bool,
    /// Commands for the app's control channel (the desktop control server's entry point).
    control: Sender<ControlRequest>,
    /// The touch command palette is open.
    palette: bool,
    /// Press bookkeeping for the long-press gesture.
    press: Option<(std::time::Instant, egui::Pos2)>,
}

impl TouchShell {
    /// Phone adjustments: 40 pt hit targets, roomier padding, no tooltips from a resting finger.
    /// Runs after the first `ui` pass, which is where PhotoCraft installs its theme.
    fn tune(ctx: &egui::Context) {
        let before = ctx.pixels_per_point();
        if (TOUCH_SCALE - 1.0).abs() > f32::EPSILON {
            ctx.set_pixels_per_point((before * TOUCH_SCALE).min(4.0));
        }
        ctx.all_styles_mut(|style| {
            style.spacing.interact_size = egui::vec2(40.0, 40.0);
            style.spacing.button_padding = egui::vec2(10.0, 8.0);
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.slider_width = style.spacing.slider_width.max(160.0);
            style.interaction.show_tooltips_only_when_still = true;
        });
        let rect = ctx.viewport_rect();
        log::info!(
            "photocraft-android: touch mode applied (ppp {before}, logical {}×{} pt, interact {} pt)",
            rect.width().round(),
            rect.height().round(),
            ctx.style_of(ctx.theme()).spacing.interact_size.x,
        );
    }

    /// Whether the theme's own fonts are live (it defines named families "medium"/"semibold",
    /// which egui panics on when they are bound to no font).
    fn theme_fonts_live(ctx: &egui::Context) -> bool {
        ctx.fonts(|f| f.definitions().families.keys().any(|k| matches!(k, egui::FontFamily::Name(_))))
    }

    /// Whether the fallback font is missing from the current definitions (a theme change replaces
    /// them from scratch, so the shell re-appends it).
    fn font_missing(ctx: &egui::Context) -> bool {
        ctx.fonts(|f| !f.definitions().font_data.contains_key(SYSTEM_FONT))
    }

    /// The 500 ms long-press in the window's top-left corner (the brand mark and the empty margin
    /// beside it — no menu, button or drag handle lives there, so the gesture cannot also click
    /// something) opens the touch command palette.
    fn gestures(&mut self, ctx: &egui::Context) {
        let (down, pos, released) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.pointer.primary_released()));
        if !down || released {
            self.press = None;
            return;
        }
        let Some(p) = pos else { return };
        // While the finger is down, keep frames coming so the 500 ms threshold can pass.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        // Only the brand mark and the margin around it: the menu titles start right after it, and a
        // long press there would also click the menu on release (the FilmCraft pilot hit exactly
        // that). The mark itself is `Sense::hover()`, so the gesture cannot activate anything.
        let rect = ctx.viewport_rect();
        let zone = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 40.0, rect.top() + 48.0));
        match self.press {
            None => self.press = Some((std::time::Instant::now(), p)),
            Some((started, origin)) => {
                if origin.distance(p) > 16.0 {
                    self.press = None; // a drag, not a press
                } else if started.elapsed() > std::time::Duration::from_millis(500) {
                    if zone.contains(p) && !self.palette {
                        self.palette = true;
                        log::info!("photocraft-android: long-press on the title strip → command palette");
                    }
                    self.press = None;
                }
            }
        }
    }

    /// The touch replacement for the File menu: the desktop menu bar is 12 pt text in a 32 pt
    /// strip, which a finger cannot hit. Every entry is PhotoCraft's *own* command id, run through
    /// the app's control channel with menu semantics (`ui.menu.invoke`) so the commands behave
    /// exactly as they do from the menu — File ▸ Open starts the SAF picker, Save As asks where to
    /// write, Export As opens its dialog. The last section brings back the dock panels the phone
    /// layout starts without ([`phone_layout`]).
    fn command_palette(&mut self, ctx: &egui::Context) {
        const FILE: [(&str, &str); 8] = [
            ("打开…", "file.open"),
            ("新建…", "file.new"),
            ("保存", "file.save"),
            ("另存为…", "file.saveAs"),
            ("快速导出 PNG", "file.export.quickExportAsPng"),
            ("导出为…", "file.export.exportAs"),
            ("关闭", "file.close"),
            ("全部命令（应用自带搜索）", "edit.search"),
        ];
        const PANELS: [(&str, &str); 3] =
            [("面板：图层", "window.panel.layers"), ("面板：颜色", "window.panel.color"), ("面板：属性", "window.panel.properties")];
        let mut close = false;
        // A short phone screen must still reach the last row: scroll rather than run off it.
        let max_height = (ctx.viewport_rect().height() * 0.72).max(240.0);
        egui::Area::new(egui::Id::new("touch-palette")).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(320.0);
                ui.label(egui::RichText::new("PhotoCraft 命令（长按左上角图标打开）").size(13.0));
                ui.separator();
                egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                    for (label, id) in FILE {
                        if ui.add_sized([320.0, 52.0], egui::Button::new(egui::RichText::new(label).size(18.0))).clicked() {
                            self.send(ctx, label, id);
                            close = true;
                        }
                    }
                    ui.separator();
                    for (label, id) in PANELS {
                        if ui.add_sized([320.0, 52.0], egui::Button::new(egui::RichText::new(label).size(18.0))).clicked() {
                            self.send(ctx, label, id);
                            close = true;
                        }
                    }
                    if ui.add_sized([320.0, 44.0], egui::Button::new(egui::RichText::new("取消").size(16.0))).clicked() {
                        close = true;
                    }
                });
            });
        });
        if close {
            self.palette = false;
        }
    }

    /// Run one of PhotoCraft's commands over the app's control channel — the same entry point the
    /// desktop control server, MCP and the CLI use. The reply is logged from its own thread: a
    /// command that opens a dialog answers at once, and one that starts a job must not block the
    /// UI thread.
    fn send(&mut self, ctx: &egui::Context, label: &str, id: &str) {
        let payload = serde_json::json!({"id": id, "params": {}});
        let (request, reply) = ControlRequest::new("ui.menu.invoke", payload);
        match self.control.send(request) {
            Ok(()) => log::info!("palette: {label} → ui.menu.invoke {id}"),
            Err(_) => {
                log::warn!("palette: control channel is closed");
                return;
            }
        }
        ctx.request_repaint();
        // `id` is a borrow; clone it before moving into the thread (which needs 'static). `label`
        // stays here: it is only used in the log line above.
        let id = id.to_owned();
        std::thread::spawn(move || match reply.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(value) => log::info!("palette: {id} replied {value}"),
            Err(e) => log::warn!("palette: no reply for {id}: {e}"),
        });
    }
}

impl eframe::App for TouchShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        if !self.tuned {
            // The first `ui` pass runs after PhotoCraft installed its theme (in `logic`), so the
            // phone adjustments land on top of the theme instead of being replaced by it.
            self.tuned = true;
            Self::tune(ui.ctx());
            phone_layout(&mut self.inner);
        }
        if !self.font_done && Self::theme_fonts_live(ui.ctx()) {
            // The theme's fonts only become visible once egui has rebuilt them, a frame after the
            // theme install: adding the CJK fallback any earlier would drop the theme's families
            // (egui panics on a family that is bound to no font).
            self.font_done = true;
            install_system_font(ui.ctx());
        } else if self.font_done && Self::font_missing(ui.ctx()) {
            // A theme change (Preferences ▸ Interface) rebuilds the fonts from scratch.
            install_system_font(ui.ctx());
        }
        self.gestures(ui.ctx());
        if self.palette {
            self.command_palette(ui.ctx());
        }
    }
}

/// The phone layout: canvas first.
///
/// The right dock cannot be narrower than 250 pt (`photocraft_ui_egui::panels`' `DOCK_WIDTH`), and
/// a phone gives 411 pt of logical width — the desktop dock leaves the canvas about 110 pt, so the
/// panels are cramped *and* the picture is unusable. The shell therefore starts with the dock's
/// panels hidden (toolbar, options bar and canvas keep the whole screen) and puts the three panels
/// an editor needs most in the touch palette; every other panel is one search away in PhotoCraft's
/// own `Edit ▸ Search…` palette, and the choice is remembered like any other panel toggle.
fn phone_layout(app: &mut PhotocraftApp) {
    let panels = &mut app.ui.panels;
    panels.color = false;
    panels.properties = false;
    panels.layers = false;
}

/// Append the device's CJK font to egui's *current* fallback chain: the bundled Inter has no CJK
/// glyphs, so Chinese/Japanese/Korean text would render as tofu. This must run **after**
/// PhotoCraft's theme install (which replaces the fonts), and it edits the current definitions
/// rather than starting from the defaults, so the theme's own fonts and named families stay.
fn install_system_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/system/fonts/NotoSansCJK-Regular.ttc",
        "/system/fonts/NotoSerifCJK-Regular.ttc",
        "/system/fonts/DroidSansFallback.ttf",
        "/system/fonts/DroidSansFallbackFull.ttf",
    ];
    for path in CANDIDATES {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut fonts = ctx.fonts(|f| f.definitions().clone());
                fonts.font_data.insert(SYSTEM_FONT.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                // Every family, including the theme's named weights: a CJK label drawn in
                // "semibold" must find the glyphs through its own stack.
                for family in fonts.families.values_mut() {
                    family.push(SYSTEM_FONT.to_owned());
                }
                ctx.set_fonts(fonts);
                log::info!("photocraft-android: loaded system CJK font {path}");
                return;
            }
            Err(e) => log::warn!("photocraft-android: font {path} unavailable: {e}"),
        }
    }
    log::warn!("photocraft-android: no system CJK font found; CJK text will be tofu");
}

/// Install the last-resort panic hook (PhotoCraft's `AGENTS.md`, Never crash): a panic that escapes
/// the codecs is logged with its location before the process goes down, so logcat names the cause.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PhotoCraft internal error: {info}");
        default(info);
    }));
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("photocraft-android: android_main entered");
    install_panic_hook();
    // Application-private storage: preferences, crash recovery, imported documents, scratch files.
    let Some(data_dir) = app.internal_data_path() else {
        log::error!("photocraft-android: the activity has no internal data path; refusing to start without storage");
        return;
    };
    log::info!("photocraft-android: data directory {}", data_dir.display());
    let paths = services::Paths::resolve(data_dir);
    // The Java activity object: `ndk_context` only hands out the Application, which cannot start
    // the document picker for a result.
    let activity = app.activity_as_ptr() as usize;
    let sources = saf::Sources::default();
    let incoming = services::Incoming::default();
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    // The app's control channel: the touch palette turns its buttons into PhotoCraft commands, the
    // same way the desktop control server, the CLI and MCP drive the app.
    let (control, requests) = std::sync::mpsc::channel::<ControlRequest>();
    // The self-test logs the round trip of the services below on a background thread.
    let started = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            let saf = services::Saf { activity, sources: sources.clone(), ctx: cc.egui_ctx.clone() };
            let mut app = PhotocraftApp::new(Session::new(), services::services(&paths, &saf, &incoming));
            if let Some(rs) = cc.wgpu_render_state.clone() {
                // The custom WGSL canvas renderer; without a render state the CPU canvas is used.
                app.set_wgpu(rs);
            }
            app = app.with_control(requests);
            selftest::spawn(paths.clone(), saf, incoming.clone());
            Ok(Box::new(TouchShell { inner: app, tuned: false, font_done: false, control, palette: false, press: None }))
        }),
    );
    if let Err(e) = started {
        log::error!("photocraft-android: eframe exited: {e}");
    }
}
