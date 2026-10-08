//! Read-only independent EXR sample oracle: `exr_region_probe file.exr x y [x y ...]`.
//! Uses the dependency's filtered block reader, independently of our band conversion.
use exr::{block::reader::ChunksReader, meta::attribute::SampleType};
use std::io::BufReader;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: exr_region_probe file.exr x y [x y ...]")?;
    let coordinates = args.get(1..).ok_or("missing coordinates")?;
    if coordinates.is_empty() || !coordinates.len().is_multiple_of(2) || coordinates.len() > 512 {
        return Err("provide 1 to 256 coordinate pairs".into());
    }
    let points: Vec<(usize, usize)> = coordinates
        .chunks_exact(2)
        .map(|p| Ok((p.first().ok_or("x")?.parse()?, p.get(1).ok_or("y")?.parse()?)))
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    // Bound metadata, dimensions and block sizes before using the oracle reader.
    let info = photocraft_codecs::exr_stream::Decoder::new(std::fs::File::open(path)?, &|| false)?.info();
    if points.iter().any(|&(x, y)| x >= info.width as usize || y >= info.height as usize) {
        return Err("coordinate outside the data window".into());
    }
    let reader = exr::block::read(BufReader::new(std::fs::File::open(path)?), false)?;
    let mut blocks = reader
        .filter_chunks(false, |_, _, block| points.iter().any(|&(_, y)| y >= block.pixel_position.1 && y < block.pixel_position.1 + block.pixel_size.1))?
        .sequential_decompressor(false);
    let channels = blocks.meta_data().headers.first().ok_or("missing header")?.channels.clone();
    let mut samples = vec![[0_f32, 0.0, 0.0, 1.0]; points.len()];
    let mut seen = vec![[false; 3]; points.len()];
    while let Some(block) = blocks.decompress_next_block() {
        let block = block?;
        for line in block.lines(&channels) {
            let channel = channels.list.get(line.location.channel).ok_or("channel")?;
            let name = channel.name.to_string();
            let component = match name.rsplit('.').next() {
                Some("R") => 0,
                Some("G") => 1,
                Some("B") => 2,
                _ => continue,
            };
            for (index, &(x, y)) in points.iter().enumerate().filter(|(_, p)| p.1 == line.location.position.1) {
                let value = match channel.sample_type {
                    SampleType::F16 => line.read_samples::<photocraft_codecs::f16>().nth(x).ok_or("sample")??.to_f32(),
                    SampleType::F32 => line.read_samples::<f32>().nth(x).ok_or("sample")??,
                    SampleType::U32 => line.read_samples::<u32>().nth(x).ok_or("sample")?? as f32,
                };
                if !value.is_finite() {
                    return Err("non-finite sample cannot be represented in JSON".into());
                }
                *samples.get_mut(index).and_then(|s| s.get_mut(component)).ok_or("sample index")? = value;
                *seen.get_mut(index).and_then(|s| s.get_mut(component)).ok_or("sample index")? = true;
                let _ = y;
            }
        }
    }
    if seen.iter().flatten().any(|v| !v) {
        return Err("missing RGB samples".into());
    }
    println!("{{\"width\":{},\"height\":{},\"samples\":[", info.width, info.height);
    for (index, (&(x, y), sample)) in points.iter().zip(samples).enumerate() {
        println!("{}{{\"x\":{x},\"y\":{y},\"rgba\":{:?}}}", if index == 0 { "" } else { "," }, sample);
    }
    println!("]}}");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
