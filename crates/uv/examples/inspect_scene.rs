//! Local-only mesh inspection; never copies model assets into the repository.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Supply a model filename")?;
    if std::fs::metadata(&path)?.len() > photocraft_uv::MAX_FILE_BYTES {
        return Err("Model input exceeds the UV file budget".into());
    }
    let base = std::path::Path::new(&path).parent().ok_or("No model directory")?;
    let start = std::time::Instant::now();
    let scene =
        photocraft_uv::import::load(&path, &std::fs::read(&path)?, 0, |name| std::fs::read(base.join(name)).map_err(|e| photocraft_uv::Error(e.to_string())))?;
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
#[cfg(target_arch = "wasm32")]
fn main() {}
