//! Runs the installed models on a photo, for eyeballing results:
//! `cargo run --release -p photocraft-ml --example try_models -- segment in.jpg matte.png`
//! `cargo run --release -p photocraft-ml --example try_models -- inpaint in.jpg mask.png out.png`
//! `cargo run --release -p photocraft-ml --example try_models -- remove in.jpg out.png` (matte, grow, inpaint)

use photocraft_ml::{Rgb, inpaint, segment_subject};
use std::time::Instant;

fn load(path: &str) -> Result<Rgb, Box<dyn std::error::Error>> {
    let img = image::open(path)?.to_rgb8();
    let (w, h) = img.dimensions();
    Ok(Rgb::new(w as usize, h as usize, img.as_raw().iter().map(|v| f32::from(*v) / 255.0).collect())?)
}

fn save_rgb(img: &Rgb, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let raw: Vec<u8> = img.px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    image::RgbImage::from_raw(img.w as u32, img.h as u32, raw).ok_or("size")?.save(path)?;
    Ok(())
}

fn save_gray(m: &[f32], w: usize, h: usize, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let raw: Vec<u8> = m.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    image::GrayImage::from_raw(w as u32, h as u32, raw).ok_or("size")?.save(path)?;
    Ok(())
}

fn grow(mask: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; mask.len()];
    // Separable max filter (square), good enough for a try-out.
    let mut tmp = vec![0.0f32; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(w);
            tmp[y * w + x] = (lo..hi).map(|k| mask[y * w + k]).fold(0.0, f32::max);
        }
    }
    for y in 0..h {
        let lo = y.saturating_sub(r);
        let hi = (y + r + 1).min(h);
        for x in 0..w {
            out[y * w + x] = (lo..hi).map(|k| tmp[k * w + x]).fold(0.0, f32::max);
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let t = Instant::now();
    match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["segment", input, output] => {
            let img = load(input)?;
            let m = segment_subject(&img)?;
            save_gray(&m, img.w, img.h, output)?;
        }
        ["inpaint", input, mask, output] => {
            let img = load(input)?;
            let mask = image::open(mask)?.to_luma8();
            let m: Vec<f32> = mask.as_raw().iter().map(|v| f32::from(*v) / 255.0).collect();
            save_rgb(&inpaint(&img, &m)?, output)?;
        }
        ["remove", input, output] => {
            let img = load(input)?;
            let m = segment_subject(&img)?;
            println!("segmented {:?}", t.elapsed());
            let bin: Vec<f32> = m.iter().map(|v| if *v >= 0.5 { 1.0 } else { 0.0 }).collect();
            let g = grow(&bin, img.w, img.h, (img.w.max(img.h) / 100).max(6));
            save_gray(&g, img.w, img.h, &format!("{output}.mask.png"))?;
            save_rgb(&inpaint(&img, &g)?, output)?;
        }
        _ => return Err("usage: try_models segment <in> <out> | inpaint <in> <mask> <out> | remove <in> <out>".into()),
    }
    println!("done {:?}", t.elapsed());
    Ok(())
}
