use super::*;

const APPLY: &str = "image.applyImage";

#[test]
fn opens_with_the_single_target_and_keeps_the_other_defaults() {
    let (mut app, ctx) = app_with(1);
    app.run("layer.new.layer", json!({"name": "Source Layer"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
    app.run("channel.target", json!({"channel": "composite"})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    let composite = fields(&app, dialog);
    assert_eq!(composite["sourceChannel"], "RGB");
    cancel(&mut app, &ctx, dialog);

    for (target, name) in [(json!("red"), "Red"), (json!(0), "Alpha 1")] {
        app.run("channel.target", json!({"channel": target})).unwrap();
        let dialog = open_form(&mut app, &ctx, APPLY);
        let mut expected = composite.clone();
        expected["sourceChannel"] = json!(name);
        assert_eq!(fields(&app, dialog), expected);
        cancel(&mut app, &ctx, dialog);
    }

    app.run("channel.target", json!({"channel": "red"})).unwrap();
    app.run("channel.target", json!({"channel": "green", "extend": true})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    assert_eq!(fields(&app, dialog), composite);
    cancel(&mut app, &ctx, dialog);
}

#[test]
fn single_target_takes_precedence_over_a_successfully_remembered_source() {
    let (mut app, ctx) = app_with(1);
    let dialog = open_form(&mut app, &ctx, APPLY);
    set(&mut app, &ctx, dialog, "sourceChannel", json!("Blue"));
    set(&mut app, &ctx, dialog, "opacity", json!(40.0));
    confirm(&mut app, &ctx, dialog).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    assert_eq!(fields(&app, dialog)["sourceChannel"], "Blue");
    cancel(&mut app, &ctx, dialog);

    app.run("channel.target", json!({"channel": "red"})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    let shown = fields(&app, dialog);
    assert_eq!(shown["sourceChannel"], "Red");
    assert_eq!(shown["opacity"], 40.0);
    cancel(&mut app, &ctx, dialog);

    app.run("channel.target", json!({"channel": "composite"})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    assert_eq!(fields(&app, dialog)["sourceChannel"], "Blue", "cancel leaves the remembered source intact");
}

#[test]
fn accepting_uses_the_selected_alpha_and_only_edits_that_channel() {
    for name in ["Alpha 1", "Red", "selection", ""] {
        let (mut app, ctx) = app_with(1);
        app.run("edit.fill", json!({"color": "#304050"})).unwrap();
        app.run("channel.new", json!({"name": name, "fill": "white"})).unwrap();
        let pixel = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(4, 4)
        };
        let before = pixel(&app);
        let steps = app.session.active().unwrap().history.past_len();
        let dialog = open_form(&mut app, &ctx, APPLY);
        set(&mut app, &ctx, dialog, "sourceInvert", json!(true));
        set(&mut app, &ctx, dialog, "blending", json!("normal"));
        confirm(&mut app, &ctx, dialog).unwrap();
        assert!(!app.ui.status_error, "{name:?}: {}", app.ui.status);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.channels[0].surface.sample_channel(4, 4, 0), 0.0, "{name:?}: white source inverts to black");
        assert_eq!(pixel(&app), before, "{name:?}: layer pixels");
        assert_eq!(st.history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.channels[0].surface.sample_channel(4, 4, 0), 1.0);
    }
}

#[test]
fn composite_and_native_channel_defaults_follow_the_document_mode() {
    for (mode, target, default) in [("cmyk", "black", "Black"), ("lab", "lightness", "Lightness"), ("grayscale", "gray", "Gray")] {
        let (mut app, ctx) = app_with(0);
        app.run("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": 16})).unwrap();
        app.run("channel.target", json!({"channel": target})).unwrap();
        let dialog = open_form(&mut app, &ctx, APPLY);
        assert_eq!(fields(&app, dialog)["sourceChannel"], default, "{mode}");
        cancel(&mut app, &ctx, dialog);
    }
}

#[test]
fn choosing_another_source_still_works_with_a_single_destination() {
    let (mut app, ctx) = app_with(1);
    app.run("edit.fill", json!({"color": "#304080"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 1", "fill": "white"})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    assert_eq!(fields(&app, dialog)["sourceChannel"], "Alpha 1");
    set(&mut app, &ctx, dialog, "sourceChannel", json!("Blue"));
    set(&mut app, &ctx, dialog, "blending", json!("normal"));
    confirm(&mut app, &ctx, dialog).unwrap();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let sample = app.session.active().unwrap().doc.channels[0].surface.sample_channel(4, 4, 0);
    assert!((sample - 128.0 / 255.0).abs() < 0.003, "{sample}");
}

#[test]
fn a_targeted_alpha_can_share_a_name_with_another_saved_channel() {
    let (mut app, ctx) = app_with(1);
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("channel.new", json!({"name": "Saved", "fill": "black"})).unwrap();
    app.run("channel.new", json!({"name": "Saved", "fill": "white"})).unwrap();
    let dialog = open_form(&mut app, &ctx, APPLY);
    set(&mut app, &ctx, dialog, "sourceInvert", json!(true));
    set(&mut app, &ctx, dialog, "blending", json!("normal"));
    confirm(&mut app, &ctx, dialog).unwrap();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let channels = &app.session.active().unwrap().doc.channels;
    assert_eq!(channels[0].surface.sample_channel(4, 4, 0), 0.0);
    assert_eq!(channels[1].surface.sample_channel(4, 4, 0), 0.0);
}
