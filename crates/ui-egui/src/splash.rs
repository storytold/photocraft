//! The startup splash screen: shown during application initialization when startup
//! takes longer than a short threshold (e.g. background font database warming).
//!
//! Closes immediately when the system is ready without waiting for an artificial timer.

use craft::ui::contract::{Props as _, Show as _, Sizing};
use craft::ui::display::Progress;
use craft::ui::runtime::env::Env;
use craft::ui::style::{UiTokens, rgba_color};
use craft::window::desktop::{WindowState, visible_part};
use egui::{CornerRadius, FontId, Rect, RichText, Shadow, Stroke, TextStyle, Ui, vec2};

/// Threshold in seconds before the splash screen appears. If startup completes faster than
/// this short duration, the splash is not presented to prevent visual flickering.
pub const SPLASH_APPEAR_DELAY: f64 = 0.15;

/// Maximum safety duration before the splash is automatically dismissed (in seconds)
/// to ensure the workspace is never blocked indefinitely.
pub const SPLASH_SAFETY_TIMEOUT: f64 = 5.0;

/// State for managing the startup splash screen overlay and display timing.
#[derive(Clone, Debug)]
pub struct SplashState {
    /// Start time of the splash display in seconds (from egui `time`).
    pub start_time: f64,
    /// Total duration before safety timeout automatically finishes the splash (in seconds).
    pub duration: f64,
    /// Whether the splash screen has been explicitly dismissed by the user or system.
    pub dismissed: bool,
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
        Self { start_time: 0.0, duration: SPLASH_SAFETY_TIMEOUT, dismissed: false }
    }

    /// Ensures that `start_time` is initialized to the current time on the first tick before checks.
    pub fn ensure_started(&mut self, now: f64) {
        if self.start_time <= 0.0 {
            self.start_time = now;
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

    /// Returns the remaining duration until the splash screen should appear, or `None` if it has already appeared or is dismissed.
    pub fn time_until_appearance(&self, now: f64) -> Option<std::time::Duration> {
        if self.dismissed || self.start_time <= 0.0 {
            return None;
        }
        let elapsed = now - self.start_time;
        if elapsed < SPLASH_APPEAR_DELAY {
            let remaining = (SPLASH_APPEAR_DELAY - elapsed).max(0.0);
            Some(std::time::Duration::from_secs_f64(remaining))
        } else {
            None
        }
    }

    /// Dismisses the splash screen immediately, transitioning to the main workspace.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Renders the startup splash screen card, brand mark, and progress bar using the framework tokens.
    pub fn show(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let now = ctx.input(|i| i.time);
        self.ensure_started(now);

        let elapsed = (now - self.start_time).max(0.0);

        // Any explicit user click or keypress after appear delay dismisses the splash screen.
        let dismissed_by_user = ctx.input(|i| i.pointer.any_click() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if dismissed_by_user && elapsed > SPLASH_APPEAR_DELAY {
            self.dismiss();
            ctx.request_repaint();
            return;
        }

        // Request repaint while splash is active for smooth progress animation.
        ctx.request_repaint();

        let tokens = Env::of(ui).tokens.clone();
        let screen_rect = ctx.content_rect();
        let card_rect = splash_card_rect(ctx, screen_rect);
        paint_backdrop_and_card(ui.painter(), &tokens, screen_rect, card_rect);

        let inner_rect = card_rect.shrink2(vec2(28.0, 20.0));
        let mut inner_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
        inner_ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
        paint_card_contents(&mut inner_ui, &tokens);
    }
}

/// Computes the centered card rectangle within the visible work area of the monitor.
fn splash_card_rect(ctx: &egui::Context, screen_rect: Rect) -> Rect {
    let card_size = vec2(520.0, 310.0);
    let visible_area = ctx.input(|i| {
        let v = i.viewport();
        visible_part(screen_rect, v.monitor_size, v.inner_rect, WindowState::of(v))
    });
    let center =
        if visible_area.is_finite() && visible_area.width() > 100.0 && visible_area.height() > 100.0 { visible_area.center() } else { screen_rect.center() };
    // Optical center: adjust 8pt upward for natural human visual balance.
    let optical_center = egui::pos2(center.x, center.y - 8.0);
    Rect::from_center_size(optical_center, card_size)
}

/// Paints the backdrop, elevation shadow, card surface, and top accent highlight from theme tokens.
fn paint_backdrop_and_card(painter: &egui::Painter, tokens: &UiTokens, screen_rect: Rect, card_rect: Rect) {
    painter.rect_filled(screen_rect, CornerRadius::ZERO, rgba_color(tokens.color.surface.background));
    painter.rect_filled(screen_rect, CornerRadius::ZERO, rgba_color(tokens.color.surface.scrim));

    let radius_window = UiTokens::corner(tokens.shape.radius.window);
    let shadow_color = rgba_color(tokens.color.surface.shadow);
    if shadow_color.a() > 0 {
        let elev = tokens.elevation.popup;
        let shadow = Shadow {
            offset: [elev.offset[0].round().clamp(-128.0, 127.0) as i8, elev.offset[1].round().clamp(-128.0, 127.0) as i8],
            blur: elev.blur.round().clamp(0.0, 255.0) as u8,
            spread: elev.spread.round().clamp(0.0, 255.0) as u8,
            color: shadow_color,
        };
        painter.add(shadow.as_shape(card_rect, radius_window));
    }

    painter.rect(
        card_rect,
        radius_window,
        rgba_color(tokens.color.surface.panel),
        Stroke::new(tokens.stroke.thin, rgba_color(tokens.color.line.border)),
        egui::StrokeKind::Inside,
    );

    let top_bar = Rect::from_min_size(card_rect.min, vec2(card_rect.width(), 2.5));
    painter.rect_filled(top_bar, CornerRadius { nw: radius_window.nw, ne: radius_window.ne, sw: 0, se: 0 }, rgba_color(tokens.color.accent.solid));
}

/// Renders the brand mark, app title, version tagline, progress bar, status text, and skip hint.
fn paint_card_contents(ui: &mut Ui, tokens: &UiTokens) {
    let text_normal = rgba_color(tokens.color.text.normal);
    let text_muted = rgba_color(tokens.color.text.muted);
    let text_faint = rgba_color(tokens.color.text.faint);
    let title_size = (TextStyle::Body.resolve(ui.style()).size * (26.0 / 12.0)).max(1.0);

    ui.vertical_centered(|ui| {
        ui.add_space(10.0);

        // Brand icon (72 x 72 pt) - uses the app's clean-room brand mark texture.
        let icon_size = 72.0;
        let (rect, _) = ui.allocate_exact_size(vec2(icon_size, icon_size), egui::Sense::hover());
        crate::panels::brand::paint_mark(ui, rect);

        ui.add_space(8.0);

        // App Title
        ui.label(RichText::new("PhotoCraft").font(FontId::proportional(title_size)).color(text_normal).strong());

        // Version & Subtitle
        let version_text = crate::i18n::fmt(tl!("v{version}  ·  Open Source Native Image Editor"), &[("version", env!("CARGO_PKG_VERSION"))]);
        ui.label(RichText::new(version_text).text_style(TextStyle::Body).color(text_faint));

        ui.add_space(16.0);

        let status_msg = tl!("Loading workspace & font assets…");
        let bar_width = 340.0;
        let bar_height = tokens.size.groove * 2.0;
        ui.allocate_ui(vec2(bar_width, bar_height), |ui| {
            let _ = Progress::indeterminate().label(status_msg).sizing(Sizing::FillWidth).show(ui);
        });

        ui.add_space(8.0);

        // Status message
        ui.label(RichText::new(status_msg).text_style(TextStyle::Body).color(text_muted));

        ui.add_space(10.0);

        // Skip hint
        ui.label(RichText::new(tl!("Click anywhere to skip")).text_style(TextStyle::Small).color(text_faint));
    });
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

    /// Verifies that `ensure_started` initializes `start_time` and allows `should_render` to work without `show`.
    #[test]
    fn splash_ensure_started_enables_render_and_timeout() {
        let mut splash = SplashState::new();
        assert_eq!(splash.start_time, 0.0);
        assert!(!splash.should_render(1.0));

        // First tick calls ensure_started(1.0)
        splash.ensure_started(1.0);
        assert_eq!(splash.start_time, 1.0);

        // Before appear delay: should not render
        assert!(!splash.should_render(1.05));

        // After appear delay: should render
        assert!(splash.should_render(1.0 + SPLASH_APPEAR_DELAY + 0.05));

        // After safety timeout: is_active becomes false
        assert!(!splash.is_active(1.0 + SPLASH_SAFETY_TIMEOUT + 0.1));
    }

    /// Verifies that [`SplashState::default`] initializes state identically to [`SplashState::new`].
    #[test]
    fn splash_state_default() {
        let default = SplashState::default();
        let new = SplashState::new();
        assert_eq!(default.duration, new.duration);
        assert_eq!(default.dismissed, new.dismissed);
    }

    /// Verifies calculation of remaining time until appearance before and after the appear delay threshold.
    #[test]
    fn splash_time_until_appearance() {
        let mut splash = SplashState::new();
        // Not started yet
        assert_eq!(splash.time_until_appearance(1.0), None);

        splash.ensure_started(1.0);
        // Halfway through appear delay
        let remaining = splash.time_until_appearance(1.0 + SPLASH_APPEAR_DELAY * 0.5);
        assert!(remaining.is_some());
        let secs = remaining.unwrap().as_secs_f64();
        assert!((secs - SPLASH_APPEAR_DELAY * 0.5).abs() < 1e-6);

        // After delay threshold
        assert_eq!(splash.time_until_appearance(1.0 + SPLASH_APPEAR_DELAY + 0.01), None);

        // Dismissed
        splash.dismiss();
        assert_eq!(splash.time_until_appearance(1.0), None);
    }

    /// Verifies that every user-visible splash string (including the tagline) goes through `i18n`
    /// while keeping the product name `PhotoCraft` untranslated.
    #[test]
    fn splash_show_routes_copy_through_i18n() {
        fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(shape) => out.push(shape.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
                _ => {}
            }
        }
        let catalog = "\tv{version}  ·  Open Source Native Image Editor\tv{version}  ·  Translated Editor\n\
                       \tLoading workspace & font assets…\tTranslated Load…\n\
                       \tClick anywhere to skip\tTranslated Skip\n";
        let lang = crate::i18n::fixture_lang("fr", catalog);
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, Default::default());
        let mut splash = SplashState::new();
        splash.start_time = 0.01;
        let mut output = crate::i18n::with_language(lang, || {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 768.0))), time: Some(0.5), ..Default::default() };
            ctx.run_ui(input, |ui| splash.show(&ctx, ui))
        });
        output.textures_delta.clear();
        let mut drawn = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut drawn);
        }
        let expected_tagline = format!("v{}  ·  Translated Editor", env!("CARGO_PKG_VERSION"));
        assert!(drawn.iter().any(|s| s == "PhotoCraft"), "product name stays English: {drawn:?}");
        assert!(drawn.iter().any(|s| s == &expected_tagline), "tagline must be translated: {drawn:?}");
        assert!(drawn.iter().any(|s| s == "Translated Skip"), "skip hint must be translated: {drawn:?}");
        assert!(drawn.iter().any(|s| s == "Translated Load…"), "status message must be translated: {drawn:?}");
    }
}
