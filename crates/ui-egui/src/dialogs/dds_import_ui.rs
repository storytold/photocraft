//! DDS Import: the File › Open prompt for a DirectDraw Surface's alpha and mip chain (#8632),
//! the choices the NVIDIA/Intel plug-ins ask: the alpha as the layer's transparency or as an
//! "Alpha 1" document channel (game textures often carry data, such as specular maps, in it
//! rather than transparency), every mip level as a layer or the top level only, and the rows
//! flipped on open.
//!
//! Opens park in [`PhotocraftApp::dds_imports`] while the modal is up and the file is decoded
//! once answered; Cancel drops the open. Commands skip the prompt by passing the import answers
//! (`ddsAlphaChannel`, `ddsMipmaps`, `ddsFlip` on `file.open`, `file.openAs`).

use photocraft_engine::jobs::OpenSource;
use photocraft_io::{DdsCompression, DdsImport, DdsInfo};

use crate::PhotocraftApp;

/// An open waiting on the DDS Import answer.
pub struct Prompt {
    /// The name the document opens under (templates already resolved to "Untitled-N").
    pub name: String,
    /// The file it came from, for File › Save and Open Recent.
    pub path: Option<String>,
    /// The bytes or path the open reads once answered.
    pub source: OpenSource,
    /// The chosen answers: the texture's alpha as a channel, its mip chain as layers, rows
    /// flipped.
    pub options: DdsImport,
    /// Header facts for the dialog, probed without decoding.
    pub info: DdsInfo,
    /// The tab position a drop on the tabs asked for; the open lands there once answered.
    pub slot: Option<usize>,
    /// Its place in a set of files opened together, when it is one of several (see [`OpenOrder`]).
    pub order: Option<crate::jobs_ui::OpenOrder>,
}

/// The import answers a command's params chose (`"ddsAlphaChannel"`, `"ddsMipmaps"`,
/// `"ddsFlip"`), skipping the prompt.
pub fn params(p: &serde_json::Value) -> Result<Option<DdsImport>, String> {
    photocraft_engine::file_cmds::dds_import_param(p, "file.open").map_err(|e| e.to_string())
}

/// What opening `bytes` would ask, from the file's header: a mip chain or an alpha channel
/// there are choices about. `None` for a file that isn't a readable DDS or has none (the
/// answers would change nothing).
fn wants(bytes: &[u8]) -> Option<DdsInfo> {
    let info = photocraft_codecs::dds_info(bytes).ok()?;
    (info.levels > 1 || info.alpha).then_some(info)
}

/// `wants` for a path whose bytes aren't read yet: the probe reads only the header.
#[cfg(not(target_arch = "wasm32"))]
pub fn wants_path(path: &str) -> Option<DdsInfo> {
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    wants(&head[..n])
}

/// Park `name`'s open behind the prompt. The queue keeps later opens' prompts in order.
pub fn park(app: &mut PhotocraftApp, name: String, path: Option<String>, source: OpenSource, info: DdsInfo) {
    app.dds_imports.push_back(Prompt { name, path, source, options: DdsImport::default(), info, slot: None, order: None });
}

/// Park an open whose bytes are in hand (`bytes` is the whole file), when it has choices to
/// make. Returns whether the open parked.
pub fn park_bytes(app: &mut PhotocraftApp, name: String, path: Option<String>, bytes: &[u8]) -> bool {
    match wants(bytes) {
        Some(info) => {
            park(app, name, path, OpenSource::Bytes(std::sync::Arc::new(bytes.to_vec())), info);
            true
        }
        None => false,
    }
}

/// Answers the front prompt and runs its open.
fn confirm(app: &mut PhotocraftApp, p: Prompt) {
    let (docs, opens, held) = (app.session.documents().len(), app.jobs.opens.len(), app.held_opens.len());
    let r = if app.background_jobs {
        crate::jobs_ui::start_open_opts(app, &p.name, p.path.clone(), p.source, Some(p.options))
    } else {
        let bytes: std::borrow::Cow<'_, [u8]> = match &p.source {
            OpenSource::Bytes(b) => std::borrow::Cow::Borrowed(&b[..]),
            #[cfg(not(target_arch = "wasm32"))]
            OpenSource::Path(path) => match photocraft_format::read_file(std::path::Path::new(&path)) {
                Ok(b) => std::borrow::Cow::Owned(b),
                Err(e) => {
                    app.ui.status = crate::i18n::fmt(tl!("Open failed: {error}"), &[("error", &e.to_string())]);
                    app.ui.status_error = true;
                    return;
                }
            },
            #[cfg(target_arch = "wasm32")]
            OpenSource::Path(_) => return,
        };
        app.open_bytes_opts(&p.name, &bytes, p.path.as_deref(), Some(p.options)).map(|_| ())
    };
    if let Err(e) = r {
        app.open_failed(&p.name, &e);
        crate::jobs_ui::end_batch(app);
        return;
    }
    // Where the open lands: a background open's tab, or the document an inline open already
    // appended — a drop's tab slot or the place it held in a set opened together.
    if let Some(tab) = app.jobs.opens.get_mut(opens) {
        if p.slot.is_some() {
            tab.slot = p.slot;
        }
        if p.order.is_some() {
            tab.order = p.order;
        }
        return;
    }
    // A Color Settings or damage prompt can hold the open back: no document was appended to
    // move, so the slot rides the Held and applies when the prompt resolves.
    if app.held_opens.len() > held {
        if let Some(h) = app.held_opens.back_mut() {
            h.slot = p.slot.or_else(|| p.order.and_then(|o| crate::jobs_ui::batch_slot(&mut app.jobs, o)));
        }
        crate::jobs_ui::end_batch(app);
        return;
    }
    let slot = p.slot.or_else(|| p.order.and_then(|o| crate::jobs_ui::batch_slot(&mut app.jobs, o)));
    if let Some(slot) = slot
        && app.session.documents().len() > docs
    {
        let _ = app.run("document.move", serde_json::json!({"document": docs, "to": slot}));
    }
    crate::jobs_ui::end_batch(app);
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.dds_imports.front_mut() else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut options = p.options;
    let modal = egui::Modal::new(egui::Id::unique("dds-import")).show(ctx, |ui| {
        ui.set_max_width(340.0);
        ui.label(egui::RichText::new(tl!("DDS Import")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(crate::i18n::fmt(
            tl!("{name} ({width} × {height})"),
            &[("name", &p.name), ("width", &p.info.width.to_string()), ("height", &p.info.height.to_string())],
        ));
        // What the header holds, like the plug-ins' info line: "BC3 / DXT5 · 10 mip levels".
        let mut facts = vec![
            match p.info.compression {
                Some(DdsCompression::None) | None => tl!("Uncompressed"),
                Some(c) => crate::dialogs::dds_options_ui::compression_label(c),
            }
            .to_string(),
        ];
        if p.info.levels > 1 {
            facts.push(crate::i18n::trn(crate::i18n::current(), u64::from(p.info.levels), "{n} mip level", "{n} mip levels"));
        }
        ui.label(egui::RichText::new(facts.join(" · ")).weak());
        ui.add_space(8.0);
        // Choices the file doesn't have stay out entirely, so the prompt is exactly as big
        // as what it asks.
        if p.info.alpha {
            crate::widgets::checkbox(ui, &mut options.alpha_as_channel, tl!("Load transparency as alpha channel"));
        }
        if p.info.levels > 1 {
            crate::widgets::checkbox(ui, &mut options.mips_as_layers, tl!("Mipmaps as separate layers"));
        }
        crate::widgets::checkbox(ui, &mut options.flip, tl!("Flip vertically"));
        ui.add_space(12.0);
        // The strip gets only the button height: `with_layout` would give it the area's whole
        // remembered rect, centring the buttons in leftover space and keeping a queued prompt
        // the size of the tallest one.
        let strip = egui::vec2(ui.available_width(), crate::widgets::button_height(ui.ctx()));
        ui.allocate_ui_with_layout(strip, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.options = options;
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        let Some(p) = app.dds_imports.pop_front() else { return };
        confirm(app, p);
    } else if cancel {
        app.dds_imports.pop_front();
        crate::jobs_ui::end_batch(app);
        app.ui.status = tl!("Open cancelled").into();
        app.ui.status_error = false;
    }
}
