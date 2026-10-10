use super::*;

pub(super) fn encode(f: F, img: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, E> {
    let (w, h) = img.dimensions();
    let ch = img.layout().channels();
    let mut out = Vec::new();
    match f {
        F::Farbfeld => {
            out.extend_from_slice(b"farbfeld");
            out.extend_from_slice(&w.to_be_bytes());
            out.extend_from_slice(&h.to_be_bytes());
            for v in img.data().as_chunks::<2>().0 {
                let n = u16::from_ne_bytes(*v);
                out.extend_from_slice(&n.to_be_bytes());
            }
        }
        F::Sgi => {
            out.extend_from_slice(&[1, 218, 0, img.sample_type().bytes() as u8]);
            for v in [if ch == 1 { 2 } else { 3 }, w as u16, h as u16, ch as u16] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(if img.sample_type() == S::U16 { 65535u32 } else { 255 }).to_be_bytes());
            out.resize(512, 0);
            let bps = img.sample_type().bytes();
            for c in 0..ch {
                for y in (0..h as usize).rev() {
                    for x in 0..w as usize {
                        let i = (y * w as usize + x) * ch + c;
                        if bps == 1 {
                            out.push(byte(img.data(), i, f)?);
                        } else {
                            let v = img.data().get(i * 2..i * 2 + 2).ok_or_else(|| E::encode(f, "short sample"))?;
                            out.extend_from_slice(&u16::from_ne_bytes(v.try_into().map_err(|_| E::encode(f, "short sample"))?).to_be_bytes());
                        }
                    }
                }
            }
        }
        F::Pcx => {
            out.extend_from_slice(&[10, 5, 1, 8]);
            for v in [0, 0, w as u16 - 1, h as u16 - 1, 72, 72] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.resize(65, 0);
            out.push(ch as u8);
            let stride = (w as usize + 1) & !1;
            let stride16 = u16::try_from(stride).map_err(|_| E::encode(f, "PCX padded row exceeds 65535 bytes"))?;
            out.extend_from_slice(&stride16.to_le_bytes());
            out.extend_from_slice(&(if ch == 1 { 2u16 } else { 1 }).to_le_bytes());
            out.resize(128, 0);
            for row in img.data().chunks_exact(w as usize * ch) {
                for c in 0..ch {
                    for x in 0..stride {
                        let v = if x < w as usize { byte(row, x * ch + c, f)? } else { 0 };
                        if v >= 192 {
                            out.push(193);
                        }
                        out.push(v);
                    }
                }
            }
            if ch == 1 {
                out.push(12);
                for v in 0..=255 {
                    out.extend_from_slice(&[v, v, v]);
                }
            }
        }
        F::SunRaster => {
            let stride = (w as usize * ch + 1) & !1;
            let len = u32::try_from(stride.checked_mul(h as usize).ok_or_else(|| E::encode(f, "size overflow"))?)
                .map_err(|_| E::encode(f, "raster exceeds 4 GiB"))?;
            for v in [0x59a66a95, w, h, (ch * 8) as u32, len, if ch == 3 { 3 } else { 1 }, 0, 0] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            for row in img.data().chunks_exact(w as usize * ch) {
                out.extend_from_slice(row);
                if row.len() < stride {
                    out.push(0);
                }
            }
        }
        F::Gbr | F::GimpPat => {
            let name = b"PhotoCraft\0";
            let header = if f == F::Gbr { 28 } else { 24 };
            for v in [header + name.len() as u32, if f == F::Gbr { 2 } else { 1 }, w, h, ch as u32] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            out.extend_from_slice(if f == F::Gbr { b"GIMP" } else { b"GPAT" });
            if f == F::Gbr {
                out.extend_from_slice(&25u32.to_be_bytes());
            }
            out.extend_from_slice(name);
            out.extend_from_slice(img.data());
        }
        F::Wbmp => {
            out.extend_from_slice(&[0, 0]);
            mbi(&mut out, w);
            mbi(&mut out, h);
            for row in img.data().chunks_exact(w as usize) {
                for px in row.chunks(8) {
                    out.push(px.iter().enumerate().fold(0, |v, (i, p)| v | u8::from(*p >= 128) << (7 - i)));
                }
            }
        }
        F::Cur => {
            // Conventional 32-bit DIB XOR bitmap plus padded one-bit AND mask. Unlike a PNG
            // cursor this representation opens in older Windows/GIMP/Pillow cursor readers.
            let stride = (w as usize).div_ceil(32) * 4;
            let payload_len = 40 + w as usize * h as usize * 4 + stride * h as usize;
            out.extend_from_slice(&[0, 0, 2, 0, 1, 0, w as u8, h as u8, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&(payload_len as u32).to_le_bytes());
            out.extend_from_slice(&22u32.to_le_bytes());
            out.extend_from_slice(&40u32.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&(h * 2).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(w * h * 4).to_le_bytes());
            out.resize(62, 0);
            for y in (0..h as usize).rev() {
                for x in 0..w as usize {
                    let i = (y * w as usize + x) * 4;
                    out.extend_from_slice(&[byte(img.data(), i + 2, f)?, byte(img.data(), i + 1, f)?, byte(img.data(), i, f)?, byte(img.data(), i + 3, f)?]);
                }
            }
            for y in (0..h as usize).rev() {
                for x in 0..stride {
                    let mut mask = 0;
                    for bit in 0..8 {
                        let px = x * 8 + bit;
                        if px < w as usize && byte(img.data(), (y * w as usize + px) * 4 + 3, f)? == 0 {
                            mask |= 128 >> bit;
                        }
                    }
                    out.push(mask);
                }
            }
        }
        F::Icns => {
            let kind = match (w, h) {
                (16, 16) => b"icp4",
                (32, 32) => b"icp5",
                (64, 64) => b"icp6",
                (128, 128) => b"ic07",
                (256, 256) => b"ic08",
                (512, 512) => b"ic09",
                (1024, 1024) => b"ic10",
                _ => return Err(E::encode(f, "ICNS requires a square image of 16, 32, 64, 128, 256, 512 or 1024 pixels")),
            };
            let png = super::super::png::encode(img, Plan { layout: L::Rgba, sample: S::U8 }, opts)?;
            let len = u32::try_from(png.len()).map_err(|_| E::encode(f, "image too large"))?;
            out.extend_from_slice(b"icns");
            out.extend_from_slice(&(len + 16).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(&(len + 8).to_be_bytes());
            out.extend_from_slice(&png);
        }
        _ => return Err(E::unsupported(f, "not a binary legacy format")),
    }
    Ok(out)
}
fn mbi(out: &mut Vec<u8>, mut n: u32) {
    let mut b = vec![(n & 127) as u8];
    n >>= 7;
    while n > 0 {
        b.push((n & 127) as u8 | 128);
        n >>= 7;
    }
    out.extend(b.into_iter().rev());
}
fn read_mbi(r: &mut Reader<'_>) -> Result<u32, E> {
    let mut n = 0u32;
    for _ in 0..5 {
        let b = r.u8()?;
        n = n.checked_mul(128).and_then(|n| n.checked_add(u32::from(b & 127))).ok_or_else(|| r.bad("dimension overflow"))?;
        if b & 128 == 0 {
            return Ok(n);
        }
    }
    Err(r.bad("invalid variable-length integer"))
}
pub(super) fn wbmp_header(b: &[u8]) -> Result<(u32, u32, usize), E> {
    let mut r = Reader::new(b, F::Wbmp);
    if r.take(2)? != [0, 0] {
        return Err(r.bad("unsupported WBMP header"));
    }
    let w = read_mbi(&mut r)?;
    let h = read_mbi(&mut r)?;
    Ok((w, h, r.pos))
}

pub(super) fn decode(f: F, b: &[u8], l: &Limits) -> Result<Image, E> {
    let mut r = Reader::new(b, f);
    match f {
        F::Farbfeld => {
            if r.take(8)? != b"farbfeld" {
                return Err(r.bad("bad magic"));
            }
            let w = r.be32()?;
            let h = r.be32()?;
            l.check(w, h, L::Rgba, S::U16)?;
            let n = w as usize * h as usize * 8;
            let pixels = r.take(n)?;
            let mut out = buffer(n, l)?;
            for (px, stored) in out.as_chunks_mut::<2>().0.iter_mut().zip(pixels.as_chunks::<2>().0) {
                px.copy_from_slice(&u16::from_be_bytes(*stored).to_ne_bytes());
            }
            Image::from_raw(w, h, L::Rgba, S::U16, out)
        }
        F::Pcx => pcx(b, l),
        F::Sgi => sgi(b, l),
        F::SunRaster => sun(b, l),
        F::Gbr | F::GimpPat => {
            let header = r.be32()? as usize;
            let version = r.be32()?;
            let w = r.be32()?;
            let h = r.be32()?;
            let ch = r.be32()?;
            if r.take(4)? != if f == F::Gbr { b"GIMP" } else { b"GPAT" } || version != if f == F::Gbr { 2 } else { 1 } {
                return Err(E::unsupported(f, "unsupported asset version"));
            }
            if f == F::Gbr {
                r.be32()?;
                if !matches!(ch, 1 | 4) {
                    return Err(E::unsupported(f, "unsupported brush channels"));
                }
            }
            if header <= r.pos || header > 1 << 20 {
                return Err(r.bad("invalid name header"));
            }
            let name = r.take(header - r.pos)?;
            if name.last() != Some(&0) {
                return Err(r.bad("unterminated asset name"));
            }
            let layout = layout(ch, f)?;
            l.check(w, h, layout, S::U8)?;
            let n = w as usize * h as usize * ch as usize;
            let data = r.take(n)?;
            Image::from_u8(w, h, layout, data.to_vec())
        }
        F::Wbmp => {
            let (w, h, off) = wbmp_header(b)?;
            l.check(w, h, L::Gray, S::U8)?;
            let stride = (w as usize).div_ceil(8);
            let bits = b.get(off..off + stride * h as usize).ok_or_else(|| r.bad("truncated bitmap"))?;
            let mut out = buffer(w as usize * h as usize, l)?;
            for (i, p) in out.iter_mut().enumerate() {
                let y = i / w as usize;
                let x = i % w as usize;
                *p = if byte(bits, y * stride + x / 8, f)? & (128 >> (x % 8)) == 0 { 0 } else { 255 };
            }
            Image::from_u8(w, h, L::Gray, out)
        }
        F::Cur => {
            if r.take(4)? != [0, 0, 2, 0] || r.le16()? != 1 {
                return Err(E::unsupported(f, "only single-image cursor files are supported"));
            }
            let width = r.u8()?;
            let height = r.u8()?;
            r.take(6)?;
            let len = r.le32()? as usize;
            let off = r.le32()? as usize;
            if off < 22 {
                return Err(r.bad("cursor image overlaps header"));
            }
            let payload = b.get(off..off.checked_add(len).ok_or_else(|| r.bad("image offset overflow"))?).ok_or_else(|| r.bad("truncated cursor"))?;
            let img = if payload.starts_with(b"\x89PNG") {
                super::super::png::decode(payload, l)?
            } else {
                if b.len() as u64 > l.max_alloc {
                    return Err(E::LimitExceeded("cursor input exceeds allocation budget".into()));
                }
                let mut ico = b.to_vec();
                set(&mut ico, 2, 1, f)?;
                for (i, v) in [1, 0, 32, 0].into_iter().enumerate() {
                    set(&mut ico, 10 + i, v, f)?;
                }
                super::super::via_image::decode(F::Ico, &ico, l)?
            };
            if img.width() != if width == 0 { 256 } else { u32::from(width) } || img.height() != if height == 0 { 256 } else { u32::from(height) } {
                return Err(r.bad("cursor dimensions disagree"));
            }
            Ok(img)
        }
        F::Icns => {
            if r.take(4)? != b"icns" {
                return Err(r.bad("bad magic"));
            }
            let len = r.be32()? as usize;
            if len != b.len() {
                return Err(r.bad("container length disagrees"));
            }
            let mut best = None;
            while r.pos < len {
                let kind = r.take(4)?;
                let n = r.be32()? as usize;
                if n < 8 {
                    return Err(r.bad("invalid chunk length"));
                }
                let payload = r.take(n - 8)?;
                let size = match kind {
                    b"icp4" => 16,
                    b"icp5" => 32,
                    b"icp6" => 64,
                    b"ic07" => 128,
                    b"ic08" => 256,
                    b"ic09" => 512,
                    b"ic10" => 1024,
                    _ => continue,
                };
                if payload.starts_with(b"\x89PNG") && best.as_ref().is_none_or(|(s, _)| size > *s) {
                    best = Some((size, payload));
                }
            }
            let (size, payload) = best.ok_or_else(|| E::unsupported(f, "no supported PNG icon representation"))?;
            let img = super::super::png::decode(payload, l)?;
            if img.dimensions() != (size, size) {
                return Err(r.bad("icon dimensions disagree"));
            }
            Ok(img)
        }
        _ => Err(E::unsupported(f, "not a binary legacy format")),
    }
}

fn pcx(b: &[u8], l: &Limits) -> Result<Image, E> {
    let f = F::Pcx;
    let mut r = Reader::new(b, f);
    if r.u8()? != 10 {
        return Err(r.bad("bad magic"));
    }
    r.u8()?;
    if r.u8()? != 1 || r.u8()? != 8 {
        return Err(E::unsupported(f, "only 8-bit PCX planes are supported"));
    }
    let x = r.le16()?;
    let y = r.le16()?;
    let xmax = r.le16()?;
    let ymax = r.le16()?;
    let w = u32::from(xmax.checked_sub(x).ok_or_else(|| r.bad("invalid width"))?) + 1;
    let h = u32::from(ymax.checked_sub(y).ok_or_else(|| r.bad("invalid height"))?) + 1;
    r.take(53)?;
    let ch = r.u8()?;
    let stride = r.le16()? as usize;
    if !matches!(ch, 1 | 3) {
        return Err(E::unsupported(f, "only grayscale/palette and RGB PCX are supported"));
    }
    if stride < w as usize || stride == 0 {
        return Err(r.bad("invalid scanline stride"));
    }
    r.take(128 - r.pos)?;
    let out_layout = L::Rgb;
    l.check(w, h, out_layout, S::U8)?;
    let palette = if ch == 1 {
        let start = b.len().checked_sub(769).ok_or_else(|| r.bad("missing palette"))?;
        if b.get(start) == Some(&12) { b.get(start + 1..) } else { None }
    } else {
        None
    };
    let minimum = stride.saturating_mul(ch as usize).div_ceil(63).saturating_mul(h as usize);
    if b.len().saturating_sub(r.pos) < minimum {
        return Err(r.bad("PCX pixels cannot fit declared dimensions"));
    }
    let mut out = buffer(w as usize * h as usize * 3, l)?;
    let mut row = buffer(stride * ch as usize, l)?;
    for y in 0..h as usize {
        let mut pos = 0;
        while pos < row.len() {
            let v = r.u8()?;
            let (n, v) = if v & 192 == 192 { ((v & 63) as usize, r.u8()?) } else { (1, v) };
            if n == 0 || n > row.len() - pos {
                return Err(r.bad("RLE run crosses scanline"));
            }
            row.get_mut(pos..pos + n).ok_or_else(|| r.bad("bad RLE offset"))?.fill(v);
            pos += n;
        }
        for x in 0..w as usize {
            for c in 0..3 {
                let v = if ch == 3 {
                    byte(&row, c * stride + x, f)?
                } else {
                    let index = byte(&row, x, f)?;
                    if let Some(palette) = palette { byte(palette, index as usize * 3 + c, f)? } else { index }
                };
                set(&mut out, (y * w as usize + x) * 3 + c, v, f)?;
            }
        }
    }
    Image::from_u8(w, h, L::Rgb, out)
}
fn sgi(b: &[u8], l: &Limits) -> Result<Image, E> {
    let f = F::Sgi;
    let mut r = Reader::new(b, f);
    if r.be16()? != 474 {
        return Err(r.bad("bad magic"));
    }
    let storage = r.u8()?;
    let bps = r.u8()?;
    r.be16()?;
    let w = u32::from(r.be16()?);
    let h = u32::from(r.be16()?);
    let ch = u32::from(r.be16()?);
    let lay = layout(ch, f)?;
    let sample = match bps {
        1 => S::U8,
        2 => S::U16,
        _ => return Err(E::unsupported(f, "unsupported sample width")),
    };
    l.check(w, h, lay, sample)?;
    r.take(92)?;
    if r.be32()? != 0 {
        return Err(E::unsupported(f, "colormapped SGI is unsupported"));
    }
    r.take(512 - r.pos)?;
    let output_bytes = w as usize * h as usize * ch as usize * bps as usize;
    if storage == 0 && b.get(r.pos..r.pos.checked_add(output_bytes).ok_or_else(|| r.bad("size overflow"))?).is_none() {
        return Err(r.bad("truncated SGI pixels"));
    }
    let rows = h as usize * ch as usize;
    let offsets = if storage == 1 { Some(r.take(rows * 4)?) } else { None };
    let lengths = if storage == 1 { Some(r.take(rows * 4)?) } else { None };
    if storage > 1 {
        return Err(E::unsupported(f, "unsupported SGI storage"));
    }
    let mut out = buffer(output_bytes, l)?;
    for c in 0..ch as usize {
        for y in 0..h as usize {
            let mut rr;
            let row = if let (Some(offsets), Some(lengths)) = (offsets, lengths) {
                let i = (c * h as usize + y) * 4;
                let off = u32::from_be_bytes(offsets.get(i..i + 4).ok_or_else(|| r.bad("short offset table"))?.try_into().map_err(|_| r.bad("short offset"))?)
                    as usize;
                let len = u32::from_be_bytes(lengths.get(i..i + 4).ok_or_else(|| r.bad("short length table"))?.try_into().map_err(|_| r.bad("short length"))?)
                    as usize;
                let payload = b.get(off..off.checked_add(len).ok_or_else(|| r.bad("offset overflow"))?).ok_or_else(|| r.bad("truncated RLE row"))?;
                rr = Reader::new(payload, f);
                sgi_rle(&mut rr, w as usize, bps, l)?
            } else {
                r.take(w as usize * bps as usize)?.to_vec()
            };
            for x in 0..w as usize {
                let dest = (((h as usize - 1 - y) * w as usize + x) * ch as usize + c) * bps as usize;
                if bps == 1 {
                    set(&mut out, dest, byte(&row, x, f)?, f)?;
                } else {
                    let bytes = row.get(x * 2..x * 2 + 2).ok_or_else(|| r.bad("short sample"))?;
                    let v = u16::from_be_bytes(bytes.try_into().map_err(|_| r.bad("short sample"))?).to_ne_bytes();
                    out.get_mut(dest..dest + 2).ok_or_else(|| r.bad("bad pixel offset"))?.copy_from_slice(&v);
                }
            }
        }
    }
    Image::from_raw(w, h, lay, sample, out)
}
fn sgi_rle(r: &mut Reader<'_>, w: usize, bps: u8, l: &Limits) -> Result<Vec<u8>, E> {
    let mut out = buffer(w * bps as usize, l)?;
    let mut pos = 0;
    loop {
        let token = if bps == 1 { u16::from(r.u8()?) } else { r.be16()? };
        let n = (token & 127) as usize;
        if n == 0 {
            if pos != out.len() {
                return Err(r.bad("short RLE row"));
            }
            return Ok(out);
        }
        let bytes = n * bps as usize;
        if bytes > out.len().saturating_sub(pos) {
            return Err(r.bad("RLE exceeds scanline"));
        }
        if token & 128 != 0 {
            out.get_mut(pos..pos + bytes).ok_or_else(|| r.bad("bad RLE offset"))?.copy_from_slice(r.take(bytes)?);
        } else {
            let v = r.take(bps as usize)?;
            for dst in out.get_mut(pos..pos + bytes).ok_or_else(|| r.bad("bad RLE offset"))?.chunks_exact_mut(bps as usize) {
                dst.copy_from_slice(v);
            }
        }
        pos += bytes;
    }
}
fn sun(b: &[u8], l: &Limits) -> Result<Image, E> {
    let f = F::SunRaster;
    let mut r = Reader::new(b, f);
    if r.be32()? != 0x59a66a95 {
        return Err(r.bad("bad magic"));
    }
    let w = r.be32()?;
    let h = r.be32()?;
    let depth = r.be32()?;
    r.be32()?;
    let kind = r.be32()?;
    let maptype = r.be32()?;
    let maplen = r.be32()? as usize;
    if !matches!(kind, 0 | 1 | 3) || !matches!(depth, 8 | 24) {
        return Err(E::unsupported(f, "only uncompressed 8/24-bit Sun raster is supported"));
    }
    l.check(w, h, L::Rgb, S::U8)?;
    if maplen > 768 || (maptype != 0 && maptype != 1) {
        return Err(E::unsupported(f, "unsupported color map"));
    }
    let map = r.take(maplen)?;
    if maplen > 0 && (!maplen.is_multiple_of(3) || depth != 8) {
        return Err(r.bad("invalid color map"));
    }
    let ch = depth as usize / 8;
    let stride = (w as usize * ch + 1) & !1;
    let length = stride.checked_mul(h as usize).ok_or_else(|| r.bad("pixel size overflow"))?;
    if b.get(r.pos..r.pos.checked_add(length).ok_or_else(|| r.bad("pixel size overflow"))?).is_none() {
        return Err(r.bad("truncated Sun raster"));
    }
    let mut out = buffer(w as usize * h as usize * 3, l)?;
    for y in 0..h as usize {
        let row = r.take(stride)?;
        for x in 0..w as usize {
            for c in 0..3 {
                let v = if ch == 3 {
                    byte(row, x * 3 + if kind == 3 { c } else { 2 - c }, f)?
                } else {
                    let index = byte(row, x, f)?;
                    if map.is_empty() { index } else { byte(map, c * (maplen / 3) + index as usize, f)? }
                };
                set(&mut out, (y * w as usize + x) * 3 + c, v, f)?;
            }
        }
    }
    Image::from_u8(w, h, L::Rgb, out)
}
