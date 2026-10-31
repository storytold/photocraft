//! Running a G'MIC pipeline on a layer surface.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::cimg::Image;
use crate::runner::{Gmic, MAX_SAMPLES, RunOptions};
use crate::{Error, Result};

/// G'MIC filters expect RGB(A) or grayscale(+alpha) pixels; other colour models are an error.
pub fn check_format(f: PixelFormat) -> Result<()> {
    match f.mode {
        ColorMode::Rgb | ColorMode::Grayscale => Ok(()),
        other => Err(Error::Image(format!("G'MIC filters work on RGB and Grayscale images; convert this {other:?} image with Image › Mode first"))),
    }
}

/// For each surface channel, the result channel it takes (`None`: keep the original).
fn channel_map(n: usize, m: usize, alpha: bool) -> Result<Vec<Option<usize>>> {
    let colour = if alpha { n - 1 } else { n };
    let map = if m == n {
        (0..n).map(Some).collect()
    } else if alpha && m == colour {
        // The filter dropped alpha: keep the layer's own.
        (0..n).map(|c| (c < colour).then_some(c)).collect()
    } else if !alpha && m == n + 1 {
        // The filter added alpha: a layer without transparency keeps none.
        (0..n).map(Some).collect()
    } else if colour >= 3 && (m == 1 || m == 2) {
        // A grey result on a colour layer: grey in every colour channel.
        (0..n).map(|c| if c < colour { Some(0) } else if m == 2 { Some(1) } else { None }).collect()
    } else {
        return Err(Error::Image(format!(
            "the filter returned {m}-channel pixels for a {n}-channel layer{}",
            if colour < 3 && m >= 3 { "; convert the document to RGB Color first" } else { "" }
        )));
    };
    Ok(map)
}

/// Runs `pipeline` on `surface` and returns the new surface. G'MIC sees the whole canvas, in
/// 0–255 values (G'MIC's convention, whatever the bit depth: 16-bit and 32-bit data keep their
/// precision as float32); `selection` (coverage in channel 0) limits and feathers the result,
/// as for built-in filters. The filter must keep the image size.
pub fn apply(gmic: &Gmic, surface: &Surface, canvas: Rect, selection: Option<&Surface>, pipeline: &str, opts: &RunOptions<'_>) -> Result<Surface> {
    let fmt = surface.format();
    check_format(fmt)?;
    let out_area = match selection {
        Some(s) => s.content_bounds().intersect(&canvas),
        None => canvas,
    };
    if canvas.is_empty() || out_area.is_empty() {
        return Ok(surface.clone());
    }
    let n = fmt.channels();
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let count = w.checked_mul(h).and_then(|p| p.checked_mul(n)).filter(|c| *c <= MAX_SAMPLES / 2);
    let count = count.ok_or_else(|| Error::Image("the image is too large to send to G'MIC".into()))?;
    let src = surface.read_region(canvas);
    let mut planar = vec![0.0f32; count];
    for (i, px) in src.chunks_exact(n).enumerate() {
        for (c, v) in px.iter().enumerate() {
            if let Some(d) = planar.get_mut(c * w * h + i) {
                *d = v * 255.0;
            }
        }
    }
    let input = Image { width: canvas.width(), height: canvas.height(), channels: n as u32, data: planar };
    drop(src);
    let results = gmic.run(std::slice::from_ref(&input), pipeline, opts)?;
    let result = results.into_iter().find(|i| !i.data.is_empty()).ok_or_else(|| Error::Failed("the command left no image".into()))?;
    if (result.width, result.height) != (input.width, input.height) {
        return Err(Error::Image(format!(
            "the filter changed the image size ({}×{} → {}×{}); filters that resize the image are not supported yet",
            input.width, input.height, result.width, result.height
        )));
    }
    let map = channel_map(n, result.channels as usize, fmt.alpha)?;
    let int_depth = fmt.sample != SampleType::F32;
    let (ow, oh) = (out_area.width() as usize, out_area.height() as usize);
    let orig = surface.read_region(out_area);
    let sel: Option<Vec<f32>> = selection.map(|s| {
        let sn = s.channels().max(1);
        s.read_region(out_area).chunks_exact(sn).map(|p| p.first().copied().unwrap_or(0.0).clamp(0.0, 1.0)).collect()
    });
    let mut out = vec![0.0f32; ow * oh * n];
    let (dx, dy) = ((out_area.x0 - canvas.x0) as u32, (out_area.y0 - canvas.y0) as u32);
    for row in 0..oh {
        for col in 0..ow {
            let i = row * ow + col;
            let k = sel.as_ref().map_or(1.0, |s| s.get(i).copied().unwrap_or(0.0));
            for (ch, src) in map.iter().enumerate() {
                let o = orig.get(i * n + ch).copied().unwrap_or(0.0);
                let mut v = src.and_then(|c| result.get(dx + col as u32, dy + row as u32, c as u32)).map_or(o, |v| v / 255.0);
                if !v.is_finite() {
                    v = o;
                }
                if int_depth || (fmt.alpha && ch == n - 1) {
                    v = v.clamp(0.0, 1.0);
                }
                if let Some(d) = out.get_mut(i * n + ch) {
                    *d = o + (v - o) * k;
                }
            }
        }
    }
    let mut s = surface.clone();
    s.write_region(out_area, &out);
    s.prune();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_maps() {
        assert_eq!(channel_map(4, 4, true).unwrap(), [Some(0), Some(1), Some(2), Some(3)]);
        assert_eq!(channel_map(4, 3, true).unwrap(), [Some(0), Some(1), Some(2), None]);
        assert_eq!(channel_map(3, 4, false).unwrap(), [Some(0), Some(1), Some(2)]);
        assert_eq!(channel_map(4, 1, true).unwrap(), [Some(0), Some(0), Some(0), None]);
        assert_eq!(channel_map(4, 2, true).unwrap(), [Some(0), Some(0), Some(0), Some(1)]);
        assert!(channel_map(1, 3, false).unwrap_err().to_string().contains("RGB"));
        assert!(channel_map(4, 7, true).is_err());
    }

    #[test]
    fn formats() {
        assert!(check_format(PixelFormat::RGBA16).is_ok());
        assert!(check_format(PixelFormat::GRAY8).is_ok());
        assert!(check_format(PixelFormat::CMYKA8).is_err());
    }
}
