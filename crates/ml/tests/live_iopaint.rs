//! Talks to a real IOPaint helper. Ignored by default; run with
//! `cargo test -p photocraft-ml --test live_iopaint -- --ignored` while
//! `iopaint start --model=lama --device=cpu --port=8080` is up.

#![cfg(all(feature = "http", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use photocraft_ml::{DEFAULT_ENDPOINT, GenerateOptions, GenerativeProvider, IoPaintProvider, Rect, RgbaImage, UreqTransport, expand_mask};

#[test]
#[ignore = "needs a running IOPaint helper"]
fn fills_a_right_hand_strip() -> Result<(), photocraft_ml::Error> {
    let endpoint = std::env::var("PHOTOCRAFT_ML_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_owned());
    let p = IoPaintProvider::new(&endpoint, Box::new(UreqTransport::new(Duration::from_secs(300))));
    p.ping()?;

    // 256x128 canvas: left 192 px is a vertical gradient from blue to white, right 64 px is grey
    // placeholder that the model must replace.
    let (w, h) = (256u32, 128u32);
    let mut img = RgbaImage::filled(w, h, [128, 128, 128, 255]).ok_or(photocraft_ml::Error::InvalidBuffer)?;
    for y in 0..h {
        let t = y as f32 / (h - 1) as f32;
        let px = [(40.0 + 215.0 * t) as u8, (90.0 + 165.0 * t) as u8, 220, 255];
        for x in 0..192 {
            let i = ((y * w + x) * 4) as usize;
            img.data[i..i + 4].copy_from_slice(&px);
        }
    }
    let mask = expand_mask(w, h, Rect { x: 0, y: 0, w: 192, h }, 16).ok_or(photocraft_ml::Error::InvalidBuffer)?;

    let t0 = Instant::now();
    let out = p.generate(&img, &mask, &GenerateOptions::default(), &|| false)?;
    eprintln!("live IOPaint round trip: {:.1}s", t0.elapsed().as_secs_f32());

    assert_eq!((out.width, out.height), (w, h));
    // The filled strip should now look like the gradient, not grey: sample the far right column.
    let i = ((64 * w + 250) * 4) as usize;
    let [r, g, b] = [out.data[i], out.data[i + 1], out.data[i + 2]];
    assert!(b > r && b > g, "right strip should be bluish like its neighbours, got {r},{g},{b}");
    assert!(!(r == 128 && g == 128 && b == 128), "placeholder grey must be gone");
    Ok(())
}
