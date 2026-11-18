//! #10879: setting a Layers panel filter deselects the selected layers it hides, against the cases
//! measured in Photoshop. The panel's own filter row does the same in craft-layers; these set the
//! filter as the control channel's `ui.set {layerFilter}` does: through `patched` and `apply`.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// "Red" and "Deep Pixel" (pixel layers) and "Hello" (type) over the Background, in the panel.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Deep Pixel"})).unwrap();
    let text = s.execute("type.create", json!({"text": "Hello", "x": 4, "y": 20, "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": text, "name": "Hello"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    h
}

fn id(h: &Harness<'_, PhotocraftApp>, name: &str) -> LayerId {
    h.state().session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id).unwrap()
}

fn select(h: &mut Harness<'_, PhotocraftApp>, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let layer = id(h, name).0;
        h.state_mut().session.execute("layer.select", json!({"layer": layer, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
    h.run_steps(4);
}

/// The selected layers' names, bottom to top, and the active layer's.
fn selection(h: &Harness<'_, PhotocraftApp>) -> (Vec<String>, Option<String>) {
    let st = h.state().session.active().unwrap();
    let name = |id: LayerId| st.doc.layer(id).unwrap().name.clone();
    (st.selected_layers().into_iter().map(name).collect(), st.active_layer.map(name))
}

enum Step {
    /// The `ui.set {layerFilter}` patch, applied to the filter.
    Filter(Value),
    /// Select these layers (the last one active).
    Select(&'static [&'static str]),
}

#[test]
fn setting_a_filter_deselects_the_layers_it_hides() {
    use Step::*;
    type Case = (&'static str, &'static [&'static str], Vec<(Step, &'static [&'static str], Option<&'static str>)>);
    let cases: Vec<Case> = vec![
        (
            "Kind › Type hides Red; turning the filter off does not bring the selection back",
            &["Red"],
            vec![(Filter(json!({"kinds": ["type"]})), &[], None), (Filter(json!({"kinds": []})), &[], None)],
        ),
        ("Smart Object › Embedded lists neither", &["Hello", "Deep Pixel"], vec![(Filter(json!({"by": "smartObject", "smart": ["embedded"]})), &[], None)]),
        (
            "the listed layers stay selected, one of them active; new criteria act again",
            &["Red", "Hello", "Deep Pixel"],
            vec![(Filter(json!({"kinds": ["type"]})), &["Hello"], Some("Hello")), (Filter(json!({"by": "name", "name": "zzzz"})), &[], None)],
        ),
        ("Name hides Red", &["Red"], vec![(Filter(json!({"by": "name", "name": "hello"})), &[], None)]),
        (
            "a layer selected while the filter hides it stays selected: the filter acts only when it changes",
            &["Hello"],
            vec![(Filter(json!({"kinds": ["type"]})), &["Hello"], Some("Hello")), (Select(&["Red"]), &["Red"], Some("Red"))],
        ),
    ];
    for (case, start, steps) in cases {
        let mut h = harness();
        select(&mut h, start);
        let history = h.state().session.active().unwrap().history.entries().len();
        for (step, selected, active) in steps {
            match step {
                Filter(patch) => {
                    let filter = crate::layer_filter_ui::patched(&h.state().ui.layer_filter, &patch).unwrap();
                    crate::layer_filter_ui::apply(h.state_mut(), filter);
                }
                Select(names) => select(&mut h, names),
            }
            h.run_steps(4);
            let want = (selected.iter().map(|s| s.to_string()).collect::<Vec<_>>(), active.map(str::to_string));
            assert_eq!(selection(&h), want, "{case}");
        }
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), history, "{case}: deselecting is no history step");
    }
}
