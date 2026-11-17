//! Local-only mesh inspection; never copies model assets into the repository.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Supply a model filename")?;
    let path = std::path::Path::new(&path).canonicalize()?;
    let base = path.parent().ok_or("No model directory")?;
    let start = std::time::Instant::now();
    let bytes = read_bounded(&path)?;
    let scene = photocraft_uv::import::load(path.to_string_lossy().as_ref(), &bytes, 0, |name| {
        let dependency = base.join(name).canonicalize().map_err(|e| photocraft_uv::Error(e.to_string()))?;
        if !dependency.starts_with(base) {
            return Err(photocraft_uv::Error("Material dependency escapes the model directory".into()));
        }
        read_bounded(&dependency).map_err(|e| photocraft_uv::Error(e.to_string()))
    })?;
    println!(
        "positions={} triangles={} materials={} import={:.2}ms",
        scene.positions.len(),
        scene.triangles.len(),
        scene.materials.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    for (id, m) in scene.materials.iter().enumerate() {
        println!(
            "material {id}: {} triangles={} color={:?} normal={:?} rgba={:?}",
            m.name,
            scene.triangles.iter().filter(|t| t.material == id).count(),
            m.base_color,
            m.normal_map,
            m.color
        );
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(photocraft_uv::MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > photocraft_uv::MAX_FILE_BYTES {
        return Err("Input exceeds the UV file budget".into());
    }
    Ok(bytes)
}
#[cfg(target_arch = "wasm32")]
fn main() {}
