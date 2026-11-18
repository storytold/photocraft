//! The Brush Preset picker's remembered view: what `save` writes into the preferences comes back through `restore`.

use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn the_saved_view_round_trips_through_the_prefs_and_old_keys_still_restore() {
    // Nothing saved: PhotoCraft's default (name and stroke on, tip off, search on).
    let mut a = app();
    restore(&mut a);
    let st = &a.ui.brush_picker_state;
    assert_eq!((st.parts.name, st.parts.stroke, st.parts.tip, st.search), (true, true, false, true));

    // A view the user changed is written by `save` and read back by `restore` in a fresh state.
    let mut settings = a.ui.brush_picker_state.settings();
    settings.parts = settings.parts.set(craft::draw::ui::brush_presets::BrushCardPart::Tip, true);
    settings.search = false;
    save(&mut a, &settings);
    assert_ne!(a.session.prefs().brush_picker, Value::Null, "the view is in the prefs");
    a.ui.brush_picker_state = BrushPickerState::new();
    restore(&mut a);
    let st = &a.ui.brush_picker_state;
    assert!(st.parts.tip && !st.search);

    // The old keys restore once, and a view with no part on gets its parts back.
    let mut old = app();
    old.session.prefs.edit(|p| p.brush_picker = json!({"showTip": true, "showSearch": false}));
    restore(&mut old);
    let st = &old.ui.brush_picker_state;
    assert!(st.parts.name && st.parts.tip && !st.search);
    old.session.prefs.edit(|p| p.brush_picker = json!({"showName": false, "showStroke": false, "showTip": false}));
    restore(&mut old);
    let p = old.ui.brush_picker_state.parts;
    assert!(p.name || p.stroke || p.tip, "an empty card would show no brush");
}
