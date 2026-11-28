//! The Other Cursors preference changes the Eyedropper over the document canvas.

use craft::behavior::{Cursor, IconId, PaintedCursor};
use craft::ui::primitives::painted_image;
use craft::ui::runtime::{env::Env, windows::WindowHost};
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

fn other_cursor(h: &mut Harness<'static, Box<PhotocraftApp>>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": value})).unwrap();
}

fn pointer(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) -> Cursor {
    hover(h, 100.0, 150.0, mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|response| response.cursor.clone()).expect("the tool cursor over the canvas")
}

#[test]
fn precise_eyedropper_draws_a_centred_target() {
    let mut h = harness(Tool::Eyedropper);
    WindowHost::of(&h.ctx).set_caps_lock(Some(false));
    other_cursor(&mut h, "precise");
    let cursor = pointer(&mut h, Modifiers::NONE);
    let Cursor::Painted(painted @ PaintedCursor::Circle { centre: true, .. }) = cursor else {
        panic!("Precise Eyedropper should show a circle and crosshair, got {cursor:?}");
    };
    let bitmap = painted_image(&Env::of_root(&h.ctx), &painted, 2.0).expect("the target has a cursor bitmap");
    assert_eq!(bitmap.hotspot, bitmap.size.map(|side| side / 2), "the hotspot is the bitmap's centre");

    let width = usize::from(bitmap.size[0]);
    let [cx, cy] = bitmap.hotspot.map(usize::from);
    let mut light_on_black = false;
    let mut dark_on_white = false;
    for (i, &[r, g, b, a]) in bitmap.rgba.as_chunks::<4>().0.iter().enumerate() {
        // Sample off both axes so the contrast check uses the ring, away from the centre cross.
        if (i % width).abs_diff(cx) <= 2 || (i / width).abs_diff(cy) <= 2 {
            continue;
        }
        let alpha = f32::from(a) / 255.0;
        let lightest = f32::from(r.max(g).max(b));
        let darkest = f32::from(r.min(g).min(b));
        light_on_black |= darkest * alpha >= 100.0;
        dark_on_white |= lightest * alpha + 255.0 * (1.0 - alpha) <= 145.0;
    }
    assert!(light_on_black && dark_on_white, "the ring has a visible stroke on black and on white");
}

#[test]
fn standard_eyedropper_and_unaffected_tools_keep_their_cursors() {
    let mut h = harness(Tool::Eyedropper);
    let window = WindowHost::of(&h.ctx);
    window.set_caps_lock(Some(false));
    let standard = pointer(&mut h, Modifiers::NONE);
    assert!(matches!(&standard, Cursor::Painted(PaintedCursor::Tool { icon, .. }) if *icon == IconId::new("eyedropper")));
    window.set_caps_lock(Some(true));
    assert_eq!(pointer(&mut h, Modifiers::NONE), Cursor::Painted(PaintedCursor::Crosshair), "Caps Lock alone still uses the crosshair");
    other_cursor(&mut h, "precise");
    assert_eq!(pointer(&mut h, Modifiers::NONE), standard, "Caps Lock still inverts the preference");
    window.set_caps_lock(Some(false));

    for tool in [Tool::Move, Tool::Type, Tool::Hand, Tool::Zoom] {
        h.state_mut().ui.tool = tool;
        other_cursor(&mut h, "standard");
        let standard = pointer(&mut h, Modifiers::NONE);
        other_cursor(&mut h, "precise");
        assert_eq!(pointer(&mut h, Modifiers::NONE), standard, "{tool:?} keeps its cursor");
    }
    h.state_mut().ui.tool = Tool::Brush;
    assert_eq!(pointer(&mut h, Modifiers::ALT), Cursor::Painted(PaintedCursor::Crosshair), "the Alt stand-in keeps its cursor");
}
