//! The Recently Used Colors of the shape Fill and Stroke chips (the options bar's and the Properties
//! panel's `PaintChip::recent`). They live in `Preferences::recent_colors`, so they survive a
//! restart. The Eyedropper adds what it samples, as Photoshop's single history of lately used
//! colours (#11524).

use crate::PhotocraftApp;

/// How many Recently Used Colors are kept (one row of the popover).
pub(crate) const MAX_RECENT: usize = 15;

/// The Recently Used Colors, newest first (at most [`MAX_RECENT`]; unreadable entries skipped).
pub(crate) fn recent_colors(session: &photocraft_engine::Session) -> Vec<[f32; 3]> {
    session.prefs().recent_colors.iter().filter_map(|h| crate::color_picker_ui::parse_hex(h)).take(MAX_RECENT).collect()
}

/// Put `rgb` at the front of the Recently Used Colors (once), as Photoshop does for a colour that
/// was used.
pub(crate) fn remember(app: &mut PhotocraftApp, rgb: [f32; 3]) {
    if rgb.iter().any(|c| !c.is_finite()) {
        return;
    }
    let hex = crate::color_picker_ui::hex(rgb);
    let mut next: Vec<String> = vec![hex.clone()];
    next.extend(recent_colors(&app.session).into_iter().map(crate::color_picker_ui::hex).filter(|h| *h != hex));
    next.truncate(MAX_RECENT);
    if app.session.prefs().recent_colors != next {
        app.session.prefs.edit(|p| p.recent_colors = next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_moves_a_colour_to_the_front_once_and_caps_the_row() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        for i in 0..20 {
            remember(&mut app, [i as f32 / 255.0, 0.0, 0.0]);
        }
        let recent = recent_colors(&app.session);
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(crate::color_picker_ui::hex(recent[0]), "#130000", "newest first");
        remember(&mut app, [10.0 / 255.0, 0.0, 0.0]);
        let recent = recent_colors(&app.session);
        assert_eq!(crate::color_picker_ui::hex(recent[0]), "#0a0000");
        assert_eq!(recent.iter().filter(|c| crate::color_picker_ui::hex(**c) == "#0a0000").count(), 1, "no duplicates");
        // Non-finite colours and hand-edited junk never reach the row.
        remember(&mut app, [f32::NAN, 0.0, 0.0]);
        app.session.prefs.edit(|p| p.recent_colors.insert(0, "not a colour é".into()));
        assert_eq!(crate::color_picker_ui::hex(recent_colors(&app.session)[0]), "#0a0000");
    }

    /// #11524: the Swatches panel's history is this very list, so it is the preferences' own
    /// `recentColors`: it round-trips, and a file written by an older version (without the key)
    /// still loads, with an empty row.
    #[test]
    fn recently_used_colors_survive_a_restart() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        remember(&mut app, [0.25, 0.5, 0.75]);
        remember(&mut app, [1.0, 0.5, 0.0]);
        let json = serde_json::to_value(app.session.prefs()).expect("the preferences serialize");
        assert_eq!(json["recentColors"], serde_json::json!(["#ff8000", "#4080bf"]), "newest first on disk");
        let back: photocraft_engine::prefs::Preferences = serde_json::from_value(json).expect("and load back");
        assert_eq!(back.recent_colors, app.session.prefs().recent_colors, "the same row next launch");
        let old: photocraft_engine::prefs::Preferences = serde_json::from_str("{}").expect("an older preferences file loads");
        assert!(old.recent_colors.is_empty(), "which held no history");
    }
}
