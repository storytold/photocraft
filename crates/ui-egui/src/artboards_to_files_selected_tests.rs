//! #13632: File › Export › Artboards to Files has the Export Selected Artboards checkbox. It is
//! enabled and ticked when an artboard is the active layer, greyed and unticked otherwise, and set
//! from the selection at every open. Ticked, only the selected boards are written; unticked,
//! every board is, as before.
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
use photocraft_doc::LayerId;
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const CHECKBOX: &str = "Export Selected Artboards";

/// Red Board and Blue Board of the issue's test document, and the pixel layer inside Red Board.
struct Boards {
    red: u64,
    blue: u64,
    red_layer: u64,
}

/// Adds a white artboard named `name` at `rect` ([x, y, w, h]), with a pixel layer `layer`
/// filled with `color` over `fill` ([x, y, w, h]) inside it; returns the board and layer ids.
fn board(app: &mut PhotocraftApp, name: &str, rect: [i32; 4], layer: Option<(&str, [i32; 4], &str)>) -> (u64, Option<u64>) {
    let r = app.run("layer.new.artboard", json!({"rect": rect, "name": name, "background": "white"})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let inner = layer.map(|(layer, [x, y, width, height], color)| {
        app.run("layer.select", json!({"layer": id})).unwrap();
        let l = app.run("layer.new.layer", json!({"name": layer})).unwrap()["layer"].as_u64().unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height, "antiAlias": false})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        l
    });
    (id, inner)
}

/// The app with the issue's document open.
fn harness() -> (Harness<'static, PhotocraftApp>, Boards) {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 800, "height": 400, "background": "transparent", "name": "abfiles-test"})).unwrap();
    let (red, red_layer) = board(app, "Red Board", [0, 0, 200, 150], Some(("Red", [20, 20, 100, 80], "#DC1E1E")));
    let (blue, _) = board(app, "Blue Board", [120, 30, 200, 200], Some(("Blue", [130, 40, 100, 100], "#1E1EDC")));
    let (empty, _) = board(app, "Empty Board", [600, 0, 100, 100], None);
    let red_layer = red_layer.unwrap();
    let doc = &app.session.active().unwrap().doc;
    let ids: Vec<u64> = doc.artboards().iter().map(|b| b.0.0).collect();
    assert_eq!(ids, [red, blue, empty], "the boards, bottom to top");
    assert_eq!(doc.artboard_of(LayerId(red_layer)), Some(LayerId(red)), "Red is inside Red Board");
    app.sync_views();
    (h, Boards { red, blue, red_layer })
}

/// Makes `layers` the selection, the first one active.
fn select(h: &mut Harness<'_, PhotocraftApp>, layers: &[u64]) {
    for (i, layer) in layers.iter().enumerate() {
        let mode = if i == 0 { "replace" } else { "add" };
        h.state_mut().run("layer.select", json!({"layer": layer, "mode": mode})).unwrap();
    }
    h.state_mut().sync_views();
}

/// Opens Artboards to Files from the menu, writing PNGs with the prefix `out` into `dir`.
fn open(h: &mut Harness<'_, PhotocraftApp>, dir: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("dir".into(), json!(dir));
    fields.insert("prefix".into(), json!("out"));
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

/// One written file: its name, size and the 8-bit RGBA pixels at the given points.
#[derive(Debug, PartialEq)]
struct Written {
    name: String,
    size: (u32, u32),
    pixels: Vec<[u8; 4]>,
}

/// OK: runs the export and reads back every file in `dir`, in name order, after checking that
/// they are exactly the files the command reports.
fn run(h: &mut Harness<'_, PhotocraftApp>, dialog: u64, dir: &std::path::Path, probes: &[(i32, i32)]) -> Vec<Written> {
    let before = h.state().session.active().map(|st| (st.revision, st.history.entries()));
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.active().map(|st| (st.revision, st.history.entries())), before, "the document and its History are unchanged");
    let mut reported: Vec<std::path::PathBuf> = r["files"].as_array().unwrap().iter().map(|f| std::path::PathBuf::from(f.as_str().unwrap())).collect();
    let mut on_disk: Vec<std::path::PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    reported.sort();
    on_disk.sort();
    assert_eq!(reported, on_disk, "the files written are the ones reported");
    let files = on_disk
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let doc = photocraft_io::import(&name, &std::fs::read(path).unwrap()).unwrap().document;
            let pixels = probes
                .iter()
                .filter(|(x, y)| (*x as u32) < doc.size.width && (*y as u32) < doc.size.height)
                .map(|&(x, y)| photocraft_compose::render(&doc, photocraft_geom::Rect::new(x, y, x + 1, y + 1)).px[0].map(|c| (c * 255.0).round() as u8))
                .collect();
            Written { name, size: (doc.size.width, doc.size.height), pixels }
        })
        .collect();
    for path in on_disk {
        std::fs::remove_file(path).unwrap();
    }
    files
}

/// Red Board's file: 200 x 150, white at (0, 0), the red layer at (30, 30), white at (150, 120).
fn red_board(name: String) -> Written {
    Written { name, size: (200, 150), pixels: vec![[255, 255, 255, 255], [220, 30, 30, 255], [255, 255, 255, 255]] }
}

/// Blue Board's file: 200 x 200, white at (0, 0), the blue layer at (30, 30) (130, 60 on the
/// canvas), white at (150, 120).
fn blue_board(name: String) -> Written {
    Written { name, size: (200, 200), pixels: vec![[255, 255, 255, 255], [30, 30, 220, 255], [255, 255, 255, 255]] }
}

/// Empty Board's file: 100 x 100 of its white background (the third probe is outside it).
fn empty_board(name: String) -> Written {
    Written { name, size: (100, 100), pixels: vec![[255, 255, 255, 255], [255, 255, 255, 255]] }
}

const PROBES: [(i32, i32); 3] = [(0, 0), (30, 30), (150, 120)];

/// The one file of `files` whose name holds `board` (`Red`, `Blue`, `Empty`), whatever the
/// naming scheme around it.
fn named<'a>(files: &'a [Written], board: &str) -> &'a Written {
    let hits: Vec<&Written> = files.iter().filter(|f| f.name.contains(board)).collect();
    assert_eq!(hits.len(), 1, "one file for {board}: {files:?}");
    hits[0]
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn with_red_board_active_the_ticked_checkbox_writes_only_red_board() {
    let (mut h, b) = harness();
    let dir = tmp();
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    assert_eq!(checkbox(&h), (true, true), "an artboard is active: enabled and ticked");
    assert_eq!(params(&mut h, dialog)["artboards"], json!([b.red]), "OK sends Red Board's id");
    let files = run(&mut h, dialog, dir.path(), &PROBES);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = named(&files, "Red").name.clone();
    assert!(name.starts_with("out") && name.ends_with(".png"), "{name}");
    assert_eq!(files, [red_board(name)]);
}

#[test]
fn two_selected_boards_are_both_written() {
    let (mut h, b) = harness();
    let dir = tmp();
    select(&mut h, &[b.blue, b.red]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    assert_eq!(checkbox(&h), (true, true));
    let mut sent: Vec<u64> = params(&mut h, dialog)["artboards"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    sent.sort_unstable();
    assert_eq!(sent, [b.red, b.blue], "OK sends both boards' ids");
    let files = run(&mut h, dialog, dir.path(), &PROBES);
    assert_eq!(files.len(), 2, "{files:?}");
    assert_eq!(named(&files, "Red"), &red_board(named(&files, "Red").name.clone()));
    assert_eq!(named(&files, "Blue"), &blue_board(named(&files, "Blue").name.clone()));
}

#[test]
fn unticked_writes_every_board() {
    let (mut h, b) = harness();
    let dir = tmp();
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    click(&mut h);
    assert_eq!(checkbox(&h), (true, false), "the click unticks it");
    assert!(params(&mut h, dialog)["artboards"].is_null(), "unticked, no boards are named");
    let files = run(&mut h, dialog, dir.path(), &PROBES);
    assert_eq!(files.len(), 3, "every board, the empty one included: {files:?}");
    assert_eq!(named(&files, "Red"), &red_board(named(&files, "Red").name.clone()));
    assert_eq!(named(&files, "Blue"), &blue_board(named(&files, "Blue").name.clone()));
    assert_eq!(named(&files, "Empty"), &empty_board(named(&files, "Empty").name.clone()));
    // Ticked again, the selected board is sent again.
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    click(&mut h);
    click(&mut h);
    assert_eq!((checkbox(&h), &params(&mut h, dialog)["artboards"]), ((true, true), &json!([b.red])));
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn without_an_active_artboard_the_checkbox_is_greyed_and_every_board_is_written() {
    let (mut h, b) = harness();
    let dir = tmp();
    // A layer inside a board active (not measured: today's export of every board), alone and
    // with Blue Board also selected, and no layer at all.
    for active in [vec![b.red_layer], vec![b.blue, b.red_layer], vec![]] {
        if active.is_empty() {
            h.state_mut().session.active_mut().unwrap().active_layer = None;
            h.state_mut().sync_views();
        } else {
            select(&mut h, &active);
            let st = h.state().session.active().unwrap();
            assert_eq!((st.active_layer, st.selected_layers().len()), (Some(LayerId(b.red_layer)), active.len()), "{active:?}");
        }
        let dialog = open(&mut h, &dir.path().to_string_lossy());
        assert_eq!(checkbox(&h), (false, false), "{active:?}: greyed and unticked");
        click(&mut h);
        assert_eq!(checkbox(&h), (false, false), "{active:?}: a click on the greyed checkbox does nothing");
        assert!(params(&mut h, dialog)["artboards"].is_null(), "{active:?}");
        let files = run(&mut h, dialog, dir.path(), &PROBES);
        assert_eq!(files.len(), 3, "{active:?}: {files:?}");
        assert_eq!(named(&files, "Empty"), &empty_board(named(&files, "Empty").name.clone()));
    }
}

#[test]
fn a_hidden_selected_board_keeps_every_board() {
    let (mut h, b) = harness();
    let dir = tmp();
    h.state_mut().run("layer.hideLayers", json!({"layer": b.blue})).unwrap();
    select(&mut h, &[b.red, b.blue]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    assert_eq!(checkbox(&h), (false, false), "not measured: greyed, as before");
    assert!(params(&mut h, dialog)["artboards"].is_null());
    assert_eq!(run(&mut h, dialog, dir.path(), &PROBES).len(), 3);
}

#[test]
fn the_checkbox_follows_the_selection_at_every_open() {
    let (mut h, b) = harness();
    let dir = tmp();
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    click(&mut h);
    assert_eq!(checkbox(&h), (true, false));
    assert_eq!(run(&mut h, dialog, dir.path(), &PROBES).len(), 3, "an unticked run writes every board");
    // Not remembered: the next open is ticked again, now for Blue Board.
    select(&mut h, &[b.blue]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    assert_eq!(checkbox(&h), (true, true), "ticked again after an unticked run");
    let files = run(&mut h, dialog, dir.path(), &PROBES);
    assert_eq!(files, [blue_board(named(&files, "Blue").name.clone())]);
    // And greyed again once no artboard is active.
    select(&mut h, &[b.red_layer]);
    let dialog = open(&mut h, &dir.path().to_string_lossy());
    assert_eq!(checkbox(&h), (false, false));
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn the_checkbox_sits_after_the_prefix_and_layer_comps_keeps_its_fields() {
    let (mut h, b) = harness();
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, ".");
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(crate::view_cmds::form_keys(&fields), ["dir", "prefix", "artboards", "format", "quality"]);
    let prefix = h.query_all_by_label("Prefix").next().unwrap().rect();
    let check = h.get_by_role_and_label(Role::CheckBox, CHECKBOX).rect();
    let format = h.query_all_by_label("Format").next().unwrap().rect();
    assert!(prefix.bottom() <= check.top() && check.bottom() <= format.top(), "Prefix {prefix:?}, the checkbox {check:?}, Format {format:?}");
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
    // Layer Comps to Files: its own fields, no artboards.
    h.state_mut().run("layerComp.new", json!({"name": "Comp"})).unwrap();
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, "file.export.layerCompsToFiles", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(crate::view_cmds::form_keys(&fields), ["dir", "prefix", "selectedOnly", "format", "quality"]);
    assert!(h.query_by_label(CHECKBOX).is_none(), "Layer Comps to Files has no Export Selected Artboards");
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}
