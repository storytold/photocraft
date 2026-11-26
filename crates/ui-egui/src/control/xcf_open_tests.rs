//! Real app.open dispatch: complete options reach typed XCF admission.
use super::{ControlRequest, Reply, handle};
use crate::{PhotocraftApp, Services};
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// A single raw RGBA16 tile in a version-10 file. Structural words are always BE;
/// the intentionally asymmetric sample bytes let the command's explicit choice be observed.
fn historical_rgba16() -> Vec<u8> {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn patch(out: &mut [u8], slot: usize, target: usize) {
        out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v010\0".to_vec();
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // no compression
    for value in [0, 0] {
        word(&mut out, value);
    }
    let layer = out.len();
    for value in [0, 0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, layer, at);
    for value in [1, 1, 1, 2] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"L\0");
    for value in [0, 0] {
        word(&mut out, value);
    }
    let hierarchy = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, hierarchy, at);
    for value in [1, 1, 8] {
        word(&mut out, value);
    }
    let level = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, level, at);
    for value in [1, 1] {
        word(&mut out, value);
    }
    let tile = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, tile, at);
    out.extend_from_slice(&[0x12, 0x34, 0x23, 0x45, 0x34, 0x56, 0xff, 0xff]);
    out
}

fn call(app: &mut PhotocraftApp, params: Value) -> Value {
    let ctx = app.ctx().clone();
    let (request, _reply) = ControlRequest::new("app.open", params);
    match handle(app, &ctx, &request) {
        Reply::Done(reply) => reply,
        _ => panic!("open must finish synchronously"),
    }
}

#[test]
fn app_open_forwards_explicit_historical_order_to_real_import_pixels_and_warnings() {
    let source = historical_rgba16();
    let original = source.clone();
    let services = Services {
        automation_read: Some(Box::new(move |path| {
            if path != "source.xcf" {
                return Err("read scope refused".into());
            }
            Ok(crate::AutomationReadResult { name: "source.xcf".into(), bytes: source.clone(), save_path: None })
        })),
        import: Some(Box::new(|_, _, _| Err("expected typed XCF admission".into()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    for (order, expected) in [("little-endian", [0x3412u16, 0x4523, 0x5634]), ("big-endian", [0x1234u16, 0x2345, 0x3456])] {
        let params = json!({"path":"source.xcf","approveHighMemory":true,"historicalXcfSampleOrder":order});
        let options =
            photocraft_engine::file_cmds::import_options_from_params(photocraft_engine::jobs::import_options(app.session.prefs()), &params, "app.open")
                .unwrap();
        let expected_warnings = photocraft_io::import_with_options("source.xcf", &original, &options).unwrap().warnings;
        let reply = call(&mut app, params);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["warnings"], json!(expected_warnings));
        let opened = app.session.active().unwrap();
        let actual = opened.doc.layers[0].surface().unwrap().pixel(0, 0);
        assert_eq!(actual, [f32::from(expected[0]) / 65535., f32::from(expected[1]) / 65535., f32::from(expected[2]) / 65535., 1.]);
        assert!(opened.source_read_only);
        assert!(opened.path.is_none());
        assert!(app.session.prefs().script_events.enabled);
    }
    assert_eq!(original, historical_rgba16());
}

#[test]
fn app_open_validates_order_before_source_acquisition_and_keeps_denials_atomic() {
    let reads = Rc::new(Cell::new(0));
    let read_count = reads.clone();
    let imports = Rc::new(Cell::new(0));
    let import_count = imports.clone();
    let services = Services {
        automation_read: Some(Box::new(move |path| {
            read_count.set(read_count.get() + 1);
            if path != "source.xcf" {
                return Err("read scope refused".into());
            }
            Ok(crate::AutomationReadResult { name: path.into(), bytes: historical_rgba16(), save_path: None })
        })),
        import: Some(Box::new(move |name, bytes, options| {
            import_count.set(import_count.get() + 1);
            photocraft_io::import_with_options(name, bytes, options).map(|imported| (imported.document, imported.warnings)).map_err(|error| error.to_string())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    let before = app.session.active().unwrap().doc.clone();
    let recent = app.ui.recent_files.as_slice().to_vec();
    for invalid in [json!("native"), json!(1), json!(false), json!({})] {
        let reply = call(&mut app, json!({"path":"source.xcf","historicalXcfSampleOrder":invalid}));
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply.to_string().contains("historicalXcfSampleOrder"));
        assert_eq!(reads.get(), 0);
        assert_eq!(imports.get(), 0);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
    let reply = call(&mut app, json!({"path":"source.xcf","approveHighMemory":"true"}));
    assert!(reply.to_string().contains("expected boolean for `approveHighMemory`"));
    let reply = call(&mut app, json!({"historicalXcfSampleOrder":"big-endian"}));
    assert!(reply.to_string().contains("missing `path`"));
    assert_eq!(reads.get(), 0);
    for params in [
        json!({"path":"source.xcf"}),
        json!({"path":"source.xcf","historicalXcfSampleOrder":null}),
        json!({"path":"../outside.xcf","historicalXcfSampleOrder":"little-endian"}),
    ] {
        let reply = call(&mut app, params);
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.ui.recent_files.as_slice(), recent);
        assert!(app.session.prefs().script_events.enabled);
    }
    assert_eq!(reads.get(), 3);
    assert_eq!(imports.get(), 0, "XCF uses typed admission without calling the tuple adapter");
}
