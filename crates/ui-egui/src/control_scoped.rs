//! Capability-scoped file methods of the control channel: `app.place`, `app.replaceContents`
//! and `app.exportLayer`.
//!
//! The commands they stand in for (`file.placeEmbedded`, `layer.smartObjects.replaceContents`,
//! `layer.exportAs`) take an ambient path, so the control channel refuses them. Here the file is
//! read or written through the automation roots the app was launched with, and the command's own
//! body does the rest, as `doc.place` does on the headless server. A pipeline tool uses them to
//! load a published file into the open document, swap it for a newer version, and write a layer
//! or group out as an image.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::input_object;

const NO_READ: &str = "automation read authority is not configured";
const NO_WRITE: &str = "automation write authority is not configured";

/// The request's `path`: a non-empty string.
fn path_of(p: &Value) -> Result<&str, String> {
    match p.get("path") {
        Some(Value::String(path)) if !path.is_empty() => Ok(path),
        Some(Value::String(_)) | None => Err("missing `path`".into()),
        Some(_) => Err("`path` must be a string".into()),
    }
}

/// The request's `layer` (a layer id of the active document), or the active layer without one.
fn layer_of(app: &PhotocraftApp, p: &Value) -> Result<LayerId, String> {
    let st = app.session.active().ok_or("no document open")?;
    let id = match p.get("layer") {
        None => st.active_layer.ok_or("no layer given and no active layer")?,
        Some(v) => LayerId(v.as_u64().ok_or("`layer` must be a layer id")?),
    };
    st.doc.layer(id).map(|_| id).ok_or_else(|| format!("no layer with id {}", id.0))
}

/// An optional boolean field (false when absent).
fn flag(p: &Value, key: &str) -> Result<bool, String> {
    match p.get(key) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// An optional finite number field.
fn number(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| format!("`{key}` must be a number")),
    }
}

/// The active document can be edited now: there is one, and no background job is running on it
/// (the dispatcher's check, for a command body run directly).
fn editable(app: &PhotocraftApp) -> Result<(), String> {
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    app.session.edit_conflict().map_or(Ok(()), Err)
}

fn read(app: &mut PhotocraftApp, path: &str) -> Result<(String, Vec<u8>), String> {
    let read = app.services.automation_read.as_mut().ok_or(NO_READ)?;
    read(path)
}

/// Last-resort guard for a command body run outside the engine's dispatcher, which guards the
/// commands it runs: a panic becomes an error. `Session::edit` commits only after its closure
/// returns, so the document is unchanged.
fn guarded<T>(what: &str, run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .unwrap_or_else(|_| Err(format!("{what} failed with an internal error (logged); the document is unchanged")))
}

/// `app.place {path, scale?, fit?, center?, smartObject?}`: place a file from the read root into
/// the active document as an embedded Smart Object layer (File › Place Embedded). Replies as
/// the command does: `{layer, scale, bounds, smartObject}`, `bounds` being `[left, top, right,
/// bottom]` in document pixels.
pub(crate) fn place(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let p = input_object(p, None, &["path", "scale", "fit", "center", "smartObject"])?;
    let path = path_of(&p)?;
    if number(&p, "scale")?.is_some_and(|scale| scale <= 0.0) {
        return Err("`scale` must be a percentage above 0".into());
    }
    flag(&p, "fit")?;
    flag(&p, "smartObject")?;
    if let Some(center) = p.get("center") {
        let ok = center.as_array().is_some_and(|c| c.len() == 2 && c.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)));
        if !ok {
            return Err("`center` must be [x, y] in document pixels".into());
        }
    }
    editable(app)?;
    let (name, bytes) = read(app, path)?;
    let placed = guarded("Place", || photocraft_engine::file_cmds::place_bytes(&mut app.session, &name, bytes, None, &p).map_err(|e| e.to_string()));
    app.sync_views();
    placed
}

/// `app.replaceContents {path, layer?}`: replace the contents of a Smart Object layer (the
/// active one without `layer`) with a file from the read root, keeping the layer, its transform,
/// filters and masks (Layer › Smart Objects › Replace Contents). Replies `{layer}`.
pub(crate) fn replace_contents(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let p = input_object(p, None, &["path", "layer"])?;
    let path = path_of(&p)?;
    let layer = layer_of(app, &p)?;
    editable(app)?;
    // Before the read: a file is not fetched for a layer that can't take it.
    let smart = app.session.active().and_then(|st| st.doc.layer(layer)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Smart(_)));
    if !smart {
        return Err(format!("layer {} is not a Smart Object", layer.0));
    }
    let (name, bytes) = read(app, path)?;
    let replaced =
        guarded("Replace Contents", || photocraft_engine::smart_cmds::replace_contents_bytes(&mut app.session, layer, &name, bytes).map_err(|e| e.to_string()));
    app.sync_views();
    replaced
}

/// `app.exportLayer {path, layer?, scale?, trim?, overwrite?}`: write one layer or group (the
/// active one without `layer`), rendered alone, to a file beneath the write root, in the format
/// its extension names (Layer › Export As). `trim: false` keeps the document's canvas. The file
/// must be new unless `overwrite: true`, and the file of an open document is never written over.
/// Replies `{path, bytes, width, height, warnings}`.
pub(crate) fn export_layer(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let p = input_object(p, None, &["path", "layer", "scale", "trim", "overwrite"])?;
    let path = path_of(&p)?;
    layer_of(app, &p)?;
    flag(&p, "trim")?;
    let overwrite = flag(&p, "overwrite")?;
    // Fail closed before rendering anything.
    if app.services.automation_export.is_none() {
        return Err(NO_WRITE.into());
    }
    let (bytes, reply) =
        guarded("Export", || photocraft_engine::layer_menu_cmds::export_layer_bytes(&app.session, &p, "app.exportLayer", false).map_err(|e| e.to_string()))?;
    let open: Vec<String> = app.session.documents().iter().filter_map(|d| d.path.clone()).collect();
    let open: Vec<&str> = open.iter().map(String::as_str).collect();
    let write = app.services.automation_export.as_mut().ok_or(NO_WRITE)?;
    write(path, &bytes, overwrite, &open)?;
    Ok(json!({"path": path, "bytes": reply["bytes"], "width": reply["width"], "height": reply["height"], "warnings": reply["warnings"]}))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use photocraft_doc::{LayerContent, SmartSource};

    use super::*;
    use crate::control::{ControlRequest, Outcome, handle};

    fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, &egui::Context::default(), &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    /// A `w` x `h` PNG of one colour.
    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let px: Vec<u8> = (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
        let image = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, px).unwrap();
        photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    }

    type Log = Rc<RefCell<Vec<String>>>;

    /// One export write: the path, the bytes, `overwrite` and the open documents' files.
    struct Export {
        path: String,
        bytes: Vec<u8>,
        overwrite: bool,
        open: Vec<String>,
    }
    type Exports = Rc<RefCell<Vec<Export>>>;

    /// An app whose read root holds `pub/hero_v001.png` (red, 8 x 6) and `pub/hero_v002.png`
    /// (blue, 16 x 12), with the paths read and the export writes made. The write root refuses
    /// `denied/`, a second write to a path without `overwrite`, and an open document's file.
    fn app() -> (PhotocraftApp, Log, Exports) {
        let (reads, exports): (Log, Exports) = Default::default();
        let (r, w) = (reads.clone(), exports.clone());
        let services = crate::Services {
            automation_read: Some(Box::new(move |path: &str| {
                r.borrow_mut().push(path.to_string());
                let bytes = match path {
                    "pub/hero_v001.png" => png(8, 6, [255, 0, 0]),
                    "pub/hero_v002.png" => png(16, 12, [0, 0, 255]),
                    "pub/notes.txt" => b"not an image".to_vec(),
                    _ => return Err(format!("automation path rejected: `{path}`")),
                };
                Ok((path.rsplit('/').next().unwrap_or(path).to_string(), bytes))
            })),
            automation_export: Some(Box::new(move |path: &str, bytes: &[u8], overwrite: bool, open: &[&str]| {
                if path.starts_with("denied/") {
                    return Err(format!("automation path rejected: `{path}`"));
                }
                if open.contains(&path) {
                    return Err(format!("`{path}` is the file of an open document"));
                }
                if !overwrite && w.borrow().iter().any(|e| e.path == path) {
                    return Err(format!("`{path}` already exists"));
                }
                let open = open.iter().map(|o| o.to_string()).collect();
                w.borrow_mut().push(Export { path: path.to_string(), bytes: bytes.to_vec(), overwrite, open });
                Ok(())
            })),
            ..Default::default()
        };
        (PhotocraftApp::new(photocraft_engine::Session::new(), services), reads, exports)
    }

    fn layers(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().doc.layer_count()
    }

    fn source_name(app: &PhotocraftApp, layer: u64) -> String {
        match &app.session.active().unwrap().doc.layer(LayerId(layer)).unwrap().content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Embedded { file_name, .. } => file_name.clone(),
                SmartSource::Linked { path } => format!("linked:{path}"),
            },
            other => format!("not smart: {}", other.kind_name()),
        }
    }

    #[test]
    fn place_reads_beneath_the_read_root_and_adds_a_smart_object() {
        let (mut app, reads, _) = app();
        // No document: nothing is read.
        let r = call(&mut app, "app.place", json!({"path": "pub/hero_v001.png"}));
        assert_eq!(r["error"], "no document open", "{r}");
        assert!(reads.borrow().is_empty());

        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let steps = app.session.active().unwrap().history.entries().len();
        let r = call(&mut app, "app.place", json!({"path": "pub/hero_v001.png", "center": [10, 12]}));
        assert_eq!(r["ok"], true, "{r}");
        let layer = r["result"]["layer"].as_u64().unwrap();
        assert_eq!(r["result"]["smartObject"], true);
        assert_eq!(r["result"]["bounds"], json!([6.0, 9.0, 14.0, 15.0]), "[left, top, right, bottom], centred on [10, 12]: {r}");
        assert_eq!(source_name(&app, layer), "hero_v001.png");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer(LayerId(layer)).unwrap().name, "hero_v001");
        assert_eq!(st.active_layer, Some(LayerId(layer)));
        assert_eq!(st.history.entries().len(), steps + 1, "one undo step");
        assert_eq!(*reads.borrow(), ["pub/hero_v001.png"]);

        // Options are the command's own; a pixel layer on request.
        let r = call(&mut app, "app.place", json!({"path": "pub/hero_v002.png", "scale": 50, "smartObject": false}));
        assert_eq!((r["result"]["scale"].as_f64(), r["result"]["smartObject"].as_bool()), (Some(50.0), Some(false)), "{r}");
    }

    #[test]
    fn place_fails_closed_and_leaves_the_document_alone() {
        let (mut app, reads, _) = app();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let before = (layers(&app), app.session.active().unwrap().revision);
        for (params, want) in [
            (json!({}), "missing `path`"),
            (json!({"path": ""}), "missing `path`"),
            (json!({"path": 7}), "`path` must be a string"),
            (json!({"path": "pub/hero_v001.png", "linked": true}), "unknown field `linked`"),
            (json!({"path": "pub/hero_v001.png", "scale": "big"}), "`scale` must be a number"),
            (json!({"path": "pub/hero_v001.png", "scale": 0}), "`scale` must be a percentage above 0"),
            (json!({"path": "pub/hero_v001.png", "scale": -50}), "`scale` must be a percentage above 0"),
            (json!({"path": "pub/hero_v001.png", "fit": 1}), "`fit` must be true or false"),
            (json!({"path": "pub/hero_v001.png", "smartObject": "no"}), "`smartObject` must be true or false"),
            (json!({"path": "pub/hero_v001.png", "center": [1]}), "`center` must be [x, y]"),
            (json!({"path": "pub/hero_v001.png", "center": ["a", "b"]}), "`center` must be [x, y]"),
            (json!({"path": "pub/hero_v001.png", "center": {"x": 1}}), "`center` must be [x, y]"),
        ] {
            let r = call(&mut app, "app.place", params.clone());
            assert!(r["error"].as_str().is_some_and(|e| e.contains(want)), "{params}: {r}");
        }
        assert!(reads.borrow().is_empty(), "bad params never reach the file system");
        // The root's refusal, a file that isn't an image and an absurd size are errors too.
        for params in [
            json!({"path": "/etc/passwd"}),
            json!({"path": "../outside.png"}),
            json!({"path": "pub/notes.txt"}),
            json!({"path": "pub/hero_v001.png", "scale": 1e12}),
            json!({"path": "pub/hero_v001.png", "center": [1e300, 0]}),
        ] {
            let r = call(&mut app, "app.place", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
        }
        assert_eq!((layers(&app), app.session.active().unwrap().revision), before, "nothing was placed");

        // Without a read root the method is refused, even with a document open.
        let mut bare = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        bare.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        for method in ["app.place", "app.replaceContents"] {
            let r = call(&mut bare, method, json!({"path": "pub/hero_v001.png"}));
            assert_eq!(r["ok"], false, "{method}: {r}");
        }
        let r = call(&mut bare, "app.place", json!({"path": "pub/hero_v001.png"}));
        assert_eq!(r["error"], NO_READ);
    }

    #[test]
    fn replace_contents_swaps_the_file_and_keeps_the_layer() {
        let (mut app, reads, _) = app();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let layer = call(&mut app, "app.place", json!({"path": "pub/hero_v001.png"}))["result"]["layer"].as_u64().unwrap();
        app.run("layer.new.layer", json!({"name": "paint"})).unwrap();
        let paint = app.session.active().unwrap().active_layer.unwrap().0;
        let count = layers(&app);

        // By id, without making it the active layer.
        let r = call(&mut app, "app.replaceContents", json!({"path": "pub/hero_v002.png", "layer": layer}));
        assert_eq!(r["result"]["layer"], layer, "{r}");
        assert_eq!(source_name(&app, layer), "hero_v002.png");
        assert_eq!(layers(&app), count, "the same layer, not a new one");
        assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(paint)), "the active layer stays");
        // One undo step back to the old contents.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(source_name(&app, layer), "hero_v001.png");

        // The active layer by default; a pixel layer is refused before its file is read.
        reads.borrow_mut().clear();
        let r = call(&mut app, "app.replaceContents", json!({"path": "pub/hero_v002.png"}));
        assert_eq!(r["error"], format!("layer {paint} is not a Smart Object"), "{r}");
        assert!(reads.borrow().is_empty());
        for (params, want) in [
            (json!({"layer": layer}), "missing `path`"),
            (json!({"path": "pub/hero_v002.png", "layer": 999_999}), "no layer with id 999999"),
            (json!({"path": "pub/hero_v002.png", "layer": "top"}), "`layer` must be a layer id"),
            (json!({"path": "pub/hero_v002.png", "layer": -3}), "`layer` must be a layer id"),
            (json!({"path": "pub/hero_v002.png", "layer": layer, "scale": 2}), "unknown field `scale`"),
        ] {
            let r = call(&mut app, "app.replaceContents", params.clone());
            assert!(r["error"].as_str().is_some_and(|e| e.contains(want)), "{params}: {r}");
        }
        // A file the root refuses, or that isn't an image, leaves the contents as they were.
        for path in ["../v003.png", "pub/notes.txt"] {
            let r = call(&mut app, "app.replaceContents", json!({"path": path, "layer": layer}));
            assert_eq!(r["ok"], false, "{path}: {r}");
        }
        assert_eq!(source_name(&app, layer), "hero_v001.png");
    }

    #[test]
    fn export_layer_writes_beneath_the_write_root() {
        let (mut app, _, exports) = app();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let layer = call(&mut app, "app.place", json!({"path": "pub/hero_v001.png"}))["result"]["layer"].as_u64().unwrap();
        app.run("layer.new.layer", json!({"name": "empty"})).unwrap();
        let revision = app.session.active().unwrap().revision;

        // Trimmed to the layer's pixels by default; the document's canvas with `trim: false`.
        let r = call(&mut app, "app.exportLayer", json!({"path": "out/hero.png", "layer": layer}));
        assert_eq!((r["result"]["width"].as_u64(), r["result"]["height"].as_u64()), (Some(8), Some(6)), "{r}");
        let r = call(&mut app, "app.exportLayer", json!({"path": "out/hero_canvas.png", "layer": layer, "trim": false}));
        assert_eq!((r["result"]["width"].as_u64(), r["result"]["height"].as_u64()), (Some(64), Some(48)), "{r}");
        assert_eq!(r["result"]["path"], "out/hero_canvas.png");
        {
            let written = exports.borrow();
            assert_eq!(written.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["out/hero.png", "out/hero_canvas.png"]);
            for e in written.iter() {
                assert!(e.bytes.starts_with(&[0x89, b'P', b'N', b'G']), "{}", e.path);
                assert!(!e.overwrite, "{}: a new file unless asked", e.path);
            }
            assert_eq!(r["result"]["bytes"].as_u64(), Some(written[1].bytes.len() as u64));
        }
        assert_eq!(app.session.active().unwrap().revision, revision, "an export changes nothing");

        // The same path again needs `overwrite: true`.
        let r = call(&mut app, "app.exportLayer", json!({"path": "out/hero.png", "layer": layer}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("already exists")), "{r}");
        let r = call(&mut app, "app.exportLayer", json!({"path": "out/hero.png", "layer": layer, "overwrite": true}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(exports.borrow().last().is_some_and(|e| e.overwrite));

        // Errors write nothing: bad params, an empty layer, a path the root refuses.
        let empty = app.session.active().unwrap().active_layer.unwrap().0;
        for (params, want) in [
            (json!({"layer": layer}), "missing `path`"),
            (json!({"path": "out/a.png", "layer": 999_999}), "no layer with id 999999"),
            (json!({"path": "out/a.png", "layer": [layer]}), "`layer` must be a layer id"),
            (json!({"path": "out/a.png", "layer": layer, "trim": "no"}), "`trim` must be true or false"),
            (json!({"path": "out/a.png", "layer": layer, "overwrite": 1}), "`overwrite` must be true or false"),
            (json!({"path": "out/a.png", "layer": layer, "scale": "2x"}), "`scale` must be a percentage from 1 to 1000"),
            (json!({"path": "out/a.png", "layer": layer, "scale": 5000}), "`scale` must be a percentage from 1 to 1000"),
            (json!({"path": "out/a.png", "layer": layer, "format": "png"}), "unknown field `format`"),
            (json!({"path": "out/a.unknownformat", "layer": layer}), ""),
            (json!({"path": "out/a.png", "layer": empty}), "the layer is empty"),
            (json!({"path": "denied/a.png", "layer": layer}), "automation path rejected"),
        ] {
            let r = call(&mut app, "app.exportLayer", params.clone());
            assert!(r["error"].as_str().is_some_and(|e| e.contains(want)), "{params}: {r}");
        }
        assert_eq!(exports.borrow().len(), 3);

        // Without a write root it fails closed, whatever ambient writer the app has.
        let services = crate::Services { write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))), ..Default::default() };
        let mut bare = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        bare.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let r = call(&mut bare, "app.exportLayer", json!({"path": "out/a.png"}));
        assert_eq!(r["error"], NO_WRITE, "{r}");
    }

    /// The write root is told every open document's file, so an export never writes over the
    /// workfile, even with `overwrite: true`.
    #[test]
    fn export_layer_never_writes_over_an_open_document() {
        let (mut app, _, exports) = app();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let layer = call(&mut app, "app.place", json!({"path": "pub/hero_v001.png"}))["result"]["layer"].as_u64().unwrap();
        app.session.active_mut().unwrap().saved_to("work/shot_v003.psd".into());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.session.active_mut().unwrap().saved_to("/abs/other.psd".into());
        app.session.set_active(0);
        let r = call(&mut app, "app.exportLayer", json!({"path": "work/shot_v003.psd", "layer": layer, "overwrite": true}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("open document")), "{r}");
        let r = call(&mut app, "app.exportLayer", json!({"path": "work/hero.png", "layer": layer}));
        assert_eq!(r["ok"], true, "{r}");
        let written = exports.borrow();
        let open = &written.last().unwrap().open;
        assert!(open.contains(&"work/shot_v003.psd".to_string()) && open.contains(&"/abs/other.psd".to_string()), "{open:?}");
    }

    /// The ambient commands stay refused: the scoped methods are the only way to a file.
    #[test]
    fn the_path_commands_stay_refused() {
        fn deny(id: &str, _: &Value) -> Result<(), String> {
            if id.starts_with("file.") || id == "layer.exportAs" || id.starts_with("layer.smartObjects.") { Err(format!("`{id}` is disabled")) } else { Ok(()) }
        }
        let (mut app, reads, exports) = app();
        app.services.automation_command = Some(Box::new(deny));
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        for (command, params) in [
            ("file.placeEmbedded", json!({"path": "pub/hero_v001.png"})),
            ("layer.smartObjects.replaceContents", json!({"path": "pub/hero_v002.png"})),
            ("layer.exportAs", json!({"path": "out/a.png"})),
        ] {
            let r = call(&mut app, "engine.execute", json!({"command": command, "params": params}));
            assert_eq!(r["error"], format!("`{command}` is disabled"), "{r}");
        }
        assert!(reads.borrow().is_empty() && exports.borrow().is_empty());
        assert_eq!(call(&mut app, "app.place", json!({"path": "pub/hero_v001.png"}))["ok"], true);
    }
}
