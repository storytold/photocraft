//! Local BiRefNet General Lite inference. No image data leaves the computer.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use image::{
    GrayImage, Rgb32FImage,
    imageops::{FilterType, resize},
};

const SIDE: usize = 1024;
const MAX_PIXELS: usize = 100_000_000;

/// Infer a continuous foreground matte, resized to the requested output dimensions.
/// RGB samples are straight, display-referred sRGB floats; the source pixels are never changed.
pub fn foreground(rgb: &[[f32; 3]], width: usize, height: usize, out_width: u32, out_height: u32) -> Result<Vec<u8>, String> {
    let count = width.checked_mul(height).filter(|n| *n > 0 && *n <= 4_194_304).ok_or("AI input must contain 1–4194304 pixels")?;
    let output_count = (out_width as usize)
        .checked_mul(out_height as usize)
        .filter(|n| *n > 0 && *n <= MAX_PIXELS)
        .ok_or("AI background removal supports images up to 100 megapixels")?;
    if rgb.len() != count || rgb.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Invalid AI image samples".into());
    }
    let raw: Vec<f32> = rgb.iter().flatten().map(|v| v.clamp(0.0, 1.0)).collect();
    let image = Rgb32FImage::from_raw(width as u32, height as u32, raw).ok_or("Invalid AI image dimensions")?;
    let image = resize(&image, SIDE as u32, SIDE as u32, FilterType::Lanczos3);
    // rembg's BiRefNet session scales by the resized image's brightest channel.
    let peak = image.pixels().flat_map(|p| p.0).map(|v| v.clamp(0.0, 1.0)).fold(1e-6f32, f32::max);
    let mut tensor = vec![0.0; 3 * SIDE * SIDE];
    for (c, (mean, std)) in [(0.485, 0.229), (0.456, 0.224), (0.406, 0.225)].into_iter().enumerate() {
        for (i, pixel) in image.pixels().enumerate() {
            let sample = pixel.0.get(c).ok_or("Invalid RGB channel")?;
            let dest = tensor.get_mut(c * SIDE * SIDE + i).ok_or("Invalid input tensor")?;
            *dest = (sample.clamp(0.0, 1.0) / peak - mean) / std;
        }
    }
    let logits = infer(tensor)?;
    let mask = normalize(&logits)?;
    let mask = GrayImage::from_raw(SIDE as u32, SIDE as u32, mask).ok_or("Invalid AI matte dimensions")?;
    let result = resize(&mask, out_width, out_height, FilterType::Lanczos3).into_raw();
    if result.len() != output_count {
        return Err("Invalid resized AI matte".into());
    }
    Ok(result)
}

fn normalize(logits: &[f32]) -> Result<Vec<u8>, String> {
    if logits.len() != SIDE * SIDE || logits.iter().any(|v| !v.is_finite()) {
        return Err("BiRefNet returned an invalid matte".into());
    }
    let probabilities: Vec<f32> = logits.iter().map(|v| 1.0 / (1.0 + (-v).exp())).collect();
    let min = probabilities.iter().copied().fold(f32::INFINITY, f32::min);
    let max = probabilities.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max - min <= f32::EPSILON {
        return Err("No foreground found by BiRefNet".into());
    }
    Ok(probabilities.into_iter().map(|v| (((v - min) / (max - min)) * 255.0).clamp(0.0, 255.0) as u8).collect())
}

#[cfg(all(not(target_arch = "wasm32"), feature = "onnx"))]
fn model_path() -> Result<std::path::PathBuf, String> {
    let explicit = std::env::var_os("PHOTOCRAFT_BACKGROUND_MODEL");
    let mut paths = Vec::new();
    if let Some(path) = explicit {
        paths.push(std::path::PathBuf::from(path));
    } else {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            paths.push(dir.join("models/birefnet-general-lite.onnx"));
            paths.push(dir.join("../Resources/models/birefnet-general-lite.onnx"));
        }
        paths.push(std::path::PathBuf::from("assets/models/birefnet-general-lite.onnx"));
        #[cfg(debug_assertions)]
        paths.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/birefnet-general-lite.onnx"));
    }
    paths.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        "BiRefNet model is missing. Run scripts/setup-background-removal.sh, or set PHOTOCRAFT_BACKGROUND_MODEL to birefnet-general-lite.onnx".into()
    })
}

#[cfg(all(not(target_arch = "wasm32"), feature = "onnx"))]
fn infer(tensor: Vec<f32>) -> Result<Vec<f32>, String> {
    use ort::{session::Session, value::Tensor};
    let path = model_path()?;
    let mut session = Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| format!("Could not load BiRefNet: {e}"))?;
    let input = Tensor::from_array(([1usize, 3, SIDE, SIDE], tensor)).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs![input]).map_err(|e| format!("BiRefNet inference failed: {e}"))?;
    let output = outputs.iter().next().ok_or("BiRefNet returned no output")?.1;
    let (shape, values) = output.try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    if shape.iter().copied().collect::<Vec<_>>() != [1, 1, SIDE as i64, SIDE as i64] {
        return Err("Unexpected BiRefNet output shape".into());
    }
    Ok(values.to_vec())
}

#[cfg(any(target_arch = "wasm32", not(feature = "onnx")))]
fn infer(_: Vec<f32>) -> Result<Vec<f32>, String> {
    Err("AI background removal requires the desktop app with ONNX support".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires BiRefNet model; release benchmark for a 24 MP matte"]
    fn local_ai_24mp_benchmark() {
        let rgb: Vec<[f32; 3]> = (0..160)
            .flat_map(|y| {
                (0..200).map(move |x| {
                    let dx = x as f32 - 100.0;
                    let dy = y as f32 - 80.0;
                    if dx * dx + dy * dy < 45.0 * 45.0 { [0.85, 0.2, 0.15] } else { [0.25, 0.45, 0.7] }
                })
            })
            .collect();
        let start = std::time::Instant::now();
        let matte = foreground(&rgb, 200, 160, 6000, 4000).unwrap();
        eprintln!("BiRefNet inference + 24 MP matte resize: {:.3}s", start.elapsed().as_secs_f64());
        assert_eq!(matte.len(), 24_000_000);
        assert!(matte.iter().any(|v| *v < 25));
        assert!(matte.iter().any(|v| *v > 230));
    }
    #[test]
    fn invalid_images_fail_without_inference() {
        for (w, h) in [(0, 0), (usize::MAX, 2), (3, 4)] {
            assert!(foreground(&[], w, h, 2, 2).is_err());
        }
        assert!(foreground(&[[f32::NAN; 3]], 1, 1, 2, 2).is_err());
        assert!(foreground(&[[0.5; 3]], 1, 1, u32::MAX, u32::MAX).is_err());
    }
    #[test]
    fn matte_is_continuous_and_rejects_bad_model_output() {
        assert!(normalize(&[]).is_err());
        assert!(normalize(&vec![f32::NAN; SIDE * SIDE]).is_err());
        assert!(normalize(&vec![0.0; SIDE * SIDE]).is_err());
        let logits: Vec<f32> = (0..SIDE * SIDE).map(|i| (i % 3) as f32 - 1.0).collect();
        let mask = normalize(&logits).unwrap();
        assert_eq!(mask[0], 0);
        assert!((126..=128).contains(&mask[1]));
        assert_eq!(mask[2], 255);
    }
}
