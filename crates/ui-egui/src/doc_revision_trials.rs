//! Count trials for what the chrome reads from the document each frame (#15709): a layer thumbnail follows its own
//! layer's pixels and nothing else, and a frame that changes nothing walks no document.

use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use egui_kittest::Harness;
use photocraft_doc::{Fill, Layer, LayerContent, LayerId, PixelFormat};
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::layers_tree::PhotoThumbs;
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

/// With the status bar, the document tabs, the Layers panel and the Move tool's snapping on screen, a frame that
/// changes nothing walks no document and rebuilds no layer index; after an edit the next idle frames walk none
/// either, so what the chrome derives is rebuilt once per change, not per frame. The panels' command states
/// (`panels::registry::states`, run once a frame) still ask some commands' preconditions, which walk; #15709 task a
/// moves those onto revisions, and until then the trial counts the walks of one such run per frame.
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
    let mut h = harness(s);
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);
    let serial = |h: &H| h.state().session.active().unwrap().doc.layers.index().serial();
    let states = |h: &H| {
        let walks = photocraft_doc::walks();
        drop(crate::panels::registry::states(h.state()));
        photocraft_doc::walks() - walks
    };
    let (walks, index) = (photocraft_doc::walks(), serial(&h));
    h.run_steps(10);
    assert_eq!((photocraft_doc::walks() - walks, serial(&h)), (10 * states(&h), index), "idle frames walk nothing outside the command states");

    h.state_mut().run("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    h.state_mut().run("edit.fill", json!({"contents": "black"})).unwrap();
    h.run_steps(4);
    let (per_frame, walks) = (states(&h), photocraft_doc::walks());
    h.run_steps(10);
    assert_eq!(photocraft_doc::walks() - walks, 10 * per_frame, "after an edit, idle frames walk nothing again");
}
