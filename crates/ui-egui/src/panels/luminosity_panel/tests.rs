use super::*;

fn app_with_doc() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 16})).unwrap();
    // Left half mid gray, right half the white Background.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16, "mode": "replace"})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app
}

fn draw(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::theme::install_fonts(ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui));
    out.textures_delta.clear();
}

#[test]
fn params_for_a_tonal_mask() {
    let st = PanelState::default();
    let p = mask_params("lights", 3, &st);
    assert_eq!(p, json!({"kind": "lights", "output": "layerMask", "sampleAllLayers": true, "degree": 3}));
}

#[test]
fn params_for_a_colour_mask_follow_the_narrowing() {
    let mut st = PanelState { output: Output::Channel, composite: false, ..PanelState::default() };
    let plain = mask_params("reds", 1, &st);
    assert_eq!(plain, json!({"kind": "reds", "output": "channel", "sampleAllLayers": false}));
    st.colour_tone = Some("darks");
    st.colour_degree = 4;
    let narrowed = mask_params("reds", 1, &st);
    assert_eq!(narrowed["tone"], "darks");
    assert_eq!(narrowed["toneDegree"], 4);
    assert!(narrowed.get("degree").is_none());
}

#[test]
fn every_params_the_panel_can_build_is_accepted_by_the_engine() {
    let mut app = app_with_doc();
    for output in Output::ALL {
        for composite in [true, false] {
            for (_, kind) in TONES {
                for degree in 1..=MAX_DEGREE {
                    let mut st = PanelState { output, composite, ..PanelState::default() };
                    create(&mut app, kind, degree, &mut st);
                    assert_eq!(st.error, None, "{kind} {degree} -> {output:?}");
                }
            }
            for (_, kind, _) in COLOURS {
                for tone in [None, Some("lights"), Some("darks"), Some("midtones")] {
                    let mut st = PanelState { output, composite, colour_tone: tone, colour_degree: MAX_DEGREE, ..PanelState::default() };
                    create(&mut app, kind, 1, &mut st);
                    assert_eq!(st.error, None, "{kind} {tone:?} -> {output:?}");
                }
            }
        }
    }
}

#[test]
fn a_click_puts_the_mask_where_the_output_says() {
    let mut app = app_with_doc();
    let mut st = PanelState { output: Output::Channel, ..PanelState::default() };
    create(&mut app, "lights", 1, &mut st);
    assert_eq!(app.session.active().unwrap().doc.channels.len(), 1);

    st.output = Output::LayerMask;
    create(&mut app, "darks", 2, &mut st);
    let d = app.session.active().unwrap();
    assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_some());
}

#[test]
fn a_refusal_is_kept_for_display_and_cleared_by_the_next_success() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut st = PanelState::default();
    create(&mut app, "lights", 1, &mut st);
    assert!(st.error.is_some(), "no document open");
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    create(&mut app, "lights", 1, &mut st);
    assert_eq!(st.error, None);
}

#[test]
fn the_panel_draws_with_and_without_a_document() {
    let ctx = egui::Context::default();
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    draw(&mut empty, &ctx);
    let mut app = app_with_doc();
    draw(&mut app, &ctx);
    draw(&mut app, &ctx);
}
