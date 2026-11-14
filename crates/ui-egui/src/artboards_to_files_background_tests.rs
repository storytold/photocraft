//! #13631: File › Export › Artboards to Files has the Include Background in Export checkbox,
//! ticked by default, right before the file type. Ticked, the files keep each board's white fill
//! (a PNG is RGB); unticked, a PNG is RGBA and transparent wherever the board's layers have no
//! pixels, while an empty board and formats without alpha keep today's file.
//!
//! The document is the issue's: 800 x 400, RGB, 8 bit, transparent, with, bottom to top, `Red
//! Board` 0,0-200,150 (white, pixel layer `Red` 220,30,30 at 20,20-120,100), `Blue Board`
//! 120,30-320,230 (white, pixel layer `Blue` 30,30,220 at 130,40-230,140) and `Empty Board`
//! 600,0-700,100 (white, no layers).
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const CHECKBOX: &str = "Include Background in Export";
const RED: [u8; 4] = [220, 30, 30, 255];
const BLUE: [u8; 4] = [30, 30, 220, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// The app with the issue's document open; no layer is selected.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 800, "height": 400, "background": "transparent", "name": "abfiles-test"})).unwrap();
    for (name, rect, layer) in [
        ("Red Board", [0, 0, 200, 150], Some(("Red", [20, 20, 100, 80], "#DC1E1E"))),
        ("Blue Board", [120, 30, 200, 200], Some(("Blue", [130, 40, 100, 100], "#1E1EDC"))),
        ("Empty Board", [600, 0, 100, 100], None),
    ] {
        let id = app.run("layer.new.artboard", json!({"rect": rect, "name": name, "background": "white"})).unwrap()["layer"].clone();
        if let Some((layer, [x, y, width, height], color)) = layer {
            app.run("layer.select", json!({"layer": id})).unwrap();
            app.run("layer.new.layer", json!({"name": layer})).unwrap();
            app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height, "antiAlias": false})).unwrap();
            app.run("edit.fill", json!({"color": color})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
        }
    }
    app.session.active_mut().unwrap().active_layer = None;
    app.sync_views();
    h
}

/// Opens Artboards to Files from the menu, writing `format` files with the prefix `out` into `dir`.
fn open(h: &mut Harness<'_, PhotocraftApp>, dir: &str, format: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("dir".into(), json!(dir));
    fields.insert("prefix".into(), json!("out"));
    fields.insert("format".into(), json!(format));
    h.run_steps(4);
    dialog
}

/// The params OK sends.
fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

/// The checkbox's (enabled, ticked) state as the accessibility tree reports it.
fn checkbox(h: &Harness<'_, PhotocraftApp>) -> (bool, bool) {
    let node = h.get_by_role_and_label(Role::CheckBox, CHECKBOX);
    let node = node.accesskit_node();
    (!node.is_disabled(), node.toggled() == Some(Toggled::True))
}

/// Clicks the checkbox, as a user does.
fn click(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_role_and_label(Role::CheckBox, CHECKBOX).click();
    h.run_steps(2);
}

/// One written file: its bytes, size and every pixel as 8-bit RGBA, row by row.
struct Written {
    bytes: Vec<u8>,
    size: (u32, u32),
    px: Vec<[u8; 4]>,
}

impl Written {
    /// The pixel at (`x`, `y`).
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        self.px[(y * self.size.0 + x) as usize]
    }

    /// A PNG's IHDR colour type: 2 is RGB, 6 is RGBA.
    fn png_colour_type(&self) -> u8 {
        assert_eq!(&self.bytes[12..16], b"IHDR");
        self.bytes[25]
    }

    /// Asserts every pixel: `ink` inside `rect` ([x0, y0, x1, y1]), elsewhere exactly
    /// `outside`, or fully transparent when `outside` is `None`.
    fn assert_pixels(&self, rect: [u32; 4], ink: [u8; 4], outside: Option<[u8; 4]>) {
        let [x0, y0, x1, y1] = rect;
        for (i, px) in self.px.iter().enumerate() {
            let (x, y) = (i as u32 % self.size.0, i as u32 / self.size.0);
            match outside {
                _ if (x0..x1).contains(&x) && (y0..y1).contains(&y) => assert_eq!(*px, ink, "({x}, {y})"),
                Some(outside) => assert_eq!(*px, outside, "({x}, {y})"),
                None => assert_eq!(px[3], 0, "({x}, {y}) is transparent: {px:?}"),
            }
        }
    }
}

/// OK: runs the export and reads back the files for Red, Blue and Empty Board, after checking
/// that they are exactly the files the command reports and that the document is unchanged.
fn run(h: &mut Harness<'_, PhotocraftApp>, dialog: u64, dir: &std::path::Path) -> [Written; 3] {
    let before = h.state().session.active().map(|st| (st.revision, st.history.entries()));
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.active().map(|st| (st.revision, st.history.entries())), before, "the document and its History are unchanged");
    let mut reported: Vec<std::path::PathBuf> = r["files"].as_array().unwrap().iter().map(|f| std::path::PathBuf::from(f.as_str().unwrap())).collect();
    let mut on_disk: Vec<std::path::PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    reported.sort();
    on_disk.sort();
    assert_eq!(reported, on_disk, "the files written are the ones reported");
    let read = |board: &str| {
        let path = on_disk.iter().find(|p| p.file_name().unwrap().to_string_lossy().contains(board)).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let doc = photocraft_io::import(&path.to_string_lossy(), &bytes).unwrap().document;
        let px = photocraft_compose::flatten(&doc).px.iter().map(|px| px.map(|c| (c * 255.0).round() as u8)).collect();
        Written { bytes, size: (doc.size.width, doc.size.height), px }
    };
    let files = [read("Red"), read("Blue"), read("Empty")];
    assert_eq!(on_disk.len(), 3, "{on_disk:?}");
    for path in on_disk {
        std::fs::remove_file(path).unwrap();
    }
    files
}

#[test]
fn the_checkbox_is_ticked_right_before_the_file_type_and_layer_comps_has_none() {
    let mut h = harness();
    let dialog = open(&mut h, ".", "png");
    assert_eq!(checkbox(&h), (true, true), "enabled and ticked");
    assert_eq!(params(&mut h, dialog)["includeBackground"], json!(true));
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    let keys = crate::view_cmds::form_keys(&fields);
    let at = |key: &str| keys.iter().position(|k| k == key).unwrap();
    assert!(at("prefix") < at("includeBackground") && at("includeBackground") + 1 == at("format"), "{keys:?}");
    let prefix = h.query_all_by_label("Prefix").next().unwrap().rect();
    let check = h.get_by_role_and_label(Role::CheckBox, CHECKBOX).rect();
    let format = h.query_all_by_label("Format").next().unwrap().rect();
    assert!(prefix.bottom() <= check.top() && check.bottom() <= format.top(), "Prefix {prefix:?}, the checkbox {check:?}, Format {format:?}");
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
    // Layer Comps to Files shares the branch but keeps its own fields.
    h.state_mut().run("layerComp.new", json!({"name": "Comp"})).unwrap();
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, "file.export.layerCompsToFiles", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(crate::view_cmds::form_keys(&fields), ["dir", "prefix", "selectedOnly", "format", "quality"]);
    assert!(h.query_by_label(CHECKBOX).is_none(), "Layer Comps to Files has no Include Background in Export");
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn ticked_writes_rgb_pngs_with_the_white_fill() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, &dir.path().to_string_lossy(), "png");
    let [red, blue, empty] = run(&mut h, dialog, dir.path());
    assert_eq!((red.size, red.png_colour_type(), red.at(0, 0), red.at(30, 30)), ((200, 150), 2, WHITE, RED), "RGB, white kept");
    red.assert_pixels([20, 20, 120, 100], RED, Some(WHITE));
    assert_eq!((blue.size, blue.png_colour_type()), ((200, 200), 2));
    blue.assert_pixels([10, 10, 110, 110], BLUE, Some(WHITE));
    assert_eq!((empty.size, empty.png_colour_type()), ((100, 100), 2));
    assert!(empty.px.iter().all(|px| *px == WHITE));
}

#[test]
fn unticked_writes_rgba_pngs_transparent_outside_the_layers() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, &dir.path().to_string_lossy(), "png");
    click(&mut h);
    assert_eq!(checkbox(&h), (true, false), "the click unticks it");
    assert_eq!(params(&mut h, dialog)["includeBackground"], json!(false));
    let [red, blue, empty] = run(&mut h, dialog, dir.path());
    assert_eq!((red.size, red.png_colour_type(), red.at(0, 0)[3], red.at(30, 30)), ((200, 150), 6, 0, RED), "RGBA, the white left out");
    red.assert_pixels([20, 20, 120, 100], RED, None);
    // Blue's layer, 130,40 on the canvas, is at 10,10 in its board.
    assert_eq!((blue.size, blue.png_colour_type()), ((200, 200), 6));
    blue.assert_pixels([10, 10, 110, 110], BLUE, None);
    // Not measured: the empty board keeps today's 100 x 100 white file.
    assert_eq!((empty.size, empty.png_colour_type()), ((100, 100), 2));
    assert!(empty.px.iter().all(|px| *px == WHITE));
}

#[test]
fn unticked_still_writes_flattened_jpeg_and_bmp_files() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    for format in ["bmp", "jpg"] {
        let dialog = open(&mut h, &dir.path().to_string_lossy(), format);
        let ticked = run(&mut h, dialog, dir.path());
        let dialog = open(&mut h, &dir.path().to_string_lossy(), format);
        click(&mut h);
        assert_eq!(params(&mut h, dialog)["includeBackground"], json!(false), "{format}");
        let unticked = run(&mut h, dialog, dir.path());
        for (ticked, unticked) in ticked.iter().zip(&unticked) {
            assert!(unticked.bytes == ticked.bytes, "{format}: unticked writes today's file");
            // JPEG is lossy: the red layer's colour bleeds a little into the corner's 8 x 8 block.
            let corner = unticked.at(0, 0);
            assert!(corner[3] == 255 && corner.iter().all(|c| *c >= 240), "{format}: flattened on the board's white, not transparent: {corner:?}");
            if format == "bmp" {
                assert_eq!(corner, WHITE);
            }
        }
        assert_eq!(unticked.each_ref().map(|f| f.size), [(200, 150), (200, 200), (100, 100)], "{format}");
        if format == "bmp" {
            unticked[0].assert_pixels([20, 20, 120, 100], RED, Some(WHITE));
        }
    }
}
