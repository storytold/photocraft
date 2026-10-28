//! Reproducible decode/develop medians (two warmups, seven measured runs).
//! cargo run --release -p photocraft-raw --example rawbench -- auto|builtin|flashraw FILE...
//! Input is read once; decode includes parsing, the bridge and PhotoCraft metadata.
use std::time::Instant;

use photocraft_raw::{DecodeBackend, DevelopOptions, Limits, decode_with_backend, develop_sensor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let backend = match args.next().as_deref() {
        Some("auto") => DecodeBackend::Auto,
        Some("builtin") => DecodeBackend::Builtin,
        Some("flashraw") => DecodeBackend::FlashRaw,
        _ => return Err("usage: rawbench auto|builtin|flashraw FILE...".into()),
    };
    println!("backend\tfile\tdecode_ms\tdevelop_ms\tcombined_ms\twidth\theight");
    for file in args {
        let bytes = std::fs::read(&file)?;
        let opts = DevelopOptions::default();
        let mut dec = Vec::new();
        let mut dev = Vec::new();
        let mut total = Vec::new();
        let mut size = (0, 0);
        for iteration in 0..9 {
            let start = Instant::now();
            let sensor = match decode_with_backend(&bytes, &Limits::default(), backend) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{backend:?}\t{file}\t{e}");
                    break;
                }
            };
            let decoded = Instant::now();
            let developed = develop_sensor(&sensor, &opts)?;
            let end = Instant::now();
            std::hint::black_box(&developed.rgb);
            size = (developed.width, developed.height);
            if iteration >= 2 {
                dec.push((decoded - start).as_secs_f64() * 1000.0);
                dev.push((end - decoded).as_secs_f64() * 1000.0);
                total.push((end - start).as_secs_f64() * 1000.0);
            }
        }
        dec.sort_by(f64::total_cmp);
        dev.sort_by(f64::total_cmp);
        total.sort_by(f64::total_cmp);
        if let (Some(d), Some(v), Some(t)) = (dec.get(3), dev.get(3), total.get(3)) {
            println!("{backend:?}\t{file}\t{d:.3}\t{v:.3}\t{t:.3}\t{}\t{}", size.0, size.1);
        }
    }
    Ok(())
}
