//! #14193: the four Fill Settings open at their defaults each time; Sampling, Output To and the
//! overlay settings are still remembered.

use super::*;

fn app_with_selection() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 200, "height": 160, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..160 {
                for x in 0..200 {
                    let v = ((x / 6 + y / 6) % 2) as f32 * 0.5 + x as f32 / 400.0;
                    s.write_pixel(x, y, &[v, 0.3, 1.0 - v, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 90, "y": 70, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    app
}

fn ws(app: &PhotocraftApp) -> &CafWorkspace {
    app.distort.caf.as_ref().unwrap()
}

fn fresh() -> PhotocraftApp {
    let mut app = app_with_selection();
    app.session.prefs.edit(|p| p.dialogs.remove(REMEMBERED));
    app
}

fn non_default_fill() -> Value {
    json!({
        "colorAdaptation": "high",
        "rotationAdaptation": "medium",
        "scale": true,
        "mirror": true,
        "sampling": "rectangular",
        "output": "current",
    })
}

fn assert_defaults_and_remembered(app: &PhotocraftApp) {
    let o = &ws(app).opts;
    assert_eq!(o.color_adaptation, "default");
    assert_eq!(o.rotation, "none");
    assert!(!o.scale);
    assert!(!o.mirror);
    assert_eq!(o.sampling, "rectangular");
    assert_eq!(o.output, "current");
}

#[test]
fn fill_settings_reset_after_ok() {
    let mut app = fresh();
    open(&mut app).unwrap();
    let mut set = non_default_fill();
    set["waitPreview"] = json!(true);
    control(&mut app, &set).unwrap();
    control(&mut app, &json!({"commit": true})).unwrap();
    assert!(app.distort.caf.is_none());
    open(&mut app).unwrap();
    assert_defaults_and_remembered(&app);
}

#[test]
fn fill_settings_reset_after_apply() {
    let mut app = fresh();
    open(&mut app).unwrap();
    let mut set = non_default_fill();
    set["waitPreview"] = json!(true);
    control(&mut app, &set).unwrap();
    control(&mut app, &json!({"apply": true})).unwrap();
    cancel(&mut app);
    open(&mut app).unwrap();
    assert_defaults_and_remembered(&app);
}

#[test]
fn fill_settings_reset_after_cancel() {
    let mut app = fresh();
    open(&mut app).unwrap();
    control(&mut app, &non_default_fill()).unwrap();
    cancel(&mut app);
    open(&mut app).unwrap();
    assert_defaults_and_remembered(&app);
}

#[test]
fn an_old_remembered_entry_does_not_bring_the_fill_settings_back() {
    let mut app = fresh();
    let old = json!({
        "colorAdaptation": "veryHigh",
        "rotationAdaptation": "full",
        "scale": true,
        "mirror": true,
        "sampling": "rectangular",
        "output": "current",
    });
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), old));
    open(&mut app).unwrap();
    assert_defaults_and_remembered(&app);
}

#[test]
fn overlay_settings_still_persist_after_cancel() {
    let mut app = fresh();
    open(&mut app).unwrap();
    control(&mut app, &json!({"showOverlay": false, "opacity": 30, "color": [200, 40, 40], "excluded": true})).unwrap();
    cancel(&mut app);
    open(&mut app).unwrap();
    let o = &ws(&app).opts;
    assert!(!o.show_overlay);
    assert_eq!(o.opacity, 30.0);
    assert_eq!(o.color, [200, 40, 40]);
    assert!(o.excluded);
}
