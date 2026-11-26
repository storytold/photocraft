//! Inline Open must retain the same native view and protected origin as worker admission.
use std::{cell::Cell, rc::Rc};

use photocraft_doc::{ColorMode, Document, SampleType};
use photocraft_engine::{Session, channel_cmds::ChannelTarget};
use photocraft_geom::Size;
use serde_json::json;

use crate::{PhotocraftApp, Services};

#[derive(Default)]
struct Wire(Vec<u8>);
impl Wire {
    fn word(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }
    fn property(&mut self, kind: u32, bytes: &[u8]) {
        self.word(kind);
        self.word(bytes.len() as u32);
        self.0.extend_from_slice(bytes);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.word(0);
        at
    }
    fn point_here(&mut self, at: usize) {
        let offset = self.0.len() as u32;
        self.0[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    fn string(&mut self, text: &str) {
        self.word(text.len() as u32 + 1);
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
    }
    fn pixels(&mut self, channels: u32, pixels: &[u8]) {
        for value in [1, 1, channels] {
            self.word(value);
        }
        let level = self.slot();
        self.word(0);
        self.point_here(level);
        self.word(1);
        self.word(1);
        let tile = self.slot();
        self.word(0);
        self.point_here(tile);
        self.0.extend_from_slice(pixels);
    }
}

/// Original v3/U8 file: two selected layers, a selected/visible alpha channel and a sampler.
/// Raw tile bytes and explicit big-endian pointers avoid importer-derived fixture expectations.
fn saved_view_xcf() -> Vec<u8> {
    let mut w = Wire(b"gimp xcf v003\0".to_vec());
    for value in [1, 1, 0] {
        w.word(value);
    }
    w.property(17, &[0]);
    w.property(27, &[0; 8]);
    w.end();
    let layers = [w.slot(), w.slot()];
    w.word(0);
    let channel = w.slot();
    w.word(0);
    for ((slot, name), pixel) in layers.into_iter().zip(["Selected front", "Selected back"]).zip([[0, 255, 0, 255], [255, 0, 0, 255]]) {
        w.point_here(slot);
        for value in [1, 1, 1] {
            w.word(value);
        }
        w.string(name);
        w.property(2, &[]);
        w.property(8, &1_u32.to_be_bytes());
        w.end();
        let hierarchy = w.slot();
        w.word(0);
        w.point_here(hierarchy);
        w.pixels(4, &pixel);
    }
    w.point_here(channel);
    w.word(1);
    w.word(1);
    w.string("Selected alpha");
    w.property(3, &[]);
    w.property(8, &1_u32.to_be_bytes());
    w.end();
    let hierarchy = w.slot();
    w.point_here(hierarchy);
    w.pixels(1, &[191]);
    w.0
}

// Deterministic gzip of exactly saved_view_xcf(), generated independently with a standard
// encoder. The prefix classifier must recognize the contents even under a misleading name.
const WRAPPED_XCF: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 75, 207, 204, 45, 80, 168, 72, 78, 83, 40, 51, 48, 48, 102, 0, 2, 70, 40, 6, 1, 65, 24, 91, 26, 136, 57, 24, 48, 129,
    63, 16, 239, 130, 48, 25, 85, 160, 98, 140, 72, 152, 63, 56, 53, 39, 53, 185, 36, 53, 69, 33, 173, 40, 63, 175, 4, 36, 205, 4, 85, 6, 50, 142, 5, 201, 46,
    16, 152, 132, 102, 4, 72, 126, 25, 154, 216, 54, 48, 239, 63, 16, 162, 90, 197, 7, 183, 42, 41, 49, 57, 155, 144, 77, 127, 48, 109, 98, 20, 64, 21, 99, 84,
    128, 88, 132, 98, 19, 194, 67, 137, 57, 5, 25, 137, 32, 21, 204, 184, 173, 97, 140, 71, 117, 36, 99, 49, 154, 21, 205, 32, 206, 126, 0, 118, 207, 135, 82,
    132, 1, 0, 0,
];
const BZIP_XCF: &[u8] = &[
    66, 90, 104, 57, 49, 65, 89, 38, 83, 89, 222, 170, 45, 59, 0, 0, 155, 255, 248, 253, 193, 224, 8, 68, 0, 72, 0, 0, 0, 136, 0, 191, 239, 221, 64, 8, 0, 16,
    0, 1, 0, 1, 16, 128, 4, 160, 0, 138, 12, 137, 19, 70, 141, 24, 140, 128, 200, 245, 26, 61, 38, 212, 217, 65, 138, 40, 211, 35, 4, 200, 208, 192, 140, 76,
    154, 104, 197, 62, 239, 182, 92, 107, 153, 232, 63, 37, 143, 134, 118, 190, 170, 133, 0, 89, 57, 228, 206, 5, 145, 190, 69, 66, 67, 9, 58, 10, 133, 52, 49,
    136, 39, 25, 83, 73, 33, 173, 44, 32, 0, 226, 36, 18, 145, 3, 138, 65, 20, 18, 68, 137, 249, 168, 32, 176, 160, 18, 170, 171, 79, 167, 179, 54, 134, 134,
    164, 142, 194, 225, 15, 14, 7, 35, 129, 113, 198, 104, 211, 148, 154, 78, 167, 194, 48, 162, 218, 227, 57, 218, 25, 30, 128, 34, 21, 254, 46, 228, 138,
    112, 161, 33, 189, 84, 90, 118,
];
const XZ_XCF: &[u8] = &[
    253, 55, 122, 88, 90, 0, 0, 4, 230, 214, 180, 70, 2, 0, 33, 1, 22, 0, 0, 0, 116, 47, 229, 163, 224, 1, 131, 0, 125, 93, 0, 51, 154, 73, 251, 227, 151, 228,
    69, 84, 21, 244, 125, 131, 107, 137, 144, 31, 233, 164, 61, 148, 10, 11, 250, 170, 125, 130, 62, 64, 156, 241, 6, 180, 92, 12, 120, 226, 29, 228, 225, 79,
    240, 13, 146, 65, 183, 3, 214, 54, 210, 24, 134, 204, 20, 55, 90, 61, 93, 10, 178, 232, 93, 36, 39, 59, 69, 211, 23, 249, 80, 206, 87, 113, 123, 29, 83,
    92, 216, 49, 248, 113, 200, 79, 32, 249, 136, 166, 54, 37, 179, 97, 73, 96, 209, 20, 50, 205, 157, 25, 100, 91, 238, 2, 128, 54, 195, 129, 48, 240, 79,
    254, 242, 45, 41, 68, 73, 107, 102, 61, 4, 241, 224, 94, 228, 131, 0, 0, 0, 0, 10, 104, 107, 178, 82, 45, 93, 221, 0, 1, 153, 1, 132, 3, 0, 0, 211, 37, 33,
    164, 177, 196, 103, 251, 2, 0, 0, 0, 0, 4, 89, 90,
];
const WRAPPED_SVG: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 179, 41, 46, 75, 87, 168, 200, 205, 201, 43, 182, 85, 202, 40, 41, 41, 176, 210, 215, 47, 47, 47, 215, 43, 55, 214, 203,
    47, 74, 215, 55, 50, 48, 48, 208, 7, 170, 80, 82, 40, 207, 76, 41, 201, 176, 85, 50, 84, 82, 200, 72, 205, 76, 207, 40, 1, 49, 245, 237, 0, 90, 22, 239,
    220, 62, 0, 0, 0,
];

fn assert_saved_view(app: &PhotocraftApp, path: &str) {
    let state = app.session.active().unwrap();
    assert!(state.source_read_only);
    assert_eq!(state.path, None);
    assert_eq!(state.protected_import_source.as_deref(), Some(path));
    assert!(photocraft_engine::file_cmds::check_import_destination(state, path).is_err());
    assert_eq!(state.selected_layers.len(), 2);
    assert_eq!(state.selected_layers.iter().map(|id| state.doc.layer(*id).unwrap().name.as_str()).collect::<Vec<_>>(), ["Selected front", "Selected back"]);
    assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Selected front");
    assert_eq!(state.channel_view.target, ChannelTarget::Alpha(0));
    assert!(state.channel_view.alpha_shown(0));
    assert_eq!(state.color_samplers, [[0., 0.]]);
    assert_eq!(photocraft_compose::flatten(&state.doc).get(0, 0), [0., 1., 0., 1.]);
    assert_eq!(state.history.past_len(), 0);
    assert!(!state.is_dirty());
    let native = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
    let reopened = photocraft_io::import("saved.pcraft", &native).unwrap();
    assert!(reopened.initial_view.is_none());
    assert!(!reopened.source_read_only);
    assert_eq!(reopened.document.layers, state.doc.layers);
}

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|result| (result.document, result.warnings)).map_err(|error| error.to_string())
            })),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app.session.execute("edit.colorSettings", json!({"askOnMissing":false,"askOnMismatch":false})).unwrap();
    app
}

#[test]
fn inline_xcf_open_preserves_view_and_protected_source() {
    let plain = saved_view_xcf();
    for (name, bytes) in [
        ("saved.xcf", plain.as_slice()),
        ("renamed.bin", plain.as_slice()),
        ("renamed.bin", WRAPPED_XCF),
        ("saved.xcf.gz", WRAPPED_XCF),
        ("renamed.bin", BZIP_XCF),
        ("renamed.bin", XZ_XCF),
    ] {
        let mut app = app();
        let path = format!("/imports/{name}");
        app.open_bytes_from(name, bytes, Some(&path)).unwrap();
        assert_eq!(app.session.documents().len(), 1);
        assert_saved_view(&app, &path);
        assert_eq!(app.ui.recent_files.first(), Some(&path));
    }
}

#[test]
fn inline_xcf_profile_hold_preserves_view_until_confirmed() {
    for accept in [true, false] {
        let mut app = app();
        app.session.execute("edit.colorSettings", json!({"askOnMissing":true})).unwrap();
        app.open_bytes_from("saved.xcf", &saved_view_xcf(), Some("/imports/saved.xcf")).unwrap();
        assert!(app.session.documents().is_empty());
        assert_eq!(app.held_opens.len(), 1);
        assert!(app.held_opens[0].read_only && app.held_opens[0].initial_view.is_some());
        assert_eq!(app.held_opens[0].protected_import_source.as_deref(), Some("/imports/saved.xcf"));
        crate::open_prompt::answer(&mut app, accept.then_some("preserve")).unwrap();
        assert!(app.held_opens.is_empty());
        if accept {
            assert_saved_view(&app, "/imports/saved.xcf");
        } else {
            assert!(app.session.documents().is_empty());
        }
    }
}

#[test]
fn inline_non_xcf_wrappers_retain_configured_import_service() {
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    let mut app = app();
    app.services.import = Some(Box::new(move |name, _, _| {
        seen.set(seen.get() + 1);
        Ok((Document::new(name, Size::new(1, 1), ColorMode::Rgb, SampleType::U8), vec!["custom importer warning".into()]))
    }));
    for (name, bytes) in [("custom.svgz", WRAPPED_SVG), ("unrelated.bin", &[0x1f, 0x8b][..]), ("custom.bin", b"custom".as_slice())] {
        assert!(!photocraft_io::is_xcf_input(name, bytes));
        assert_eq!(app.open_bytes(name, bytes).unwrap(), ["custom importer warning"]);
        assert_eq!(app.session.active().unwrap().doc.name, name);
        assert!(!app.session.active().unwrap().source_read_only);
    }
    assert_eq!(calls.get(), 3);
}

#[test]
fn inline_xcf_prefix_recognition_does_not_accept_a_corrupt_trailer() {
    let mut damaged = WRAPPED_XCF.to_vec();
    let crc = damaged.len() - 8;
    damaged[crc] ^= 1;
    assert!(photocraft_io::is_xcf_input("renamed.bin", &damaged), "header recognition is provisional");
    let mut app = app();
    assert!(app.open_bytes_from("renamed.bin", &damaged, Some("/imports/renamed.bin")).is_err());
    assert!(app.session.documents().is_empty());
    assert!(app.held_opens.is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(photocraft_io::is_xcf_input("malformed.xcf", b"not an XCF"), "an explicit decoder choice retains its normal error path");
}

fn automation_open(app: &mut PhotocraftApp, path: &str, approved: bool) -> serde_json::Value {
    let ctx = app.ctx().clone();
    let (request, _reply) = crate::control::ControlRequest::new("app.open", json!({"path":path,"approveHighMemory":approved}));
    match crate::control::handle(app, &ctx, &request) {
        crate::control::Reply::Done(reply) => reply,
        _ => panic!("the authorized inline open must complete"),
    }
}

#[test]
fn automation_xcf_open_preserves_typed_view_and_protected_authorized_origin() {
    let plain = saved_view_xcf();
    for source in [plain.as_slice(), WRAPPED_XCF, BZIP_XCF, XZ_XCF] {
        let mut app = app();
        app.session.execute("edit.colorSettings", json!({"askOnMissing":true})).unwrap();
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        let reads = Rc::new(Cell::new(0));
        let count = reads.clone();
        let expected = source.to_vec();
        let read_bytes = expected.clone();
        app.services.automation_read = Some(Box::new(move |path| {
            assert_eq!(path, "incoming/source.psd");
            count.set(count.get() + 1);
            Ok(crate::AutomationReadResult { name: "source.psd".into(), bytes: read_bytes.clone(), save_path: Some(path.into()) })
        }));
        app.services.automation_locate = Some(Box::new(|path| {
            assert_eq!(path, "incoming/source.psd");
            Some("/imports/source.psd".into())
        }));
        app.services.import = Some(Box::new(|_, _, _| Err("the tuple importer must not discard XCF metadata".into())));
        let reply = automation_open(&mut app, "incoming/source.psd", true);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reads.get(), 1);
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.held_opens.is_empty(), "automation does not invoke profile-path prompts");
        assert!(app.session.prefs().script_events.enabled);
        assert_saved_view(&app, "/imports/source.psd");
        // Guard assertions only: no save/export/writer is invoked against the source.
        let state = app.session.active().unwrap();
        assert!(photocraft_engine::file_cmds::check_import_destination(state, "/imports/source.psd").is_err());
        assert!(photocraft_engine::file_cmds::check_import_destination(state, "/exports/copy.pcraft").is_ok());
        let initial = app.session.active_mut().unwrap().take_fresh_import_view().unwrap();
        assert_eq!(initial.selected_layers.len(), 2);
        assert_eq!(initial.samplers, [[0., 0.]]);
        assert!(app.session.active_mut().unwrap().take_fresh_import_view().is_none(), "the typed handoff is one-time");
        assert_eq!(source, expected);
    }
}

#[test]
fn automation_relative_save_checks_write_root_identity_before_any_writer() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let source = root.join("source.psd").to_string_lossy().into_owned();
    for background in [false, true] {
        let mut app = app();
        let origin = source.clone();
        app.services.automation_read = Some(Box::new(|path| {
            assert_eq!(path, "source.psd");
            Ok(crate::AutomationReadResult { name: "source.psd".into(), bytes: BZIP_XCF.to_vec(), save_path: Some(path.into()) })
        }));
        app.services.automation_locate = Some(Box::new(move |_| Some(origin.clone())));
        let write_root = root.clone();
        app.services.automation_write_location = Some(Box::new(move |path| Ok(write_root.join(path).to_string_lossy().into_owned())));
        let writes = Arc::new(AtomicUsize::new(0));
        let count = writes.clone();
        app.services.automation_write = Some(Box::new(move |_, _| {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }));
        let count = writes.clone();
        app.services.automation_save = Some(Arc::new(move |_, _, _, _| {
            count.fetch_add(1, Ordering::Relaxed);
            Err("stub worker writer must not be reached".into())
        }));
        let encodes = Rc::new(Cell::new(0));
        let count = encodes.clone();
        app.services.export = Some(Box::new(move |_, _, _| {
            count.set(count.get() + 1);
            Ok((Vec::new(), Vec::new()))
        }));
        assert_eq!(automation_open(&mut app, "source.psd", false)["ok"], true);
        assert_eq!(app.session.active().unwrap().protected_import_source.as_deref(), Some(source.as_str()));
        app.background_jobs = background;
        let ctx = app.ctx().clone();
        let (request, _reply) = crate::control::ControlRequest::new("app.save", json!({"path":"source.psd"}));
        let crate::control::Reply::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("synchronous guard") };
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(encodes.get(), 0);
        assert!(app.jobs.saves.is_empty());
        assert!(app.session.active().unwrap().path.is_none());

        // A different authorized destination still reaches a stub writer; no
        // destination is created, including on the regression's failing path.
        app.background_jobs = false;
        let (request, _reply) = crate::control::ControlRequest::new("app.save", json!({"path":"copy.pcraft"}));
        let crate::control::Reply::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("stub save") };
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        assert_eq!(encodes.get(), 1);
    }
    assert!(std::fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn automation_sync_and_worker_save_use_the_same_authorized_destination() {
    use std::sync::{Arc, Mutex};

    let exports = tempfile::tempdir().unwrap();
    let write_root = std::fs::canonicalize(exports.path()).unwrap();
    // A temporary ambient file gives the identity check a real same-file
    // witness. Neither of the stub save writers opens it or the destination.
    let ambient_file = tempfile::Builder::new().prefix("relative-xcf-save-").suffix(".psd").tempfile_in(std::env::current_dir().unwrap()).unwrap();
    let request_path = ambient_file.path().file_name().unwrap().to_str().unwrap();
    let ambient = ambient_file.path().to_string_lossy().into_owned();
    for background in [false, true] {
        let mut app = app();
        let origin = ambient.clone();
        app.services.automation_read =
            Some(Box::new(|path| Ok(crate::AutomationReadResult { name: path.into(), bytes: BZIP_XCF.to_vec(), save_path: Some(path.into()) })));
        app.services.automation_locate = Some(Box::new(move |_| Some(origin.clone())));
        assert_eq!(automation_open(&mut app, request_path, false)["ok"], true);
        let state = app.session.active().unwrap();
        assert_eq!(state.protected_import_source.as_deref(), Some(ambient.as_str()));
        assert!(photocraft_engine::file_cmds::check_import_destination(state, request_path).is_err());

        let write_root = write_root.clone();
        let resolutions = Rc::new(Cell::new(0));
        let seen = resolutions.clone();
        app.services.automation_write_location = Some(Box::new(move |path| {
            seen.set(seen.get() + 1);
            Ok(write_root.join(path).to_string_lossy().into_owned())
        }));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let seen = writes.clone();
        app.services.automation_save = Some(Arc::new(move |_, path, _, _| {
            seen.lock().unwrap().push(path.to_owned());
            Ok(Vec::new())
        }));
        let seen = writes.clone();
        app.services.automation_write = Some(Box::new(move |path, _| {
            seen.lock().unwrap().push(path.to_owned());
            Ok(())
        }));
        app.services.export = Some(Box::new(|_, _, _| Ok((Vec::new(), Vec::new()))));
        app.background_jobs = background;
        let reply = app.save_automation(Some(request_path.into())).unwrap();
        assert_eq!(reply["path"], request_path);
        let ctx = app.ctx().clone();
        if background {
            assert_eq!(reply["pending"], true);
            for _ in 0..200 {
                crate::jobs_ui::tick(&mut app, &ctx);
                if !app.saving() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(!app.saving());
        assert_eq!(resolutions.get(), 1);
        assert_eq!(*writes.lock().unwrap(), [request_path]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(request_path));
    }
    assert!(std::fs::read_dir(exports.path()).unwrap().next().is_none());
}

#[test]
fn imported_text_edits_follow_the_saved_xcf_naming_flag() {
    for (flags, expected) in [(None, "Goodbye"), (Some(0u32), "Goodbye"), (Some(1u32), "Hello")] {
        let mut xcf = Wire(b"gimp xcf v003\0".to_vec());
        for value in [1, 1, 0] {
            xcf.word(value);
        }
        xcf.property(17, &[0]);
        xcf.end();
        let layer = xcf.slot();
        xcf.word(0);
        xcf.word(0);
        xcf.point_here(layer);
        for value in [1, 1, 1] {
            xcf.word(value);
        }
        xcf.string("Hello");
        xcf.property(8, &1u32.to_be_bytes());
        if let Some(flags) = flags {
            xcf.property(26, &flags.to_be_bytes());
        }
        let style = br#"(text "Hello") (box-mode fixed) (box-width 64) (box-height 32)"#;
        let mut parasite = Wire::default();
        parasite.string("gimp-text-layer");
        parasite.word(0);
        parasite.word(style.len() as u32);
        parasite.0.extend_from_slice(style);
        xcf.property(21, &parasite.0);
        xcf.end();
        let hierarchy = xcf.slot();
        xcf.word(0);
        xcf.point_here(hierarchy);
        xcf.pixels(4, &[0, 0, 0, 255]);

        let imported = photocraft_io::import("text.xcf", &xcf.0).unwrap();
        let mut session = Session::new();
        let id = imported.document.layers[0].id;
        assert_eq!(imported.document.layers[0].name, "Hello", "import keeps the saved name");
        session.admit_import_with_initial_view(imported.document, None, imported.source_read_only, imported.initial_view);
        session.execute("type.edit", json!({"layer": id.0, "text": "Goodbye"})).unwrap();
        let layer = session.active().unwrap().doc.layer(id).unwrap();
        assert_eq!(layer.name, expected, "saved flags: {flags:?}");
        assert!(matches!(&layer.content, photocraft_doc::LayerContent::Text(text) if text.text == "Goodbye"));
    }
}

#[test]
fn automation_non_xcf_keeps_configured_importer_approval_and_save_provenance() {
    let mut app = app();
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    app.services.import = Some(Box::new(move |name, bytes, options| {
        assert_eq!(bytes, b"custom source");
        assert!(options.psd_high_memory_approved);
        seen.set(seen.get() + 1);
        Ok((Document::new(name, Size::new(1, 1), ColorMode::Rgb, SampleType::U8), vec!["custom warning".into()]))
    }));
    app.services.automation_read =
        Some(Box::new(|path| Ok(crate::AutomationReadResult { name: "custom.psd".into(), bytes: b"custom source".to_vec(), save_path: Some(path.into()) })));
    let reply = automation_open(&mut app, "custom.psd", true);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["warnings"], json!(["custom warning"]));
    assert_eq!(calls.get(), 1);
    let state = app.session.active_mut().unwrap();
    assert!(!state.source_read_only);
    assert_eq!(state.path.as_deref(), Some("custom.psd"));
    assert!(state.protected_import_source.is_none());
    assert!(state.take_fresh_import_view().is_none());
}

#[test]
fn automation_xcf_decode_failure_preserves_existing_document_and_view() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    let before = app.session.active().unwrap().doc.clone();
    let selected = app.session.active().unwrap().selected_layers.clone();
    let mut damaged = WRAPPED_XCF.to_vec();
    let crc = damaged.len() - 8;
    damaged[crc] ^= 1;
    app.services.automation_read =
        Some(Box::new(move |_| Ok(crate::AutomationReadResult { name: "source.psd".into(), bytes: damaged.clone(), save_path: Some("source.psd".into()) })));
    app.services.import = Some(Box::new(|_, _, _| panic!("a typed decode failure must not fall back to the tuple importer")));
    let reply = automation_open(&mut app, "source.psd", false);
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(app.session.documents().len(), 1);
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().selected_layers, selected);
    assert!(app.session.prefs().script_events.enabled);
    assert!(app.ui.recent_files.is_empty());
    assert!(app.held_opens.is_empty());
}
