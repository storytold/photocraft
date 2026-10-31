//! Regression tests for issue #709: the single-filter gallery commands must
//! record the session foreground/background colours for every filter that
//! reads them. Colored Pencil (background) and Neon Glow (both) read the
//! colours in their pixel code but omit them from their params notation, so
//! the injection was skipped and they always ran with the defaults.
use photocraft_engine::Session;
use serde_json::json;

fn px(s: &mut Session, x: i32, y: i32) -> Vec<f32> {
    serde_json::from_value(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap()).unwrap()
}

fn image(s: &mut Session) -> Vec<f32> {
    (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).flat_map(|(x, y)| px(s, x, y)).collect()
}

fn one(filter: &str, bg: &str) -> Vec<f32> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#808080"})).unwrap();
    s.execute("tools.setColors", json!({"background": bg})).unwrap();
    s.execute(&format!("filter.gallery.{filter}"), json!({})).unwrap();
    image(&mut s)
}

#[test]
fn colored_pencil_honours_session_background() {
    assert_ne!(one("coloredPencil", "#ffffff"), one("coloredPencil", "#0000ff"), "filter.gallery.coloredPencil must honour the session background");
}

#[test]
fn neon_glow_honours_session_background() {
    assert_ne!(one("neonGlow", "#ffffff"), one("neonGlow", "#0000ff"), "filter.gallery.neonGlow must honour the session background");
}

/// Stamp uses the selected ink and paper without adding intermediate colors (#9912).
#[test]
fn stamp_uses_only_the_selected_colors() {
    for depth in [8, 16, 32] {
        for (fg, bg, ink, paper) in [("#000000", "#ffffff", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), ("#ff0000", "#0000ff", [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])] {
            for smoothness in [1, 5, 50] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "#808080"})).unwrap();
                s.execute("tools.setColors", json!({"foreground": fg, "background": bg})).unwrap();
                s.execute("filter.gallery.stamp", json!({"lightDarkBalance": 25, "smoothness": smoothness})).unwrap();
                for p in image(&mut s).chunks_exact(4) {
                    assert!(p[..3] == ink || p[..3] == paper, "{depth}-bit, smoothness {smoothness}: Stamp added an intermediate color: {p:?}");
                    assert_eq!(p[3], 1.0);
                }
            }
        }
    }
}

/// Balance controls the chosen color, with the selection boundary and one-step Undo intact.
#[test]
fn stamp_balance_selection_and_undo() {
    for depth in [8, 16, 32] {
        for (balance, expected) in [(0, 1.0), (40, 0.0), (50, 0.0)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "#808080"})).unwrap();
            s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8, "antiAlias": false})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let outside = px(&mut s, 2, 2);
            s.execute("filter.gallery.stamp", json!({"lightDarkBalance": balance, "smoothness": 1})).unwrap();
            assert_eq!(px(&mut s, 8, 8), [expected, expected, expected, 1.0]);
            assert_eq!(px(&mut s, 2, 2), outside);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            s.undo();
            assert_eq!(*s.active().unwrap().doc, *before);
        }
    }
}

/// Smoothness changes spatial detail without adding intermediate colors.
#[test]
fn stamp_smoothness_changes_spatial_detail() {
    for depth in [8, 16, 32] {
        let render = |smoothness| {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "#ffffff"})).unwrap();
            s.execute("select.rect", json!({"x": 6, "y": 6, "width": 4, "height": 4, "antiAlias": false})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.execute("filter.gallery.stamp", json!({"lightDarkBalance": 25, "smoothness": smoothness})).unwrap();
            image(&mut s)
        };
        let fine = render(1);
        let smooth = render(50);
        assert_ne!(fine, smooth, "{depth}-bit: Smoothness must change the spatial result");
        for p in fine.chunks_exact(4).chain(smooth.chunks_exact(4)) {
            assert!(p[..3] == [0.0; 3] || p[..3] == [1.0; 3]);
        }
    }
}
