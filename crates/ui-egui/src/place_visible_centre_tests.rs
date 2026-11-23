//! File › Place Embedded / Place Linked centre the placed file on the part of the canvas in view
//! (#14103), and Place Embedded on a selection, which wins over the view (#14102). Measured in the
//! reference application on a 400 × 300 document: at 500 % with the view centred on 147.5,82 a
//! 100 × 60 file lands at 97.5,52 to 197.5,112, and a 1000 × 800 file is still fitted to the
//! canvas (375 × 300) but centred there, -40,-68 to 335,232.

use crate::canvas::testing;
use crate::{PhotocraftApp, Services, menus};
use craft::host::{Picked, ScriptedFiles};
use serde_json::json;
use std::sync::Arc;

/// A 400 × 300 document without rulers, its view at `zoom` (device pixels per document pixel)
/// centred on `center` and turned by `rotation`, shown on the canvas; the file dialog answers
/// (see [`place`]) with a gray `w` × `h` PNG on disk named `name` (a path, so Place Linked can
/// link to it).
fn placing_app(zoom: f32, center: [f32; 2], rotation: f32, name: &str, (w, h): (u32, u32)) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>, std::path::PathBuf) {
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, vec![128; (w * h * 3) as usize]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("photocraft-place-visible-centre-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    let files = Arc::new(ScriptedFiles::held());
    let services = Services { files: Some(files.clone()), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 400, "height": 300, "depth": 8})).unwrap();
    app.ui.extras.rulers = false;
    show(&mut app, zoom, center, rotation);
    (app, files, path)
}

/// Shows the view at `zoom`, centred on `center` and turned by `rotation`, on the canvas.
fn show(app: &mut PhotocraftApp, zoom: f32, center: [f32; 2], rotation: f32) {
    testing::shown(app);
    let v = &mut app.ui.views[0];
    (v.zoom, v.center, v.rotation) = (zoom, center, rotation);
    v.clear_pending();
    testing::shown(app);
    let shown = testing::view(app).expect("the canvas shows the view");
    // The canvas aligns the view to device pixels, so its centre may move by up to half of one.
    let near = |a: f64, b: f32| (a - f64::from(b)).abs() <= 0.5 / f64::from(zoom) + 1e-3;
    assert!(near(shown.center.x, center[0]) && near(shown.center.y, center[1]), "the canvas shows the view asked for: {:?}", shown.center);
}

/// The part of the canvas the view shows: `[x0, y0, x1, y1]` in document pixels.
fn visible(app: &PhotocraftApp) -> [f64; 4] {
    let r = testing::view(app).expect("the canvas shows a view").visible();
    [r.x0, r.y0, r.x1, r.y1]
}

/// Runs `command` from the menu and answers its file dialog with `path`; the placed layer's box.
fn place(app: &mut PhotocraftApp, files: &ScriptedFiles, path: &std::path::Path, command: &str) -> [i32; 4] {
    let ctx = app.ctx().clone();
    menus::invoke(app, &ctx, command, json!({})).unwrap();
    files.answer(Picked::Paths(vec![path.to_path_buf()])).expect("a file dialog is open");
    app.poll_file_dialog();
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, photocraft_doc::LayerContent::Smart(_)), "{command} placed the file");
    let b = photocraft_engine::transform_cmds::transform_bounds(&st.doc, layer);
    [b.x0, b.y0, b.x1, b.y1]
}

/// The measured view: 500 %, centred on 147.5,82 (160 × 120 document pixels in sight).
const ZOOMED: (f32, [f32; 2]) = (5.0, [147.5, 82.0]);

/// The whole canvas in view: centred on the canvas, as before (100 × 60 at 150,120).
#[test]
fn with_the_whole_canvas_in_view_a_place_centres_on_the_canvas() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        let (mut app, files, path) = placing_app(1.0, [200.0, 150.0], 0.0, "src.png", (100, 60));
        let [x0, y0, x1, y1] = visible(&app);
        assert!(x0 <= 0.0 && y0 <= 0.0 && x1 >= 400.0 && y1 >= 300.0, "the whole canvas is in view: {x0},{y0} to {x1},{y1}");
        assert_eq!(place(&mut app, &files, &path, command), [150, 120, 250, 180], "{command}");
    }
}

/// Part of the canvas in view: the box is centred there. The measured box is 97.5,52 to
/// 197.5,112; a placed image starts on a whole pixel, so it is 98 to 198 across (half a pixel
/// right, as the canvas centring of an odd-sized file already is) and exactly 52 to 112 down.
#[test]
fn with_part_of_the_canvas_in_view_a_place_centres_on_that_part() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        let (mut app, files, path) = placing_app(ZOOMED.0, ZOOMED.1, 0.0, "src.png", (100, 60));
        let [x0, y0, x1, y1] = place(&mut app, &files, &path, command);
        assert_eq!((x1 - x0, y1 - y0), (100, 60), "{command}: the size is unchanged");
        assert_eq!((y0, y1), (52, 112), "{command}");
        assert!((f64::from(x0 + x1) / 2.0 - 147.5).abs() <= 0.5, "{command}: centred on 147.5 across: {x0}..{x1}");
    }
}

/// Resize Image During Place still fits a larger file to the canvas; only the centre moves, and
/// the box may reach past the canvas edge.
#[test]
fn a_fitted_place_keeps_its_size_and_moves_its_centre() {
    let (mut app, files, path) = placing_app(ZOOMED.0, ZOOMED.1, 0.0, "big.png", (1000, 800));
    assert_eq!(place(&mut app, &files, &path, "file.placeEmbedded"), [-40, -68, 335, 232]);
}

/// A selection wins over the view (#14102): Place Embedded centres on it and removes it.
#[test]
fn a_selection_wins_over_the_view() {
    let (mut app, files, path) = placing_app(ZOOMED.0, ZOOMED.1, 0.0, "src.png", (100, 60));
    app.run("select.rect", json!({"x": 300, "y": 200, "width": 80, "height": 80})).unwrap();
    assert_eq!(place(&mut app, &files, &path, "file.placeEmbedded"), [290, 210, 390, 270]);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the place removes the selection");
}

/// The views not measured keep the canvas centre: one showing the whole canvas on one axis, and
/// a rotated one.
#[test]
fn a_view_cut_on_one_axis_or_rotated_keeps_the_canvas_centre() {
    // At 100 % moved right so the canvas's left 100 pixels are cut off; the whole 300 down shows.
    let (mut app, files, path) = placing_app(1.0, [200.0, 150.0], 0.0, "src.png", (100, 60));
    let [x0, _, x1, _] = visible(&app);
    show(&mut app, 1.0, [(100.0 + (x1 - x0) / 2.0) as f32, 150.0], 0.0);
    let [x0, y0, _, y1] = visible(&app);
    assert!(x0 > 0.0 && y0 <= 0.0 && y1 >= 300.0, "the view cuts the canvas across only: {x0},{y0} to {y1} down");
    assert_eq!(place(&mut app, &files, &path, "file.placeEmbedded"), [150, 120, 250, 180], "cut across only");

    let (mut app, files, path) = placing_app(ZOOMED.0, ZOOMED.1, 15.0, "src.png", (100, 60));
    assert_eq!(place(&mut app, &files, &path, "file.placeEmbedded"), [150, 120, 250, 180], "rotated");
}

/// Into a selected frame the image fills the frame wherever the view is: the view's centre is
/// not passed, since a frame place's `center` is in the frame's own space.
#[test]
fn a_place_into_a_frame_ignores_the_view() {
    let (mut app, files, path) = placing_app(ZOOMED.0, ZOOMED.1, 0.0, "src.png", (100, 100));
    let frame = app.run("layer.new.frame", json!({"x": 10, "y": 10, "width": 80, "height": 80})).unwrap()["layer"].as_u64().unwrap();
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    files.answer(Picked::Paths(vec![path])).expect("a file dialog is open");
    app.poll_file_dialog();
    let doc = &app.session.active().unwrap().doc;
    let image = &doc.layer(photocraft_doc::LayerId(frame)).unwrap().children().unwrap()[0];
    let photocraft_doc::LayerContent::Smart(so) = &image.content else { panic!("the frame holds the placed image") };
    let [a, _, _, d, e, f] = so.transform.m;
    let placed = [e, f, e + 100.0 * a, f + 100.0 * d].map(|v| (v * 1000.0).round() / 1000.0);
    assert_eq!(placed, [10.0, 10.0, 90.0, 90.0], "the image fills the frame");
}
