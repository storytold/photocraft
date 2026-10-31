//! Mesh/solver diagnostics only; no rendering or file writes.
//! cargo run --release -p photocraft-ui-egui --example puppet_mesh_bench -- [input.psd ["Layer name"]]
//! Without a layer name, requires one visible, non-background, alpha raster layer at the document root.
use photocraft_algo::puppet::{ITERATIONS, PuppetDensity, PuppetMode, PuppetSolver, build_mesh};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use std::{error::Error, hint::black_box, io::Write, time::Instant};

fn bench(name: &str, surface: &Surface, bounds: Rect) -> Result<(), Box<dyn Error>> {
    if bounds.is_empty() || bounds.width().max(bounds.height()) > 16384 || u64::from(bounds.width()) * u64::from(bounds.height()) > 64_000_000 {
        return Err("benchmark requires nonempty bounds, sides <=16384 and area <=64MP".into());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "\n{name:?}: {}x{}; expansion=2px, mode=Normal, 40 solves x {ITERATIONS} iterations, init=None", bounds.width(), bounds.height())?;
    writeln!(
        out,
        "{:<8} {:>8} {:>9} {:>9} {:>10} {:>10} {:>11} {:>11}",
        "density", "vertices", "triangles", "bandwidth", "mesh_ms", "prep_ms", "median_ms", "p95_ms"
    )?;
    for density in [PuppetDensity::Fewer, PuppetDensity::Normal, PuppetDensity::More] {
        let start = Instant::now();
        let mesh = build_mesh(surface, bounds, density, 2.0);
        let mesh_ms = start.elapsed().as_secs_f64() * 1000.0;
        if mesh.tris.is_empty() {
            return Err(format!("{name}: {density:?} produced no triangles").into());
        }
        // Use only triangle-referenced vertices; choose extremes along the longer axis in O(n).
        let axis = usize::from(bounds.height() > bounds.width());
        let mut points = mesh.tris.iter().flatten().filter_map(|&i| mesh.verts.get(i));
        let first = *points.next().ok_or("mesh has no usable vertices")?;
        let (mut left, mut right) = (first, first);
        for &p in points {
            if p[axis] < left[axis] {
                left = p;
            }
            if p[axis] > right[axis] {
                right = p;
            }
        }
        if left == right {
            return Err("mesh has no distinct pin positions".into());
        }
        // Half-bandwidth of the mesh adjacency, in the builder's reordered vertex numbering.
        let bandwidth = mesh.tris.iter().map(|&[a, b, c]| a.abs_diff(b).max(b.abs_diff(c)).max(c.abs_diff(a))).max().unwrap_or(0);
        let (nv, nt) = (mesh.verts.len(), mesh.tris.len());
        let start = Instant::now();
        let solver = PuppetSolver::new(mesh, &[left, right], &[false; 2]);
        let prep_ms = start.elapsed().as_secs_f64() * 1000.0;
        let amplitude = (right[0] - left[0]).hypot(right[1] - left[1]) * 0.08;
        let mut times = [0.0; 40];
        // Two untimed warmups; every solve still starts from rest, not the preceding result.
        for round in 0..42 {
            let mut moved = right;
            moved[1 - axis] += amplitude * (round as f64 * 0.23).cos();
            let start = Instant::now();
            let result = black_box(solver.solve(&[left, moved], &[None; 2], PuppetMode::Normal, ITERATIONS, None));
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if result.0.iter().flatten().chain(result.1.iter()).any(|v| !v.is_finite()) {
                return Err(format!("{name}: {density:?} produced a nonfinite solve").into());
            }
            if round >= 2 {
                *times.get_mut(round - 2).ok_or("invalid timing slot")? = ms;
            }
        }
        times.sort_by(f64::total_cmp);
        let median = (times[19] + times[20]) / 2.0;
        let p95 = times[37]; // Nearest rank: ceil(0.95 * 40).
        writeln!(out, "{density:<8?} {nv:>8} {nt:>9} {bandwidth:>9} {mesh_ms:>10.2} {prep_ms:>10.2} {median:>11.2} {p95:>11.2}")?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn Error>> {
    use photocraft_doc::LayerContent;
    use std::io::Read;
    let mut args = std::env::args_os().skip(1);
    if let Some(path) = args.next() {
        let layer_name = args.next().map(|s| s.into_string().map_err(|_| "layer name must be Unicode")).transpose()?;
        if args.next().is_some() {
            return Err("usage: puppet_mesh_bench [input.psd [\"Layer name\"]]".into());
        }
        // Bound the source buffer and open read-only; import and selection are not timed.
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 256 * 1024 * 1024 {
            return Err("PSD exceeds benchmark's 256MiB input limit".into());
        }
        if !photocraft_io::is_psd(&bytes) {
            return Err("input must be a PSD/PSB file".into());
        }
        let imported = photocraft_io::import(&path.to_string_lossy(), &bytes)?;
        drop(bytes);
        for warning in &imported.warnings {
            writeln!(std::io::stderr().lock(), "import: {warning}")?;
        }
        let doc = &imported.document;
        let mut candidates = doc.layers.iter().filter_map(|layer| {
            let LayerContent::Raster(surface) = &layer.content else { return None };
            let eligible = match &layer_name {
                Some(name) => layer.name == *name,
                None => layer.visible && !doc.is_background(layer.id) && surface.format().alpha,
            };
            eligible.then_some((layer, surface))
        });
        let (layer, surface) = candidates.next().ok_or("no matching root raster layer; supply an exact layer name")?;
        if candidates.next().is_some() {
            return Err("ambiguous root raster layers; supply a unique exact layer name".into());
        }
        return bench(&layer.name, surface, surface.content_bounds());
    }
    // Build one fixture at a time; solid tiles are shared, never a 24MP point/pixel vector.
    for name in ["broad6000x4000", "strip5800x40", "channel5800x800", "body_with_tail", "curved_stroke"] {
        let mut surface = Surface::new(PixelFormat::RGBA8);
        let red = [1.0, 0.0, 0.0, 1.0];
        let bounds = match name {
            "broad6000x4000" => {
                let r = Rect::new(0, 0, 6000, 4000);
                surface.fill_rect(r, &red);
                r
            }
            "strip5800x40" => {
                let r = Rect::new(0, 0, 5800, 40);
                surface.fill_rect(r, &red);
                r
            }
            "channel5800x800" => {
                let r = Rect::new(0, 0, 5800, 800);
                surface.fill_rect(r, &red);
                r
            }
            "body_with_tail" => {
                surface.fill_rect(Rect::new(0, 0, 2000, 1800), &red);
                surface.fill_rect(Rect::new(1900, 880, 5800, 920), &red);
                Rect::new(0, 0, 5800, 1800)
            }
            _ => {
                for x in 0..5800 {
                    let y = 600 + (500.0 * (x as f64 / 900.0).sin()) as i32;
                    surface.fill_rect(Rect::new(x, y - 20, x + 1, y + 20), &red);
                }
                Rect::new(0, 0, 5800, 1200)
            }
        };
        bench(name, &surface, bounds)?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() -> Result<(), Box<dyn Error>> {
    Err("puppet_mesh_bench requires a native target".into())
}
