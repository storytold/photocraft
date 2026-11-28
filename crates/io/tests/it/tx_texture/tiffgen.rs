//! A tiny hand-rolled TIFF / BigTIFF writer for the `.tx` tests: either byte order, strips or
//! tiles, several directories in one IFD chain. Trimmed from the writer that lived in
//! `crates/codecs/tests/it/common/tiffgen.rs` before the codecs moved to craft-libs.

/// TIFF field types used by the tests.
pub const SHORT: u16 = 3;
pub const LONG: u16 = 4;
const LONG8: u16 = 16;

fn type_size(ty: u16) -> usize {
    match ty {
        SHORT => 2,
        LONG => 4,
        _ => 8,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Dir {
    /// `(tag, type, values)`; strip / tile offsets and byte counts are added from `chunks`.
    pub entries: Vec<(u16, u16, Vec<u64>)>,
    /// Pixel data, one blob per strip or tile.
    pub chunks: Vec<Vec<u8>>,
    /// Tiles (322/323 must be in `entries`) instead of strips.
    pub tiled: bool,
}

impl Dir {
    pub fn tag(mut self, tag: u16, ty: u16, values: &[u64]) -> Self {
        self.entries.push((tag, ty, values.to_vec()));
        self
    }
}

/// A basic directory: `w`×`h`, `spp` samples of `bits` bits, the given photometric, chunky,
/// uncompressed, one strip per `rows` rows holding `data` (already in file byte order).
pub fn strips(w: u32, h: u32, bits: u16, spp: u16, photometric: u16, rows: u32, data: &[u8]) -> Dir {
    let row = (w as usize * bits as usize * spp as usize).div_ceil(8);
    let chunks = data.chunks((row * rows as usize).max(1)).map(<[u8]>::to_vec).collect();
    Dir { entries: base(w, h, bits, spp, photometric), chunks, tiled: false }.tag(278, LONG, &[u64::from(rows)])
}

pub fn base(w: u32, h: u32, bits: u16, spp: u16, photometric: u16) -> Vec<(u16, u16, Vec<u64>)> {
    vec![
        (256, LONG, vec![u64::from(w)]),
        (257, LONG, vec![u64::from(h)]),
        (258, SHORT, vec![u64::from(bits); spp as usize]),
        (259, SHORT, vec![1]),
        (262, SHORT, vec![u64::from(photometric)]),
        (277, SHORT, vec![u64::from(spp)]),
    ]
}

/// Where things ended up in a built file.
pub struct Built {
    pub bytes: Vec<u8>,
    /// Offset of each directory (`dirs` order).
    pub ifd_at: Vec<u64>,
    /// Offset of each directory's next-IFD pointer field.
    pub next_at: Vec<u64>,
}

/// Builds a file whose IFD chain lists `dirs` in the order of `chain`.
pub fn build(little: bool, big: bool, dirs: &[Dir], chain: &[usize]) -> Built {
    let e16 = |v: u16| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let e32 = |v: u32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let e64 = |v: u64| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let enc = |ty: u16, v: u64| -> Vec<u8> {
        match type_size(ty) {
            2 => e16(v as u16),
            4 => e32(v as u32),
            _ => e64(v),
        }
    };
    let header = if big { 16 } else { 8 };
    let entry_len = if big { 20 } else { 12 };
    let inline = if big { 8 } else { 4 };
    let off_ty = if big { LONG8 } else { LONG };

    // Complete entry lists (with chunk offsets / byte counts), sorted by tag.
    let mut full: Vec<Vec<(u16, u16, Vec<u64>)>> = Vec::new();
    for d in dirs {
        let mut e = d.entries.clone();
        let (ot, ct) = if d.tiled { (324, 325) } else { (273, 279) };
        e.retain(|x| x.0 != ot && x.0 != ct);
        e.push((ot, off_ty, vec![0; d.chunks.len()]));
        e.push((ct, off_ty, d.chunks.iter().map(|c| c.len() as u64).collect()));
        e.sort_by_key(|x| x.0);
        full.push(e);
    }

    // Pass 1: positions. Layout: header, then per directory its chunks, its out-of-line values
    // and its table.
    let mut pos = header as u64;
    let mut ifd_at = Vec::new();
    let mut chunk_at = Vec::new();
    for (d, e) in dirs.iter().zip(&full) {
        let mut ca = Vec::new();
        for c in &d.chunks {
            ca.push(pos);
            pos += c.len() as u64;
        }
        chunk_at.push(ca);
        for (_, ty, vals) in e {
            let n = type_size(*ty) * vals.len();
            if n > inline {
                pos += n as u64;
            }
        }
        pos += pos % 2; // word alignment
        ifd_at.push(pos);
        pos += (if big { 8 } else { 2 }) + (e.len() * entry_len) as u64 + if big { 8 } else { 4 };
    }

    // Pass 2: bytes.
    let mut f = Vec::new();
    f.extend_from_slice(if little { b"II" } else { b"MM" });
    let first = chain.first().map_or(0, |&i| ifd_at[i]);
    if big {
        f.extend(e16(43));
        f.extend(e16(8));
        f.extend(e16(0));
        f.extend(e64(first));
    } else {
        f.extend(e16(42));
        f.extend(e32(first as u32));
    }
    let mut next_at = vec![0; dirs.len()];
    for (k, (d, e)) in dirs.iter().zip(&full).enumerate() {
        for c in &d.chunks {
            f.extend_from_slice(c);
        }
        let ot = if d.tiled { 324 } else { 273 };
        let mut table = Vec::new();
        table.extend(if big { e64(e.len() as u64) } else { e16(e.len() as u16) });
        for (tag, ty, vals) in e {
            let mut data = Vec::new();
            for (i, &v) in vals.iter().enumerate() {
                data.extend(enc(*ty, if *tag == ot { chunk_at[k][i] } else { v }));
            }
            table.extend(e16(*tag));
            table.extend(e16(*ty));
            table.extend(if big { e64(vals.len() as u64) } else { e32(vals.len() as u32) });
            if data.len() <= inline {
                data.resize(inline, 0);
                table.extend(data);
            } else {
                let at = f.len() as u64;
                f.extend(data);
                table.extend(if big { e64(at) } else { e32(at as u32) });
            }
        }
        if f.len() % 2 == 1 {
            f.push(0);
        }
        assert_eq!(f.len() as u64, ifd_at[k]);
        let next = chain.iter().position(|&c| c == k).and_then(|p| chain.get(p + 1)).map_or(0, |&n| ifd_at[n]);
        f.extend(table);
        next_at[k] = f.len() as u64;
        f.extend(if big { e64(next) } else { e32(next as u32) });
    }
    Built { bytes: f, ifd_at, next_at }
}

/// Splits an interleaved `w`×`h` image of `spp` samples (`bps` bytes each) into chunky tiles of
/// `tw`×`th` (one blob per tile), padded with zeros.
pub fn tile_split(px: &[u8], w: usize, h: usize, spp: usize, bps: usize, tw: usize, th: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for ty in 0..h.div_ceil(th) {
        for tx in 0..w.div_ceil(tw) {
            let mut t = vec![0u8; tw * th * spp * bps];
            for y in 0..th {
                for x in 0..tw {
                    let (ix, iy) = (tx * tw + x, ty * th + y);
                    if ix >= w || iy >= h {
                        continue;
                    }
                    let src = (iy * w + ix) * spp * bps;
                    let dst = (y * tw + x) * spp * bps;
                    t[dst..dst + spp * bps].copy_from_slice(&px[src..src + spp * bps]);
                }
            }
            out.push(t);
        }
    }
    out
}
