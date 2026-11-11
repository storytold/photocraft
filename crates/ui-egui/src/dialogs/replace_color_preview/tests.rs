use serde_json::json;

use super::*;
use crate::filter_dialog::open;

const REPLACE_COLOR: &str = "image.adjustments.replaceColor";

/// 40 × 30 on one layer: red (0..20, 0..15), blue (20..40, 0..15), black and white below.
fn app_with_doc() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "#ffffff"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    for (rect, color) in [([0, 0, 20, 15], "#ff0000"), ([20, 0, 20, 15], "#0000ff"), ([0, 15, 20, 15], "#000000")] {
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app
}

fn fields(app: &PhotocraftApp) -> Map<String, Value> {
    app.ui.dialogs.last().unwrap().fields.clone()
}

/// #6281: Selection shows the sampled red as changed and the rest as unchanged.
#[test]
fn selection_whites_out_the_sampled_colour_only() {
    let mut app = app_with_doc();
    open(&mut app, REPLACE_COLOR).unwrap();
    assert!(crate::filter_dialog::color_eyedropper_sample(&mut app, 5.0, 5.0, egui::Modifiers::NONE));
    let (mask, [w, _]) = mask_values(&app, &fields(&app)).unwrap();
    assert!(mask[5 * w + 5] > 0.9, "red is changed: {}", mask[5 * w + 5]);
    assert!(mask[5 * w + 25] < 0.1, "blue is not: {}", mask[5 * w + 25]);
    assert!(mask[20 * w + 5] < 0.1, "black is not: {}", mask[20 * w + 5]);
}

/// The thumbnail shows what OK changes, pixel for pixel at full size, for one colour, Localized
/// Color Clusters, and added and subtracted samples.
#[test]
fn selection_matches_what_ok_changes() {
    for clicks in [
        vec![([5.0, 5.0], egui::Modifiers::NONE)],
        vec![([5.0, 5.0], egui::Modifiers::NONE), ([25.0, 5.0], egui::Modifiers::SHIFT)],
        vec![([5.0, 5.0], egui::Modifiers::NONE), ([25.0, 5.0], egui::Modifiers::SHIFT), ([25.0, 5.0], egui::Modifiers::ALT)],
    ] {
        for localized in [false, true] {
            let mut app = app_with_doc();
            let id = open(&mut app, REPLACE_COLOR).unwrap();
            for (at, mods) in &clicks {
                assert!(crate::filter_dialog::color_eyedropper_sample(&mut app, at[0], at[1], *mods));
            }
            {
                let f = &mut app.ui.dialog_mut(id).unwrap().fields;
                f.insert("localized".into(), json!(localized));
                f.insert("range".into(), json!(30));
                f.insert("fuzziness".into(), json!(10));
                f.insert("lightness".into(), json!(-100));
            }
            let (mask, [w, h]) = mask_values(&app, &fields(&app)).unwrap();
            assert_eq!([w, h], [40, 30], "a small document is judged at full size");
            let before = app.session.active().unwrap().doc.clone();
            crate::dialogs::confirm(&mut app, id).unwrap();
            let after = app.session.active().unwrap().doc.clone();
            let (b, a) = (before.layers[0].surface().unwrap(), after.layers[0].surface().unwrap());
            for y in 0..30u32 {
                for x in 0..40u32 {
                    let changed = (0..3).any(|c| (b.rgba(x as i32, y as i32)[c] - a.rgba(x as i32, y as i32)[c]).abs() > 1.0 / 255.0);
                    let shown = mask[y as usize * w + x as usize] > 0.5 / 255.0;
                    assert_eq!(shown, changed, "({x}, {y}) for {clicks:?}, localized {localized}");
                }
            }
        }
    }
}

/// Clicking the thumbnail samples like a click on the image.
#[test]
fn clicking_the_thumbnail_samples() {
    let mut app = app_with_doc();
    open(&mut app, REPLACE_COLOR).unwrap();
    let mut f = fields(&app);
    assert!(crate::filter_dialog::sample_into(&mut app, &mut f, 25.0, 5.0, egui::Modifiers::NONE));
    assert_eq!(f["color"], json!("#0000ff"));
    assert_eq!(f["points"], json!([[25.0, 5.0]]));
    // Off the image nothing changes.
    let kept = f.clone();
    assert!(!crate::filter_dialog::sample_into(&mut app, &mut f, -3.0, 5.0, egui::Modifiers::NONE));
    assert_eq!(f, kept);
}

/// A big document is judged on a reduced copy, and the view is a dialog-only field.
#[test]
fn big_documents_are_reduced_and_the_view_is_not_a_param() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 1600, "height": 900, "background": "#ff0000"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let id = open(&mut app, REPLACE_COLOR).unwrap();
    assert!(crate::filter_dialog::color_eyedropper_sample(&mut app, 10.0, 10.0, egui::Modifiers::NONE));
    app.ui.dialog_mut(id).unwrap().fields.insert(VIEW.into(), json!("image"));
    let (mask, [w, h]) = mask_values(&app, &fields(&app)).unwrap();
    assert_eq!([w, h], [400, 225]);
    assert!(mask.iter().all(|&v| v > 0.9));
    assert!(crate::filter_dialog::params_of(&fields(&app)).get(VIEW).is_none());
}

/// Nothing to judge (a bad colour) is an error shown in the thumbnail, never a panic.
#[test]
fn bad_settings_are_an_error() {
    let mut app = app_with_doc();
    let id = open(&mut app, REPLACE_COLOR).unwrap();
    {
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("localized".into(), json!(true));
        f.remove("points");
    }
    assert!(mask_values(&app, &fields(&app)).is_err());
}
