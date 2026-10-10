use super::*;
use std::collections::BTreeMap;
use std::fmt::Write;

pub(super) fn encode(f: F, img: &Image) -> Result<Vec<u8>, E> {
    let (w, h) = img.dimensions();
    let mut out = String::new();
    if f == F::Xbm {
        write!(out, "#define photocraft_width {w}\n#define photocraft_height {h}\nstatic unsigned char photocraft_bits[] = {{\n")
            .map_err(|e| E::encode(f, e))?;
        for row in img.data().chunks_exact(w as usize) {
            for px in row.chunks(8) {
                let v = px.iter().enumerate().fold(0, |v, (i, p)| v | u8::from(*p < 128) << i);
                write!(out, "0x{v:02x},").map_err(|e| E::encode(f, e))?;
            }
            out.push('\n');
        }
        out.push_str("};\n");
    } else {
        // Eight ASCII hexadecimal digits per pixel avoids palette size/cpp mismatches and
        // C-string escaping. Transparency is one-bit, as declared by fidelity warnings.
        let mut colors = BTreeMap::new();
        for px in img.data().as_chunks::<4>().0 {
            let key = xpm_color(px)?;
            let n = colors.len();
            if n >= 1 << 20 && !colors.contains_key(&key) {
                return Err(E::encode(f, "XPM exceeds one million palette entries"));
            }
            colors.entry(key).or_insert(n);
        }
        write!(out, "/* XPM */\nstatic const char *photocraft[] = {{\n\"{w} {h} {} 8\",\n", colors.len()).map_err(|e| E::encode(f, e))?;
        for (color, id) in &colors {
            let value = if color.get(3) == Some(&0) {
                "None".to_string()
            } else {
                format!("#{:02x}{:02x}{:02x}", byte(color, 0, f)?, byte(color, 1, f)?, byte(color, 2, f)?)
            };
            writeln!(out, "\"{id:08x} c {value}\",").map_err(|e| E::encode(f, e))?;
        }
        for row in img.data().chunks_exact(w as usize * 4) {
            out.push('"');
            for px in row.as_chunks::<4>().0 {
                let id = colors.get(&xpm_color(px)?).ok_or_else(|| E::encode(f, "missing palette entry"))?;
                write!(out, "{id:08x}").map_err(|e| E::encode(f, e))?;
            }
            out.push_str("\",\n");
        }
        out.push_str("};\n");
    }
    Ok(out.into_bytes())
}
fn xpm_color(px: &[u8]) -> Result<[u8; 4], E> {
    if byte(px, 3, F::Xpm)? < 128 { Ok([0, 0, 0, 0]) } else { Ok([byte(px, 0, F::Xpm)?, byte(px, 1, F::Xpm)?, byte(px, 2, F::Xpm)?, 255]) }
}

pub(super) fn decode(f: F, b: &[u8], l: &Limits) -> Result<Image, E> {
    // Bound the text/palette parser itself, independently of the declared pixel buffer.
    if b.len() as u64 > l.max_alloc {
        return Err(E::LimitExceeded("text raster exceeds parse budget".into()));
    }
    let s = std::str::from_utf8(b).map_err(|e| E::malformed(f, e))?;
    if f == F::Xbm {
        return xbm(s, l);
    }
    let mut strings = s.split('"');
    strings.next();
    let mut values = Vec::new();
    while let Some(value) = strings.next() {
        if value.contains('\\') {
            return Err(E::unsupported(f, "escaped XPM strings are unsupported"));
        }
        if values.len() >= (1 << 20) + l.max_height as usize || values.len() as u64 * 16 >= l.max_alloc {
            return Err(E::LimitExceeded("XPM string table exceeds budget".into()));
        }
        values.push(value);
        strings.next();
    }
    let header = values.first().ok_or_else(|| E::malformed(f, "missing XPM header"))?;
    let mut fields = header.split_whitespace();
    let mut num = || fields.next().ok_or_else(|| E::malformed(f, "short header"))?.parse::<u32>().map_err(|e| E::malformed(f, e));
    let w = num()?;
    let h = num()?;
    let count = num()? as usize;
    let cpp = num()? as usize;
    l.check(w, h, L::Rgba, S::U8)?;
    if count == 0 || count > 1 << 20 || count as u64 * 64 > l.max_alloc || !(1..=8).contains(&cpp) {
        return Err(E::LimitExceeded("XPM palette/cpp exceeds budget".into()));
    }
    let mut palette = BTreeMap::new();
    for line in values.get(1..1 + count).ok_or_else(|| E::malformed(f, "short palette"))? {
        if !line.is_ascii() {
            return Err(E::unsupported(f, "non-ASCII palette keys"));
        }
        let key = line.get(..cpp).ok_or_else(|| E::malformed(f, "short palette key"))?;
        let mut fields = line.get(cpp..).unwrap_or_default().split_whitespace();
        let mut color = None;
        while let Some(field) = fields.next() {
            let value = fields.next().ok_or_else(|| E::malformed(f, "short palette entry"))?;
            if field == "c" {
                color = Some(parse_color(value)?);
            }
        }
        if palette.insert(key, color.ok_or_else(|| E::unsupported(f, "palette entry has no RGB color"))?).is_some() {
            return Err(E::malformed(f, "duplicate palette key"));
        }
    }
    let rows = values.get(1 + count..1 + count + h as usize).ok_or_else(|| E::malformed(f, "missing pixel rows"))?;
    let mut out = buffer(w as usize * h as usize * 4, l)?;
    for (y, row) in rows.iter().enumerate() {
        if !row.is_ascii() || row.len() != w as usize * cpp {
            return Err(E::malformed(f, "pixel row length disagrees"));
        }
        for x in 0..w as usize {
            let key = row.get(x * cpp..(x + 1) * cpp).ok_or_else(|| E::malformed(f, "bad pixel key"))?;
            let color = palette.get(key).ok_or_else(|| E::malformed(f, "unknown pixel key"))?;
            out.get_mut((y * w as usize + x) * 4..(y * w as usize + x + 1) * 4).ok_or_else(|| E::malformed(f, "bad pixel offset"))?.copy_from_slice(color);
        }
    }
    Image::from_u8(w, h, L::Rgba, out)
}
fn parse_color(s: &str) -> Result<[u8; 4], E> {
    let named = match s.to_ascii_lowercase().as_str() {
        "none" => Some([0, 0, 0, 0]),
        "black" => Some([0, 0, 0, 255]),
        "white" => Some([255, 255, 255, 255]),
        "red" => Some([255, 0, 0, 255]),
        "green" => Some([0, 255, 0, 255]),
        "blue" => Some([0, 0, 255, 255]),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c);
    }
    let hex = s.strip_prefix('#').ok_or_else(|| E::unsupported(F::Xpm, "named X11 color is unsupported"))?;
    if !hex.is_ascii() || !matches!(hex.len(), 3 | 6 | 9 | 12) {
        return Err(E::malformed(F::Xpm, "invalid RGB hex color"));
    }
    let n = hex.len() / 3;
    let mut c = [0, 0, 0, 255];
    for (i, dst) in c.iter_mut().take(3).enumerate() {
        let v =
            u32::from_str_radix(hex.get(i * n..(i + 1) * n).ok_or_else(|| E::malformed(F::Xpm, "short color"))?, 16).map_err(|e| E::malformed(F::Xpm, e))?;
        let max = (1u32 << (n * 4)) - 1;
        *dst = ((v * 255 + max / 2) / max) as u8;
    }
    Ok(c)
}
fn xbm(s: &str, l: &Limits) -> Result<Image, E> {
    let f = F::Xbm;
    let mut w = None;
    let mut h = None;
    for line in s.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("#define") {
            continue;
        }
        let key = fields.next().unwrap_or_default();
        let v = fields.next().unwrap_or_default().parse::<u32>().map_err(|e| E::malformed(f, e))?;
        if key.ends_with("_width") {
            w = Some(v);
        } else if key.ends_with("_height") {
            h = Some(v);
        }
    }
    let w = w.ok_or_else(|| E::malformed(f, "missing width"))?;
    let h = h.ok_or_else(|| E::malformed(f, "missing height"))?;
    l.check(w, h, L::Gray, S::U8)?;
    let (decl, tail) = s.split_once('{').ok_or_else(|| E::malformed(f, "missing bitmap"))?;
    if !decl.contains("char") {
        return Err(E::unsupported(f, "only byte-packed XBM is supported"));
    }
    let (body, _) = tail.split_once('}').ok_or_else(|| E::malformed(f, "unterminated bitmap"))?;
    let stride = (w as usize).div_ceil(8);
    let count = stride * h as usize;
    let mut bits = Vec::new();
    bits.try_reserve_exact(count).map_err(|e| E::LimitExceeded(e.to_string()))?;
    for token in body.split([',', ' ', '\n', '\r', '\t']).filter(|v| !v.is_empty()) {
        if bits.len() >= count {
            return Err(E::malformed(f, "too many bitmap bytes"));
        }
        let value = if let Some(hex) = token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) { u8::from_str_radix(hex, 16) } else { token.parse() }
            .map_err(|e| E::malformed(f, e))?;
        bits.push(value);
    }
    if bits.len() != count {
        return Err(E::malformed(f, "short bitmap"));
    }
    let mut out = buffer(w as usize * h as usize, l)?;
    for (i, p) in out.iter_mut().enumerate() {
        let x = i % w as usize;
        let y = i / w as usize;
        *p = if byte(&bits, y * stride + x / 8, f)? & (1 << (x % 8)) != 0 { 0 } else { 255 };
    }
    Image::from_u8(w, h, L::Gray, out)
}
