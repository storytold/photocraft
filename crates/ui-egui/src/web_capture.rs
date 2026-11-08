//! Screen capture for the web preview's screenshot button, plus the pass that removes the XYZ
//! axes a 3D viewer draws.
//!
//! `wry` exposes no capture API, so the page is grabbed from the screen: the WebView2 child
//! window is the topmost window over the preview's body, so a monitor capture cropped to that
//! rectangle is the page. The axis helper lines are pure red / green / blue and run across the
//! whole viewport, so long straight runs of one saturated colour are recognised and replaced by
//! their neighbours — the shot then carries no XYZ axes while the model and its labels stay.

/// One captured rectangle, in device pixels.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Capture a screen rectangle in device pixels (the preview window's client area).
#[cfg(windows)]
pub fn capture_screen(x: i32, y: i32, w: u32, h: u32) -> Result<Shot, String> {
    let m = xcap::Monitor::from_point(x + 1, y + 1).map_err(|e| format!("no screen under the preview: {e}"))?;
    let (mx, my) = (m.x().map_err(|e| e.to_string())?, m.y().map_err(|e| e.to_string())?);
    let img = m.capture_image().map_err(|e| format!("screen capture failed: {e}"))?;
    let (iw, ih) = (img.width(), img.height());
    // Crop to the window and clamp to the monitor: a window hanging over the edge loses only the
    // sliver that is off-screen.
    let cx = (x - mx).clamp(0, iw as i32) as u32;
    let cy = (y - my).clamp(0, ih as i32) as u32;
    let cw = w.min(iw - cx).max(1);
    let ch = h.min(ih - cy).max(1);
    let raw = img.as_raw();
    let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
    for row in 0..ch {
        let start = (((cy + row) * iw + cx) * 4) as usize;
        rgba.extend_from_slice(&raw[start..start + (cw * 4) as usize]);
    }
    Ok(Shot { width: cw, height: ch, rgba })
}

#[cfg(not(windows))]
pub fn capture_screen(_x: i32, _y: i32, _w: u32, _h: u32) -> Result<Shot, String> {
    Err("screen capture is only supported on Windows".into())
}

/// A screenshot of the preview as PNG bytes, handed to `edit.placeImage` in memory: nothing is
/// written to disk, so placing a shot leaves no file behind.
pub fn png_bytes(shot: &Shot) -> Result<Vec<u8>, String> {
    let img = photocraft_codecs::Image::from_u8(shot.width, shot.height, photocraft_codecs::ChannelLayout::Rgba, shot.rgba.clone())
        .map_err(|e| e.to_string())?;
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default())
        .map_err(|e| format!("could not encode the shot: {e}"))
}

/// A pixel of one axis colour: the high channel saturated, the other two near black. `which` is
/// (high, low, low).
fn axis_pixel(px: &[u8], which: (usize, usize, usize)) -> bool {
    if px[3] < 8 {
        return false;
    }
    let v = [px[0] as i32, px[1] as i32, px[2] as i32];
    let hi = v[which.0];
    let (l1, l2) = (v[which.1], v[which.2]);
    // Faded by anti-aliasing over a light page the line still reads as, say, (150, 150, 255),
    // so the test is on the dominance of one channel, not on a pure primary.
    hi >= 140 && hi - l1.min(l2) >= 80 && (l1 - l2).abs() <= 45
}

/// Remove the XYZ axis lines from `shot` in place.
///
/// A viewer draws its axes as pure red / green / blue helper lines that span the whole 3D view.
/// For each of the three colours the matching pixels are fitted to a principal line: when at
/// least 90 % of them sit within 2.5 px of it and the run covers 40 % of the image diagonal, that
/// colour is an axis, and those pixels are replaced by the nearest non-axis pixel beside them. A
/// plate number is also saturated blue but its strokes are thick and not collinear, so it never
/// passes both tests.
pub fn strip_axes(shot: &mut Shot) {
    let (w, h) = (shot.width as usize, shot.height as usize);
    if w < 16 || h < 16 {
        return;
    }
    let mut axis = vec![false; w * h];
    let mut found = false;
    for which in [(0usize, 1usize, 2usize), (1, 0, 2), (2, 0, 1)] {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                if axis_pixel(&shot.rgba[o..o + 4], which) {
                    pts.push((x as f64, y as f64));
                }
            }
        }
        // A line covering this much of the picture has at least this many pixels; anything
        // sparser is an icon or a selection highlight, not an axis.
        if pts.len() < (w.min(h) / 3).max(8) {
            continue;
        }
        let n = pts.len() as f64;
        let (mut cx, mut cy) = (0.0, 0.0);
        for p in &pts {
            cx += p.0;
            cy += p.1;
        }
        cx /= n;
        cy /= n;
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for p in &pts {
            let (dx, dy) = (p.0 - cx, p.1 - cy);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        if sxx + syy <= 0.0 {
            continue;
        }
        let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let (ux, uy) = (theta.cos(), theta.sin());
        let mut near = 0usize;
        let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &pts {
            let (dx, dy) = (p.0 - cx, p.1 - cy);
            if (-dx * uy + dy * ux).abs() <= 3.0 {
                near += 1;
            }
            let t = dx * ux + dy * uy;
            tmin = tmin.min(t);
            tmax = tmax.max(t);
        }
        // The axis of a 3D view inside a page is only part of the shot, so it has to be a
        // straight run that still crosses a good share of the picture, not all of it.
        let longest = w.max(h) as f64;
        if near as f64 / n < 0.85 || (tmax - tmin) < (0.22 * longest).max(120.0) {
            continue;
        }
        found = true;
        // Only pixels sitting on the fitted line belong to the axis. A plate number drawn in the
        // same blue sits off it, so it survives.
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                if axis_pixel(&shot.rgba[o..o + 4], which) {
                    let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                    if (-dx * uy + dy * ux).abs() <= 4.0 {
                        axis[y * w + x] = true;
                    }
                }
            }
        }
    }
    if !found {
        return;
    }
    let src = shot.rgba.clone();
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            if !axis[y * w + x] {
                continue;
            }
            let d = (y * w + x) * 4;
            // The nearest pixel of the model in any direction: the axis line is one or two
            // pixels wide, so the first non-axis neighbour is the surface behind it.
            'out: for r in 1..=32isize {
                for (nx, ny) in [(x as isize - r, y as isize), (x as isize + r, y as isize), (x as isize, y as isize - r), (x as isize, y as isize + r)] {
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                        continue;
                    }
                    if axis[ny as usize * w + nx as usize] {
                        continue;
                    }
                    let o = (ny as usize * w + nx as usize) * 4;
                    out[d..d + 4].copy_from_slice(&src[o..o + 4]);
                    break 'out;
                }
            }
        }
    }
    shot.rgba = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(w: u32, h: u32) -> Shot {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_exact_mut(4) {
            px[0] = 240;
            px[1] = 240;
            px[2] = 235;
            px[3] = 255;
        }
        Shot { width: w, height: h, rgba }
    }

    fn paint(shot: &mut Shot, x: u32, y: u32, rgb: [u8; 3]) {
        let o = ((y * shot.width + x) * 4) as usize;
        shot.rgba[o..o + 3].copy_from_slice(&rgb);
    }

    #[test]
    fn removes_a_long_blue_axis() {
        let mut s = shot(64, 64);
        for y in 4..60 {
            paint(&mut s, 30, y, [0, 0, 255]);
        }
        strip_axes(&mut s);
        let o = ((20 * 64 + 30) * 4) as usize;
        assert_ne!(s.rgba[o + 2], 255, "the blue axis pixel was replaced");
        assert_eq!(s.rgba[o], 240, "with the plate behind it");
    }

    #[test]
    fn keeps_blue_plate_numbers() {
        let mut s = shot(96, 96);
        // A fat blue "number": an 8×8 block, far too thick and short to be an axis.
        for y in 40..48 {
            for x in 40..48 {
                paint(&mut s, x, y, [0, 0, 255]);
            }
        }
        let before = s.rgba.clone();
        strip_axes(&mut s);
        assert_eq!(before, s.rgba, "a number is left alone");
    }
}
