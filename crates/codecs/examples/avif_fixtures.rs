//! Generate synthetic AVIFs and their 8-bit PNG references for an independent decoder.
//! Run: `cargo run -p photocraft-codecs --features avif --example avif_fixtures -- OUTPUT_DIR`
//! Add `--24mp` after the directory for a 6000x4000 RGB8 q85 benchmark.
//! AVIF alpha is lossy; compare visible colours or composites, not hidden RGB at alpha zero.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(feature = "avif")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
    use std::path::PathBuf;
    use std::time::Instant;

    let directory = PathBuf::from(std::env::args_os().nth(1).ok_or("expected an output directory")?);
    std::fs::create_dir_all(&directory)?;
    let options = EncodeOptions { embed_icc: false, embed_metadata: false, ..Default::default() };
    if let Some(mode) = std::env::args_os().nth(2) {
        if mode != "--24mp" {
            return Err("expected OUTPUT_DIR [--24mp]".into());
        }
        // Fixed dimensions keep this probe bounded. Build RGB8 directly so the measurement
        // includes no large floating-point reference buffer or unrelated PNG encoding.
        let (width, height) = (6000u32, 4000u32);
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(72_000_000)?;
        let mut rng = 7u64;
        for y in 0..height {
            for x in 0..width {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let noise = ((rng >> 56) as i32 - 128) / 16;
                let red = (x * 255 / (width - 1)) as i32;
                let green = (y * 255 / (height - 1)) as i32;
                pixels.extend([
                    (red + noise).clamp(0, 255) as u8,
                    (green + noise).clamp(0, 255) as u8,
                    (200 - red / 3 - green / 4 + noise).clamp(0, 255) as u8,
                ]);
            }
        }
        let image = Image::from_u8(width, height, ChannelLayout::Rgb, pixels)?;
        let started = Instant::now();
        let bytes = encode(&image, Format::Avif, &EncodeOptions { jpeg_quality: 85, ..options })?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        std::fs::write(directory.join("rgb-24mp-q85.avif"), &bytes)?;
        println!("file\twidth\theight\tbytes\tencode_ms");
        println!("rgb-24mp-q85.avif\t{width}\t{height}\t{}\t{elapsed:.3}", bytes.len());
        return Ok(());
    }
    let cases = [
        ("rgb", 97u32, 61u32, ChannelLayout::Rgb, SampleType::U8),
        ("rgba", 97, 61, ChannelLayout::Rgba, SampleType::U8),
        ("rgba16", 97, 61, ChannelLayout::Rgba, SampleType::U16),
        ("rgb32f", 97, 61, ChannelLayout::Rgb, SampleType::F32),
        ("tiny", 1, 1, ChannelLayout::Rgba, SampleType::U8),
        ("narrow", 1, 17, ChannelLayout::Rgba, SampleType::U8),
    ];
    println!("file\twidth\theight\tbytes\tencode_ms");
    for (name, width, height, layout, sample) in cases {
        let mut values = Vec::new();
        let mut rng = 7u64;
        for y in 0..height {
            for x in 0..width {
                let fx = x as f32 / (width.max(2) - 1) as f32;
                let fy = y as f32 / (height.max(2) - 1) as f32;
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let noise = ((rng >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.08;
                values.extend([(fx + noise).clamp(0.0, 1.0), (fy + noise).clamp(0.0, 1.0), (0.8 - fx * 0.35 - fy * 0.2 + noise).clamp(0.0, 1.0)]);
                if layout.has_alpha() {
                    // Fully transparent and opaque regions plus intermediate alpha.
                    values.push(((fx + fy) * 1.5 - 0.5).clamp(0.0, 1.0));
                }
            }
        }
        let image = Image::from_normalized(width, height, layout, sample, &values)?;
        let expected = image.convert(layout, SampleType::U8);
        std::fs::write(directory.join(format!("{name}-reference.png")), encode(&expected, Format::Png, &options)?)?;
        for quality in [20u8, 85, 100] {
            let started = Instant::now();
            let bytes = encode(&image, Format::Avif, &EncodeOptions { jpeg_quality: quality, ..options.clone() })?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let file = format!("{name}-q{quality}.avif");
            std::fs::write(directory.join(&file), &bytes)?;
            println!("{file}\t{width}\t{height}\t{}\t{elapsed:.3}", bytes.len());
        }
    }
    Ok(())
}

#[cfg(not(feature = "avif"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("this example requires --features avif".into())
}
