//! A PDF binder is inspected before any editing documents are created.
use std::{
    collections::VecDeque,
    sync::{Arc, mpsc},
};

use crate::PhotocraftApp;
use photocraft_engine::jobs::OpenSource;

type Preview = Result<(u32, u32, Vec<u8>), String>;
type Thumbnail = Result<egui::TextureHandle, String>;
const THUMBNAIL_CACHE_LIMIT: usize = 32;

#[derive(Clone, Copy, Default, PartialEq)]
enum ThumbnailSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl ThumbnailSize {
    fn image_size(self) -> egui::Vec2 {
        match self {
            Self::Small => egui::vec2(78.0, 104.0),
            Self::Medium => egui::vec2(117.0, 156.0),
            Self::Large => egui::vec2(156.0, 208.0),
        }
    }
}

pub(crate) struct Picker {
    name: String,
    path: Option<String>,
    bytes: Arc<Vec<u8>>,
    sizes: Vec<(f32, f32)>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    preview: usize,
    loading: Option<(usize, mpsc::Receiver<Preview>)>,
    thumbnails: VecDeque<(usize, Thumbnail)>,
    thumbnail_size: ThumbnailSize,
    resolution: f32,
    pub slot: Option<usize>,
    pub target: Option<photocraft_doc::DocId>,
}

pub(crate) fn is_pdf(name: &str, bytes: &[u8]) -> bool {
    name.to_ascii_lowercase().ends_with(".pdf") || bytes.starts_with(b"%PDF-")
}

pub(crate) fn queue(app: &mut PhotocraftApp, name: &str, path: Option<String>, bytes: &[u8]) -> Result<(), String> {
    let sizes = photocraft_io::pdf::page_sizes(bytes).map_err(|e| e.to_string())?;
    app.pdf_pickers.push_back(Picker {
        name: name.into(),
        path,
        bytes: Arc::new(bytes.to_vec()),
        selected: (0..sizes.len()).map(|i| i == 0).collect(),
        sizes,
        anchor: Some(0),
        preview: 0,
        loading: None,
        thumbnails: VecDeque::new(),
        thumbnail_size: ThumbnailSize::default(),
        resolution: 144.0,
        slot: None,
        target: app.session.active().map(|d| d.doc.id),
    });
    app.ui.status = "Choose PDF pages".into();
    app.ui.status_error = false;
    Ok(())
}

impl PhotocraftApp {
    /// Confirm pages using the destination captured when the picker opened.
    pub fn confirm_pdf_pages(&mut self, pages: &[usize]) -> Result<(), String> {
        let p = self.pdf_pickers.front().ok_or("no PDF is waiting for a page selection")?;
        let Some(target) = p.target else { return self.open_pdf_pages(pages) };
        if pages.is_empty() || pages.iter().any(|&i| i >= p.sizes.len()) {
            return Err("Choose at least one valid page".into());
        }
        let (name, bytes, resolution) = (p.name.clone(), p.bytes.clone(), p.resolution);
        self.refocus(target)?;
        photocraft_engine::file_cmds::place_bytes(
            &mut self.session,
            &name,
            bytes.as_ref().clone(),
            None,
            &serde_json::json!({"pages": pages, "resolution": resolution}),
        )
        .map_err(|e| e.to_string())?;
        self.pdf_pickers.pop_front();
        self.sync_views();
        self.ui.status = format!("Placed {} PDF page(s) as smart objects", pages.len());
        self.ui.status_error = false;
        Ok(())
    }

    /// Set the raster resolution for the pending open/drop picker.
    pub fn set_pdf_import_resolution(&mut self, resolution: f32) -> Result<(), String> {
        photocraft_io::pdf::ImportOptions { resolution }.validate().map_err(|e| e.to_string())?;
        self.pdf_pickers.front_mut().ok_or("no PDF is waiting for a page selection")?.resolution = resolution;
        Ok(())
    }

    /// The pending PDF binder's page count. No page is opened until confirmed.
    pub fn pending_pdf_pages(&self) -> Option<usize> {
        self.pdf_pickers.front().map(|p| p.sizes.len())
    }

    /// Confirm zero-based page indices from the PDF picker. Each becomes a separate tab.
    pub fn open_pdf_pages(&mut self, pages: &[usize]) -> Result<(), String> {
        let p = self.pdf_pickers.front().ok_or("no PDF is waiting for a page selection")?;
        if pages.is_empty() || pages.iter().any(|&i| i >= p.sizes.len()) {
            return Err("Choose at least one valid page".into());
        }
        // Retain the picker until decoding or starting the job succeeds. A bad resolution
        // must be correctable without reopening the source or losing the selected pages.
        let (name, path, bytes, resolution, slot) = (p.name.clone(), p.path.clone(), Arc::clone(&p.bytes), p.resolution, p.slot);
        if self.background_jobs {
            let count = self.jobs.opens.len();
            crate::jobs_ui::start_open(self, &name, path, OpenSource::PdfPages { bytes, pages: pages.to_vec(), resolution })?;
            if let Some(tab) = self.jobs.opens.get_mut(count) {
                tab.slot = slot;
            }
        } else {
            let imported = photocraft_io::pdf::import_pdf_pages_with(
                &name,
                &bytes,
                Some(pages),
                photocraft_io::pdf::ImportOptions { resolution },
                &photocraft_raster::Interrupt::default(),
            )
            .map_err(|e| e.to_string())?;
            let documents = photocraft_engine::jobs::pdf_page_documents(&imported.document).map_err(|e| e.to_string())?;
            let first = self.session.documents().len();
            for doc in documents {
                self.session.open_document(doc, None);
                if let Some(st) = self.session.active_mut() {
                    st.source_read_only = true;
                }
            }
            self.session.set_active(first);
            if let Some(path) = path {
                self.opened_from(&path);
            }
            self.sync_views();
            self.ui.status = format!("Opened {} PDF page(s) in separate tabs", pages.len());
        }
        self.pdf_pickers.pop_front();
        Ok(())
    }

    /// Dismiss the current picker without creating or changing a document.
    pub fn cancel_pdf_import(&mut self) {
        self.pdf_pickers.pop_front();
        self.ui.status = "PDF import cancelled".into();
    }
}

fn select(selected: &mut [bool], anchor: &mut Option<usize>, index: usize, shift: bool, value: bool) {
    if shift && let Some(start) = *anchor {
        selected[start.min(index)..=start.max(index)].fill(true);
    } else {
        selected[index] = value;
        *anchor = Some(index);
    }
}

fn click_page(p: &mut Picker, index: usize, modifiers: egui::Modifiers) {
    let Some(&selected) = p.selected.get(index) else { return };
    let additive = modifiers.ctrl || modifiers.command;
    if !modifiers.shift && !additive {
        p.selected.fill(false);
    }
    select(&mut p.selected, &mut p.anchor, index, modifiers.shift, !additive || !selected);
    p.preview = index;
}

fn cache_thumbnail(p: &mut Picker, index: usize, thumbnail: Thumbnail) {
    p.thumbnails.retain(|(page, _)| *page != index);
    while p.thumbnails.len() >= THUMBNAIL_CACHE_LIMIT {
        p.thumbnails.pop_front();
    }
    p.thumbnails.push_back((index, thumbnail));
}

fn poll_preview(p: &mut Picker, ctx: &egui::Context) {
    if let Some((index, rx)) = &p.loading {
        match rx.try_recv() {
            Ok(result) => {
                let thumbnail = result.and_then(|(w, h, pixels)| {
                    if w == 0 || h == 0 || w > 420 || h > 420 || pixels.len() != w as usize * h as usize * 4 {
                        return Err("Invalid PDF preview dimensions".into());
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
                    Ok(ctx.load_texture(format!("pdf-page-thumbnail-{index}"), image, egui::TextureOptions::LINEAR))
                });
                let index = *index;
                cache_thumbnail(p, index, thumbnail);
                p.loading = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let index = *index;
                cache_thumbnail(p, index, Err("Preview unavailable".into()));
                p.loading = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}

fn request_preview(p: &mut Picker, ctx: &egui::Context, visible: &[usize]) {
    if p.loading.is_none()
        && let Some(&index) = visible.iter().find(|&&index| !p.thumbnails.iter().any(|(page, _)| *page == index))
    {
        let (tx, rx) = mpsc::channel();
        let bytes = p.bytes.clone();
        let render = move || {
            let _ = tx.send(photocraft_io::pdf::page_preview(bytes, index).map_err(|e| e.to_string()));
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(render);
        #[cfg(target_arch = "wasm32")]
        render();
        p.loading = Some((index, rx));
    }
    if p.loading.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn thumbnail(ui: &mut egui::Ui, p: &mut Picker, index: usize) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let max = p.thumbnail_size.image_size();
    let (rect, response) = ui.allocate_exact_size(max + egui::vec2(8.0, 26.0), egui::Sense::click());
    let selected = p.selected.get(index).copied().unwrap_or(false);
    let label = crate::i18n::fmt(tl!("Page {number}"), &[("number", &(index + 1).to_string())]);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, &label));
    if ui.is_rect_visible(rect) {
        if selected || response.hovered() {
            ui.painter().rect_filled(rect, t.radius_sm, if selected { t.accent_soft } else { t.hover });
        }
        let (w, h) = p.sizes.get(index).copied().unwrap_or((1.0, 1.0));
        let size = if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
            let normalized = egui::vec2(w, h) / w.max(h);
            normalized * (max.x / normalized.x).min(max.y / normalized.y)
        } else {
            max
        };
        let paper = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 4.0 + max.y / 2.0), size);
        let cached = p.thumbnails.iter().find(|(page, _)| *page == index).map(|(_, thumbnail)| thumbnail);
        match cached {
            Some(Ok(texture)) => {
                ui.painter().image(texture.id(), paper, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            _ => {
                ui.painter().rect_filled(paper, 0.0, t.card);
                let placeholder = if matches!(cached, Some(Err(_))) { "!" } else { "…" };
                ui.painter().text(paper.center(), egui::Align2::CENTER_CENTER, placeholder, egui::TextStyle::Body.resolve(ui.style()), t.text_dim);
            }
        }
        ui.painter().rect_stroke(
            paper.expand(2.0),
            0.0,
            egui::Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.field_border }),
            egui::StrokeKind::Outside,
        );
        if response.has_focus() {
            ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        ui.painter().text(
            egui::pos2(rect.center().x, rect.bottom() - 10.0),
            egui::Align2::CENTER_CENTER,
            (index + 1).to_string(),
            egui::TextStyle::Body.resolve(ui.style()),
            t.text,
        );
    }
    let response = match p.thumbnails.iter().find(|(page, _)| *page == index) {
        Some((_, Err(error))) => response.on_hover_text(error),
        _ => response.on_hover_text(&label),
    };
    if response.clicked() || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter))) {
        response.request_focus();
        click_page(p, index, ui.input(|i| i.modifiers));
    }
    response
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.pdf_pickers.front_mut() else { return };
    poll_preview(p, ctx);
    let mut visible = Vec::new();
    let (mut open, mut all, mut cancel) = (false, false, false);
    let modal = egui::Modal::new(egui::Id::new("pdf-page-picker")).show(ctx, |ui| {
        ui.set_width(650.0);
        let placing = p.target.is_some();
        ui.heading(if placing { tl!("Place PDF pages as smart objects") } else { tl!("Open PDF pages") });
        ui.label(&p.name);
        ui.label(crate::i18n::fmt(
            if placing {
                tl!("{count} pages • Each selected page becomes a smart object layer")
            } else {
                tl!("{count} pages • Each selected page opens in its own tab")
            },
            &[("count", &p.sizes.len().to_string())],
        ));
        ui.label(tl!("Ctrl/Cmd-click adds pages. Shift-click selects a range."));
        ui.separator();
        let t = crate::theme::Tokens::get(ui.ctx());
        egui::Frame::new().fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).inner_margin(8.0).show(ui, |ui| {
            let card = p.thumbnail_size.image_size() + egui::vec2(8.0, 26.0);
            let columns = ((ui.available_width() - ui.spacing().scroll.bar_width) / (card.x + ui.spacing().item_spacing.x)).floor().max(1.0) as usize;
            egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, false]).id_salt("pdf-pages").show_rows(
                ui,
                card.y,
                p.sizes.len().div_ceil(columns),
                |ui, rows| {
                    for row in rows {
                        ui.horizontal(|ui| {
                            for index in row * columns..((row + 1) * columns).min(p.sizes.len()) {
                                let response = ui.push_id(index, |ui| thumbnail(ui, p, index)).inner;
                                if ui.is_rect_visible(response.rect) {
                                    visible.push(index);
                                }
                            }
                        });
                    }
                },
            );
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Thumbnail size"));
            crate::widgets::dropdown(
                ui,
                "pdf-thumbnail-size",
                &mut p.thumbnail_size,
                &[(ThumbnailSize::Small, tl!("Small")), (ThumbnailSize::Medium, tl!("Medium")), (ThumbnailSize::Large, tl!("Large"))],
                110.0,
            );
        });
        ui.separator();
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Resolution (ppi)"));
            crate::widgets::value_field(ui, &mut p.resolution, 1.0..=2400.0, "", 80.0).labelled_by(label.id);
        });
        let (w, h) = p.sizes[p.preview];
        ui.label(crate::i18n::fmt(
            tl!("Page {number} — {width} × {height} in"),
            &[("number", &(p.preview + 1).to_string()), ("width", &format!("{:.1}", w / 72.0)), ("height", &format!("{:.1}", h / 72.0))],
        ));
        ui.label(crate::i18n::fmt(
            tl!("Preview page: {width} × {height} pixels"),
            &[("width", &format!("{:.0}", (w * p.resolution / 72.0).ceil())), ("height", &format!("{:.0}", (h * p.resolution / 72.0).ceil()))],
        ));
        ui.label(if placing {
            tl!("Smart objects contain rasterized pages at this resolution. PDF text and vectors remain rasterized.")
        } else {
            tl!("Pages open as RGB 8-bit images. PDF text and vector objects are rasterized.")
        });
        ui.horizontal(|ui| {
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 90.0).clicked();
            let all_label = crate::i18n::fmt(
                if placing { tl!("Place all {count} pages") } else { tl!("Open all {count} pages") },
                &[("count", &p.sizes.len().to_string())],
            );
            all = crate::widgets::secondary_button(ui, &all_label, 130.0).clicked();
            let count = p.selected.iter().filter(|&&v| v).count();
            let selected_label =
                crate::i18n::fmt(if placing { tl!("Place selected ({count})") } else { tl!("Open selected ({count})") }, &[("count", &count.to_string())]);
            open = ui.add_enabled_ui(count > 0, |ui| crate::widgets::primary_button(ui, &selected_label, 140.0)).inner.clicked();
        });
    });
    cancel |= modal.should_close();
    if !open && !all && !cancel {
        request_preview(p, ctx, &visible);
    }
    if open || all {
        let pages: Vec<usize> = p.selected.iter().enumerate().filter_map(|(i, &v)| (all || v).then_some(i)).collect();
        let name = p.name.clone();
        if let Err(e) = app.confirm_pdf_pages(&pages) {
            app.open_failed(&name, &e);
        }
    } else if cancel {
        app.cancel_pdf_import();
    }
}

#[cfg(test)]
mod tests {
    use super::select;

    #[test]
    fn picker_controls_fit_and_shift_click_selects_visible_range() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.pdf_pickers.push_back(super::Picker {
            name: "Binder.pdf".into(),
            path: None,
            bytes: std::sync::Arc::new(vec![]),
            sizes: vec![(72.0, 36.0); 13],
            selected: vec![false; 13],
            anchor: None,
            preview: 0,
            loading: None,
            thumbnails: (0..13).map(|i| (i, Err("Test preview".into()))).collect(),
            thumbnail_size: super::ThumbnailSize::Small,
            resolution: 144.0,
            slot: None,
            target: None,
        });
        // The harness draws immediately; bind themed fonts before drawing buttons.
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let draw_ready = ready.clone();
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 750.0)).build_ui_state(
            move |ui, app: &mut crate::PhotocraftApp| {
                if draw_ready.get() {
                    super::show(app, ui.ctx());
                }
            },
            app,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        ready.set(true);
        h.run_steps(4);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 750.0));
        for label in ["Open PDF pages", "Page 1", "Open all 13 pages", "Cancel"] {
            assert!(viewport.contains_rect(h.get_by_label(label).rect()), "{label} must fit on screen");
        }
        h.get_by_label("Page 2").click();
        h.run_steps(4);
        assert_eq!(h.state().pdf_pickers[0].anchor, Some(1));
        h.get_by_label("Page 5").click_modifiers(egui::Modifiers::SHIFT);
        h.run_steps(4);
        assert!(h.state().pdf_pickers[0].selected[1..5].iter().all(|&v| v), "{:?}", h.state().pdf_pickers[0].selected);
        assert_eq!(h.state().pdf_pickers[0].selected.iter().filter(|&&v| v).count(), 4);
        assert!(viewport.contains_rect(h.get_by_label("Open selected (4)").rect()));
        h.get_by_label("Page 7").click_modifiers(egui::Modifiers::CTRL);
        h.run_steps(4);
        assert_eq!(h.state().pdf_pickers[0].selected.iter().filter(|&&v| v).count(), 5);
        h.get_by_label("Page 7").click_modifiers(egui::Modifiers::CTRL);
        h.run_steps(4);
        assert_eq!(h.state().pdf_pickers[0].selected.iter().filter(|&&v| v).count(), 4);
        h.get_by_label("Page 1").click();
        h.run_steps(4);
        assert_eq!(h.state().pdf_pickers[0].selected.iter().filter(|&&v| v).count(), 1);
        assert!(h.state().pdf_pickers[0].selected[0]);
    }

    #[test]
    fn shift_click_selects_inclusive_ranges_in_both_directions() {
        let mut selected = [false; 8];
        let mut anchor = None;
        select(&mut selected, &mut anchor, 2, false, true);
        select(&mut selected, &mut anchor, 5, true, true);
        assert_eq!(selected, [false, false, true, true, true, true, false, false]);
        select(&mut selected, &mut anchor, 0, true, true);
        assert_eq!(selected, [true, true, true, true, true, true, false, false]);
        select(&mut selected, &mut anchor, 4, false, false);
        assert!(!selected[4]);
        assert_eq!(anchor, Some(4));
    }

    #[test]
    fn picker_actions_fit_small_windows_at_both_scales() {
        use egui_kittest::{Harness, kittest::Queryable};
        for scale in [1.0, 2.0] {
            for placing in [false, true] {
                for thumbnail_size in [super::ThumbnailSize::Small, super::ThumbnailSize::Medium, super::ThumbnailSize::Large] {
                    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.pdf_pickers.push_back(super::Picker {
                        name: "Binder.pdf".into(),
                        path: None,
                        bytes: std::sync::Arc::new(vec![]),
                        sizes: vec![(72.0, 36.0); 2],
                        selected: vec![true, false],
                        anchor: Some(0),
                        preview: 0,
                        loading: None,
                        thumbnails: (0..2).map(|i| (i, Err("Test preview".into()))).collect(),
                        thumbnail_size,
                        resolution: 144.0,
                        slot: None,
                        target: placing.then(photocraft_doc::DocId::fresh),
                    });
                    let ready = std::rc::Rc::new(std::cell::Cell::new(false));
                    let draw_ready = ready.clone();
                    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_pixels_per_point(scale).build_ui_state(
                        move |ui, app: &mut crate::PhotocraftApp| {
                            if draw_ready.get() {
                                super::show(app, ui.ctx());
                            }
                        },
                        app,
                    );
                    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
                    ready.set(true);
                    h.run_steps(4);
                    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
                    for label in [
                        if placing { "Place PDF pages as smart objects" } else { "Open PDF pages" },
                        "Resolution (ppi)",
                        "Cancel",
                        if placing { "Place selected (1)" } else { "Open selected (1)" },
                    ] {
                        assert!(viewport.contains_rect(h.get_by_label(label).rect()), "{label} must fit at {scale}x");
                    }
                }
            }
        }
    }
}
