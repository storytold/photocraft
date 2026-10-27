//! Banded native Gray/RGB transport, retaining the surface's precision, alpha and selection.
use super::{client::Launch, wire::Request};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use std::io;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
pub fn apply(
    launch: &Launch,
    request: Request,
    surface: &Surface,
    area: Rect,
    selection: Option<&Surface>,
    cancelled: impl Fn() -> bool,
) -> io::Result<Surface> {
    let fmt = surface.format();
    if !matches!(fmt.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(fmt.sample, SampleType::U8 | SampleType::U16) {
        return Err(invalid("8BF bridge supports 8-bit and 16-bit RGB or Grayscale pixel layers only"));
    }
    let colors = fmt.mode.color_channels();
    let sample_bytes = fmt.sample.bytes();
    let color_bytes = colors * sample_bytes;
    let pixel_bytes = fmt.bytes_per_pixel();
    if usize::from(request.channels) != colors || u32::from(request.depth) != fmt.sample.bits() {
        return Err(invalid("8BF request format must match the pixel layer"));
    }
    let length = request.pixel_len()?;
    if area.is_empty() || area.width() != request.width || area.height() != request.height {
        return Err(invalid("8BF image dimensions do not match the requested area"));
    }
    // Only the compact color transport spans the full image. Native-endian surface bytes
    // stay in 64-row bands; alpha is never sent to the filter or re-quantized on return.
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|e| io::Error::other(format!("8BF input allocation: {e}")))?;
    for row in (0..area.height()).step_by(64) {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "8BF filter cancelled"));
        }
        let band = band(area, row)?;
        let original = surface.to_interleaved(band);
        for pixel in original.chunks_exact(pixel_bytes) {
            let color = pixel.get(..color_bytes).ok_or_else(|| invalid("invalid 8BF source pixel"))?;
            if sample_bytes == 1 {
                bytes.extend_from_slice(color);
            } else {
                for value in color.as_chunks::<2>().0 {
                    bytes.extend_from_slice(&u16::from_ne_bytes(*value).to_le_bytes());
                }
            }
        }
    }
    let result = super::client::exchange(launch, request, bytes, &cancelled)?;
    let mut output = surface.clone();
    for row in (0..area.height()).step_by(64) {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "8BF filter cancelled"));
        }
        let band = band(area, row)?;
        let mut original = surface.to_interleaved(band);
        let coverage = selection.map(|s| (s.read_region(band), s.channels()));
        let first =
            (row as usize).checked_mul(area.width() as usize).and_then(|n| n.checked_mul(color_bytes)).ok_or_else(|| invalid("8BF band offset overflow"))?;
        let count = (original.len() / pixel_bytes).checked_mul(color_bytes).ok_or_else(|| invalid("8BF band length overflow"))?;
        let end = first.checked_add(count).ok_or_else(|| invalid("8BF band offset overflow"))?;
        let filtered = result.get(first..end).ok_or_else(|| invalid("8BF pixel buffer is truncated"))?;
        for (i, (pixel, filtered)) in original.chunks_exact_mut(pixel_bytes).zip(filtered.chunks_exact(color_bytes)).enumerate() {
            let k = coverage
                .as_ref()
                .map_or(1.0, |(values, n)| i.checked_mul(*n).and_then(|at| values.get(at)).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0));
            if k == 0.0 {
                continue;
            }
            let color = pixel.get_mut(..color_bytes).ok_or_else(|| invalid("invalid 8BF destination pixel"))?;
            for (stored, filtered) in color.chunks_exact_mut(sample_bytes).zip(filtered.chunks_exact(sample_bytes)) {
                if sample_bytes == 1 {
                    let value = filtered.first().copied().ok_or_else(|| invalid("8BF sample is truncated"))?;
                    if k == 1.0 {
                        stored.copy_from_slice(filtered);
                    } else {
                        let original = photocraft_color::read_sample(stored, fmt.sample, 0);
                        photocraft_color::write_sample(stored, fmt.sample, 0, original + (f32::from(value) / 255.0 - original) * k);
                    }
                } else {
                    let value = u16::from_le_bytes(filtered.try_into().map_err(|_| invalid("8BF sample is truncated"))?);
                    if k == 1.0 {
                        stored.copy_from_slice(&value.to_ne_bytes());
                    } else {
                        let original = photocraft_color::read_sample(stored, fmt.sample, 0);
                        photocraft_color::write_sample(stored, fmt.sample, 0, original + (f32::from(value) / 65535.0 - original) * k);
                    }
                }
            }
        }
        output.write_interleaved(band, &original);
    }
    Ok(output)
}
fn band(area: Rect, row: u32) -> io::Result<Rect> {
    Ok(Rect::from_xywh(area.x0, area.y0.checked_add(row as i32).ok_or_else(|| invalid("8BF row origin overflow"))?, area.width(), 64.min(area.height() - row)))
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    #[test]
    fn native_result_preserves_alpha_and_exact_outside_selection() {
        let area = Rect::new(0, 0, 3, 129);
        let mut source = Surface::new(PixelFormat::RGBA8);
        source.write_region(area, &[0.2, 0.4, 0.6, 0.5, 0.2, 0.4, 0.6, 0.5, 0.2, 0.4, 0.6, 0.5].repeat(129));
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.write_region(area, &[0.0, 0.5, 1.0].repeat(129));
        let script = r#"import sys,struct,json
r=sys.stdin.buffer; w=sys.stdout.buffer
r.read(8); n,p=struct.unpack('<IQ',r.read(12)); m=json.loads(r.read(n)); d=r.read(p)
h=json.dumps({'status':'ok','message':'','width':m['width'],'height':m['height'],'channels':3,'depth':8}).encode(); w.write(b'PC8BF001'+struct.pack('<IQ',len(h),p)+h+bytes(255-v for v in d))
"#;
        let launch = Launch {
            program: "python3".into(),
            args: vec!["-c".into(), script.into()],
            environment: Default::default(),
            timeout: std::time::Duration::from_secs(3),
        };
        let request = Request { path: "test.8bf".into(), entry: "PluginMain".into(), width: 3, height: 129, channels: 3, depth: 8, show_ui: false };
        let out = apply(&launch, request, &source, area, Some(&selection), || false).unwrap();
        for y in [0, 63, 64, 127, 128] {
            assert_eq!(out.pixel(0, y), source.pixel(0, y));
            assert_eq!(out.pixel(2, y)[3], source.pixel(2, y)[3]);
            assert!((out.pixel(2, y)[0] - 0.8).abs() < 0.01);
        }
        assert_eq!(out.pixel(1, 0)[3], source.pixel(1, 0)[3]);
        assert_eq!(out.pixel(2, 0)[3], source.pixel(2, 0)[3]);
        assert!((out.pixel(1, 0)[0] - 0.5).abs() < 0.01);
        assert!((out.pixel(2, 0)[0] - 0.8).abs() < 0.01);
        assert!(
            apply(
                &launch,
                Request { path: "test".into(), entry: "PluginMain".into(), width: 3, height: 1, channels: 3, depth: 8, show_ui: false },
                &Surface::new(PixelFormat::RGBA16),
                area,
                None,
                || false
            )
            .is_err()
        );
    }
    const GENERIC: &str = r#"import sys,struct,json
r=sys.stdin.buffer;w=sys.stdout.buffer
assert r.read(8)==b'PC8BF001'
n,p=struct.unpack('<IQ',r.read(12));m=json.loads(r.read(n));d=r.read(p)
if len(sys.argv)>1:
    if m['depth']==16:
        values=struct.unpack('<'+'H'*(len(d)//2),d);d=struct.pack('<'+'H'*len(values),*(65535-v for v in values))
    else:d=bytes(255-v for v in d)
m.update(status='ok',message='')
h=json.dumps(m).encode();w.write(b'PC8BF001'+struct.pack('<IQ',len(h),len(d))+h+d)
"#;
    fn generic(invert: bool) -> Launch {
        let mut args = vec!["-c".into(), GENERIC.into()];
        if invert {
            args.push("invert".into());
        }
        Launch { program: "python3".into(), args, environment: Default::default(), timeout: std::time::Duration::from_secs(3) }
    }
    #[test]
    fn supported_formats_identity_keeps_every_encoded_color_and_alpha_sample() {
        use photocraft_color::{ColorMode, SampleType};
        let area = Rect::from_xywh(-2, -3, 3, 129);
        for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
            for sample in [SampleType::U8, SampleType::U16] {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, sample, alpha);
                    let count = 3 * 129 * fmt.channels();
                    let bytes: Vec<u8> = if sample == SampleType::U16 {
                        (0..count).flat_map(|i| (((i * 257 + 1) % 65536) as u16).to_ne_bytes()).collect()
                    } else {
                        (0..count).map(|i| i as u8).collect()
                    };
                    let source = Surface::from_interleaved(fmt, area, &bytes);
                    let request = Request {
                        path: "test".into(),
                        entry: "PluginMain".into(),
                        width: 3,
                        height: 129,
                        channels: mode.color_channels() as u8,
                        depth: sample.bits() as u8,
                        show_ui: false,
                    };
                    let output = apply(&generic(false), request, &source, area, None, || false).unwrap();
                    assert_eq!(output.format(), fmt);
                    assert_eq!(output.to_interleaved(area), bytes, "identity changed {fmt:?}");
                }
            }
        }
    }
    #[test]
    fn gray_and_rgb16_selection_blends_color_without_touching_alpha_or_unselected_pixels() {
        use photocraft_color::{ColorMode, SampleType};
        let area = Rect::new(0, 0, 3, 129);
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.write_region(area, &[0.0, 128.0 / 255.0, 1.0].repeat(129));
        for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
            let fmt = PixelFormat::new(mode, SampleType::U16, true);
            let channels = fmt.channels();
            let color = mode.color_channels();
            let values: Vec<u16> =
                (0..3 * 129).flat_map(|i| (0..channels).map(move |c| if c == color { 65534 } else { 1 + (i % 3) as u16 * 8192 + c as u16 * 1003 })).collect();
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let source = Surface::from_interleaved(fmt, area, &bytes);
            let request = Request { path: "test".into(), entry: "PluginMain".into(), width: 3, height: 129, channels: color as u8, depth: 16, show_ui: false };
            let output = apply(&generic(true), request, &source, area, Some(&selection), || false).unwrap();
            let result = output.to_interleaved(area);
            let after: Vec<u16> = result.as_chunks::<2>().0.iter().map(|b| u16::from_ne_bytes(*b)).collect();
            for y in [0, 63, 64, 127, 128] {
                for x in 0..3 {
                    let first = (y * 3 + x) * channels;
                    assert_eq!(after[first + color], values[first + color]);
                    for c in 0..color {
                        let before = values[first + c];
                        let got = after[first + c];
                        if x == 0 {
                            assert_eq!(got, before);
                        } else if x == 2 {
                            assert_eq!(got, 65535 - before);
                        } else {
                            let expected = (f64::from(before) + (65535.0 - 2.0 * f64::from(before)) * (128.0 / 255.0)).round() as u16;
                            assert!(got.abs_diff(expected) <= 1);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn wire_format_must_match_surface_before_worker_launch() {
        let launch = Launch {
            program: "/nonexistent/photocraft-worker-must-not-run".into(),
            args: Vec::new(),
            environment: Default::default(),
            timeout: std::time::Duration::from_secs(3),
        };
        for (fmt, channels, depth) in [(PixelFormat::RGBA8, 1, 8), (PixelFormat::GRAYA8, 3, 8), (PixelFormat::RGBA16, 3, 8)] {
            let request = Request { path: "test".into(), entry: "PluginMain".into(), width: 3, height: 1, channels, depth, show_ui: false };
            assert_eq!(apply(&launch, request, &Surface::new(fmt), Rect::new(0, 0, 3, 1), None, || false).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }
}
