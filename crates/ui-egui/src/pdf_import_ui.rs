//! A PDF binder is inspected before any editing documents are created.
use std::sync::{Arc, mpsc};

use crate::PhotocraftApp;
use photocraft_engine::jobs::OpenSource;

type Preview = Result<(u32, u32, Vec<u8>), String>;

pub(crate) struct Picker {
    name: String,
    path: Option<String>,
    bytes: Arc<Vec<u8>>,
    sizes: Vec<(f32, f32)>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    preview: usize,
    loading: Option<(usize, mpsc::Receiver<Preview>)>,
    shown: Option<usize>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
    resolution: f32,
    pub slot: Option<usize>,
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
        selected: vec![false; sizes.len()],
        sizes,
        anchor: None,
        preview: 0,
        loading: None,
        shown: None,
        texture: None,
        error: None,
        resolution: 144.0,
        slot: None,
    });
    app.ui.status = "Choose PDF pages to open".into();
    app.ui.status_error = false;
    Ok(())
}

impl PhotocraftApp {
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

fn preview(p: &mut Picker, ctx: &egui::Context) {
    if let Some((index, rx)) = &p.loading {
        match rx.try_recv() {
            Ok(result) => {
                if *index == p.preview {
                    p.shown = Some(*index);
                    match result {
                        Ok((w, h, pixels)) => {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
                            p.texture = Some(ctx.load_texture("pdf-page-preview", image, egui::TextureOptions::LINEAR));
                            p.error = None;
                        }
                        Err(e) => {
                            p.error = Some(e);
                            p.texture = None;
                        }
                    }
                }
                p.loading = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                p.loading = None;
                p.shown = Some(p.preview);
                p.error = Some("Preview unavailable".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    if p.loading.is_none() && p.shown != Some(p.preview) {
        let (tx, rx) = mpsc::channel();
        let bytes = p.bytes.clone();
        let index = p.preview;
        p.texture = None;
        p.error = None;
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

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.pdf_pickers.front_mut() else { return };
    preview(p, ctx);
    let (mut open, mut all, mut cancel) = (false, false, false);
    let modal = egui::Modal::new(egui::Id::new("pdf-page-picker")).show(ctx, |ui| {
        ui.set_width(650.0);
        ui.heading(tl!("Open PDF pages"));
        ui.label(&p.name);
        ui.label(crate::i18n::fmt(tl!("{count} pages • Each selected page opens in its own tab"), &[("count", &p.sizes.len().to_string())]));
        ui.label(tl!("Click to select pages. Shift-click selects a range."));
        ui.separator();
        ui.horizontal_top(|ui| {
            ui.set_height(360.0);
            egui::ScrollArea::vertical().max_width(200.0).max_height(360.0).id_salt("pdf-pages").show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(185.0);
                    for i in 0..p.selected.len() {
                        ui.horizontal(|ui| {
                            let mut checked = p.selected[i];
                            let label = crate::i18n::fmt(tl!("Page {number}"), &[("number", &(i + 1).to_string())]);
                            if crate::widgets::checkbox(ui, &mut checked, &label).changed() {
                                select(&mut p.selected, &mut p.anchor, i, ui.input(|i| i.modifiers.shift), checked);
                                p.preview = i;
                            }
                            if ui.small_button(tl!("Preview")).clicked() {
                                p.preview = i;
                            }
                        });
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_max_size(egui::vec2(420.0, 360.0));
                ui.set_min_size(egui::vec2(420.0, 360.0));
                let (w, h) = p.sizes[p.preview];
                ui.label(crate::i18n::fmt(
                    tl!("Page {number} — {width} × {height} in"),
                    &[("number", &(p.preview + 1).to_string()), ("width", &format!("{:.1}", w / 72.0)), ("height", &format!("{:.1}", h / 72.0))],
                ));
                if let Some(texture) = &p.texture {
                    ui.add(egui::Image::new(texture).max_size(egui::vec2(420.0, 330.0)).maintain_aspect_ratio(true));
                } else if let Some(error) = &p.error {
                    ui.label(error);
                } else {
                    ui.spinner();
                    ui.label(tl!("Loading preview…"));
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Resolution (ppi)"));
            crate::widgets::value_field(ui, &mut p.resolution, 1.0..=2400.0, "", 80.0).labelled_by(label.id);
        });
        let (w, h) = p.sizes[p.preview];
        ui.label(crate::i18n::fmt(
            tl!("Preview page: {width} × {height} pixels"),
            &[("width", &format!("{:.0}", (w * p.resolution / 72.0).ceil())), ("height", &format!("{:.0}", (h * p.resolution / 72.0).ceil()))],
        ));
        ui.label(tl!("Pages open as RGB 8-bit images. PDF text and vector objects are rasterized."));
        ui.horizontal(|ui| {
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 90.0).clicked();
            let all_label = crate::i18n::fmt(tl!("Open all {count} pages"), &[("count", &p.sizes.len().to_string())]);
            all = crate::widgets::secondary_button(ui, &all_label, 130.0).clicked();
            let count = p.selected.iter().filter(|&&v| v).count();
            let selected_label = crate::i18n::fmt(tl!("Open selected ({count})"), &[("count", &count.to_string())]);
            open = ui.add_enabled_ui(count > 0, |ui| crate::widgets::primary_button(ui, &selected_label, 140.0)).inner.clicked();
        });
    });
    cancel |= modal.should_close();
    if open || all {
        let pages: Vec<usize> = p.selected.iter().enumerate().filter_map(|(i, &v)| (all || v).then_some(i)).collect();
        let name = p.name.clone();
        if let Err(e) = app.open_pdf_pages(&pages) {
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
            shown: Some(0),
            texture: None,
            error: None,
            resolution: 144.0,
            slot: None,
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
                shown: Some(0),
                texture: None,
                error: None,
                resolution: 144.0,
                slot: None,
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
            for label in ["Open PDF pages", "Resolution (ppi)", "Cancel", "Open selected (1)"] {
                assert!(viewport.contains_rect(h.get_by_label(label).rect()), "{label} must fit at {scale}x");
            }
        }
    }
}
