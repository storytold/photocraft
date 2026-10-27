//! Generate original, deterministic PSD artwork reproducing oblique group stroke artifacts.
//! `cargo run -p photocraft-io --example stroke_repro -- /tmp/stroke-repro`
//! Then resize master.psd to 588² and replace its Bar smart object's contents with bar.psb.
//! Use the CLI's info command on each PSD to find its imported layer ids.

#[path = "../../compose/tests/support/stroke_fixture.rs"]
mod stroke_fixture;

use std::{error::Error, path::PathBuf, sync::Arc};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_io::{ExportOptions, export};
use photocraft_raster::Surface;

fn detail_repro(path: &std::path::Path, opts: &ExportOptions) -> Result<(), Box<dyn Error>> {
    // Horizontal lines must survive a reduction of the width alone. The old isotropic
    // prefilter blurred them vertically before placing the already-reduced image again.
    let mut pixels = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    for y in 0..80 {
        let v = if y % 3 == 0 { 0.8 } else { 0.2 };
        pixels.fill_rect(Rect::new(0, y, 96, y + 1), &[v, 1.0 - v, 0.4, 1.0]);
    }
    let mut source = Document::new("Original stripe pattern", Size::new(96, 80), ColorMode::Rgb, SampleType::U8);
    source.layers.push(Layer::new("Stripes", LayerContent::Raster(pixels.clone())));
    let bytes = export(&source, "detail.psb", opts)?.bytes;
    std::fs::write(path.join("detail.psb"), &bytes)?;
    std::fs::write(path.join("detail-source.png"), export(&source, "detail.png", opts)?.bytes)?;
    let transform = Affine { m: [0.5, 0.0, 0.0, 1.0, 12.25, 12.0] };
    let placed = photocraft_algo::warp::place_source(&pixels, source.bounds(), &transform, None)?;
    let smart = SmartObject::new(SmartSource::Embedded { file_name: "detail.psb".into(), bytes: Arc::new(bytes) }, transform, Some(placed));
    let mut master = Document::new("One-axis detail reproduction", Size::new(80, 104), ColorMode::Rgb, SampleType::U8);
    master.layers.push(Layer::new("Detail", LayerContent::Smart(smart)));
    std::fs::write(path.join("detail-master.psd"), export(&master, "detail.psd", opts)?.bytes)?;
    std::fs::write(path.join("detail.png"), export(&master, "detail.png", opts)?.bytes)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(std::env::args_os().nth(1).ok_or("provide an output directory")?);
    std::fs::create_dir_all(&path)?;
    let mut master = stroke_fixture::fixture(1, SampleType::U8, true);
    let native = stroke_fixture::fixture(4, SampleType::U8, false);
    let pixels = native.layers.first().and_then(Layer::children).and_then(|l| l.first()).and_then(Layer::surface).ok_or("missing native pixels")?;
    let mut source = Document::new("Original blue bar", native.size, native.mode, native.depth);
    source.layers.push(Layer::new("Bar pixels", LayerContent::Raster(pixels.clone())));
    let opts = ExportOptions::default();
    let bytes = export(&source, "bar.psb", &opts)?.bytes;
    std::fs::write(path.join("bar.psb"), &bytes)?;
    let group = master.layers.first_mut().ok_or("missing group")?;
    let child = group.children_mut().and_then(|l| l.first_mut()).ok_or("missing smart object")?;
    if let LayerContent::Smart(sm) = &mut child.content {
        sm.source = SmartSource::Embedded { file_name: "bar.psb".into(), bytes: Arc::new(bytes) };
        sm.transform.m = [0.25, 0.0, 0.0, 0.25, 0.0, 0.0];
    }
    std::fs::write(path.join("master.psd"), export(&master, "master.psd", &opts)?.bytes)?;
    std::fs::write(path.join("master.png"), export(&master, "master.png", &opts)?.bytes)?;
    std::fs::write(path.join("direct-588.png"), export(&stroke_fixture::fixture(3, SampleType::U8, true), "direct.png", &opts)?.bytes)?;
    // A fractional native placement generates separated positive cubic lobes in the old
    // sampler. The styled group turns those nearly invisible samples into white bumps.
    let mut shifted = stroke_fixture::fixture(3, SampleType::U8, true);
    let transform = photocraft_geom::Affine { m: [0.75, 0.0, 0.0, 0.75, 0.25, 0.25] };
    let cache = photocraft_algo::warp::place_source(pixels, native.bounds(), &transform, None)?;
    let group = shifted.layers.first_mut().ok_or("missing shifted group")?;
    let child = group.children_mut().and_then(|l| l.first_mut()).ok_or("missing shifted smart object")?;
    if let LayerContent::Smart(sm) = &mut child.content {
        sm.source = SmartSource::Embedded { file_name: "bar.psb".into(), bytes: Arc::new(std::fs::read(path.join("bar.psb"))?) };
        sm.transform = transform;
        sm.cache = Some(cache);
    }
    std::fs::write(path.join("shifted-master.psd"), export(&shifted, "shifted.psd", &opts)?.bytes)?;
    std::fs::write(path.join("shifted.png"), export(&shifted, "shifted.png", &opts)?.bytes)?;
    detail_repro(&path, &opts)?;
    println!("Generated master.psd, shifted-master.psd and detail-master.psd; use photocraft-cli info to find their layer ids.");
    Ok(())
}
