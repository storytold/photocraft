//! PhotoCraft on iPadOS and Android tablets, hosted by `craft-mobile`.
//!
//! The same [`PhotocraftApp`] as the desktop and web builds. The native shells (SwiftUI with
//! CraftKit, Compose with craftkit) own the app and its panels; this crate only says how
//! PhotoCraft meets them:
//! - **Files:** File › Open asks the shell's document picker for contents (as on the web);
//!   File › Save and Export hand the encoded bytes to the shell's exporter. Files the system
//!   hands over (Files, Share) open through the app's inbox.
//! - **Native controls** map to PhotoCraft's own commands, exactly as its menu items run them.
//! - **Panels** see the active document and its layers, top first.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, PoisonError};

use craft_mobile::{Action, CraftApp, DocumentInfo, Item, PickedFile, Platform, Snapshot, Stylus};
use egui::TouchPhase;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::stylus::PenSample;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, Inbox, PhotocraftApp, Recoverable, Services};
use serde_json::json;

/// PhotoCraft as `craft-mobile` hosts it.
pub struct PhotoCraftMobile {
    app: PhotocraftApp,
    /// Files the system handed over, opened by the app on its next frame.
    inbox: Inbox,
}

impl CraftApp for PhotoCraftMobile {
    fn create(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>, platform: Platform) -> Self {
        let inbox = Inbox::default();
        let app = create_app(ctx, wgpu, services(&platform, inbox.clone()));
        Self { app, inbox }
    }

    fn wgpu_configuration() -> eframe::egui_wgpu::WgpuConfiguration {
        let mut config = eframe::egui_wgpu::WgpuConfiguration::default();
        // Large canvases need the adapter's own texture limits, as on the desktop.
        photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut config.wgpu_setup);
        config
    }

    fn open_file(&mut self, ctx: &egui::Context, file: PickedFile) {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner).push((file.name, file.bytes));
        ctx.request_repaint();
    }

    fn action(&mut self, ctx: &egui::Context, action: &Action) -> Result<(), String> {
        let menu = |app: &mut PhotocraftApp, id: &str| {
            // As its menu item runs it: enable rules, dialogs and status messages included.
            photocraft_ui_egui::shortcut_dispatch::dispatch(app, ctx, id);
            Ok(())
        };
        let invoke = |app: &mut PhotocraftApp, id: &str, params| photocraft_ui_egui::menus::invoke(app, ctx, id, params).map(drop);
        match action {
            Action::Open => menu(&mut self.app, "file.open"),
            Action::Save => menu(&mut self.app, "file.save"),
            Action::Undo => menu(&mut self.app, "edit.undo"),
            Action::Redo => menu(&mut self.app, "edit.redo"),
            Action::Select { id } => invoke(&mut self.app, "layer.select", json!({ "layer": id })),
            Action::SetVisible { id, visible } => invoke(&mut self.app, "layer.setProps", json!({ "layer": id, "visible": visible })),
            Action::Command { id, params } if params.is_null() => menu(&mut self.app, id),
            Action::Command { id, params } => invoke(&mut self.app, id, params.clone()),
        }
    }

    fn snapshot(&self) -> Snapshot {
        let Some(st) = self.app.session.active() else {
            return Snapshot { items_title: "Layers".into(), ..Snapshot::default() };
        };
        let doc = &st.doc;
        let mut items = Vec::new();
        let selected = |id| st.active_layer == Some(id) || st.selected_layers.contains(&id);
        layer_rows(&doc.layers, 0, &selected, &mut items);
        Snapshot {
            document: Some(DocumentInfo {
                name: doc.name.clone(),
                detail: format!("{} × {} px", doc.size.width, doc.size.height),
                dirty: st.revision != st.saved_revision,
            }),
            can_undo: st.history.can_undo(),
            can_redo: st.history.can_redo(),
            items_title: "Layers".into(),
            items,
        }
    }

    /// Write crash-recovery copies of every unsaved document and wait for them: the OS may end
    /// the app without another word once it's off screen.
    fn suspend(&mut self, _ctx: &egui::Context) {
        let unsaved: Vec<(Arc<Document>, u64, Option<String>)> = self
            .app
            .session
            .documents()
            .iter()
            .filter(|st| st.revision != st.saved_revision)
            .map(|st| (st.doc.clone(), st.revision, st.path.clone()))
            .collect();
        if let Some(autosave) = self.app.services.autosave.as_mut() {
            for (doc, revision, path) in &unsaved {
                if let Err(e) = autosave(doc, *revision, path.as_deref()) {
                    log::warn!("photocraft-mobile: no recovery copy of {}: {e}", doc.name);
                }
            }
        }
        if let Some(flush) = self.app.services.flush_autosaves.as_mut() {
            let written = flush();
            log::info!(
                "photocraft-mobile: suspended; {} of {} unsaved documents written for recovery",
                written.iter().filter(|w| w.2.is_ok()).count(),
                unsaved.len()
            );
        }
    }

    /// Rendered smart-object stages and sources rebuild on demand; undo history is the user's work.
    fn trim_memory(&mut self, ctx: &egui::Context) {
        let freed = photocraft_engine::cmd::smart_cmds::purge_stack_cache() + photocraft_engine::cmd::smart_cmds::purge_source_cache();
        log::info!("photocraft-mobile: memory warning; released {} MB of caches", freed / (1024 * 1024));
        ctx.request_repaint();
    }

    /// Pencil samples feed the brush engine's pressure and tilt, as a desktop tablet's do.
    fn stylus(&mut self, sample: &Stylus) {
        self.app.stylus.feed.set(pen_sample(sample));
    }
}

/// A stylus reading in PhotoCraft's terms, or `None` once it lifts.
fn pen_sample(sample: &Stylus) -> Option<PenSample> {
    if matches!(sample.phase, TouchPhase::End | TouchPhase::Cancel) {
        return None;
    }
    let (tilt_x, tilt_y) = sample.tilt_degrees();
    Some(PenSample { pressure: sample.pressure, tilt_x, tilt_y, rotation: sample.azimuth.to_degrees().rem_euclid(360.0), eraser: sample.eraser })
}

/// Layers top first, as the Layers panel lists them (documents store them bottom first), with
/// each group's children under it.
fn layer_rows(layers: &[Layer], depth: u32, selected: &dyn Fn(photocraft_doc::LayerId) -> bool, out: &mut Vec<Item>) {
    for layer in layers.iter().rev() {
        out.push(Item {
            id: layer.id.0,
            name: layer.name.clone(),
            kind: layer.content.kind_name().into(),
            depth,
            visible: Some(layer.visible),
            selected: selected(layer.id),
        });
        if let photocraft_doc::LayerContent::Group(group) = &layer.content {
            layer_rows(&group.children, depth + 1, selected, out);
        }
    }
}

impl eframe::App for PhotoCraftMobile {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}

craft_mobile::export!(PhotoCraftMobile);

/// Build the app on `ctx`, drawing its canvas with wgpu when a render state is given.
fn create_app(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>, services: Services) -> PhotocraftApp {
    PhotocraftApp::install_type_reset_hooks(ctx);
    PhotocraftApp::setup_context(ctx, ThemeKind::Pro);
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.set_theme(ctx, ThemeKind::Pro);
    // Long commands run as background jobs with progress, as on the desktop.
    app.background_jobs = true;
    if let Some(rs) = wgpu {
        log::info!("photocraft-mobile: wgpu {:?} on {}", rs.adapter.get_info().backend, rs.adapter.get_info().name);
        app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        app.set_wgpu(rs);
        app.start_compositor(ctx);
    }
    app
}

/// The in-process codecs (as on the web), and files through the shell's picker and exporter.
fn services(platform: &Platform, inbox: Inbox) -> Services {
    let picker = platform.clone();
    let exporter = platform.clone();
    let clipboard = platform.clone();
    let browser = platform.clone();
    let mut services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &photocraft_doc::Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        file_dialog: Some(Box::new(move |request, _frame, reply| match request {
            // The picker hands over contents, not paths, as a browser does.
            FileDialogRequest::Open { multiple, extensions, .. } => {
                picker.pick_files(multiple, extensions.unwrap_or_default(), Box::new(move |files| reply.send(pick_answer(multiple, files))))
            }
            // Engine commands that read a folder need paths; a picker's folder isn't one.
            FileDialogRequest::Folder { .. } => reply.send(None),
            // The exporter asks where; here only the name is settled.
            FileDialogRequest::Save { suggested } => reply.send(Some(FileDialogAnswer::SaveTo(file_name(&suggested)))),
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            exporter.export(file_name(path), bytes.to_vec());
            Ok(())
        })),
        // The user may cancel the exporter, and nothing reports back: never mark the document saved.
        write_unconfirmed: true,
        inbox: Some(inbox),
        clipboard_set_image: Some(Box::new(move |w, h, rgba: &[u8]| {
            clipboard.copy_image(w, h, rgba.to_vec());
            Ok(())
        })),
        open_url: Some(Box::new(move |url: &str| {
            browser.open_url(url.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    recovery(&mut services, platform.data_dir().map(|dir| dir.join("Recovery")));
    services
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Run `f` on the recovery store (`Err` without a data folder). The service closures never call
/// each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no data folder for crash recovery")?))
}

/// Crash recovery in the app's data folder, as the desktop app keeps it: background .pcraft
/// autosaves, offered again after the OS ended the app.
fn recovery(services: &mut Services, dir: Option<PathBuf>) {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
    let (s1, s2, s3, s4) = (store.clone(), store.clone(), store.clone(), store.clone());
    services.autosave = Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
        with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
    }));
    services.autosave_results = Some(Box::new(move || with_store(&s2, |s| s.take_completed()).unwrap_or_default()));
    services.flush_autosaves = Some(Box::new(move || with_store(&s3, RecoveryStore::flush).unwrap_or_default()));
    services.discard_autosave = Some(Box::new(move |id: u64| {
        let _ = with_store(&s4, |s| s.discard(id));
    }));
    services.recover = Some(Box::new(move || {
        let Some(dir) = recover_dir.as_ref() else { return Vec::new() };
        photocraft_format::list_recovery(dir)
            .into_iter()
            .map(|e| Recoverable {
                key: e.info.key.clone(),
                name: e.info.document_name.clone(),
                path: e.info.original_path.clone(),
                load: Box::new(move || photocraft_format::recover(&e).map_err(|e| e.to_string())),
            })
            .collect()
    }));
    services.adopt_autosave = Some(Box::new(move |id: u64, key: &str| {
        let _ = with_store(&store, |s| s.adopt(id, key));
    }));
}

fn pick_answer(multiple: bool, files: Vec<PickedFile>) -> Option<FileDialogAnswer> {
    let mut files = files.into_iter().map(|f| (f.name, f.bytes));
    if multiple {
        let all: Vec<_> = files.collect();
        (!all.is_empty()).then_some(FileDialogAnswer::ContentsMany(all))
    } else {
        files.next().map(|(name, bytes)| FileDialogAnswer::Contents(name, bytes))
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PickedFile {
        PickedFile { name: name.into(), bytes: vec![1] }
    }

    /// File › Open (several files) and a command reading one file get the shapes they expect;
    /// a cancelled picker answers nothing, never an empty list.
    #[test]
    fn picked_files_answer_the_dialog_that_asked() {
        assert!(matches!(pick_answer(true, vec![file("a.png"), file("b.psd")]), Some(FileDialogAnswer::ContentsMany(f)) if f.len() == 2));
        assert!(matches!(pick_answer(false, vec![file("a.png")]), Some(FileDialogAnswer::Contents(n, _)) if n == "a.png"));
        assert_eq!(pick_answer(true, vec![]), None);
        assert_eq!(pick_answer(false, vec![]), None);
    }

    #[test]
    fn a_save_path_becomes_the_suggested_file_name() {
        assert_eq!(file_name("/var/mobile/Untitled-1.psd"), "Untitled-1.psd");
        assert_eq!(file_name("Untitled-1.pcraft"), "Untitled-1.pcraft");
    }

    /// A Pencil sample reaches the brush engine with its pressure and tilt while it touches, and
    /// lifting it ends the stroke's pen data, as a desktop tablet's does.
    #[test]
    fn pencil_samples_feed_pressure_and_tilt_until_it_lifts() {
        let sample = Stylus { id: 1, phase: TouchPhase::Move, pos: egui::Pos2::ZERO, pressure: 0.4, altitude: 0.0, azimuth: 0.0, eraser: false };
        let pen = pen_sample(&sample).expect("a pen in contact");
        assert!((pen.pressure - 0.4).abs() < 1e-6);
        assert!((pen.tilt_x - 90.0).abs() < 1e-3, "lying flat, leaning right: {pen:?}");
        assert!(pen_sample(&Stylus { phase: TouchPhase::End, ..sample }).is_none());
        assert!(pen_sample(&Stylus { phase: TouchPhase::Cancel, ..sample }).is_none());
        assert!(pen_sample(&Stylus { eraser: true, ..sample }).is_some_and(|p| p.eraser));
    }

    /// The panel lists layers top first, children under their group, as the desktop panel does.
    #[test]
    fn layers_are_listed_top_first_with_groups_nested() {
        let with_id = |mut layer: Layer, id| {
            layer.id = photocraft_doc::LayerId(id);
            layer
        };
        let bottom = with_id(Layer::group("Bottom", vec![]), 1);
        let top = with_id(Layer::group("Top", vec![with_id(Layer::group("Child", vec![]), 3)]), 2);
        let mut rows = Vec::new();
        layer_rows(&[bottom, top], 0, &|id| id.0 == 3, &mut rows);
        let shown: Vec<(&str, u32, bool)> = rows.iter().map(|r| (r.name.as_str(), r.depth, r.selected)).collect();
        assert_eq!(shown, [("Top", 0, false), ("Child", 1, true), ("Bottom", 0, false)]);
    }
}
