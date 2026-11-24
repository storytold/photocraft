//! The Patch tool's cursor shows a right-pointing arrow over the selection (#12130).

use craft::behavior::CursorBadge;
use craft::keymap::Mods;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::tool_feedback::patch_badge;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
    app.ui.tool = tool;
    app
}

const INSIDE: [f64; 2] = [30.0, 30.0];
const OUTSIDE: [f64; 2] = [80.0, 80.0];
const SHIFT: Mods = Mods { shift: true, ..Mods::NONE };
const ALT: Mods = Mods { alt: true, ..Mods::NONE };
const BOTH: Mods = Mods { shift: true, alt: true, ..Mods::NONE };

/// (tool, pointer, press point and modifiers of the drag under way, modifiers held now) → badge.
/// A drag keeps what its press decided: the point it started at and the modifiers it started with.
#[test]
fn patch_badge_follows_its_press() {
    use CursorBadge::{Add, Arrow, Intersect, Subtract};
    let rows = [
        // No drag: the pointer and the held modifiers decide.
        (Tool::Patch, INSIDE, None, Mods::NONE, Some(Arrow)),
        (Tool::Patch, OUTSIDE, None, Mods::NONE, None),
        (Tool::Patch, INSIDE, None, SHIFT, Some(Add)),
        (Tool::Patch, INSIDE, None, ALT, Some(Subtract)),
        (Tool::Patch, INSIDE, None, BOTH, Some(Intersect)),
        (Tool::ContentAwareMove, INSIDE, None, Mods::NONE, None),
        // A drag started inside keeps the arrow wherever the pointer goes and whatever is pressed now.
        (Tool::Patch, OUTSIDE, Some((INSIDE, Mods::NONE)), Mods::NONE, Some(Arrow)),
        (Tool::Patch, OUTSIDE, Some((INSIDE, Mods::NONE)), SHIFT, Some(Arrow)),
        (Tool::Patch, OUTSIDE, Some((INSIDE, Mods::NONE)), ALT, Some(Arrow)),
        (Tool::Patch, OUTSIDE, Some((INSIDE, Mods::NONE)), BOTH, Some(Arrow)),
        (Tool::ContentAwareMove, INSIDE, Some((INSIDE, Mods::NONE)), Mods::NONE, None),
        // A drag started outside does not show the arrow on entering the selection.
        (Tool::Patch, INSIDE, Some((OUTSIDE, Mods::NONE)), Mods::NONE, None),
        // A drag pressed with Shift or Alt keeps that badge after the key is let go.
        (Tool::Patch, INSIDE, Some((INSIDE, SHIFT)), Mods::NONE, Some(Add)),
        (Tool::Patch, INSIDE, Some((INSIDE, ALT)), Mods::NONE, Some(Subtract)),
    ];
    for (tool, at, press, mods, want) in rows {
        let app = app(tool);
        assert_eq!(patch_badge(&app, at, press, mods), want, "{tool:?} at {at:?} press {press:?} now {mods:?}");
    }
    // Other tools have none.
    assert_eq!(patch_badge(&app(Tool::Lasso), INSIDE, None, SHIFT), None);
}
