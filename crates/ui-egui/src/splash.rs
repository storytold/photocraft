//! The startup splash screen: shown during application initialization when startup
//! takes longer than a short threshold (e.g. background font database warming).
//!
//! Closes immediately when the system is ready without waiting for an artificial timer.

use crate::theme::Tokens;
use egui::{Color32, CornerRadius, FontId, Rect, RichText, Stroke, Ui, vec2};

/// Threshold in seconds before the splash screen appears. If startup completes faster than
/// this short duration, the splash is not presented to prevent visual flickering.
pub const SPLASH_APPEAR_DELAY: f64 = 0.15;

/// Maximum safety duration before the splash is automatically dismissed (in seconds)
/// to ensure the workspace is never blocked indefinitely.
pub const SPLASH_SAFETY_TIMEOUT: f64 = 5.0;

/// State for managing the startup splash screen overlay, display timing, and font preloading.
#[derive(Clone, Debug)]
pub struct SplashState {
    /// Start time of the splash display in seconds (from egui `time`).
    pub start_time: f64,
    /// Total duration before safety timeout automatically finishes the splash (in seconds).
    pub duration: f64,
    /// Whether the splash screen has been explicitly dismissed by the user or system.
    pub dismissed: bool,
    /// Retained for test compatibility; font loading does not hold the splash hostage.
    pub fonts_preloaded: bool,
}

impl Default for SplashState {
    /// Creates a default [`SplashState`] with the standard startup safety duration.
    fn default() -> Self {
        Self::new()
    }
}

impl SplashState {
    /// Creates a new startup splash screen state.
    pub fn new() -> Self {
        Self {
            start_time: 0.0,
            duration: SPLASH_SAFETY_TIMEOUT,
            dismissed: false,
            fonts_preloaded: true,
        }
    }

    /// Returns whether the splash screen is currently active and within safety timeout limits.
    pub fn is_active(&self, now: f64) -> bool {
        if self.dismissed {
            return false;
        }
        if self.start_time <= 0.0 {
            return true;
        }
        let elapsed = now - self.start_time;
        // Never wait for an artificial timer; active only while startup is ongoing and under safety timeout.
        elapsed < self.duration
    }

    /// Returns whether the splash screen should actually be rendered on screen.
    ///
    /// It appears only if startup takes longer than [`SPLASH_APPEAR_DELAY`] to prevent
    /// visual flashing on fast launches.
    pub fn should_render(&self, now: f64) -> bool {
        if self.dismissed || self.start_time <= 0.0 {
            return false;
        }
        let elapsed = now - self.start_time;
        elapsed >= SPLASH_APPEAR_DELAY && elapsed < self.duration
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

        let elapsed = (now - self.start_time).max(0.0);

        // Any explicit user click or keypress after appear delay dismisses the splash screen
        let dismissed_by_user = ctx.input(|i| i.pointer.any_click() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if dismissed_by_user && elapsed > SPLASH_APPEAR_DELAY {
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
        painter.rect_filled(screen_rect, CornerRadius::ZERO, Color32::from_black_alpha(240));

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

            // Brand icon (72 x 72 pt) - uses existing clean-room vector brand mark
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

            // Progress Bar
            let bar_width = 340.0;
            let bar_height = 5.0;
            let (bar_rect, _) = ui.allocate_exact_size(vec2(bar_width, bar_height), egui::Sense::hover());

            // Track
            ui.painter().rect_filled(bar_rect, CornerRadius::same(2), Color32::from_white_alpha(20));

            let font_ready = photocraft_text::try_shared().is_some();
            let progress = if font_ready {
                1.0_f32
            } else {
                (elapsed as f32 / 1.5).clamp(0.15, 0.90)
            };

            // Filled progress
            let filled_width = (bar_width * progress).max(5.0);
            let filled_rect = Rect::from_min_size(bar_rect.min, vec2(filled_width, bar_height));
            ui.painter().rect_filled(filled_rect, CornerRadius::same(2), t.accent);

            ui.add_space(8.0);

            // Status message
            let status_msg = if font_ready {
                crate::i18n::t("Ready")
            } else if progress < 0.5 {
                crate::i18n::t("Initializing graphics & engine…")
            } else {
                crate::i18n::t("Loading workspace & font assets…")
            };

            ui.label(RichText::new(status_msg).font(FontId::proportional(11.5)).color(t.text_dim));

            ui.add_space(10.0);

            // Skip hint
            ui.label(RichText::new(crate::i18n::t("Click anywhere to skip")).font(FontId::proportional(10.0)).color(Color32::from_white_alpha(45)));
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
        assert!(splash.is_active(5.0));

        // Closes when safety timeout expires
        assert!(!splash.is_active(7.0));

        // Dismiss immediately deactivates splash
        let mut dismissible = SplashState::new();
        dismissible.start_time = 1.0;
        dismissible.dismiss();
        assert!(!dismissible.is_active(1.5));
    }

    /// Verifies that splash appears only after the short appear delay to prevent flickering on fast launches.
    #[test]
    fn splash_appears_only_after_delay_threshold() {
        let mut splash = SplashState::new();
        splash.start_time = 1.0;

        // Startup faster than appear delay does not render splash
        assert!(!splash.should_render(1.0 + SPLASH_APPEAR_DELAY * 0.5));

        // After appear delay has elapsed, splash renders
        assert!(splash.should_render(1.0 + SPLASH_APPEAR_DELAY * 1.5));

        // When dismissed, it ceases rendering immediately
        splash.dismiss();
        assert!(!splash.should_render(1.0 + SPLASH_APPEAR_DELAY * 1.5));
    }

    /// Verifies that splash does not wait for an artificial timer and dismisses immediately on call.
    #[test]
    fn splash_dismisses_immediately_without_waiting_for_timer() {
        let mut splash = SplashState::new();
        splash.start_time = 1.0;
        assert!(splash.is_active(1.2));

        // When ready, caller immediately dismisses; does not wait for any fixed timer duration
        splash.dismiss();
        assert!(!splash.is_active(1.2));
        assert!(!splash.should_render(1.2));
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
