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
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
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

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with the issue's document open; no layer is selected.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
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
    app.session.active_mut().unwrap().set_active_layer(None);
    app.sync_views();
    let h = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Sets a field as automation (`ui.dialog.set`) does, then draws.
fn set(h: &mut H, dialog: DialogId, key: &str, v: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, key, v).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(4);
}

/// Opens `command` from the menu.
fn open_command(h: &mut H, command: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    dialog
}

/// Opens Artboards to Files from the menu, writing `format` files with the prefix `out` into
/// `dir`, Artboard Content Only (Red and Blue Board overlap, and Include Overlapping Areas would
/// put Blue Board's own white into Red Board's file). The form remembers the checkbox between
/// openings, so a dialog reopened after an unticked run is ticked again here.
fn open(h: &mut H, dir: &str, format: &str) -> DialogId {
    let dialog = open_command(h, ARTBOARDS);
    for (key, value) in [("dir", json!(dir)), ("prefix", json!("out")), ("format", json!(format)), ("content", json!("artboard"))] {
        set(h, dialog, key, value);
    }
    if !checkbox(h).1 {
        set(h, dialog, "includeBackground", json!(true));
    }
    dialog
}

/// The params OK sends.
fn params(h: &H, dialog: DialogId) -> Value {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap();
    crate::filter_dialog::command_params(ARTBOARDS, form.fields())
}

/// The checkbox's (enabled, ticked) state as the accessibility tree reports it.
fn checkbox(h: &H) -> (bool, bool) {
    let node = h.get_by_role_and_label(Role::CheckBox, CHECKBOX);
    let node = node.accesskit_node();
    (!node.is_disabled(), node.toggled() == Some(Toggled::True))
}

/// Clicks the checkbox, as a user does.
fn click(h: &mut H) {
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
/// that the document is unchanged.
fn run(h: &mut H, dialog: DialogId, dir: &std::path::Path) -> [Written; 3] {
    let before = h.state().session.active().map(|st| (st.revision, st.history.entries()));
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().map(|st| (st.revision, st.history.entries())), before, "the document and its History are unchanged");
    let mut on_disk: Vec<std::path::PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    on_disk.sort();
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
    let dialog = open_command(&mut h, ARTBOARDS);
    assert_eq!(checkbox(&h), (true, true), "enabled and ticked");
    assert_eq!(params(&h, dialog)["includeBackground"], json!(true));
    let prefix = h.query_all_by_label("File Name Prefix").next().unwrap().rect();
    let check = h.get_by_role_and_label(Role::CheckBox, CHECKBOX).rect();
    let format = h.query_all_by_label("File Type").next().unwrap().rect();
    assert!(prefix.bottom() <= check.top() && check.bottom() <= format.top(), "Prefix {prefix:?}, the checkbox {check:?}, File Type {format:?}");
    crate::dialogs::testing::cancel(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    // Layer Comps to Files has no such checkbox.
    h.state_mut().run("layerComp.new", json!({"name": "Comp"})).unwrap();
    let dialog = open_command(&mut h, "file.export.layerCompsToFiles");
    assert!(h.query_by_label(CHECKBOX).is_none(), "Layer Comps to Files has no Include Background in Export");
    crate::dialogs::testing::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn ticked_writes_rgb_pngs_with_the_white_fill() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, &dir.path().to_string_lossy(), "png24");
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
    let dialog = open(&mut h, &dir.path().to_string_lossy(), "png24");
    click(&mut h);
    assert_eq!(checkbox(&h), (true, false), "the click unticks it");
    assert_eq!(params(&h, dialog)["includeBackground"], json!(false));
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
        assert_eq!(params(&h, dialog)["includeBackground"], json!(false), "{format}");
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
