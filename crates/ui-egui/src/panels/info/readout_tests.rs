//! The Info panel's readouts: which rows the First and Second Color Readout modes show, for the
//! pointer and the Color Sampler points.

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;
use crate::info_panel_options::{InfoPanelState, Readout};

/// A white 80×60 sRGB document with RGB(200, 100, 50) in its left half.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#c86432"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app
}

/// Render the Info panel at its normal dock width with the shared Pro theme and settled layout.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(340.0, 600.0)).build_ui_state(|ui, app: &mut PhotocraftApp| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

/// How many labels read exactly `text`.
fn count(h: &Harness<'static, PhotocraftApp>, text: &str) -> usize {
    h.query_all_by_label(text).count()
}

fn readouts(first: Readout, second: Readout) -> InfoPanelState {
    InfoPanelState { first, second }
}

/// The defaults are Photoshop's (Actual Color, CMYK): an RGB document reads RGB on the left and
/// CMYK on the right.
#[test]
fn default_readouts_are_actual_color_and_cmyk() {
    let mut app = app();
    app.hover_doc = Some([10.0, 10.0]);
    let h = harness(app);
    for key in ["R:", "G:", "B:", "C:", "M:", "K:"] {
        assert_eq!(count(&h, key), 1, "{key} row");
    }
    assert_eq!(count(&h, "Y:"), 2, "CMYK's Y and the position's Y");
    assert_eq!(count(&h, "200"), 1, "R 200");
    assert!(h.get_by_label("R:").rect().max.x < h.get_by_label("C:").rect().min.x, "first readout left, second right");
}

/// Total Ink reads `Σ:` and Opacity `Op:`, as in the reference application: RGB(200, 100, 50)
/// adds up its CMYK plates, and an opaque pixel reads 100 %.
#[test]
fn total_ink_and_opacity_rows_use_the_reference_keys() {
    let mut app = app();
    app.ui.info_panel = readouts(Readout::TotalInk, Readout::Opacity);
    app.hover_doc = Some([10.0, 10.0]);
    let h = harness(app);
    let cmyk =
        h.state().session.active().map(|s| photocraft_engine::sampler_cmds::readouts(&s.doc, [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 1.0])).unwrap().cmyk;
    let total = format!("{}%", cmyk.iter().sum::<i32>());
    assert_eq!(count(&h, "Σ:"), 1);
    assert_eq!(count(&h, &total), 1, "Σ {total}");
    assert_eq!(count(&h, "Op:"), 1);
    assert_eq!(count(&h, "100%"), 1, "Op 100%");
    for gone in ["CMYK:", "Composite opacity:", "Spot:"] {
        assert_eq!(count(&h, gone), 0, "{gone}");
    }
}

/// Proof Color reads the sample in View › Proof Setup's profile: C/M/Y/K for the default Working
/// CMYK proof, R/G/B for an sRGB proof. Off the canvas the rows stay, with dashes.
#[test]
fn proof_color_reads_the_proof_setup() {
    let mut app = app();
    app.ui.info_panel = readouts(Readout::ProofColor, Readout::Cmyk);
    app.hover_doc = Some([10.0, 10.0]);
    let mut h = harness(app);
    for key in ["C:", "M:", "K:"] {
        assert_eq!(count(&h, key), 2, "{key} in Proof Color and CMYK");
    }
    let rgb = [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0];
    let proof = h.state().session.active().and_then(|s| photocraft_engine::sampler_cmds::proof_readout(&h.state().session.color, &s.doc, rgb)).unwrap();
    for v in &proof.values {
        assert!(count(&h, &format!("{v}%")) >= 1, "proof plate {v}% shown");
    }
    h.state_mut().run("view.proofSetup", json!({"profile": "srgb"})).unwrap();
    h.run_steps(2);
    assert_eq!(count(&h, "R:"), 1, "an sRGB proof reads R, G, B");
    assert_eq!(count(&h, "200"), 1);
    assert_eq!(count(&h, "C:"), 1, "only the CMYK readout");
    h.state_mut().hover_doc = None;
    h.run_steps(2);
    assert_eq!(count(&h, "R:"), 1, "the row stays");
    assert!(count(&h, "—") >= 3, "dashes");
}

/// Grayscale reads K % as Photoshop does (`100 − luma`), and a readout off the canvas keeps its
/// rows with dashes.
#[test]
fn grayscale_reads_the_pointer() {
    let mut app = app();
    app.ui.info_panel = readouts(Readout::Grayscale, Readout::Opacity);
    app.run("select.all", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.hover_doc = Some([10.0, 10.0]);
    let mut h = harness(app);
    assert!(h.query_by_label("R:").is_none(), "RGB is off");
    assert_eq!(count(&h, "70%"), 1, "K 70 % for red");
    h.state_mut().hover_doc = None;
    h.run_steps(2);
    assert_eq!(count(&h, "K:"), 1, "the row stays");
    assert_eq!(count(&h, "—"), 4, "K and Op dashed, as are X and Y");
}

/// Actual Color follows the document's mode: K for a grayscale document, every channel of a
/// Multichannel one.
#[test]
fn actual_color_follows_the_document_mode() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 20, "mode": "grayscale", "background": "white"})).unwrap();
    app.ui.info_panel = readouts(Readout::ActualColor, Readout::Opacity);
    app.hover_doc = Some([5.0, 5.0]);
    let h = harness(app);
    assert!(h.query_by_label("K:").is_some() && h.query_by_label("R:").is_none(), "gray reads K");
    assert!(h.query_by_label("0%").is_some(), "white = K 0 %");
}

/// A Multichannel document's Actual Color is a channel list, so a Color Sampler point gives each
/// channel its own line instead of one long line.
#[test]
fn multichannel_actual_color_samplers_list_one_channel_per_line() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 20, "background": "white"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("image.mode.multichannel", json!({})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    app.ui.info_panel = readouts(Readout::ActualColor, Readout::Opacity);
    app.hover_doc = Some([5.0, 5.0]);
    let h = harness(app);
    for key in ["Cyan:", "Magenta:", "Yellow:"] {
        assert_eq!(count(&h, key), 1, "{key} pointer row");
    }
    // Red read as CMY inks: no cyan, solid magenta and yellow; one label per channel.
    assert_eq!(count(&h, "Cyan:0%"), 1, "point 1 cyan on its own line");
    assert_eq!(count(&h, "Magenta:100%"), 1, "point 1 magenta on its own line");
    assert_eq!(count(&h, "Yellow:100%"), 1, "point 1 yellow on its own line");
    assert_eq!(h.query_all_by_label_contains("Cyan:0% Magenta").count(), 0, "not joined on one line");
}

/// Every mode renders rows for an RGB sample, on the pointer and on a Color Sampler point.
#[test]
fn every_readout_and_the_samplers() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
    let (red, proof) = {
        let s = app.session.active().unwrap();
        let red = photocraft_engine::sampler_cmds::readouts(&s.doc, [1.0, 0.0, 0.0, 1.0]);
        (red, photocraft_engine::sampler_cmds::proof_readout(&app.session.color, &s.doc, [1.0, 0.0, 0.0]).unwrap())
    };
    let plates = |v: &[i32]| format!("C:{}% M:{}% Y:{}% K:{}%", v[0], v[1], v[2], v[3]);
    let (proof_line, cmyk_line, total_line) = (plates(&proof.values), plates(&red.cmyk), format!("Σ:{}%", red.total_ink));
    let expected = [
        (Readout::ActualColor, vec!["R:"], "R:255 G:0 B:0"),
        (Readout::ProofColor, vec!["C:", "M:", "K:"], proof_line.as_str()),
        (Readout::Grayscale, vec!["K:"], "K:70%"),
        (Readout::Rgb, vec!["R:", "G:", "B:"], "R:255 G:0 B:0"),
        (Readout::Web, vec!["R:"], "R:ff G:00 B:00"),
        (Readout::Hsb, vec!["H:", "S:", "B:"], "H:0° S:100% B:100%"),
        (Readout::Hsl, vec!["H:", "S:", "L:"], "H:0° S:100% L:50%"),
        (Readout::Cmyk, vec!["C:", "M:", "K:"], cmyk_line.as_str()),
        (Readout::Lab, vec!["L:", "a:", "b:"], "L:54"),
        (Readout::TotalInk, vec!["Σ:"], total_line.as_str()),
        (Readout::Opacity, vec!["Op:"], "Op:100%"),
    ];
    assert_eq!(expected.len(), Readout::ALL.len(), "every mode");
    let mut h = harness(app);
    for (mode, keys, line) in expected {
        h.state_mut().ui.info_panel = readouts(mode, mode);
        h.state_mut().hover_doc = Some([10.0, 10.0]);
        h.run_steps(2);
        for key in keys {
            assert!(count(&h, key) >= 2, "{mode:?}: {key} in both readouts");
        }
        assert_eq!(h.query_all_by_label_contains(line).count(), 2, "{mode:?}: point 1 shows {line} for both readouts");
    }
}

/// Channel lists take the full panel width: a long channel name is cut with an ellipsis and its
/// value stays inside the panel, clear of the name.
#[test]
fn long_channel_names_never_run_under_their_value() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    app.run("image.mode.multichannel", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let long = "PANTONE 2995 C a very long spot channel name indeed";
    app.run("channel.newSpot", json!({"name": long, "color": "#00a0ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.info_panel = readouts(Readout::ActualColor, Readout::Opacity);
    app.hover_doc = Some([10.0, 10.0]);
    let width = 260.0;
    let mut h = Harness::builder().with_size(vec2(width, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    let panel = h.ctx.input(|i| i.viewport_rect());
    let name = h.get_by_label_contains("PANTONE 2995").rect();
    let value = h.query_all_by_label("100%").find(|n| (n.rect().center().y - name.center().y).abs() < 2.0).map(|n| n.rect()).unwrap();
    assert!(value.max.x <= panel.max.x + 0.5, "the ink value is inside the panel: {value:?} in {panel:?}");
    assert!(name.max.x <= value.min.x + 0.5, "the name ends before the value: {name:?} vs {value:?}");
    assert!(name.min.x < panel.min.x + 20.0, "the name starts at the panel's left edge, not in a column: {name:?}");
    assert!(name.min.y > h.get_by_label("Op:").rect().min.y, "the channel rows sit below the column rows");
}
