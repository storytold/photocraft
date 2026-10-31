//! Measure where a camera's own full-size JPEG sits in its raw sensor data: the image area that
//! `photocraft-raw`'s `cameras.rs` records for bodies that don't write one.
//!
//! The sensor data and the embedded JPEG are both reduced to half-size luminance (each 2 x 2 Bayer
//! block, gamma-encoded; the JPEG averaged the same way), and the JPEG is slid over the sensor
//! data at every even offset. The offset with the highest normalized cross-correlation is the
//! image area's origin; its size is the JPEG's.
//!
//! ```sh
//! cargo run --release -p photocraft-io --example raw_image_area -- FILE...
//! ```

use photocraft_codecs::{self as codecs, ChannelLayout, Format, SampleType};

/// Half-size luminance: `(width, height, samples)`.
type Plane = (usize, usize, Vec<f32>);

fn sensor_plane(s: &photocraft_raw::Sensor) -> Plane {
    let (w, h) = (s.width / 2, s.height / 2);
    let white = s.white.iter().copied().fold(1.0f32, f32::max);
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let at = |dx: usize, dy: usize| f32::from(s.data[(2 * y + dy) * s.width + 2 * x + dx]);
            let v = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / (4.0 * white);
            out[y * w + x] = v.clamp(0.0, 1.0).powf(1.0 / 2.2);
        }
    }
    (w, h, out)
}

fn jpeg_plane(jpeg: &[u8]) -> Result<Plane, String> {
    let img = codecs::decode_as(Format::Jpeg, jpeg).map_err(|e| e.to_string())?;
    let rgb = img.convert(ChannelLayout::Rgb, SampleType::U8);
    let (fw, fh) = (rgb.width() as usize, rgb.height() as usize);
    let d = rgb.data();
    let (w, h) = (fw / 2, fh / 2);
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = ((2 * y + dy) * fw + 2 * x + dx) * 3;
                sum += f32::from(d[p]) + f32::from(d[p + 1]) + f32::from(d[p + 2]);
            }
            out[y * w + x] = sum / (12.0 * 255.0);
        }
    }
    Ok((w, h, out))
}

/// Normalized cross-correlation of `j` placed at (`ox`, `oy`) in `s`, over every `step`-th sample.
fn ncc(s: &Plane, j: &Plane, ox: usize, oy: usize, step: usize) -> f64 {
    let (mut n, mut sa, mut sb, mut saa, mut sbb, mut sab) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for y in (0..j.1).step_by(step) {
        for x in (0..j.0).step_by(step) {
            let a = f64::from(s.2[(oy + y) * s.0 + ox + x]);
            let b = f64::from(j.2[y * j.0 + x]);
            n += 1.0;
            sa += a;
            sb += b;
            saa += a * a;
            sbb += b * b;
            sab += a * b;
        }
    }
    let cov = sab - sa * sb / n;
    let var = (saa - sa * sa / n) * (sbb - sb * sb / n);
    if var > 0.0 { cov / var.sqrt() } else { 0.0 }
}

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}: not read: {e}");
                continue;
            }
        };
        let sensor = match photocraft_raw::decode(&bytes, &Default::default()) {
            Ok(s) => s,
            Err(e) => {
                println!("{path}: not decoded: {e}");
                continue;
            }
        };
        let crop = sensor.crop;
        let Some(preview) = photocraft_raw::embedded_preview(&bytes) else {
            println!("{path}: no embedded JPEG; crop {crop:?}");
            continue;
        };
        let j = match jpeg_plane(preview.jpeg) {
            Ok(j) => j,
            Err(e) => {
                println!("{path}: embedded JPEG not decoded: {e}");
                continue;
            }
        };
        let s = sensor_plane(&sensor);
        if (crop.width, crop.height) == (2 * j.0, 2 * j.1) || (crop.width, crop.height) == (preview.width as usize, preview.height as usize) {
            println!("{path}: already cropped to the JPEG's {}x{} at ({}, {})", crop.width, crop.height, crop.x, crop.y);
            continue;
        }
        // A JPEG much smaller than the data is a reduced preview, not the image area.
        if j.0 > s.0 || j.1 > s.1 || 4 * j.0 < 3 * s.0 || 4 * j.1 < 3 * s.1 {
            println!("{path}: JPEG {}x{} doesn't measure {}x{} sensor data (crop {crop:?})", preview.width, preview.height, sensor.width, sensor.height);
            continue;
        }
        let mut scores: Vec<(f64, usize, usize)> = Vec::new();
        for oy in 0..=s.1 - j.1 {
            for ox in 0..=s.0 - j.0 {
                scores.push((ncc(&s, &j, ox, oy, 4), ox, oy));
            }
        }
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, ox, oy) = scores[0];
        let exact = ncc(&s, &j, ox, oy, 1);
        let runner = scores.iter().find(|c| c.1.abs_diff(ox) > 1 || c.2.abs_diff(oy) > 1).map_or(0.0, |c| c.0);
        println!(
            "{path}: {} {} sensor {}x{} crop {crop:?}, JPEG {}x{} at ({}, {}): NCC {exact:.4} (sampled {best:.4}, best elsewhere {runner:.4})",
            sensor.make.as_deref().unwrap_or("?").trim(),
            sensor.model.as_deref().unwrap_or("?").trim(),
            sensor.width,
            sensor.height,
            2 * j.0,
            2 * j.1,
            2 * ox,
            2 * oy
        );
    }
}
