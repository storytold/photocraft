//! Compare `photocraft-raw`'s develop with the shared engine's (`craft-develop`,
//! `photocraft_io::raw_develop`) on real raw files: times the open-time steps and writes sRGB PNGs
//! of both default develops, and the two side by side.
//!
//! ```sh
//! cargo run --release -p photocraft-io --example raw_compare -- [--mode both|classic|develop] [--out DIR] FILE...
//! ```
//!
//! Times (ms): `classic` the default open with `photocraft-raw`; for the shared engine `proxy_load`
//! (decode and develop a ≤900 px copy, as the dialog does), `proxy_render` (the first render of the
//! proxy at the defaults), `proxy_rerender` (an Exposure change, cached stages reused) and
//! `full_open` (decode, develop and render the whole frame into the document, as Open does).

use std::sync::Arc;
use std::time::Instant;

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, EncodeOptions, Format, Image};
use photocraft_io::raw_develop::{DevelopSource, import_raw_developed};

const PROXY_SIDE: usize = 900;
const OUT_EDGE: usize = 1600;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The document's first pixel layer as interleaved RGB floats.
fn rgb(doc: &photocraft_doc::Document) -> Option<(Vec<f32>, usize, usize)> {
    let s = doc.layers.iter().find_map(|l| l.surface())?;
    let b = doc.bounds();
    let (w, h) = ((b.x1 - b.x0) as usize, (b.y1 - b.y0) as usize);
    let mut px = vec![[0.0f32; 4]; w * h];
    s.read_rgba_into(b, &mut px);
    Some((px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(), w, h))
}

/// Box-downsamples interleaved RGB to at most `edge` on the long side.
fn shrink(px: &[f32], w: usize, h: usize, edge: usize) -> (Vec<f32>, usize, usize) {
    let f = w.max(h).div_ceil(edge).max(1);
    let (ow, oh) = (w / f, h / f);
    let mut out = vec![0.0f32; ow * oh * 3];
    for y in 0..oh * f {
        for x in 0..ow * f {
            let (i, o) = ((y * w + x) * 3, ((y / f) * ow + x / f) * 3);
            for k in 0..3 {
                out[o + k] += px[i + k];
            }
        }
    }
    let n = (f * f) as f32;
    out.iter_mut().for_each(|v| *v /= n);
    (out, ow, oh)
}

/// ProPhoto (the document's space) RGB as 8-bit sRGB.
fn to_srgb(px: &[f32]) -> Vec<u8> {
    let t = Transform::new(Builtin::ProPhotoCompat.profile(), Builtin::Srgb.profile(), Intent::Perceptual, true).expect("built-in profiles");
    let mut out = vec![0.0f32; px.len()];
    t.convert_f32(px, 3, &mut out, 3, false);
    out.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

fn png(path: &std::path::Path, w: usize, h: usize, rgb: Vec<u8>) {
    let img = Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, rgb).expect("image");
    let bytes = codecs::encode(&img, Format::Png, &EncodeOptions::default()).expect("png");
    std::fs::write(path, bytes).expect("write");
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let (mut mode, mut out, mut files) = ("both".to_string(), None, Vec::new());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mode" => mode = args.next().expect("--mode value"),
            "--out" => out = Some(std::path::PathBuf::from(args.next().expect("--out DIR"))),
            _ => files.push(a),
        }
    }
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).expect("output folder");
    }
    for f in files {
        let bytes = std::fs::read(&f).expect("read");
        let name = std::path::Path::new(&f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = std::path::Path::new(&name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut line = name.clone();
        let mut old = None;
        let mut new = None;
        if mode != "develop" {
            let t = Instant::now();
            match photocraft_io::raw::import_raw(&name, &bytes) {
                Ok(r) => {
                    line += &format!(" classic={:.0}", ms(t));
                    old = rgb(&r.document);
                }
                Err(e) => line += &format!(" classic=ERR({e})"),
            }
        }
        if mode != "classic" {
            let bytes = Arc::new(bytes.clone());
            let t = Instant::now();
            match DevelopSource::new(bytes.clone(), PROXY_SIDE, 0) {
                Ok(src) => {
                    line += &format!(" proxy_load={:.0}", ms(t));
                    let (pw, ph) = src.proxy_size();
                    let t = Instant::now();
                    let ok = src.develop_proxy(&CameraRaw::default(), pw, ph).is_ok();
                    line += &format!(" proxy_render={:.0}{}", ms(t), if ok { "" } else { "(ERR)" });
                    let t = Instant::now();
                    let _ = src.develop_proxy(&CameraRaw { exposure: 0.3, ..Default::default() }, pw, ph);
                    line += &format!(" proxy_rerender={:.0} proxy={pw}x{ph}", ms(t));
                }
                Err(e) => line += &format!(" proxy=ERR({e})"),
            }
            let t = Instant::now();
            match import_raw_developed(&name, &bytes, &CameraRaw::default(), None) {
                Ok((r, _)) => {
                    let fell_back = photocraft_io::raw_develop::fell_back(&r.warnings);
                    line += &format!(
                        " full_open={:.0} size={}x{}{}",
                        ms(t),
                        r.document.size.width,
                        r.document.size.height,
                        if fell_back { " (FELL BACK)" } else { "" }
                    );
                    new = rgb(&r.document);
                }
                Err(e) => line += &format!(" full_open=ERR({e})"),
            }
        }
        println!("{line}");
        let Some(dir) = &out else { continue };
        let mut halves = Vec::new();
        for (tag, img) in [("classic", &old), ("craft-develop", &new)] {
            if let Some((px, w, h)) = img {
                let (s, sw, sh) = shrink(px, *w, *h, OUT_EDGE);
                let srgb = to_srgb(&s);
                png(&dir.join(format!("{stem}.{tag}.png")), sw, sh, srgb.clone());
                halves.push((srgb, sw, sh));
            }
        }
        if let [(a, aw, ah), (b, bw, bh)] = halves.as_slice() {
            let (w, h) = (aw + 8 + bw, (*ah).max(*bh));
            let mut side = vec![32u8; w * h * 3];
            for (img, iw, ih, x0) in [(a, *aw, *ah, 0), (b, *bw, *bh, aw + 8)] {
                for y in 0..ih {
                    let dst = (y * w + x0) * 3;
                    side[dst..dst + iw * 3].copy_from_slice(&img[y * iw * 3..(y + 1) * iw * 3]);
                }
            }
            png(&dir.join(format!("{stem}.side-by-side.png")), w, h, side);
        }
    }
}
