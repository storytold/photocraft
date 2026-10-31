use super::*;
use photocraft_geom::Rect;

/// G'MIC state is per process: tests that set it take turns.
static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 28})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 40, 28))
}

#[test]
fn bad_params_are_errors_not_panics() {
    let _l = lock();
    let mut s = Session::new();
    // Hermetic: never the G'MIC installed on this machine.
    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": "/no/such/gmic"})).unwrap();
    for (cmd, p) in [
        (RUN, json!({})),
        (RUN, json!({"command": 5})),
        (RUN, json!({"command": "   "})),
        (RUN, json!({"command": "a\u{0}b"})),
        (RUN, json!({"command": "mirror x"})),
        (FILTER, json!({})),
        (FILTER, json!({"id": 7})),
        (FILTER, json!(null)),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    // Loads in the background: answers at once, and the failure shows up in the state.
    let _ = s.execute("gmic.list", json!({"refresh": "yes", "category": 3}));
    assert!(s.execute("gmic.list", json!({"wait": true})).is_err());
    assert!(s.execute("gmic.status", json!({"refresh": 1})).is_ok());
    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": ""})).unwrap();
}

#[test]
fn a_missing_gmic_is_reported() {
    let _l = lock();
    let mut s = doc();
    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": "/no/such/gmic"})).unwrap();
    let st = s.execute("gmic.status", json!({})).unwrap();
    assert_eq!(st["found"], json!(false));
    assert!(st["error"].as_str().unwrap().contains("not a file"), "{st}");
    let before = pixels(&s);
    let e = s.execute(RUN, json!({"command": "mirror x"})).unwrap_err();
    assert!(e.to_string().contains("not a file"), "{e}");
    assert_eq!(pixels(&s), before);
    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": ""})).unwrap();
}

#[cfg(unix)]
#[test]
fn runs_pipelines_and_catalogue_filters_through_a_stand_in_gmic() {
    use std::os::unix::fs::PermissionsExt;
    let _l = lock();
    let dir = std::env::temp_dir().join(format!("photocraft-gmic-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Result: a 40×28 RGBA image of 255s (opaque white).
    let mut white = b"1 float32 little_endian\n40 28 1 4\n".to_vec();
    for _ in 0..40 * 28 * 4 {
        white.extend_from_slice(&255f32.to_le_bytes());
    }
    std::fs::write(dir.join("white.cimg"), white).unwrap();
    let catalog = r#"{"format_version":"gmic_json_1.0","gmic_version":"9.9.9","categories":[{"name":"Test","filters":[
        {"name":"Glow","command":"fx_test_glow","command_preview":"fx_test_glow","parameters":[
          {"type":"float","name":"Amount","default":"1","min":"0","max":"10","pos":"1"},
          {"type":"bool","name":"Soft","default":"1","pos":"2"}]}]}]}"#;
    std::fs::write(dir.join("catalog.json"), catalog).unwrap();
    let d = dir.display();
    let script = format!(
        "#!/bin/sh\ncase \"$3\" in\n  echo) echo 9.9.9 >&2;;\n  update) ;;\n  parse_gui) cp {d}/catalog.json catalog.json;;\n  *) cp run.gmic {d}/seen.gmic; cp {d}/white.cimg out.cimg;;\nesac\n"
    );
    // Written by a `cp` child: a handle open for writing in this process could leak into a child
    // another test thread forks, and running the script would then fail with ETXTBSY.
    let gmic = dir.join("gmic");
    let staged = dir.join("gmic.staged");
    std::fs::write(&staged, script).unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(std::process::Command::new("cp").arg(&staged).arg(&gmic).status().unwrap().success());
    set_cache_dir(Some(dir.join("cache")));

    let mut s = doc();
    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": gmic.to_str().unwrap()})).unwrap();
    let st = s.execute("gmic.status", json!({})).unwrap();
    assert_eq!(st["found"], json!(true), "{st}");
    assert_eq!(st["version"], json!("9.9.9"));

    // A raw pipeline: one undo step.
    let before = pixels(&s);
    s.execute(RUN, json!({"command": "fx_anything 1,2"})).unwrap();
    assert!(pixels(&s).iter().all(|v| (*v - 1.0).abs() < 1e-6));
    assert_eq!(std::fs::read_to_string(dir.join("seen.gmic")).unwrap(), "photocraft_run :\n  fx_anything 1,2\n");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(pixels(&s), before);

    // The selection limits the result.
    s.edit("sel", |doc, _| {
        let mut sel = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 10, 28), &[1.0]);
        doc.selection = Some(sel);
        Ok(())
    })
    .unwrap();
    s.execute(RUN, json!({"command": "x"})).unwrap();
    let layer = |s: &Session| s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().pixel(30, 5);
    assert_eq!(layer(&s)[3], 0.0, "outside the selection is unchanged");
    s.execute("edit.undo", json!({})).unwrap();
    s.edit("deselect", |doc, _| {
        doc.selection = None;
        Ok(())
    })
    .unwrap();

    // The catalogue, and a catalogue filter with validated parameters.
    let list = s.execute("gmic.list", json!({"wait": true})).unwrap();
    assert_eq!(list["filters"][0]["id"], json!("Test/Glow"));
    assert_eq!(list["filters"][0]["params"], json!(r#"{"amount":0..10=1,"soft":bool=true}"#));
    assert!(dir.join("cache/catalog-9.9.9.json").exists(), "the catalogue is cached");
    s.execute(FILTER, json!({"id": "Test/Glow", "params": {"amount": 30, "soft": false}})).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("seen.gmic")).unwrap(), "photocraft_run :\n  fx_test_glow 10,0\n");
    let unchanged = s.active().unwrap().doc.clone();
    assert!(s.execute(FILTER, json!({"id": "Test/Glow", "amount": "lots"})).is_err());
    assert!(s.execute(FILTER, json!({"id": "Test/Nope"})).is_err());
    // Loaded: a plain list answers at once.
    assert_eq!(s.execute("gmic.list", json!({"category": "Test"})).unwrap()["filters"].as_array().map(Vec::len), Some(1));
    assert!(Arc::ptr_eq(&unchanged, &s.active().unwrap().doc), "refused before any edit");

    // Smart objects record catalogue filters, never raw pipelines.
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.execute(RUN, json!({"command": "x"})).unwrap_err().to_string().contains("smart filter"));
    s.execute(FILTER, json!({"id": "Test/Glow", "amount": 2})).unwrap();
    let d = s.active().unwrap();
    let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
    assert_eq!(sm.smart_filters[0].command, FILTER);
    assert_eq!(sm.smart_filters[0].params["id"], json!("Test/Glow"));
    // Re-rendering from the recorded filter goes through the catalogue.
    let surf = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let out = apply_to_surface(FILTER, &sm.smart_filters[0].params, &surf, None, Rect::new(0, 0, 40, 28)).unwrap();
    assert_eq!(out.pixel(3, 3), vec![1.0, 1.0, 1.0, 1.0]);
    assert!(apply_to_surface(RUN, &json!({"command": "x"}), &surf, None, Rect::new(0, 0, 40, 28)).is_none());

    s.execute("prefs.set", json!({"path": "plugIns.gmicPath", "value": ""})).unwrap();
    set_cache_dir(None);
    let _ = std::fs::remove_dir_all(&dir);
}
