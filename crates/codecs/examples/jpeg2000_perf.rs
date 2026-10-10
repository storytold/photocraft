//! Native 24 MP codec benchmark. Run with --release; --lossy adds quality-80 RGB8.

#![forbid(unsafe_code)]

use std::error::Error;
use std::io;
use std::time::Instant;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, decode, encode};

const WIDTH: u32 = 6000;
const HEIGHT: u32 = 4000;
type BenchResult<T> = Result<T, Box<dyn Error>>;

fn generated_image(sample: SampleType) -> BenchResult<Image> {
    let max: u32 = match sample {
        SampleType::U8 => 255,
        SampleType::U16 => 65535,
        _ => return Err(io::Error::other("benchmark requires integer samples").into()),
    };
    let length = (WIDTH as usize)
        .checked_mul(HEIGHT as usize)
        .and_then(|n| n.checked_mul(3))
        .and_then(|n| n.checked_mul(sample.bytes()))
        .ok_or_else(|| io::Error::other("benchmark image length overflows"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(length)?;
    let amplitude = (max / 80).max(1);
    let mut random = 0x1047_a53c_u32;
    for y in 0..HEIGHT {
        let gy = y * max / (HEIGHT - 1);
        for x in 0..WIDTH {
            let gx = x * max / (WIDTH - 1);
            for base in [gx, gy, (gx + gy) / 2] {
                // A deterministic small noise term exercises more than a flat ramp, without
                // allocating a second full image or using a random-number dependency.
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((random >> 8) % (2 * amplitude + 1)) as i32 - amplitude as i32;
                let value = (base as i32 + noise).clamp(0, max as i32) as u16;
                if sample == SampleType::U16 {
                    data.extend_from_slice(&value.to_ne_bytes());
                } else {
                    data.push(value as u8);
                }
            }
        }
    }
    Ok(Image::from_raw(WIDTH, HEIGHT, ChannelLayout::Rgb, sample, data)?)
}

fn check_storage(original: &Image, decoded: &Image) -> BenchResult<()> {
    if decoded.dimensions() != original.dimensions() || decoded.layout() != original.layout() || decoded.sample_type() != original.sample_type() {
        return Err(io::Error::other("JPEG 2000 roundtrip changed dimensions, layout or sample type").into());
    }
    Ok(())
}

fn lossless(image: &Image) -> BenchResult<()> {
    let start = Instant::now();
    let bytes = encode(image, Format::Jpeg2000, &EncodeOptions::default())?;
    let encode_time = start.elapsed();
    let start = Instant::now();
    let decoded = decode(&bytes)?;
    let decode_time = start.elapsed();
    check_storage(image, &decoded)?;
    if decoded.data() != image.data() {
        return Err(io::Error::other("lossless JPEG 2000 roundtrip changed native sample bytes").into());
    }
    println!(
        "{:2}-bit lossless JP2: {} bytes; encode {:.3}s; decode {:.3}s; native samples exact",
        image.sample_type().bytes() * 8,
        bytes.len(),
        encode_time.as_secs_f64(),
        decode_time.as_secs_f64()
    );
    Ok(())
}

fn lossy_eighty(image: &Image) -> BenchResult<()> {
    let options = EncodeOptions { jpeg2000_quality: Some(80), ..Default::default() };
    let start = Instant::now();
    let bytes = encode(image, Format::Jpeg2000, &options)?;
    let encode_time = start.elapsed();
    let start = Instant::now();
    let decoded = decode(&bytes)?;
    let decode_time = start.elapsed();
    check_storage(image, &decoded)?;
    let source = image.as_u8().ok_or_else(|| io::Error::other("quality benchmark requires RGB8"))?;
    let result = decoded.as_u8().ok_or_else(|| io::Error::other("quality benchmark decoded a non-U8 image"))?;
    let error: u64 = source.iter().zip(result).map(|(&a, &b)| u64::from((i32::from(a) - i32::from(b)).unsigned_abs()).pow(2)).sum();
    let mse = error as f64 / source.len() as f64;
    let psnr = if error == 0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 / mse).log10() };
    if psnr < 35.0 {
        return Err(io::Error::other(format!("quality-80 PSNR {psnr:.3} dB is below 35 dB")).into());
    }
    println!(
        " 8-bit quality-80 JP2: {} bytes; encode {:.3}s; decode {:.3}s; PSNR {:.3} dB",
        bytes.len(),
        encode_time.as_secs_f64(),
        decode_time.as_secs_f64(),
        psnr
    );
    Ok(())
}

fn main() -> BenchResult<()> {
    let mut include_lossy = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--lossy" => include_lossy = true,
            "--help" | "-h" => {
                println!("cargo run --release -p photocraft-codecs --example jpeg2000_perf -- [--lossy]");
                return Ok(());
            }
            _ => return Err(io::Error::other(format!("unknown argument {arg}; use --help")).into()),
        }
    }
    if cfg!(debug_assertions) {
        return Err(io::Error::other("run the JPEG 2000 performance example with --release").into());
    }
    println!("24 MP RGB ({WIDTH}x{HEIGHT}); generation and validation are outside codec timings");
    for sample in [SampleType::U8, SampleType::U16] {
        let image = generated_image(sample)?;
        lossless(&image)?;
        if include_lossy && sample == SampleType::U8 {
            lossy_eighty(&image)?;
        }
    }
    Ok(())
}
