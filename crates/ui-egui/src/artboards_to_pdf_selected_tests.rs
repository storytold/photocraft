//! #13647: File › Export › Artboards to PDF has the Export Selected Artboards checkbox. It is
//! enabled and ticked when an artboard is the active layer, greyed and unticked otherwise, and set
//! from the selection at every open. Ticked, only the selected boards become pages; unticked,
//! every board does, as before.
//!
//! The document is the spec's: 800 x 400, transparent, with, bottom to top, `Red Board`
//! 0,0-200,150 (white, pixel layer `Red`), `Blue Board` 120,30-320,230 (white, pixel layer
//! `Blue`) and `Empty Board` 600,0-700,100 (white, no layers). At 72 ppi a page's MediaBox is the
//! board's size in pixels, so it tells the pages apart.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::{Value, json};

const ARTBOARDS_TO_PDF: &str = "file.export.artboardsToPdf";
const CHECKBOX: &str = "Export Selected Artboards";
const RED: &str = "/MediaBox [0 0 200.000 150.000]";
const BLUE: &str = "/MediaBox [0 0 200.000 200.000]";
const EMPTY: &str = "/MediaBox [0 0 100.000 100.000]";

struct Boards {
    red: u64,
    blue: u64,
    red_layer: u64,
}

/// Adds a white artboard `name` at `rect` ([x, y, w, h]), optionally with a pixel layer filled
/// with `color` over `fill`; returns the board's and the layer's ids.
fn board(app: &mut PhotocraftApp, name: &str, rect: [i32; 4], layer: Option<(&str, [i32; 4], &str)>) -> (u64, Option<u64>) {
    let id = app.run("layer.new.artboard", json!({"rect": rect, "name": name, "background": "white"})).unwrap()["layer"].as_u64().unwrap();
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

fn harness() -> (Harness<'static, PhotocraftApp>, Boards) {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 800, "height": 400, "background": "transparent", "name": "abpdf-test"})).unwrap();
    let (red, red_layer) = board(app, "Red Board", [0, 0, 200, 150], Some(("Red", [20, 20, 100, 80], "#DC1E1E")));
    let (blue, _) = board(app, "Blue Board", [120, 30, 200, 200], Some(("Blue", [130, 40, 100, 100], "#1E1EDC")));
    let (empty, _) = board(app, "Empty Board", [600, 0, 100, 100], None);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.artboards().iter().map(|b| b.0.0).collect::<Vec<_>>(), [red, blue, empty], "the boards, bottom to top");
    let red_layer = red_layer.unwrap();
    assert_eq!(doc.artboard_of(LayerId(red_layer)), Some(LayerId(red)));
    app.sync_views();
    (h, Boards { red, blue, red_layer })
}

/// Makes `layers` the selection; the last one added is the active layer.
fn select(h: &mut Harness<'_, PhotocraftApp>, layers: &[u64]) {
    for (i, layer) in layers.iter().enumerate() {
        let mode = if i == 0 { "replace" } else { "add" };
        h.state_mut().run("layer.select", json!({"layer": layer, "mode": mode})).unwrap();
    }
    h.state_mut().sync_views();
}

/// Opens Artboards to PDF from the menu, writing to `path`.
fn open(h: &mut Harness<'_, PhotocraftApp>, path: &std::path::Path) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS_TO_PDF, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("path".into(), json!(path.to_string_lossy()));
    h.run_steps(4);
    dialog
}

fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

/// The checkbox's (enabled, ticked) state, as the accessibility tree reports it.
fn checkbox(h: &Harness<'_, PhotocraftApp>) -> (bool, bool) {
    let node = h.get_by_role_and_label(Role::CheckBox, CHECKBOX);
    let node = node.accesskit_node();
    (!node.is_disabled(), node.toggled() == Some(Toggled::True))
}

fn click(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_role_and_label(Role::CheckBox, CHECKBOX).click();
    h.run_steps(2);
}

/// Run: exports, and returns the MediaBoxes of the PDF's pages in file order.
fn run(h: &mut Harness<'_, PhotocraftApp>, dialog: u64, path: &std::path::Path) -> Vec<&'static str> {
    let before = h.state().session.active().map(|st| (st.revision, st.history.entries()));
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.active().map(|st| (st.revision, st.history.entries())), before, "the document and its History are unchanged");
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    std::fs::remove_file(path).unwrap();
    let mut pages: Vec<(usize, &'static str)> = [RED, BLUE, EMPTY].into_iter().flat_map(|b| text.match_indices(b).map(move |(i, _)| (i, b))).collect();
    pages.sort_unstable();
    assert_eq!(r["pages"], json!(pages.len()), "{r}");
    assert_eq!(text.matches("/Type /Page ").count(), pages.len());
    pages.into_iter().map(|(_, b)| b).collect()
}

fn sorted(mut pages: Vec<&'static str>) -> Vec<&'static str> {
    pages.sort_unstable();
    pages
}

#[test]
fn with_red_board_active_the_ticked_checkbox_exports_only_red_board() {
    let (mut h, b) = harness();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("out.pdf");
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &pdf);
    assert_eq!(checkbox(&h), (true, true), "an artboard is active: enabled and ticked");
    assert_eq!(params(&mut h, dialog)["artboards"], json!([b.red]), "Run sends Red Board's id");
    assert_eq!(run(&mut h, dialog, &pdf), [RED]);
}

#[test]
fn two_selected_boards_both_become_pages() {
    let (mut h, b) = harness();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("out.pdf");
    select(&mut h, &[b.blue, b.red]);
    let dialog = open(&mut h, &pdf);
    assert_eq!(checkbox(&h), (true, true));
    let mut sent: Vec<u64> = params(&mut h, dialog)["artboards"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    sent.sort_unstable();
    assert_eq!(sent, [b.red, b.blue], "Run sends both boards' ids");
    assert_eq!(sorted(run(&mut h, dialog, &pdf)), sorted(vec![RED, BLUE]));
}

#[test]
fn unticked_exports_every_board() {
    let (mut h, b) = harness();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("out.pdf");
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &pdf);
    click(&mut h);
    assert_eq!(checkbox(&h), (true, false), "the click unticks it");
    assert!(params(&mut h, dialog)["artboards"].is_null(), "unticked, no boards are named");
    assert_eq!(sorted(run(&mut h, dialog, &pdf)), sorted(vec![RED, BLUE, EMPTY]), "every board, the empty one included");
    // Ticked again, the selected board is sent again.
    let dialog = open(&mut h, &pdf);
    click(&mut h);
    click(&mut h);
    assert_eq!((checkbox(&h), params(&mut h, dialog)["artboards"].clone()), ((true, true), json!([b.red])));
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn without_an_active_artboard_the_checkbox_is_greyed_and_every_board_is_exported() {
    let (mut h, b) = harness();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("out.pdf");
    // A layer inside a board active, alone and with Blue Board also selected, and no layer at all.
    for active in [vec![b.red_layer], vec![b.blue, b.red_layer], vec![]] {
        if active.is_empty() {
            h.state_mut().session.active_mut().unwrap().active_layer = None;
            h.state_mut().sync_views();
        } else {
            select(&mut h, &active);
            let st = h.state().session.active().unwrap();
            assert_eq!((st.active_layer, st.selected_layers().len()), (Some(LayerId(b.red_layer)), active.len()), "{active:?}");
        }
        let dialog = open(&mut h, &pdf);
        assert_eq!(checkbox(&h), (false, false), "{active:?}: greyed and unticked");
        click(&mut h);
        assert_eq!(checkbox(&h), (false, false), "{active:?}: a click on the greyed checkbox does nothing");
        assert!(params(&mut h, dialog)["artboards"].is_null(), "{active:?}");
        assert_eq!(run(&mut h, dialog, &pdf).len(), 3, "{active:?}");
    }
}

#[test]
fn the_checkbox_follows_the_selection_at_every_open() {
    let (mut h, b) = harness();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("out.pdf");
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, &pdf);
    click(&mut h);
    assert_eq!(run(&mut h, dialog, &pdf).len(), 3, "an unticked run exports every board");
    // Not remembered: the next open is ticked again, now for Blue Board.
    select(&mut h, &[b.blue]);
    let dialog = open(&mut h, &pdf);
    assert_eq!(checkbox(&h), (true, true), "ticked again after an unticked run");
    assert_eq!(run(&mut h, dialog, &pdf), [BLUE]);
    // And greyed once no artboard is active.
    select(&mut h, &[b.red_layer]);
    let dialog = open(&mut h, &pdf);
    assert_eq!(checkbox(&h), (false, false));
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}

#[test]
fn the_checkbox_sits_between_the_path_and_the_quality() {
    let (mut h, b) = harness();
    select(&mut h, &[b.red]);
    let dialog = open(&mut h, std::path::Path::new("out.pdf"));
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(crate::view_cmds::form_keys(&fields), ["path", "artboards", "quality"]);
    let path = h.query_all_by_label("Path").next().unwrap().rect();
    let check = h.get_by_role_and_label(Role::CheckBox, CHECKBOX).rect();
    let quality = h.query_all_by_label("Quality").next().unwrap().rect();
    assert!(path.bottom() <= check.top() && check.bottom() <= quality.top(), "Path {path:?}, the checkbox {check:?}, Quality {quality:?}");
    crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
}
