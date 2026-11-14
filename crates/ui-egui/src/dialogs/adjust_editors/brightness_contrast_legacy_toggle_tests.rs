use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Adjustment, LayerContent};
use serde_json::json;

use super::{EditorCx, editor};
use crate::PhotocraftApp;

#[test]
fn legacy_toggle_resets_both_values_in_the_dialog() {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        session.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
        PhotocraftApp::new(session, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "image.adjustments.brightnessContrast", json!({})).unwrap()["dialog"].as_u64().unwrap();

    for (was_legacy, contrast) in [(false, 30), (true, -80)] {
        let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        fields.insert("brightness".into(), json!(40));
        fields.insert("contrast".into(), json!(contrast));
        fields.insert("legacy".into(), json!(was_legacy));
        h.run_steps(4);

        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Use Legacy").click();
        h.run_steps(3);

        let fields = &h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
        assert_eq!(fields["legacy"], !was_legacy);
        assert_eq!(fields["brightness"].as_f64(), Some(0.0));
        assert_eq!(fields["contrast"].as_f64(), Some(0.0));
        let (preview, _) = crate::adjust_preview::display_doc(h.state_mut(), 0).expect("live Brightness/Contrast preview");
        let adjustment = &preview.layer(crate::adjust_preview::PREVIEW_LAYER).unwrap().content;
        assert_eq!(adjustment, &LayerContent::Adjustment(Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0, legacy: !was_legacy }));
    }
}

#[test]
fn properties_legacy_toggle_preserves_values_and_clamps_modern_contrast() {
    let mut h = Harness::builder().with_size(egui::vec2(450.0, 250.0)).build_ui_state(
        |ui, values| {
            let cx = EditorCx {
                mem: ui.id(),
                hist: None,
                gray: false,
                swatches: [[0.0; 3]; 2],
                gradients: Vec::new(),
                dialog: false,
                dynamic_color_sliders: false,
                isolated_curve: None,
            };
            editor(ui, "brightnessContrast", values, &cx);
        },
        json!({"brightness": 40, "contrast": 30, "legacy": false}),
    );
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Use Legacy").click();
    h.run_steps(2);
    assert_eq!(*h.state(), json!({"brightness": 40, "contrast": 30, "legacy": true}));

    h.state_mut()["contrast"] = json!(-80);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Use Legacy").click();
    h.run_steps(2);
    assert_eq!(h.state()["legacy"], false);
    assert_eq!(h.state()["brightness"].as_f64(), Some(40.0));
    assert_eq!(h.state()["contrast"].as_f64(), Some(-50.0));
}
