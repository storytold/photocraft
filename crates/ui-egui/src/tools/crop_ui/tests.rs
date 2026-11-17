use super as crop_ui;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::Rect;
use craft::draw::ui::crop::{CropFill, CropLength, CropRatio, CropSettings, CropTool, ResolutionUnit};
use craft::ui::runtime::env::UnitKind;
use egui::Modifiers;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;

const NONE: Modifiers = Modifiers::NONE;

fn app(depth: SampleType) -> PhotocraftApp {
    let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, depth, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Crop;
    // These tests pin Classic Mode (the frame moves and turns over the image).
    app.ui.tool_options.crop.shield.classic_mode = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

/// The crop tool the tests set frames on.
fn tool(app: &mut PhotocraftApp) -> &mut CropTool {
    &mut app.canvas_tools.crop.tool
}

fn boxed(x0: f64, y0: f64, x1: f64, y1: f64, angle: f64) -> CropBox {
    CropBox::new(Rect::new(x0, y0, x1, y1), angle)
}

fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], mods: Modifiers) {
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    // The frame's handles take presses from the pass that drew them: a frame set since the last pass is shown first.
    crate::canvas::testing::frame(app);
    // The pointer is over a handle before it presses it.
    tool_event(app, ToolEvent::Move { x: first[0], y: first[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, mods);
    for p in &pts[1..] {
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
}

fn doc_size(app: &PhotocraftApp) -> Size {
    app.session.active().unwrap().doc.size
}

/// One commit of `crop_ui::apply`: the frame, the options and the document they act on, and the
/// document that results (`None`: the crop is refused).
struct Row {
    name: &'static str,
    depth: SampleType,
    selection: Option<[u32; 4]>,
    frame: Option<CropBox>,
    settings: fn(&mut CropSettings),
    expect: Option<(Size, f32)>,
}

fn size_target(s: &mut CropSettings, w: f64, h: f64, resolution: &str) {
    s.ratio = CropRatio::Size;
    s.size.width = Some(CropLength::new(w, UnitKind::Inches));
    s.size.height = Some(CropLength::new(h, UnitKind::Inches));
    s.size.resolution = resolution.into();
    s.size.resolution_unit = ResolutionUnit::PixelsPerInch;
}

#[test]
fn commit_maps_the_frame_and_options_to_image_crop() {
    let plain = boxed(40.0, 30.0, 100.0, 70.0, 0.0);
    let turned = boxed(40.0, 30.0, 100.0, 70.0, 90.0);
    let rows = [
        Row { name: "plain 8-bit", depth: SampleType::U8, selection: None, frame: Some(plain), settings: |_| {}, expect: Some((Size::new(60, 40), 72.0)) },
        Row { name: "turned 8-bit", depth: SampleType::U8, selection: None, frame: Some(turned), settings: |_| {}, expect: Some((Size::new(60, 40), 72.0)) },
        Row { name: "turned 16-bit", depth: SampleType::U16, selection: None, frame: Some(turned), settings: |_| {}, expect: Some((Size::new(60, 40), 72.0)) },
        Row { name: "turned 32-bit", depth: SampleType::F32, selection: None, frame: Some(turned), settings: |_| {}, expect: Some((Size::new(60, 40), 72.0)) },
        Row {
            name: "moved 16-bit",
            depth: SampleType::U16,
            selection: None,
            frame: Some(boxed(20.0, 10.0, 120.0, 60.0, 0.0)),
            settings: |_| {},
            expect: Some((Size::new(100, 50), 72.0)),
        },
        Row {
            name: "content-aware fill past the canvas",
            depth: SampleType::U8,
            selection: None,
            frame: Some(boxed(-20.0, -10.0, 120.0, 60.0, 0.0)),
            settings: |s| s.fill = CropFill::ContentAware,
            expect: Some((Size::new(140, 70), 72.0)),
        },
        Row {
            name: "W x H x Resolution",
            depth: SampleType::U16,
            selection: None,
            frame: Some(boxed(10.0, 10.0, 70.0, 85.0, 0.0)),
            settings: |s| size_target(s, 4.0, 5.0, "300"),
            expect: Some((Size::new(1200, 1500), 300.0)),
        },
        Row {
            name: "W x H without resolution keeps the document's",
            depth: SampleType::U8,
            selection: None,
            frame: Some(boxed(0.0, 0.0, 100.0, 50.0, 0.0)),
            settings: |s| size_target(s, 2.0, 1.0, ""),
            expect: Some((Size::new(144, 72), 72.0)),
        },
        Row {
            name: "untouched selection frame",
            depth: SampleType::U16,
            selection: Some([10, 20, 30, 40]),
            frame: None,
            settings: |_| {},
            expect: Some((Size::new(30, 40), 72.0)),
        },
        Row {
            name: "refused at 20000 ppi",
            depth: SampleType::U8,
            selection: None,
            frame: Some(plain),
            settings: |s| size_target(s, 4.0, 2.0, "20000"),
            expect: None,
        },
    ];
    for row in rows {
        let mut app = app(row.depth);
        (row.settings)(&mut app.ui.tool_options.crop);
        if let Some([x, y, w, h]) = row.selection {
            app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
            crop_ui::each_pass(&mut app);
        }
        if let Some(frame) = row.frame {
            tool(&mut app).crop_to(frame);
        } else {
            // The default frame (the selection's bounds) is made by the tool's first pass.
            crate::canvas::testing::frame(&mut app);
        }
        let before = (crop_ui::frame(&app), doc_size(&app));
        // The engine names a crop that filled new area after the fill.
        let canvas = Rect::new(0.0, 0.0, f64::from(before.1.width), f64::from(before.1.height));
        let fills = app.ui.tool_options.crop.fill == CropFill::ContentAware && row.frame.is_some_and(|f| f.rect != f.rect.intersect(canvas));
        let label = if fills { "Content-aware Crop" } else { "Crop" };
        let applied = crop_ui::apply(&mut app);
        let st = app.session.active().unwrap();
        match row.expect {
            Some((size, dpi)) => {
                assert!(applied, "{}", row.name);
                assert_eq!((st.doc.size, st.doc.resolution_dpi), (size, dpi), "{}", row.name);
                assert_eq!(st.doc.pixel_format().sample, row.depth, "{}", row.name);
                assert_eq!(st.history.undo_label(), Some(label), "{}", row.name);
            }
            None => {
                assert!(!applied, "{}", row.name);
                assert_eq!(st.doc.size, before.1, "{}: refused, the document stays", row.name);
                assert_eq!(crop_ui::frame(&app), before.0, "{}: the frame stays", row.name);
                assert_eq!(crop_ui::is_enabled(&app, "edit.undo"), Some(true), "{}: the box Undo stays", row.name);
            }
        }
    }
}

/// #3145: Edit › Undo while a crop is pending steps the box, not the document, and greys out when
/// the box has no step left in that direction.
#[test]
fn edit_undo_steps_the_crop_box_not_the_document() {
    let mut app = app(SampleType::U8);
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    // A library change older than the box: Edit › Undo steps the box and leaves the library edit to undo.
    app.run("actions.reset", json!({"confirm": true, "mode": "append"})).unwrap();
    let before = boxed(40.0, 30.0, 100.0, 70.0, 0.0);
    tool(&mut app).crop_to(before);
    let doc_steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, &[[70.0, 50.0], [90.0, 50.0]], NONE); // move the frame right 20
    let moved = crop_ui::frame(&app).unwrap();
    assert!((moved.rect.x0 - 60.0).abs() < 1e-3, "{moved:?}");
    assert_eq!(crop_ui::is_enabled(&app, "edit.undo"), Some(true));
    assert_eq!(crop_ui::is_enabled(&app, "edit.redo"), Some(false), "no box step to redo");
    let ctx = crate::canvas::testing::ctx(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
    assert!(app.session.actions.list.library_undo().is_some(), "the library edit remains available");
    assert_eq!(crop_ui::frame(&app), Some(before), "undo restores the box");
    assert_eq!(app.session.active().unwrap().history.past_len(), doc_steps, "the document is untouched");
    app.run("edit.redo", json!({})).unwrap();
    assert!((crop_ui::frame(&app).unwrap().rect.x0 - 60.0).abs() < 1e-3, "redo re-applies the move");
    // Both steps (Crop To, the move) back: nothing left to undo, and the document's Undo is not offered.
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(crop_ui::is_enabled(&app, "edit.undo"), Some(false), "no box step left to undo");
    assert_eq!(crop_ui::is_enabled(&app, "edit.redo"), Some(true));
    assert_eq!(app.session.active().unwrap().history.past_len(), doc_steps);
}

/// A History jump under a drawn frame would resize the document while the frame keeps its
/// coordinates, so Enter cropped the old ones (#9285): the panel's rows, snapshots and menu
/// wait for the crop, as they do for Free Transform.
#[test]
fn history_jumps_wait_for_a_pending_crop() {
    let size = |app: &PhotocraftApp| app.session.active().map(|st| (st.doc.size.width, st.doc.size.height));
    let mut app = app(SampleType::U8);
    app.run("history.newSnapshot", json!({})).unwrap();
    app.run("image.canvasSize", json!({"width": 300, "height": 200})).unwrap();
    let snapshot = app.session.active().unwrap().snapshots[0].id;
    let frame = boxed(220.0, 120.0, 290.0, 190.0, 0.0);
    // A pass first: the tool holds a frame for the document it was made for, which Discard drops.
    crate::canvas::testing::frame(&mut app);
    tool(&mut app).crop_to(frame);
    for (cmd, p) in [
        ("history.selectState", json!({"state": 0})),
        ("history.selectSnapshot", json!({"snapshot": snapshot})),
        ("history.stepBackward", json!({})),
        ("history.deleteState", json!({})),
    ] {
        assert!(app.run(cmd, p).is_err(), "{cmd} waits for the crop");
        assert_eq!(size(&app), Some((300, 200)), "{cmd} left the document as it was");
    }
    assert_eq!(crop_ui::frame(&app), Some(frame), "the frame stays");
    // Without a pending frame (the tool's own frame around the canvas) History works as before.
    crop_ui::discard(&mut app);
    app.run("history.selectState", json!({"state": 0})).unwrap();
    assert_eq!(size(&app), Some((200, 100)));
    // Step Forward has a state to redo now; a pending frame holds it back too.
    crate::canvas::testing::frame(&mut app);
    tool(&mut app).crop_to(boxed(10.0, 10.0, 50.0, 50.0, 0.0));
    assert!(app.run("history.stepForward", json!({})).is_err(), "history.stepForward waits for the crop");
    assert_eq!(size(&app), Some((200, 100)));
    crop_ui::discard(&mut app);
    app.run("history.stepForward", json!({})).unwrap();
    assert_eq!(size(&app), Some((300, 200)));
}

/// Space is the temporary Hand only while no frame is being drawn: `crop_ui::dragging` is the
/// state the held-tool rule reads, true from the press to the release.
#[test]
fn space_on_the_canvas_moves_the_frame_not_the_view() {
    let mut app = app(SampleType::U8);
    assert!(!crop_ui::dragging(&app));
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, NONE);
    assert!(crop_ui::dragging(&app), "a frame is being drawn: Space repositions it");
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 40.0 }, NONE);
    assert!(!crop_ui::dragging(&app), "released: Space pans again");
    assert_eq!(crop_ui::frame(&app), Some(boxed(20.0, 20.0, 60.0, 40.0, 0.0)));
}

/// #1918: as in Photoshop, a pending crop (not the untouched default frame) greys almost every
/// menu command; the ones Photoshop keeps stay available, and commit or cancel frees the rest.
#[test]
fn a_pending_crop_greys_the_menus_photoshop_greys() {
    let mut app = app(SampleType::U8);
    crop_ui::each_pass(&mut app);
    assert!(!crop_ui::pending(&app));
    let ids: Vec<&str> = crate::menu_catalog::CATALOG.iter().map(|e| e.3).filter(|id| *id != "---").collect();
    for id in &ids {
        assert!(crop_ui::available_while_pending(id) || !crop_ui::pending(&app), "default frame: {id} stays as it was");
    }
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    assert!(crop_ui::pending(&app));
    let allows = |app: &PhotocraftApp, id: &str| crate::menus::allows(app, &craft::keymap::CommandRef::new(id));
    let kept = [
        "file.close",
        "file.save",
        "file.saveAs",
        "file.saveACopy",
        "edit.undo",
        "edit.redo",
        "edit.toggleLastState",
        "edit.search",
        "image.crop",
        "view.proofColors",
        "view.gamutWarning",
        "view.zoomIn",
        "view.zoomOut",
        "view.fitOnScreen",
        "view.actualPixels",
        "view.twoHundredPercent",
        "view.printSize",
        "view.extras",
        "view.show.grid",
        "view.rulers",
        "view.snap",
        "view.snapTo.guides",
        "view.lockSlices",
        "view.newGuide",
        "window.arrange.tile",
        "window.panel.layers",
        "help.about",
    ];
    let greyed = [
        "file.new",
        "file.open",
        "file.openAs",
        "file.openExrParts",
        "file.closeAll",
        "file.revert",
        "file.export.exportAs",
        "file.placeEmbedded",
        "file.automate.batch",
        "file.scripts.browse",
        "file.import.notes",
        "file.fileInfo",
        "file.print",
        "edit.cut",
        "edit.fill",
        "edit.freeTransform",
        "edit.colorSettings",
        "edit.preferences.general",
        "image.imageSize",
        "image.mode.grayscale",
        "layer.new.layer",
        "select.all",
        "filter.blur.gaussianBlur",
        "view.proofSetup.workingCmyk",
        "view.pixelAspectRatio.square",
        "view.pixelAspectRatioCorrection",
        "view.thirtyTwoBitPreviewOptions",
        "view.flipHorizontal",
        "view.screenMode.fullScreen",
        "window.workspace.essentials",
        "window.panel.adjustments",
        "help.systemInfo",
    ];
    for id in kept {
        assert!(allows(&app, id), "{id} stays available");
    }
    for id in greyed {
        assert!(!allows(&app, id), "{id} is greyed");
    }
    assert!(
        ids.iter()
            .filter(|id| id.starts_with("layer.") || id.starts_with("type.") || id.starts_with("select.") || id.starts_with("filter."))
            .all(|id| !allows(&app, id))
    );
    // Committing still runs `image.crop` itself, and frees the menus again.
    assert!(crop_ui::apply(&mut app));
    assert_eq!(doc_size(&app), Size::new(40, 30));
    assert!(!crop_ui::pending(&app) && allows(&app, "file.new"));
    // Cancelling does too.
    drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
    assert!(!allows(&app, "file.new"));
    crop_ui::discard(&mut app);
    assert!(allows(&app, "file.new"));
    // Another tool: nothing is pending.
    drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
    app.ui.tool = Tool::Brush;
    crop_ui::each_pass(&mut app);
    assert!(allows(&app, "file.new"));
}

/// #3368: picking another tool with a drawn frame crops silently; an untouched default frame is
/// dropped and crops nothing.
#[test]
fn another_tool_commits_a_drawn_frame_at_8_and_16_bit() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut app = app(depth);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert!(crop_ui::pending(&app));
        app.ui.tool = Tool::Move;
        crop_ui::each_pass(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size, st.doc.depth), (Size::new(40, 30), depth));
        assert_eq!(st.history.undo_label(), Some("Crop"));
        assert_eq!(app.ui.tool, Tool::Move);
        assert_eq!(crop_ui::frame(&app), None);
    }
    let mut app = app(SampleType::U8);
    // The host's pass makes the default frame.
    crate::canvas::testing::frame(&mut app);
    assert!(crop_ui::frame(&app).is_some() && !crop_ui::pending(&app));
    let steps = app.session.active().unwrap().history.past_len();
    app.ui.tool = Tool::Move;
    crop_ui::each_pass(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), steps), "an untouched frame crops nothing");
    assert_eq!(crop_ui::frame(&app), None);
}

/// A layer 300..350 × 200..260 on a 400 × 300 canvas: the snap target of the two snap trials.
fn board() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 1.0;
    app.ui.views[0].clear_pending();
    app.run("layer.new.layer", json!({"name": "target"})).unwrap();
    app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Crop;
    app.ui.extras.snap = true;
    app.ui.tool_options.crop.shield.auto_center_preview = false;
    app
}

/// The app's snap targets reach the crop overlay: a new frame's corner snaps to a layer edge in
/// both crop modes, and the untouched default frame never snaps.
#[test]
fn drawing_a_new_crop_snaps_its_corner_not_the_default_frame() {
    for classic in [true, false] {
        let mut app = board();
        app.ui.tool_options.crop.shield.classic_mode = classic;
        crop_ui::each_pass(&mut app);
        crate::canvas::testing::frame(&mut app);
        let default = crop_ui::frame(&app);
        // A click without a drag changes nothing.
        drag(&mut app, &[[297.0, 150.0], [297.0, 150.0]], NONE);
        assert_eq!(crop_ui::frame(&app), default, "classic {classic}: a click leaves the default frame");
        assert!(!crop_ui::pending(&app), "classic {classic}");
        // The corner 3 px short of the layer's left edge snaps to it.
        drag(&mut app, &[[50.0, 50.0], [200.0, 100.0], [297.0, 150.0]], NONE);
        let frame = crop_ui::frame(&app).unwrap();
        assert!((frame.rect.x1 - 300.0).abs() < 1e-6, "classic {classic}: {frame:?}");
    }
}
