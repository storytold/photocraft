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

use std::sync::PoisonError;

use craft_mobile::{Action, CraftApp, DocumentInfo, Item, PickedFile, Platform, Snapshot};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Layer;
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, Inbox, PhotocraftApp, Services};
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
    Services {
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
        ..Default::default()
    }
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

/// The Android entry point, called by `NativeActivity` (through android-activity's glue) on its
/// own thread. Until Android moves to craft-mobile's host under Compose, eframe runs it, without
/// a shell to pick or export files.
#[cfg(target_os = "android")]
#[expect(unsafe_code, reason = "the symbol android-activity's glue looks up; Rust owns the AndroidApp it is handed")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("photocraft"));
    let options = eframe::NativeOptions {
        system_font_fallback: false,
        android_app: Some(app),
        wgpu_options: PhotoCraftMobile::wgpu_configuration(),
        ..Default::default()
    };
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| Ok(Box::new(PhotoCraftMobile::create(&cc.egui_ctx, cc.wgpu_render_state.clone(), Platform::default())))),
    );
    if let Err(e) = result {
        log::error!("photocraft-mobile: {e}");
    }
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
