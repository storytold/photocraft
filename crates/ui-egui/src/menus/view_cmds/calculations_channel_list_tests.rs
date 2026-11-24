//! #14521: Image > Calculations. The Source 1 Channel and Source 2 Channel dropdowns list the
//! reference application's entries for that source - Gray, Red, Green, Blue, then the source
//! layer's Layer Mask and Transparency, then the document's alpha channels in channel order -
//! and both rows start at Red. "selection" is not in the list, and the composite is named "Gray".
//!
//! The reference application draws a rule before the alpha channels. The hosted choice row of the
//! pinned craft framework builds its entries from `(id, label)` pairs and takes no separator
//! entries (only the blend row draws group lines, from the mode keys), so this list carries that
//! group as an order and the drawn rule waits on the framework.
//!
//! CMYK and Lab sources are marked *not measured* by the issue and keep the shared list; the
//! tests below pin that, as they pin the Load Selection and Apply Image lists staying as they are.

use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use serde_json::json;

use super::{calculations_channels, channel_form};
use crate::PhotocraftApp;

/// A document like the issue's test document "target": RGB 8 bit with a Background, a layer "Top"
/// without a mask, a layer "Masked" with a layer mask, and the alpha channel "Alpha 1". The last
/// layer made is active, so the active layer on this document is "Masked".
fn target() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12})).unwrap();
    app.run("edit.fill", json!({"color": "#c86432"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Masked"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
    app.sync_views();
    app
}

/// The issue marks Merged on a document without a Background *not measured* (whether Transparency is
/// listed there), so that source keeps today's shared list: its entries, their order and the composite
/// default. Only the measured colour modes with a Background get the per-source list.
#[test]
fn a_merged_source_without_a_background_keeps_the_list_of_today() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
    app.sync_views();
    let st = app.session.active().expect("the document is open");
    assert!(st.doc.background().is_none(), "the document has no Background layer");
    // The shared list, as `cmyk_and_lab_sources_keep_the_list_of_today` pins it for the modes the
    // issue leaves alone: no "Gray" name, no Red default, and the entries a merged source has no
    // plane of its own for.
    let want = ["RGB", "Red", "Green", "Blue", "Alpha 1", "transparency", "selection"].to_vec();
    let shared = rows(&app, "image.calculations");
    for key in ["source1Channel", "source2Channel"] {
        assert_eq!(ids(&shared, key), want, "{key} on a document without a Background");
        assert_eq!(options(&shared, key).1.as_deref(), Some("RGB"), "{key} keeps the composite default");
    }
    // The same document with a Background, which is the measured configuration, gets the per-source
    // list built from the merged source it reads.
    let measured = target();
    let measured_rows = rows(&measured, "image.calculations");
    assert_eq!(ids(&measured_rows, "source1Channel"), ["RGB", "Red", "Green", "Blue", "Alpha 1"], "with a Background the merged list is the reference one");
    assert_eq!(options(&measured_rows, "source1Channel").1.as_deref(), Some("Red"), "with a Background the row starts at Red");
}

/// The rows of `command`'s form on the document `app` builds, in English.
fn rows(app: &PhotocraftApp, command: &str) -> Vec<Param> {
    crate::i18n::with_language(crate::i18n::Lang::EN, || channel_form(app, command).expect("the command has a form").0)
}

fn row<'a>(rows: &'a [Param], key: &str) -> &'a Param {
    rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("the dialog has a {key} row"))
}

/// The options of choice row `key`: `(id, label text)` pairs, and the id the row opens on.
fn options(rows: &[Param], key: &str) -> (Vec<(String, String)>, Option<String>) {
    match &row(rows, key).kind {
        ParamKind::Choice { options, default, .. } => (options.iter().map(|(id, label)| (id.clone(), label.as_str().to_owned())).collect(), default.clone()),
        other => panic!("{key} is a plain select, not {other:?}"),
    }
}

fn ids(rows: &[Param], key: &str) -> Vec<String> {
    options(rows, key).0.iter().map(|(id, _)| id.clone()).collect()
}

fn labels(rows: &[Param], key: &str) -> Vec<String> {
    options(rows, key).0.iter().map(|(_, label)| label.clone()).collect()
}

/// The form sends no source layer until #14520 adds the Layer dropdown, which is Merged: the
/// reference list for Merged is Gray, Red, Green, Blue, rule, Alpha 1.
#[test]
fn the_merged_channel_list_is_the_reference_list_in_its_order() {
    let rows = rows(&target(), "image.calculations");
    assert_eq!(ids(&rows, "source1Channel"), ["RGB", "Red", "Green", "Blue", "Alpha 1"], "the composite and the colour channels, then the alpha channels");
    assert_eq!(ids(&rows, "source2Channel"), ids(&rows, "source1Channel"), "Source 2 offers the same list as Source 1");
}

/// A merged source has no layer of its own, so neither Layer Mask nor Transparency is listed even
/// though the document's active layer "Masked" has a mask.
#[test]
fn a_merged_source_lists_neither_layer_mask_nor_transparency() {
    let rows = rows(&target(), "image.calculations");
    for key in ["source1Channel", "source2Channel"] {
        for id in ["mask", "transparency"] {
            assert!(!ids(&rows, key).contains(&id.to_string()), "{key} offers {id} for a merged source");
        }
    }
}

/// The entries read as the reference application names them, not as the engine's ids.
#[test]
fn the_channel_list_names_its_entries_as_the_reference_application_does() {
    let rows = rows(&target(), "image.calculations");
    assert_eq!(labels(&rows, "source1Channel"), ["Gray", "Red", "Green", "Blue", "Alpha 1"]);
    assert_eq!(labels(&rows, "source2Channel"), ["Gray", "Red", "Green", "Blue", "Alpha 1"]);
}

/// The reference application has no selection entry in the Calculations channel lists.
#[test]
fn the_channel_list_has_no_selection_entry() {
    let rows = rows(&target(), "image.calculations");
    for key in ["source1Channel", "source2Channel"] {
        assert!(!ids(&rows, key).contains(&"selection".to_string()), "{key} offers the selection");
        assert!(!labels(&rows, key).contains(&"Selection".to_string()), "{key} offers a Selection label");
    }
}

/// Both Channel dropdowns start at Red on an RGB document.
#[test]
fn both_channel_rows_start_at_red() {
    let rows = rows(&target(), "image.calculations");
    for key in ["source1Channel", "source2Channel"] {
        assert_eq!(options(&rows, key).1.as_deref(), Some("Red"), "{key} starts at Red");
    }
}

/// A Grayscale source document lists Gray.
#[test]
fn a_grayscale_source_lists_gray() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12, "mode": "grayscale"})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
    app.sync_views();
    let rows = rows(&app, "image.calculations");
    assert_eq!(ids(&rows, "source1Channel"), ["Gray", "Alpha 1"], "Gray, rule, the alpha channels");
    assert_eq!(labels(&rows, "source1Channel"), ["Gray", "Alpha 1"]);
    // Gray is the only colour entry, so it is where the row starts.
    assert_eq!(options(&rows, "source1Channel").1.as_deref(), Some("Gray"), "a Grayscale source starts at Gray");
}

/// Every id the list offers is one the `image.calculations` command takes for both sources, so
/// choosing any entry sends a channel the engine reads: the merged list through the flat params the
/// form sends today, and the list of the source layer "Masked" through a source spec with that layer,
/// which is the shape #14520's Layer dropdown will send. The second round is the one that covers
/// Layer Mask and Transparency.
#[test]
fn every_listed_channel_id_reaches_the_engine() {
    let rows = rows(&target(), "image.calculations");
    for id in ids(&rows, "source1Channel") {
        let mut app = target();
        let r = app.run("image.calculations", json!({"source1Channel": id, "source2Channel": id, "blending": "multiply"}));
        assert!(r.is_ok(), "{id} is listed for Merged but the engine refuses it: {:?}", r.err());
    }
    let (layer_ids, _, _) = for_layer(&target(), "Masked");
    let source = |id: &str| json!({"channel": id, "layer": "Masked"});
    for id in layer_ids {
        let mut app = target();
        let r = app.run("image.calculations", json!({"source1": source(&id), "source2": source(&id), "blending": "multiply"}));
        assert!(r.is_ok(), "{id} is listed for the layer \"Masked\" but the engine refuses it: {:?}", r.err());
    }
}

/// The "Gray" entry carries the composite id the form sent for it before, so OK still runs the
/// command on the composite and the grey reduction the command makes for a merged source.
#[test]
fn the_gray_entry_sends_the_composite_id_the_form_sent_before() {
    let mut app = target();
    let gray = app.run("image.calculations", json!({"source1Channel": "RGB", "source2Channel": "RGB", "blending": "multiply"})).unwrap()["channel"]
        .as_u64()
        .unwrap() as usize;
    let composite = app
        .run("image.calculations", json!({"source1": {"channel": "composite"}, "source2": {"channel": "composite"}, "blending": "multiply"}))
        .unwrap()["channel"]
        .as_u64()
        .unwrap() as usize;
    let st = app.session.active().expect("the document is open");
    let (a, b) = (&st.doc.channels[gray], &st.doc.channels[composite]);
    assert_eq!(a.surface.pixel(2, 3), b.surface.pixel(2, 3), "the same grey plane");
    assert_eq!(a.surface.pixel(2, 3).len(), 1, "reduced to one grey channel");
}

/// The list of one source layer, built by the helper the two rows use: the Calculations form has no
/// Layer dropdown until #14520, so the form itself only ever shows the merged list, and the rule for
/// a chosen layer lives here.
fn for_layer(app: &PhotocraftApp, layer: &str) -> (Vec<String>, Vec<String>, String) {
    let st = app.session.active().expect("the document is open");
    let doc = &st.doc;
    let chosen = doc.walk().into_iter().find(|(_, _, l)| l.name == layer).map(|(_, _, l)| l);
    let names: Vec<String> = doc.channels.iter().map(|c| c.name.clone()).collect();
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (options, default) =
            calculations_channels(doc.pixel_format().mode, chosen, doc.background().is_some(), &names).expect("a measured mode lists its own channels");
        (options.iter().map(|(id, _)| id.clone()).collect(), options.iter().map(|(_, l)| l.as_str().to_owned()).collect(), default)
    })
}

/// The issue's "Masked" layer: Layer Mask joins the list because that layer has a mask, and it comes
/// before Transparency.
#[test]
fn a_layer_source_with_a_mask_lists_layer_mask_then_transparency() {
    let (ids, labels, default) = for_layer(&target(), "Masked");
    assert_eq!(ids, ["RGB", "Red", "Green", "Blue", "mask", "transparency", "Alpha 1"]);
    assert_eq!(labels, ["Gray", "Red", "Green", "Blue", "Layer Mask", "Transparency", "Alpha 1"]);
    assert_eq!(default, "Red");
}

/// The issue's "Top" layer has no mask, so no Layer Mask entry — for that source, whatever the
/// document's other layers hold.
#[test]
fn a_layer_source_without_a_mask_lists_no_layer_mask() {
    let (ids, labels, _) = for_layer(&target(), "Top");
    assert_eq!(ids, ["RGB", "Red", "Green", "Blue", "transparency", "Alpha 1"]);
    assert_eq!(labels, ["Gray", "Red", "Green", "Blue", "Transparency", "Alpha 1"]);
}

/// The alpha channels stand after the colour and layer entries, which is where the reference
/// application draws its rule.
#[test]
fn the_alpha_channels_follow_the_colour_and_layer_entries() {
    let (ids, _, _) = for_layer(&target(), "Masked");
    let at = |id: &str| ids.iter().position(|x| x == id).expect("the entry is listed");
    assert!(at("Blue") < at("mask"), "Layer Mask follows the colour channels");
    assert!(at("mask") < at("transparency"), "Transparency follows Layer Mask");
    assert!(at("transparency") < at("Alpha 1"), "the alpha channels follow both");
}

/// CMYK and Lab sources are marked *not measured* by the issue: their Calculations list, its order
/// and its default stay the shared list of today.
#[test]
fn cmyk_and_lab_sources_keep_the_list_of_today() {
    for (mode, want) in [
        ("cmyk", &["CMYK", "Cyan", "Magenta", "Yellow", "Black", "Alpha 1", "transparency", "selection"][..]),
        ("lab", &["Lab", "Lightness", "a", "b", "Alpha 1", "transparency", "selection"][..]),
    ] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 12, "mode": mode})).unwrap();
        app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
        app.sync_views();
        let rows = rows(&app, "image.calculations");
        assert_eq!(ids(&rows, "source1Channel"), want.to_vec(), "{mode} keeps the shared list");
        assert_eq!(options(&rows, "source1Channel").1.as_deref(), want.first().copied(), "{mode} keeps the composite default");
    }
}

/// Select… Load Selection and Image ▸ Apply Image read the shared list: it keeps its entries, its
/// order and its selection entry (#14521 touches only the Calculations arm).
#[test]
fn load_selection_and_apply_image_keep_their_channel_lists() {
    let app = target();
    let want = ["RGB", "Red", "Green", "Blue", "Alpha 1", "transparency", "mask", "selection"].to_vec();
    assert_eq!(ids(&rows(&app, "select.loadSelection"), "channel"), want, "Load Selection");
    assert_eq!(ids(&rows(&app, "image.applyImage"), "sourceChannel"), want, "Apply Image");
}
