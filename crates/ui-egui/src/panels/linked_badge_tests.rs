//! #14118: in the Layers panel a linked smart object's thumbnail carries a chain-link badge in place of the smart
//! object badge, and a red missing-link badge while its file is not there; the plain badge comes back by itself once
//! the file is back. An embedded smart object keeps its badge. craft-layers paints the badges (`ThumbBadge::Linked`,
//! `ThumbBadge::LinkMissing`) and names them in the row's description; these tests check the badge PhotoCraft gives
//! each row, and that the panel shows it.

use craft::layers::ui::filter::LayerFilter;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{ColorMode, SampleType, SmartSource};
use serde_json::json;

use super::*;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The names the panel's words give the two link badges in a row's description.
const LINKED: &str = "Linked Smart Object";
const MISSING: &str = "Linked file is missing";

/// A 100 × 60 px PNG in a fresh temporary folder; returns the folder and the file.
fn png() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("linked-100x60.png");
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 60, "background": "#336699"})).unwrap();
    s.execute("file.saveACopy", json!({"path": file.to_string_lossy(), "layers": false})).unwrap();
    (directory, file)
}

/// A 120 × 120 px document with `file` placed embedded ("Embedded"), then linked on top ("Linked"), in the whole app
/// with the Layers panel shown. Returns the harness and the (embedded, linked) layer ids.
fn harness(file: &std::path::Path) -> (H, LayerId, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 120, "height": 120})).unwrap();
    let mut place = |command: &str, name: &str| {
        s.execute(command, json!({"path": file.to_string_lossy(), "center": [60, 60], "fit": false})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": id.0, "name": name})).unwrap();
        id
    };
    let (embedded, linked) = (place("file.placeEmbedded", "Embedded"), place("file.placeLinked", "Linked"));
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    (h, embedded, linked)
}

/// Runs the link watcher's look on the spot (the app runs it every two seconds) and redraws.
fn look(h: &mut H) {
    h.state_mut().run("layer.smartObjects.updateChanged", json!({})).unwrap();
    h.run_steps(4);
}

/// The badge PhotoCraft gives layer `id`'s row, under no filter.
fn badge(h: &H, id: LayerId) -> Option<ThumbBadge> {
    let app = h.state();
    let st = app.session.active().unwrap();
    PhotoLayers::new(app, st, &LayerFilter::default(), false, None).info(&id).thumb_badge
}

/// The description the panel gives the row named `name`, which carries a link badge's name.
fn said(h: &H, name: &str) -> String {
    let row = h.get_by_role_and_label(Role::TreeItem, name);
    row.accesskit_node().description().unwrap_or_default().to_string()
}

#[test]
fn a_linked_smart_object_gets_a_chain_link_badge_and_an_embedded_one_keeps_its_badge() {
    let (_directory, file) = png();
    let (mut h, embedded, linked) = harness(&file);
    look(&mut h);
    assert_eq!(badge(&h, linked), Some(ThumbBadge::Linked));
    assert_eq!(badge(&h, embedded), Some(ThumbBadge::SmartObject));
    let (linked_said, embedded_said) = (said(&h, "Linked"), said(&h, "Embedded"));
    assert!(linked_said.contains(LINKED) && !linked_said.contains(MISSING), "{linked_said:?}");
    assert!(!embedded_said.contains(LINKED) && !embedded_said.contains(MISSING), "{embedded_said:?}");
}

/// The missing set is read for every row, not only while the Smart Object filter is on: renaming the file away marks
/// the badge missing at the next look, and renaming it back brings the plain chain link back with nothing else done.
#[test]
fn a_missing_linked_file_turns_the_badge_red_until_the_file_is_back() {
    let (directory, file) = png();
    let (mut h, embedded, linked) = harness(&file);
    look(&mut h);
    assert_ne!(h.state().ui.layer_filter.by, craft::layers::ui::filter::FilterBy::SmartObject, "no Smart Object filter");

    let away = directory.path().join("renamed.png");
    std::fs::rename(&file, &away).unwrap();
    look(&mut h);
    assert_eq!(badge(&h, linked), Some(ThumbBadge::LinkMissing));
    assert_eq!(badge(&h, embedded), Some(ThumbBadge::SmartObject), "the embedded copy is not missing");
    let gone = said(&h, "Linked");
    assert!(gone.contains(MISSING) && !gone.contains(LINKED), "the panel shows it: {gone:?}");

    std::fs::rename(&away, &file).unwrap();
    look(&mut h);
    assert_eq!(badge(&h, linked), Some(ThumbBadge::Linked));
    let back = said(&h, "Linked");
    assert!(back.contains(LINKED) && !back.contains(MISSING), "{back:?}");
}

/// A PSD's placed layer (its path is a uuid in the PSD's own file table) keeps the smart object badge, missing or not.
#[test]
fn a_psd_placed_layer_keeps_the_smart_object_badge() {
    let (_directory, file) = png();
    let (h, embedded, _) = harness(&file);
    let doc = &h.state().session.active().unwrap().doc;
    let mut psd = doc.layer(embedded).unwrap().clone();
    let LayerContent::Smart(sm) = &mut psd.content else { panic!("smart object") };
    let uuid = "a1b2c3d4-0000-1111-2222-333344445555";
    sm.source = SmartSource::Linked { path: uuid.into() };
    sm.psd_raw = Some(std::sync::Arc::new(vec![0; 4]));
    assert_eq!(thumb_badge(doc, &psd, &HashSet::from([uuid.to_string()])), Some(ThumbBadge::SmartObject));
}

/// #14118 did not measure Indexed, Bitmap and 32-bit documents or links to a network share; those keep the smart
/// object badge, missing or not. The measured modes and depths get the link badge.
#[test]
fn the_cases_the_issue_did_not_measure_keep_the_smart_object_badge() {
    let (_directory, file) = png();
    let (h, _, linked) = harness(&file);
    let mut doc = h.state().session.active().unwrap().doc.as_ref().clone();
    let layer = doc.layer(linked).unwrap().clone();
    let LayerContent::Smart(sm) = &layer.content else { panic!("smart object") };
    let SmartSource::Linked { path } = &sm.source else { panic!("linked") };
    let missing = HashSet::from([path.clone()]);
    for (mode, depth) in
        [(ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Grayscale, SampleType::U8), (ColorMode::Cmyk, SampleType::U8)]
    {
        (doc.mode, doc.depth) = (mode, depth);
        assert_eq!(thumb_badge(&doc, &layer, &missing), Some(ThumbBadge::LinkMissing), "{mode:?} {depth:?}");
        assert_eq!(thumb_badge(&doc, &layer, &HashSet::new()), Some(ThumbBadge::Linked), "{mode:?} {depth:?}");
    }
    for (mode, depth) in [(ColorMode::Indexed, SampleType::U8), (ColorMode::Bitmap, SampleType::U8), (ColorMode::Rgb, SampleType::F32)] {
        (doc.mode, doc.depth) = (mode, depth);
        assert_eq!(thumb_badge(&doc, &layer, &missing), Some(ThumbBadge::SmartObject), "{mode:?} {depth:?}");
    }
    (doc.mode, doc.depth) = (ColorMode::Rgb, SampleType::U8);
    for share in [r"\\192.0.2.1\share\linked.png", "//192.0.2.1/share/linked.png"] {
        let mut on_share = layer.clone();
        let LayerContent::Smart(sm) = &mut on_share.content else { panic!("smart object") };
        sm.source = SmartSource::Linked { path: share.into() };
        let missing = HashSet::from([share.to_string()]);
        assert_eq!(thumb_badge(&doc, &on_share, &missing), Some(ThumbBadge::SmartObject), "{share}");
    }
}

/// A linked smart object in an artboard (not measured either) keeps the smart object badge in the panel, while the
/// file is there and while it is missing.
#[test]
fn a_linked_smart_object_in_an_artboard_keeps_the_smart_object_badge() {
    let (_directory, file) = png();
    let (mut h, _, linked) = harness(&file);
    h.state_mut().run("layer.new.artboardFromLayers", json!({})).unwrap();
    h.run_steps(8);
    assert!(h.state().session.active().unwrap().doc.artboard_of(linked).is_some(), "the linked layer is in an artboard");
    assert_eq!(badge(&h, linked), Some(ThumbBadge::SmartObject));
    std::fs::remove_file(&file).unwrap();
    look(&mut h);
    assert!(!h.state().session.missing_links().is_empty(), "the watcher saw it go");
    assert_eq!(badge(&h, linked), Some(ThumbBadge::SmartObject));
}
