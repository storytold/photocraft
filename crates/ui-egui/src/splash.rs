//! The startup splash screen: shown during application initialization to provide
//! a polished, branded loading experience (comparable to Adobe Photoshop / Krita).
//!
//! Features the release artwork, brand title, version string, animated loading
//! progress bar, and status feedback while the engine and workspace initialize.

use crate::theme::Tokens;
use egui::{Color32, CornerRadius, FontId, Id, Rect, RichText, Stroke, TextureHandle, TextureOptions, Ui, pos2, vec2};
use photocraft_codecs::{ChannelLayout, SampleType};

/// Embedded splash screen artwork JPEG (1024x571, high fidelity kitsune release art).
const SPLASH_IMAGE_JPEG: &[u8] = include_bytes!("../../../assets/splash/photocraft-splash.jpg");

/// Decodes the embedded splash artwork into an [`egui::ColorImage`].
fn decode_splash_image() -> egui::ColorImage {
    let rgba = photocraft_codecs::decode(SPLASH_IMAGE_JPEG)
        .ok()
        .map(|img| img.convert(ChannelLayout::Rgba, SampleType::U8));
    match rgba {
        Some(img) if img.data().len() == img.width() as usize * img.height() as usize * 4 => {
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data())
        }
        _ => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// Retrieves or uploads the cached splash texture in the current [`egui::Context`].
fn splash_texture(ctx: &egui::Context) -> TextureHandle {
    let id = Id::unique("photocraft-splash-artwork-texture");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..TextureOptions::LINEAR
    };
    let tex = ctx.load_texture("photocraft-splash-artwork", decode_splash_image(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// State for managing the startup splash screen overlay, display timing, and animations.
#[derive(Clone, Debug)]
pub struct SplashState {
    /// Start time of the splash display in seconds (from egui `time`).
    pub start_time: f64,
    /// Total duration before the splash automatically finishes (in seconds).
    pub duration: f64,
    /// Whether the splash screen has been explicitly dismissed by the user or system.
    pub dismissed: bool,
}

impl Default for SplashState {
    /// Creates a default [`SplashState`] with the standard startup duration.
    fn default() -> Self {
        Self::new()
    }
}

impl SplashState {
    /// Creates a new startup splash screen state with standard display duration and parameters.
    pub fn new() -> Self {
        Self {
            start_time: 0.0,
            duration: 2.2,
            dismissed: false,
        }
    }

    /// Returns whether the splash screen is currently active and should be presented.
    pub fn is_active(&self, now: f64) -> bool {
        if self.dismissed {
            return false;
        }
        if self.start_time <= 0.0 {
            return true;
        }
        let elapsed = now - self.start_time;
        elapsed < self.duration
    }

    /// Dismisses the splash screen immediately, transitioning to the main workspace.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Renders the startup splash screen card, release artwork, and animated progress bar.
    pub fn show(&mut self, ctx: &egui::Context, _ui: &mut Ui) {
        let now = ctx.input(|i| i.time);
        if self.start_time <= 0.0 {
            self.start_time = now;
        }

        let elapsed = (now - self.start_time).max(0.0);
        let progress = (elapsed / (self.duration * 0.85)).clamp(0.0, 1.0) as f32;

        // Any explicit user click or keypress after 0.25s dismisses the splash screen
        let dismissed_by_user = ctx.input(|i| {
            i.pointer.any_click()
                || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        if dismissed_by_user && elapsed > 0.25 {
            self.dismiss();
            ctx.request_repaint();
            return;
        }

        // Request repaint while splash is active for smooth progress animation
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        let t = Tokens::get(ctx);
        let screen = ctx.content_rect();

        egui::Area::new(egui::Id::unique("photocraft-splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                // Clean and transparent scrim: intercepts clicks to skip without painting any dark/black background
                let (_, scrim_resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                if scrim_resp.clicked() && elapsed > 0.25 {
                    self.dismiss();
                    ctx.request_repaint();
                    return;
                }

                let painter = ui.painter();

                // Splash card dimensions: 560 x 400 pt (Artwork: 560 x 312 pt, Footer: 560 x 88 pt)
                let card_size = vec2(560.0, 400.0);

                // Center on visible work area or screen rect
                let visible_area = crate::work_area::visible_rect(ctx);
                let center = if visible_area.is_finite() && visible_area.width() > 100.0 && visible_area.height() > 100.0 {
                    visible_area.center()
                } else {
                    screen.center()
                };

                // Optical center: adjust slightly upward for visual balance
                let optical_center = pos2(center.x, center.y - 10.0);
                let card_rect = Rect::from_center_size(optical_center, card_size);

                // Multi-layered soft drop shadow to elevate the card cleanly over the workspace
                for (offset_y, blur, alpha) in [(4.0, 24.0, 30), (2.0, 12.0, 50), (0.0, 4.0, 70)] {
                    let shadow_rect = card_rect.translate(vec2(0.0, offset_y)).expand(blur * 0.5);
                    painter.rect_filled(shadow_rect, CornerRadius::same(14), Color32::from_black_alpha(alpha));
                }

                // Splash card background container
                painter.rect_filled(card_rect, CornerRadius::same(12), t.card);

                // Top artwork area (560 x 312 pt)
                let artwork_height = 312.0;
                let artwork_rect = Rect::from_min_size(card_rect.min, vec2(card_size.x, artwork_height));
                let tex = splash_texture(ctx);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(tex.id(), artwork_rect, uv, Color32::WHITE);

                // Subtle separator line below artwork
                let sep_y = card_rect.min.y + artwork_height;
                painter.line_segment(
                    [pos2(card_rect.min.x, sep_y), pos2(card_rect.max.x, sep_y)],
                    Stroke::new(1.0, t.card_border),
                );

                // Card border stroke
                painter.rect_stroke(card_rect, CornerRadius::same(12), Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);

                // Footer container (560 x 88 pt)
                let footer_rect = Rect::from_min_max(
                    pos2(card_rect.min.x + 20.0, sep_y + 12.0),
                    pos2(card_rect.max.x - 20.0, card_rect.max.y - 12.0),
                );
                let mut footer_ui = ui.new_child(egui::UiBuilder::new().max_rect(footer_rect));

        footer_ui.horizontal(|ui| {
            // Left column: Brand + Version + Status line
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let mark_size = 24.0;
                    let (mark_rect, _) = ui.allocate_exact_size(vec2(mark_size, mark_size), egui::Sense::hover());
                    crate::brand::paint_mark(ui, mark_rect);
                    ui.add_space(6.0);
                    ui.label(RichText::new("PhotoCraft").font(FontId::proportional(18.0)).strong().color(t.text));
                    ui.add_space(4.0);
                    let version_text = format!("v{}", env!("CARGO_PKG_VERSION"));
                    ui.label(RichText::new(version_text).font(FontId::proportional(12.0)).color(t.text_faint));
                });

                ui.add_space(4.0);

                let status_msg = if progress < 0.40 {
                    crate::i18n::t("Initializing graphics & engine…")
                } else if progress < 0.85 {
                    crate::i18n::t("Loading workspace & fonts…")
                } else {
                    crate::i18n::t("Ready")
                };
                ui.label(RichText::new(status_msg).font(FontId::proportional(11.0)).color(t.text_dim));
            });

            // Expand spacer between left info and right progress
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    let bar_width = 160.0;
                    let bar_height = 4.0;
                    let (bar_rect, _) = ui.allocate_exact_size(vec2(bar_width, bar_height), egui::Sense::hover());

                    // Progress track
                    ui.painter().rect_filled(bar_rect, CornerRadius::same(2), Color32::from_white_alpha(30));

                    // Filled progress
                    let filled_width = (bar_width * progress).max(4.0);
                    let filled_rect = Rect::from_min_size(bar_rect.min, vec2(filled_width, bar_height));
                    ui.painter().rect_filled(filled_rect, CornerRadius::same(2), t.accent);

                    ui.add_space(4.0);

                    // Skip hint
                    ui.label(
                        RichText::new(crate::i18n::t("Click anywhere to skip"))
                            .font(FontId::proportional(9.5))
                            .color(Color32::from_white_alpha(70)),
                    );
                });
            });
        });
    });
}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that the embedded splash image decodes to the expected dimensions.
    #[test]
    fn splash_image_decodes_successfully() {
        let img = decode_splash_image();
        assert_eq!(img.size, [1024, 571], "Splash image must decode to 1024x571");
        assert_eq!(img.pixels.len(), 1024 * 571);
    }

    /// Verifies splash screen timing lifecycle, timeout expiration, and user dismissal.
    #[test]
    fn splash_state_lifecycle() {
        let mut splash = SplashState::new();
        assert!(splash.is_active(0.0));
        assert!(splash.is_active(0.5));

        splash.start_time = 1.0;
        assert!(splash.is_active(1.5));
        assert!(splash.is_active(2.5));

        // After duration (2.2s), splash automatically finishes
        assert!(!splash.is_active(3.5));

        splash.dismiss();
        assert!(!splash.is_active(1.5));
    }

    /// Verifies that [`SplashState::default`] matches [`SplashState::new`].
    #[test]
    fn splash_state_default() {
        let default = SplashState::default();
        let new = SplashState::new();
        assert_eq!(default.duration, new.duration);
        assert_eq!(default.dismissed, new.dismissed);
    }
}
