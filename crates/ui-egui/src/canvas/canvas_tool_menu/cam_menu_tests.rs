use super::tests::open_here;
use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app
}

#[test]
fn content_aware_move_boundary_modes_and_overlay_guards() {
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ContentAwareMove;
    crate::color_hud::open(&mut empty, [12.0, 12.0], false);
    assert!(!open_here(&mut empty, Tool::ContentAwareMove, [8.0, 8.0]));
    assert!(empty.ui.color_hud.is_none());

    let mut app = app();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    // Boundary, negative, and non-finite coordinates.
    for (pt, expected) in [([4.0, 4.0], true), ([11.9, 11.9], true), ([3.9, 4.0], false), ([12.0, 12.0], false), ([-1.0, 4.0], false)] {
        assert!(open_here(&mut app, Tool::ContentAwareMove, pt));
        let m = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(inside_selection(&m), expected, "point {pt:?}");
    }
    assert!(!open_here(&mut app, Tool::ContentAwareMove, [f64::NAN, 4.0]), "a point that is not finite opens no menu");

    // Excluded color modes return false and release a non-held color HUD.
    for mode in [ColorMode::Bitmap, ColorMode::Duotone, ColorMode::Multichannel] {
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).mode = mode;
        crate::color_hud::open(&mut app, [12.0, 12.0], false);
        assert!(!open_here(&mut app, Tool::ContentAwareMove, [6.0, 6.0]));
        assert!(app.ui.color_hud.is_none());
    }
}
