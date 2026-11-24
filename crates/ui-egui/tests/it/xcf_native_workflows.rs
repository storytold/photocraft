//! Offscreen desktop-shell regressions using original specification-authored XCF bytes.
//! These exercise real background import and confirmation, not an OS picker or live browser.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_engine::{Session, channel_cmds::ChannelTarget, color_cmds::ColorExt};
use photocraft_ui_egui::{PhotocraftApp, Services, jobs_ui, menus, open_prompt};
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Wire(Vec<u8>);
impl Wire {
    /// Append a big-endian scalar to the original UI workflow fixture.
    fn word(&mut self, n: u32) {
        self.0.extend_from_slice(&n.to_be_bytes());
    }
    /// Encode a UTF-8 fixture string with its length and terminating NUL.
    fn string(&mut self, text: &str) {
        self.word(text.len() as u32 + 1);
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
    }
    /// Append one length-delimited XCF property without using a native document serializer.
    fn property(&mut self, kind: u32, bytes: &[u8]) {
        self.word(kind);
        self.word(bytes.len() as u32);
        self.0.extend_from_slice(bytes);
    }
    /// Close the current fixture property list using the wire-format terminator.
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    /// Append a 64-bit big-endian pointer for the fixture's modern record framing.
    fn pointer(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }
    /// Reserve a null pointer for a record that will be authored later in the buffer.
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.pointer(0);
        at
    }
    /// Backpatch a reserved pointer with the absolute fixture-record offset.
    fn patch(&mut self, at: usize, value: usize) {
        self.0[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
    /// Write a single 2-by-1 hierarchy and uncompressed tile with the supplied native channel bytes.
    fn pixels(&mut self, channels: u32, data: &[u8]) -> usize {
        let at = self.0.len();
        self.word(2);
        self.word(1);
        self.word(channels);
        let level = self.slot();
        self.pointer(0);
        let start = self.0.len();
        self.patch(level, start);
        self.word(2);
        self.word(1);
        let tile = self.slot();
        self.pointer(0);
        let start = self.0.len();
        self.patch(tile, start);
        self.0.extend_from_slice(data);
        at
    }
}

/// Independent documented v11 wire: one active masked layer, visible saved
/// alpha and sampler. A bounded text parasite is marked as independently modified
/// saved pixels, so its configuration is retained in the conversion report rather
/// than substituted for the authoritative raster (XCF text flag bit 1).
fn xcf() -> Vec<u8> {
    let mut w = Wire(b"gimp xcf v011\0".to_vec());
    w.word(2);
    w.word(1);
    w.word(0);
    w.word(150);
    w.property(17, &[0]);
    w.property(39, &[0u32.to_be_bytes(); 5].concat());
    w.end();
    let layer = w.slot();
    w.pointer(0);
    let alpha = w.slot();
    w.pointer(0);
    let at = w.0.len();
    w.patch(layer, at);
    w.word(2);
    w.word(1);
    w.word(1);
    w.string("Chosen source layer");
    w.property(2, &[]);
    w.property(12, &1u32.to_be_bytes());
    w.property(13, &1u32.to_be_bytes());
    w.property(7, &999u32.to_be_bytes());
    let configuration = br#"(text "Saved pixels")"#;
    let mut parasite = Wire::default();
    parasite.string("gimp-text-layer");
    parasite.word(0);
    parasite.word(configuration.len() as u32);
    parasite.0.extend_from_slice(configuration);
    w.property(21, &parasite.0);
    w.property(26, &2u32.to_be_bytes());
    w.end();
    let pixels = w.slot();
    let mask = w.slot();
    let at = w.pixels(4, &[40, 80, 120, 255, 70, 110, 150, 255]);
    w.patch(pixels, at);
    let at = w.0.len();
    w.patch(mask, at);
    w.word(2);
    w.word(1);
    w.string("Native mask");
    w.end();
    let pixels = w.slot();
    let at = w.pixels(1, &[255, 128]);
    w.patch(pixels, at);
    let at = w.0.len();
    w.patch(alpha, at);
    w.word(2);
    w.word(1);
    w.string("Saved alpha");
    w.property(8, &1u32.to_be_bytes());
    w.end();
    let pixels = w.slot();
    let at = w.pixels(1, &[64, 192]);
    w.patch(pixels, at);
    w.0
}

fn app() -> Box<PhotocraftApp> {
    let mut session = Session::new();
    session.color_mut().settings.ask_on_missing = false;
    session.color_mut().settings.ask_on_mismatch = false;
    let mut app = PhotocraftApp::new(session, Services::default());
    app.background_jobs = true;
    app
}

fn finish(app: &mut PhotocraftApp) {
    let started = Instant::now();
    while app.session.has_jobs() {
        assert!(started.elapsed() < Duration::from_secs(30), "native Open worker timed out");
        let ctx = app.ctx().clone();
        jobs_ui::tick(app, &ctx);
        std::thread::yield_now();
    }
    let ctx = app.ctx().clone();
    jobs_ui::tick(app, &ctx);
    assert!(app.jobs.opens.is_empty());
}

fn assert_native_open(app: &mut PhotocraftApp, source: &str) {
    let state = app.session.active().expect("native document installed");
    assert!(state.source_read_only && state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), Some(source));
    assert!(!state.is_dirty());
    assert_eq!(state.doc.size, photocraft_geom::Size::new(2, 1));
    let chosen = state.doc.layers.iter().find(|layer| layer.name == "Chosen source layer").unwrap();
    assert_eq!(state.active_layer, Some(chosen.id));
    assert_eq!(state.selected_layers, vec![chosen.id]);
    assert_eq!(state.channel_view.target, ChannelTarget::LayerMask);
    assert!(state.channel_view.layer_mask.is_some());
    assert!(state.channel_view.alpha_shown(0));
    assert_eq!(state.color_samplers, vec![[0., 0.]]);
    let pixel = chosen.surface().unwrap().pixel(0, 0);
    for (actual, expected) in pixel.iter().copied().zip([40. / 255., 80. / 255., 120. / 255., 1.]) {
        assert!((actual - expected).abs() < 1e-6, "source pixel {pixel:?}");
    }
    assert_eq!(state.history.past_len(), 0);
    let report = state.doc.metadata.import_conversion.clone().expect("native conversion report retained");
    let before = state.doc.clone();
    let revision = state.revision;
    let queried = app.session.execute("document.importReport", json!({"limit": 250})).unwrap();
    assert_eq!(queried["summary"]["sourceFormat"], "XCF");
    assert_eq!(queried["summary"]["attachmentCount"], 1);
    assert_eq!(queried["entries"][0]["featureCode"], "xcf.text");
    assert_eq!(queried["entries"][0]["disposition"], "retained_unapplied");
    assert!(queried["total"].as_u64().unwrap() > 0);
    let state = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&before, &state.doc), "report inspection is read-only");
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.past_len(), 0);
    let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
    let reopened = photocraft_io::import("native-copy.pcraft", &saved).unwrap();
    assert_eq!(reopened.document.metadata.import_conversion.as_ref(), Some(&report));
    assert_eq!(reopened.document.layers, state.doc.layers);
    assert!(!reopened.source_read_only);
    assert!(reopened.initial_view.is_none(), "saved native copy does not replay import view");
}

#[test]
fn background_xcf_open_and_recent_preserve_native_view_pixels_report_and_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("native-source.xcf");
    let bytes = xcf();
    std::fs::write(&source, &bytes).unwrap();
    let path = source.to_str().unwrap();
    let mut app = app();
    app.open_path(path).unwrap();
    finish(&mut app);
    assert_native_open(&mut app, path);
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(path));
    app.session.close(0).unwrap();
    let ctx = app.ctx().clone();
    menus::invoke(&mut app, &ctx, "file.openRecent.0", json!({})).unwrap();
    finish(&mut app);
    assert_native_open(&mut app, path);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
fn real_xcf_background_color_holds_preserve_each_native_result_and_cancel_without_admission() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = xcf();
    let paths: Vec<_> = ["first.xcf", "second.xcf"].map(|name| directory.path().join(name)).into_iter().collect();
    for source in &paths {
        std::fs::write(source, &bytes).unwrap();
    }
    for cancel in [false, true] {
        let mut app = app();
        app.session.color_mut().settings.ask_on_missing = true;
        app.session.color_mut().settings.ask_on_mismatch = true;
        app.session.color_mut().settings.working_rgb = craft::cms::Builtin::AdobeRgbCompat.id().into();
        for path in &paths {
            app.open_path(path.to_str().unwrap()).unwrap();
            finish(&mut app);
            assert!(app.session.documents().is_empty(), "each decoded source waits for its own answer");
        }
        assert!(app.session.documents().is_empty());
        assert!(open_prompt::waiting(&app));
        if cancel {
            open_prompt::answer(&mut app, None).unwrap();
            assert!(app.session.documents().is_empty());
        } else {
            open_prompt::answer(&mut app, Some("preserve")).unwrap();
            assert_native_open(&mut app, paths[0].to_str().unwrap());
            let first = app.session.active().unwrap().doc.clone();
            open_prompt::answer(&mut app, Some("preserve")).unwrap();
            assert_native_open(&mut app, paths[1].to_str().unwrap());
            assert_eq!(app.session.documents().len(), 2);
            assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &first));
            assert_ne!(app.session.documents()[0].doc.id, app.session.documents()[1].doc.id);
        }
        assert!(!open_prompt::waiting(&app));
        for source in &paths {
            assert_eq!(std::fs::read(source).unwrap(), bytes);
        }
    }
}
