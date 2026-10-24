//! Offscreen service/harness regressions. These exercise the shared shell
//! contracts, not an OS picker, Explorer drag gesture or a live browser.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_doc::{Document, LayerContent};
use photocraft_engine::{Session, channel_cmds::ChannelTarget};
use photocraft_geom::Rect;
use photocraft_ui_egui::{
    PhotocraftApp, Services,
    file_dialog::{FileDialogAnswer, FileDialogRequest},
    jobs_ui, menus, open_prompt,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Wire(Vec<u8>);
impl Wire {
    fn word(&mut self, n: u32) {
        self.0.extend_from_slice(&n.to_be_bytes());
    }
    fn string(&mut self, text: &str) {
        self.word(text.len() as u32 + 1);
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(0);
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
    fn pointer(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.pointer(0);
        at
    }
    fn patch(&mut self, at: usize, value: usize) {
        self.0[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
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
/// alpha, sampler and an unsupported blend mode that must reach warning UI.
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

fn app(background: bool) -> PhotocraftApp {
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    session.color.settings.ask_on_mismatch = false;
    let mut app = PhotocraftApp::new(
        session,
        Services {
            import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|e| e.to_string()))),
            ..Default::default()
        },
    );
    app.background_jobs = background;
    app
}
fn finish(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let start = Instant::now();
    while app.session.has_jobs() {
        assert!(start.elapsed() < Duration::from_secs(30), "open worker timed out");
        jobs_ui::tick(app, &ctx);
        std::thread::yield_now();
    }
    jobs_ui::tick(app, &ctx);
}
fn assert_open(app: &PhotocraftApp, path: Option<&str>) {
    let state = app.session.active().expect("native document installed");
    assert!(state.source_read_only && state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), path);
    assert!(!state.is_dirty());
    assert_eq!(state.doc.size, photocraft_geom::Size::new(2, 1));
    let chosen = state.doc.layers.iter().find(|layer| layer.name == "Chosen source layer").unwrap();
    assert_eq!(state.active_layer, Some(chosen.id));
    assert_eq!(state.selected_layers, vec![chosen.id]);
    assert_eq!(state.channel_view.target, ChannelTarget::LayerMask);
    assert!(state.channel_view.layer_mask.is_some());
    assert!(state.channel_view.alpha_shown(0));
    assert_eq!(state.color_samplers, vec![[0., 0.]]);
    assert!(state.doc.metadata.import_conversion.is_some());
    assert!(
        app.ui.notices.iter().any(|notice| notice.import_report_document == Some(state.doc.id)),
        "warning notice links to this document's native conversion report"
    );
    assert!(app.ui.notices.iter().flat_map(|notice| &notice.lines).any(|line| line.contains("unknown mode")), "source warning must reach notice UI");
    let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.metadata.import_conversion, state.doc.metadata.import_conversion);
}

#[derive(Debug)]
struct DropBytes(PathBuf, Vec<u8>);
impl egui::DroppedFile for DropBytes {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[test]
fn actual_xcf_open_recent_and_drop_share_native_installation_inline_and_async() {
    let source = xcf();
    for background in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native-source.xcf");
        std::fs::write(&path, &source).unwrap();
        let path = path.to_str().unwrap();
        let mut direct = app(background);
        direct.open_file(path, &source).unwrap();
        finish(&mut direct);
        assert_open(&direct, Some(path));
        assert_eq!(direct.ui.recent_files.first().map(String::as_str), Some(path));
        direct.session.close(0);
        menus::invoke(&mut direct, &egui::Context::default(), "file.openRecent.0", json!({})).unwrap();
        finish(&mut direct);
        assert_open(&direct, Some(path));
        let mut dropped = app(background);
        dropped.open_dropped(&egui::Context::default(), vec![Arc::new(DropBytes(PathBuf::from(path), Vec::new()))], None);
        finish(&mut dropped);
        assert_open(&dropped, Some(path));
        assert_eq!(std::fs::read(path).unwrap(), source, "all open routes leave the source immutable");
    }
}

#[test]
fn already_open_xcf_activates_existing_tab_until_native_save_as() {
    for background in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.xcf");
        std::fs::write(&source, xcf()).unwrap();
        let path = source.to_str().unwrap();
        let alias = format!("{}/./source.xcf", dir.path().display());
        let mut app = app(background);
        app.open_path(path).unwrap();
        finish(&mut app);
        let original = app.session.active().unwrap().doc.id;
        assert_open(&app, Some(path));
        app.session.execute("channel.target.composite", json!({})).unwrap();
        app.session.execute("file.new", json!({"width":2,"height":1})).unwrap();
        menus::invoke(&mut app, &egui::Context::default(), "file.openRecent.0", json!({})).unwrap();
        finish(&mut app);
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active().unwrap().doc.id, original);
        for spelling in [path, &alias] {
            app.session.set_active(1);
            app.open_path(spelling).unwrap();
            finish(&mut app);
            assert_eq!(app.session.documents().len(), 2, "same source must activate its existing tab");
            let state = app.session.active().unwrap();
            assert_eq!(state.doc.id, original);
            assert_eq!(state.channel_view.target, ChannelTarget::Composite, "activation must not reapply the source's one-time mask target");
        }
        let native = dir.path().join("saved-native.pcraft");
        app.services.export = Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|result| (result.bytes, result.warnings)).map_err(|e| e.to_string())
        }));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path":native})).unwrap();
        finish(&mut app);
        assert!(!app.session.active().unwrap().source_read_only);
        assert_eq!(app.session.active().unwrap().protected_import_source.as_deref(), Some(path));
        app.open_path(&alias).unwrap();
        finish(&mut app);
        assert_eq!(app.session.documents().len(), 3, "native Save As ends the source's open-file identity");
        assert_ne!(app.session.active().unwrap().doc.id, original);
        assert_open(&app, Some(&alias));
        assert_eq!(std::fs::read(&source).unwrap(), xcf());
        assert!(native.is_file());
        let saved = photocraft_format::load_from_bytes(&std::fs::read(native).unwrap()).unwrap();
        assert!(saved.metadata.import_conversion.is_some());
    }
}

#[test]
fn picker_paths_and_browser_contents_contract_preserve_native_xcf_state() {
    for background in [false, true] {
        for bytes_answer in [false, true] {
            let source = xcf();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("picked.xcf");
            std::fs::write(&path, &source).unwrap();
            let text = path.to_str().unwrap().to_string();
            let answer =
                if bytes_answer { FileDialogAnswer::ContentsMany(vec![("picked.xcf".into(), source)]) } else { FileDialogAnswer::Paths(vec![text.clone()]) };
            let mut answer = Some(answer);
            let mut app = app(background);
            app.services.file_dialog = Some(Box::new(move |request, _, reply| {
                assert!(matches!(request, FileDialogRequest::Open { multiple: true, .. }));
                reply.send(answer.take());
            }));
            app.open_dialog_file().unwrap();
            app.poll_file_dialog(&egui::Context::default(), None);
            finish(&mut app);
            assert_open(&app, if bytes_answer { None } else { Some(&text) });
        }
    }
}

#[test]
fn real_xcf_color_held_queue_preserves_each_result_until_answered() {
    for background in [false, true] {
        let source = xcf();
        let mut app = app(background);
        app.session.color.settings.working_rgb = photocraft_cms::Builtin::AdobeRgbCompat.id().into();
        app.session.color.settings.ask_on_mismatch = true;
        app.session.color.settings.ask_on_missing = true;
        app.open_bytes("first.xcf", &source).unwrap();
        app.open_bytes("second.xcf", &source).unwrap();
        finish(&mut app);
        assert!(app.session.documents().is_empty());
        assert!(open_prompt::waiting(&app));
        open_prompt::answer(&mut app, Some("preserve")).unwrap();
        assert_open(&app, None);
        assert!(open_prompt::waiting(&app));
        open_prompt::answer(&mut app, Some("preserve")).unwrap();
        assert_open(&app, None);
        assert_eq!(app.session.documents().len(), 2);
        assert!(!open_prompt::waiting(&app));
    }
}

#[test]
fn browser_style_inbox_is_consumed_by_offscreen_shell_frame() {
    let inbox = Arc::new(Mutex::new(vec![("inbox.xcf".into(), xcf())]));
    let sent = inbox.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(640., 480.)).with_max_steps(8).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app(false);
        app.services.inbox = Some(inbox);
        app
    });
    h.run_steps(2);
    assert!(sent.lock().unwrap().is_empty());
    assert_open(h.state(), None);
}

#[test]
fn synchronous_and_color_held_services_prepare_native_content_before_installation() {
    use photocraft_doc::{Affine, Color, Fill, Layer, Path as VectorPath, ShapeLayer, SmartObject, SmartSource, Subpath, TextLayer};
    let imported = photocraft_io::import("prepared.xcf", &xcf()).unwrap();
    let mut native = imported.document.clone();
    native.size = photocraft_geom::Size::new(32, 24);
    native.layers.push(Layer::new(
        "Editable native text",
        LayerContent::Text(TextLayer { text: "Native".into(), font_family: "Arial".into(), size_pt: 8., ..Default::default() }),
    ));
    native.layers.push(Layer::new(
        "Editable native shape",
        LayerContent::Shape(ShapeLayer {
            path: VectorPath::new(vec![Subpath::polygon(&[(8., 0.), (12., 0.), (12., 4.), (8., 4.)])]),
            fill: Some(Fill::Solid(Color::WHITE)),
            ..Default::default()
        }),
    ));
    let mut nested = Document::new("Embedded native", photocraft_geom::Size::new(2, 1), native.mode, native.depth);
    let mut raster = Layer::raster("Native pixels", nested.pixel_format());
    raster.surface_mut().unwrap().write_region(Rect::new(0, 0, 2, 1), &[0.8, 0.2, 0.1, 1., 0.8, 0.2, 0.1, 1.]);
    nested.layers.push(raster);
    let source = photocraft_format::save_to_bytes(&nested, &Default::default()).unwrap();
    native.layers.push(Layer::new(
        "Editable native smart",
        LayerContent::Smart(SmartObject::new(
            SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(source) },
            Affine { m: [1., 0., 0., 1., 4., 0.] },
            None,
        )),
    ));
    for held in [false, true] {
        let mut app = app(false);
        let doc = native.clone();
        let view = imported.initial_view.clone();
        let warnings = imported.warnings.clone();
        app.services.import = Some(Box::new(move |_, _, _| {
            Ok(photocraft_io::ImportResult {
                document: doc.clone(),
                warnings: warnings.clone(),
                source_read_only: true,
                preview_only: false,
                initial_view: view.clone(),
            })
        }));
        if held {
            app.session.color.settings.working_rgb = photocraft_cms::Builtin::AdobeRgbCompat.id().into();
            app.session.color.settings.ask_on_mismatch = true;
            app.session.color.settings.ask_on_missing = true;
        }
        app.open_bytes("prepared.xcf", b"service importer contract fixture").unwrap();
        if held {
            assert!(app.session.documents().is_empty());
            open_prompt::answer(&mut app, Some("preserve")).unwrap();
        }
        let state = app.session.active().unwrap();
        for name in ["Editable native text", "Editable native shape", "Editable native smart"] {
            let layer = state.doc.layers.iter().find(|layer| layer.name == name).unwrap();
            assert!(layer.surface().is_some(), "{name} cache must exist before the first display frame");
            assert!(
                matches!(
                    (name, &layer.content),
                    ("Editable native text", LayerContent::Text(_))
                        | ("Editable native shape", LayerContent::Shape(_))
                        | ("Editable native smart", LayerContent::Smart(_))
                ),
                "prepared content remains editable native data"
            );
        }
        let smart = state.doc.layers.iter().find(|layer| layer.name == "Editable native smart").unwrap();
        assert!(matches!(&smart.content, LayerContent::Smart(_)));
        let first_display = photocraft_compose::flatten(&state.doc);
        assert!(first_display.get(4, 0)[3] > 0.9);
        assert!(first_display.get(9, 1)[3] > 0.9);
        assert!(state.source_read_only && !state.is_dirty());
        assert!(state.doc.metadata.import_conversion.is_some());
    }
}
