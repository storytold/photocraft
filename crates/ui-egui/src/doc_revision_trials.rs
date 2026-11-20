//! Count trials for what the chrome reads from the document each frame (#15709): a layer thumbnail follows its own
//! layer's pixels and nothing else, the Layers rows follow every change they show, and a frame that changes nothing
//! walks no document and rebuilds nothing derived from it.

use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::layers::ui::tree::LayerTree;
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use egui_kittest::Harness;
use photocraft_doc::{Fill, Layer, LayerContent, LayerId, PixelFormat};
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::layers_tree::{PhotoLayers, PhotoThumbs};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(4);
    h
}

/// Every thumbnail the Layers panel can ask of the active document: each layer's picture and each mask.
fn keys(app: &PhotocraftApp) -> Vec<LayerThumb<LayerId>> {
    let doc = &app.session.active().unwrap().doc;
    let mut keys = Vec::new();
    for (_, _, l) in doc.walk() {
        keys.push(LayerThumb::pixels(l.id, ThumbContents::EntireDocument));
        if l.mask.is_some() {
            keys.push(LayerThumb::mask(l.id));
        }
    }
    keys
}

fn revisions(app: &PhotocraftApp, keys: &[LayerThumb<LayerId>]) -> Vec<u64> {
    let doc = &app.session.active().unwrap().doc;
    let thumbs = PhotoThumbs::new(app, doc);
    keys.iter().map(|k| thumbs.revision(k, &ThumbRequest::for_cell(32.0, 1.0)).get()).collect()
}

/// The keys whose revision differs between `before` and `after`.
fn moved(keys: &[LayerThumb<LayerId>], before: &[u64], after: &[u64]) -> Vec<LayerThumb<LayerId>> {
    keys.iter().zip(before.iter().zip(after)).filter(|(_, (b, a))| b != a).map(|(k, _)| k.clone()).collect()
}

/// The thumbnail store renders a picture again only when its revision moves, so these relations are the renders:
/// idle frames render none, a stroke on one layer renders that layer's picture, a rename renders none, and a fill
/// edit renders the fill's picture.
#[test]
fn thumbnail_revisions_follow_only_their_own_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = |v: serde_json::Value| LayerId(v["layer"].as_u64().unwrap());
    let a = id(s.execute("layer.new.layer", json!({"name": "A"})).unwrap());
    let b = id(s.execute("layer.new.layer", json!({"name": "B"})).unwrap());
    s.execute("layer.layerMask.revealAll", json!({"layer": b.0})).unwrap();
    s.execute("layer.newFillLayer.gradient", json!({"from": "#0000ff", "to": "#00ff00", "angle": 0})).unwrap();
    let fill = s.active().unwrap().active_layer.unwrap();
    let mut h = harness(s);
    let keys = keys(h.state());
    let start = revisions(h.state(), &keys);

    h.run_steps(10);
    assert_eq!(revisions(h.state(), &keys), start, "idle frames render nothing");

    h.state_mut().run("layer.select", json!({"layer": a.0})).unwrap();
    h.state_mut().run("paint.stroke", json!({"points": [[10, 20], [190, 80]], "size": 30, "hardness": 0.5, "color": "#2060c0"})).unwrap();
    h.run_steps(2);
    let stroked = revisions(h.state(), &keys);
    assert_eq!(moved(&keys, &start, &stroked), vec![LayerThumb::pixels(a, ThumbContents::EntireDocument)], "a stroke on A renders A only");

    h.state_mut().run("layer.renameLayer", json!({"layer": b.0, "name": "Renamed"})).unwrap();
    h.run_steps(2);
    let renamed = revisions(h.state(), &keys);
    assert_eq!(moved(&keys, &stroked, &renamed), vec![], "a rename renders nothing");

    h.state_mut()
        .session
        .edit("angle", |doc, _| {
            if let Some(LayerContent::Fill(Fill::Gradient { angle, .. })) = doc.layer_mut(fill).map(|l| &mut l.content) {
                *angle = 45.0;
            }
            Ok(())
        })
        .unwrap();
    h.run_steps(2);
    assert_eq!(
        moved(&keys, &renamed, &revisions(h.state(), &keys)),
        vec![LayerThumb::pixels(fill, ThumbContents::EntireDocument)],
        "a fill edit renders the fill"
    );
}

/// The Layers panel walks its rows again only when their revision moves: a rename and both disclosure toggles (a
/// layer's effects list, a group) move it, idle frames don't.
#[test]
fn layers_rows_follow_renames_and_both_toggles() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerStyle.dropShadow", json!({"layer": a.0})).unwrap();
    let group = Layer::group("G", vec![Layer::raster("inner", PixelFormat::RGBA8)]);
    let g = group.id;
    s.edit("group", |doc, _| {
        doc.layers.push(group);
        Ok(())
    })
    .unwrap();
    let mut h = harness(s);
    let revision = |h: &H| {
        let app = h.state();
        PhotoLayers::new(app, app.session.active().unwrap(), &Default::default(), false, None).revision()
    };
    let start = revision(&h);
    assert!(start.is_some());
    h.run_steps(10);
    assert_eq!(revision(&h), start, "idle frames keep the rows");

    let moves = |h: &mut H, command: &str, params: serde_json::Value| {
        let before = revision(h);
        h.state_mut().run(command, params).unwrap();
        h.run_steps(2);
        assert_ne!(revision(h), before, "{command} moves the rows' revision");
    };
    moves(&mut h, "layer.renameLayer", json!({"layer": a.0, "name": "Renamed"}));
    moves(&mut h, "layer.setEffectsExpanded", json!({"layer": a.0, "expanded": false}));
    moves(&mut h, "layer.setExpanded", json!({"layer": g.0, "expanded": false}));
}

/// With the status bar, the document tabs, the Layers and History panels, an artboard's pasteboard and the Move
/// tool's snapping on screen, a frame that changes nothing walks no document, rebuilds no layer index and computes
/// no derived value (title, status, pages, uncovered work, moving layers, History rows); after an edit they are
/// computed again once, and the next idle frames compute none.
#[test]
fn idle_frames_walk_no_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.edit("stack", |doc, _| {
        let children: Vec<Layer> = (0..40).map(|i| Layer::raster(format!("inner {i}"), PixelFormat::RGBA8)).collect();
        doc.layers.push(Layer::group("group", children));
        for i in 0..160 {
            doc.layers.push(Layer::raster(format!("layer {i}"), PixelFormat::RGBA8));
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 200, 150]})).unwrap();
    let mut h = harness(s);
    h.state_mut().run("window.toggle.history", json!({"show": true})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);
    let serial = |h: &H| h.state().session.active().unwrap().doc.layers.index().serial();
    let builds = |h: &H| h.state().doc_queries.builds();
    let (walks, index, built) = (photocraft_doc::walks(), serial(&h), builds(&h));
    h.run_steps(10);
    assert_eq!((photocraft_doc::walks() - walks, serial(&h)), (0, index), "idle frames walk nothing");
    assert_eq!(builds(&h), built, "idle frames compute nothing");

    // The new artboard is the active layer; the fill goes on a pixel layer.
    let pixels = h.state().session.active().unwrap().doc.layers.iter().find(|l| l.name == "layer 0").unwrap().id;
    h.state_mut().run("layer.select", json!({"layer": pixels.0})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    h.state_mut().run("edit.fill", json!({"contents": "black"})).unwrap();
    h.run_steps(4);
    assert!(builds(&h) > built, "an edit computes the values again");
    let (walks, built) = (photocraft_doc::walks(), builds(&h));
    h.run_steps(10);
    assert_eq!(photocraft_doc::walks() - walks, 0, "after an edit, idle frames walk nothing again");
    assert_eq!(builds(&h), built, "after an edit, idle frames compute nothing again");
}
