//! Reproducible PDF page-picker screenshots using only synthetic documents.
//! cargo run -p photocraft-ui-egui --example pdf_import_demo -- <output-directory>
use egui_kittest::Harness;
use photocraft_doc::{Artboard, ColorMode, Document, Group, Layer, LayerContent, PixelFormat, Rect, SampleType, Size, Surface};
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).unwrap_or_else(|| "plan/evidence".into());
    std::fs::create_dir_all(&output)?;
    let mut doc = Document::new("Synthetic pages", Size::new(144, 208), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 144.0;
    for (i, color) in [[0.2, 0.5, 0.9, 1.0], [0.9, 0.4, 0.2, 1.0]].into_iter().enumerate() {
        let y = i as i32 * 112;
        let rect = Rect::new(0, y, 144, y + 96);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        surface.write_region(rect, &color.repeat(144 * 96));
        doc.layers.push(Layer::new(
            format!("Page {}", i + 1),
            LayerContent::Group(Group {
                children: vec![Layer::new("Rectangle", LayerContent::Raster(surface))],
                expanded: true,
                artboard: Some(Artboard::new(rect)),
            }),
        ));
    }
    doc.layers.reverse();
    let bytes = photocraft_io::export(&doc, "pdf", &photocraft_io::ExportOptions::default())?.bytes;
    std::fs::write(format!("{output}/synthetic-pages.pdf"), &bytes)?;
    for (w, h) in [(1000.0, 750.0), (800.0, 600.0)] {
        for scale in [1.0, 2.0] {
            let mut harness = Harness::builder().with_size(egui::vec2(w, h)).with_pixels_per_point(scale).with_max_steps(64).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::default());
                PhotocraftApp::new(photocraft_engine::Session::new(), Services::default())
            });
            harness.state_mut().open_file("Synthetic pages.pdf", &bytes).map_err(std::io::Error::other)?;
            for theme in ThemeKind::ALL {
                let ctx = harness.ctx.clone();
                let (req, _rx) = ControlRequest::new("ui.set", serde_json::json!({"theme": theme.id()}));
                match handle(harness.state_mut(), &ctx, &req) {
                    Outcome::Done(result) if result["ok"] == true => {}
                    _ => return Err(std::io::Error::other("could not apply screenshot theme").into()),
                }
                for _ in 0..12 {
                    harness.step();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                harness.render()?.save(format!("{output}/pdf-picker-{}-{w:.0}x{h:.0}-{scale:.0}x.png", theme.id()))?;
            }
            if w == 1000.0 && scale == 1.0 {
                harness.state_mut().open_pdf_pages(&[0, 1]).map_err(std::io::Error::other)?;
                photocraft_ui_egui::export_dialog::open_all_pdf(harness.state_mut()).map_err(std::io::Error::other)?;
                harness.run_steps(12);
                harness.render()?.save(format!("{output}/pdf-binder-current.png"))?;
            }
        }
    }
    Ok(())
}
