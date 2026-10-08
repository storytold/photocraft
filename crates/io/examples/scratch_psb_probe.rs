//! Read-only large-document import probe. Never exports or changes the input file.
use photocraft_raster::spill;
use std::time::Instant;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: scratch_psb_probe <psb> <scratch-directory>")?;
    let dir = args.next().ok_or("missing scratch directory")?;
    spill::configure(Some(std::path::Path::new(&dir)), 1 << 30)?;
    let start = Instant::now();
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let result = photocraft_io::import(&path, &bytes).map_err(|e| e.to_string())?;
    drop(bytes);
    spill::settle();
    let d = &result.document;
    println!(
        "import_seconds={:.3} size={}x{} depth={:?} mode={:?} warnings={:?}",
        start.elapsed().as_secs_f64(),
        d.size.width,
        d.size.height,
        d.depth,
        d.mode,
        result.warnings
    );
    for (_, _, layer) in d.walk() {
        if let Some(surface) = layer.surface() {
            // Access source pixels directly, without creating a GPU canvas or flattening.
            let bounds = surface.tile_bounds();
            let x = bounds.x0.saturating_add(bounds.width() as i32 / 2);
            let y = bounds.y0.saturating_add(bounds.height() as i32 / 2);
            println!("layer={:?} tiles={} bounds={bounds:?} sample={:?}", layer.name, surface.tile_count(), surface.pixel(x, y));
        }
    }
    spill::check_integrity()?;
    println!("scratch={}", serde_json::to_string(&spill::stats()).map_err(|e| e.to_string())?);
    drop(result);
    spill::configure(None, 0)
}
