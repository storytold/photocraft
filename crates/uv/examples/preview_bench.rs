//! Small synthetic, original-asset benchmark; not a production-scene FPS guarantee.
//! cargo run -p photocraft-uv --release --example preview_bench
#[cfg(not(target_arch = "wasm32"))]
use photocraft_uv::{
    import,
    preview::{self, Camera, Image, Materials},
};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/normals-reflected.fbx");
    let scene = import::load("sphere.fbx", &std::fs::read(path)?, 0, |_| Err(photocraft_uv::Error("No external dependencies".into())))?;
    let colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.8, 0.8, 0.8, 1.0]; 2048 * 2048] })).collect();
    let normals: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 2048 * 2048] })).collect();
    let maps = Materials { colors: &colors, normals: &normals, ..Default::default() };
    let camera = Camera { shaded: true, ..Default::default() };
    for (w, h) in [(1024, 576), (1920, 1080), (3840, 2160)] {
        let mut times = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            let frame = preview::render_materials(&scene, camera, &maps, &[], w, h)?;
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(&frame);
        }
        times.sort_by(f64::total_cmp);
        println!("{w}x{h}: median {:.2} ms ({} triangles; 2K color and normal maps)", times.get(1).copied().unwrap_or(0.0), scene.triangles.len());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
