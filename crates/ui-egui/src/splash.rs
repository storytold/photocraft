//! The startup splash screen: shown during application initialization to provide
//! a polished, branded loading experience (similar to Krita / Photoshop).
//!
//! Preloads system CJK fallback fonts early and displays loading progress to eliminate
//! window flickering and missing glyph ('tofu') artifacts before the workspace opens.

use crate::theme::Tokens;
use egui::{Color32, CornerRadius, FontId, Rect, RichText, Stroke, Ui, vec2};

/// State for managing the startup splash screen overlay, display timing, and font preloading.
#[derive(Clone, Debug)]
pub struct SplashState {
    /// Start time of the splash display in seconds (from egui `time`).
    pub start_time: f64,
    /// Total duration before the splash automatically finishes (in seconds).
    pub duration: f64,
    /// Whether the splash screen has been explicitly dismissed by the user or system.
    pub dismissed: bool,
    /// Whether the primary CJK fallback font has been preloaded and is active.
    pub fonts_preloaded: bool,
}

impl Default for SplashState {
    /// Creates a default [`SplashState`] with the standard startup duration.
    fn default() -> Self {
        Self::new()
    }
}

impl SplashState {
    /// Creates a new startup splash screen state with standard display duration and animation parameters.
    pub fn new() -> Self {
        Self { start_time: 0.0, duration: 2.2, dismissed: false, fonts_preloaded: false }
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
        // Keep active for standard duration, and until CJK fallback font preloading completes
        // (capped at a 10.0s safety timeout to avoid indefinite stalls on slow font discovery).
        elapsed < self.duration || (!self.fonts_preloaded && elapsed < 10.0)
    }

    /// Dismisses the splash screen immediately, transitioning to the main workspace.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Renders the startup splash screen card, brand mark, and animated progress bar.
    pub fn show(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let now = ctx.input(|i| i.time);
        if self.start_time <= 0.0 {
            self.start_time = now;
        }

        // Trigger CJK fallback font preload on early splash passes
        if !self.fonts_preloaded {
            match crate::cjk_fonts::preload_cjk(ctx) {
                crate::cjk_fonts::PreloadStatus::JustInstalled => {
                    self.fonts_preloaded = true;
                    ctx.request_repaint();
                    // Newly registered fonts in egui activate on the subsequent UI frame pass.
                    // Yield this pass so localized status strings do not render with tofu glyphs.
                    return;
                }
                crate::cjk_fonts::PreloadStatus::Ready => {
                    self.fonts_preloaded = true;
                }
                crate::cjk_fonts::PreloadStatus::Pending => {
                    // Background font discovery scan is pending; keep attempting on later passes.
                }
            }
        }

        let elapsed = (now - self.start_time).max(0.0);
        let progress = if !self.fonts_preloaded {
            (elapsed / (self.duration * 0.85)).clamp(0.0, 0.85) as f32
        } else {
            (elapsed / (self.duration * 0.85)).clamp(0.0, 1.0) as f32
        };

        // Any explicit user click or keypress after 0.4s dismisses the splash screen once fonts are ready
        let dismissed_by_user = self.fonts_preloaded
            && ctx.input(|i| i.pointer.any_click() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if dismissed_by_user && elapsed > 0.4 {
            self.dismiss();
            ctx.request_repaint();
            return;
        }

        // Request repaint while splash is active for smooth progress animation
        ctx.request_repaint();

        let t = Tokens::get(ctx);
        let screen_rect = ctx.content_rect();

        // Dark dim backdrop covering the entire window
        let painter = ui.painter();
        let bg_alpha = if elapsed > self.duration * 0.85 {
            let fade = ((self.duration - elapsed) / (self.duration * 0.15)).clamp(0.0, 1.0) as f32;
            (240.0 * fade) as u8
        } else {
            240
        };
        painter.rect_filled(screen_rect, CornerRadius::ZERO, Color32::from_black_alpha(bg_alpha));

        // Splash card dimensions: 520 x 310 pt
        let card_size = vec2(520.0, 310.0);

        // Center on the visible work area of the monitor (accounting for taskbar & monitor bounds)
        let visible_area = crate::work_area::visible_rect(ctx);
        let center = if visible_area.is_finite() && visible_area.width() > 100.0 && visible_area.height() > 100.0 {
            visible_area.center()
        } else {
            screen_rect.center()
        };

        // Optical center: adjust 8pt upward for natural human visual balance
        let optical_center = egui::pos2(center.x, center.y - 8.0);
        let card_rect = Rect::from_center_size(optical_center, card_size);

        // Soft elevation drop shadow with balanced ambient glow
        for (offset_y, blur, alpha) in [(4.0, 18.0, 25), (2.0, 10.0, 40), (0.0, 4.0, 60)] {
            let shadow_rect = card_rect.translate(vec2(0.0, offset_y)).expand(blur * 0.5);
            painter.rect_filled(shadow_rect, CornerRadius::same(16), Color32::from_black_alpha(alpha));
        }

        // Splash card container
        painter.rect(card_rect, CornerRadius::same(14), t.card, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);

        // Elegant accent highlight line along the top card edge
        let top_bar = Rect::from_min_size(card_rect.min, vec2(card_size.x, 2.5));
        painter.rect_filled(top_bar, CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }, t.accent);

        // Inner layout with symmetrical top/bottom padding
        let inner_rect = card_rect.shrink2(vec2(28.0, 20.0));
        let mut inner_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
        inner_ui.spacing_mut().item_spacing = vec2(0.0, 4.0);

        inner_ui.vertical_centered(|ui| {
            ui.add_space(10.0);

            // Brand icon (72 x 72 pt)
            let icon_size = 72.0;
            let (rect, _) = ui.allocate_exact_size(vec2(icon_size, icon_size), egui::Sense::hover());
            crate::brand::paint_mark(ui, rect);

            ui.add_space(8.0);

            // App Title
            ui.label(RichText::new("PhotoCraft").font(FontId::proportional(26.0)).color(t.text).strong());

            // Version & Subtitle
            let version_text = format!("v{}  ·  Open Source Native Image Editor", env!("CARGO_PKG_VERSION"));
            ui.label(RichText::new(version_text).font(FontId::proportional(12.0)).color(t.text_faint));

            ui.add_space(16.0);

            // Progress Bar (styled nicely)
            let bar_width = 340.0;
            let bar_height = 5.0;
            let (bar_rect, _) = ui.allocate_exact_size(vec2(bar_width, bar_height), egui::Sense::hover());

            // Track
            ui.painter().rect_filled(bar_rect, CornerRadius::same(2), Color32::from_white_alpha(20));

            // Filled progress
            let filled_width = (bar_width * progress).max(5.0);
            let filled_rect = Rect::from_min_size(bar_rect.min, vec2(filled_width, bar_height));
            ui.painter().rect_filled(filled_rect, CornerRadius::same(2), t.accent);

            ui.add_space(8.0);

            // Status message corresponding to progress
            let status_msg = if progress < 0.35 {
                crate::i18n::t("Initializing graphics & engine…")
            } else if progress < 0.85 {
                crate::i18n::t("Loading workspace & font assets…")
            } else {
                crate::i18n::t("Ready")
            };

            ui.label(RichText::new(status_msg).font(FontId::proportional(11.5)).color(t.text_dim));

            ui.add_space(10.0);

            // Skip hint (shown only once fonts are ready to avoid tofu glyphs and match dismiss behavior)
            if self.fonts_preloaded {
                ui.label(RichText::new(crate::i18n::t("Click anywhere to skip")).font(FontId::proportional(10.0)).color(Color32::from_white_alpha(45)));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies splash screen timing lifecycle, timeout expiration, and user dismissal.
    #[test]
    fn splash_state_lifecycle() {
        let mut splash = SplashState::new();
        assert!(splash.is_active(0.0));
        assert!(splash.is_active(0.5));

        splash.start_time = 1.0;
        assert!(splash.is_active(1.5));
        assert!(splash.is_active(2.5));

        // Once fonts are preloaded, the splash finishes after its standard duration.
        splash.fonts_preloaded = true;
        assert!(!splash.is_active(3.5));

        splash.dismiss();
        assert!(!splash.is_active(1.5));
    }

    /// Verifies that splash screen stays active until CJK fonts are ready, with a safety timeout.
    #[test]
    fn splash_remains_active_until_cjk_fonts_ready() {
        let mut splash = SplashState::new();
        splash.start_time = 1.0;

        // While font preload is still pending, splash stays active past its 2.2s duration
        assert!(splash.is_active(3.5));
        assert!(splash.is_active(8.0));

        // Capped by the 10.0s safety ceiling to prevent indefinite hang
        assert!(!splash.is_active(12.0));

        // Marking fonts preloaded allows natural expiration
        splash.fonts_preloaded = true;
        assert!(!splash.is_active(3.5));
    }

    /// Verifies that [`SplashState::default`] initializes state identically to [`SplashState::new`].
    #[test]
    fn splash_state_default() {
        let default = SplashState::default();
        let new = SplashState::new();
        assert_eq!(default.duration, new.duration);
        assert_eq!(default.dismissed, new.dismissed);
        assert_eq!(default.fonts_preloaded, new.fonts_preloaded);
    }
}
