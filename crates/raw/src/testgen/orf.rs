use super::tiff::{TiffBuilder, Val};

// ---------------------------------------------------------------- Olympus ORF

/// A little-endian IFD whose value offsets are relative to `base` (the IFD
/// itself sits at `base_offset` from that base). Entries: (tag, type, count,
/// value bytes).
fn relative_ifd(entries: &[(u16, u16, u32, Vec<u8>)], base_offset: usize) -> Vec<u8> {
    let head = 2 + 12 * entries.len() + 4;
    let mut out = Vec::new();
    let mut extra = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, typ, count, bytes) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if bytes.len() <= 4 {
            let mut v = bytes.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&((base_offset + head + extra.len()) as u32).to_le_bytes());
            extra.extend_from_slice(bytes);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&extra);
    out
}

fn shorts(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A synthetic uncompressed Olympus ORF (`IIRO`): 12-bit `data` stored
/// left-justified in 16-bit samples, a GRBG EXIF CFAPattern and a new-style
/// maker note with ImageProcessing levels (black 64, WB 2.0 / 1.5, crop of a
/// 2-pixel border) and CameraSettings pointing at a tiny preview JPEG.
pub fn orf(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    orf_with_strip(width, height, data.iter().flat_map(|v| (v << 4).to_le_bytes()).collect(), 16)
}

/// Uncompressed ORF with ten 12-bit samples per 16-byte block and a zero final byte.
/// Independently encoded from the byte-layout description in storytold/lightcraft#651.
pub fn orf_padded12(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    assert_eq!(width % 10, 0);
    assert_eq!(data.len(), width * height);
    let mut strip = Vec::new();
    for samples in data.as_chunks::<10>().0 {
        let mut block = [0u8; 16];
        for (sample, &value) in samples.iter().enumerate() {
            assert!(value < 4096);
            for bit in 0..12 {
                let at = sample * 12 + bit;
                block[at / 8] |= (((value >> bit) & 1) as u8) << (at % 8);
            }
        }
        strip.extend_from_slice(&block);
    }
    orf_with_strip(width, height, strip, 12)
}

/// Exact 12-bit ORF: MSB-first samples stored in little-endian 32-bit words.
/// This bit-by-bit encoder follows the layout measured on the CC0 XZ-2 file.
pub fn orf_word_packed12(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    assert_eq!(width % 8, 0);
    assert_eq!(data.len(), width * height);
    let mut strip = vec![0u8; data.len() * 3 / 2];
    for (sample, &value) in data.iter().enumerate() {
        assert!(value < 4096);
        for bit in 0..12 {
            let at = sample * 12 + bit;
            let byte = at / 8;
            let stored_byte = (byte / 4) * 4 + 3 - byte % 4;
            strip[stored_byte] |= (((value >> (11 - bit)) & 1) as u8) << (7 - at % 8);
        }
    }
    orf_with_strip(width, height, strip, 12)
}

/// Two-field ORF from the public layout description in storytold/lightcraft#600.
/// Rows are MSB-first packed, all even rows then odd rows, with a 20-byte gap
/// between fields and one undeclared final row. One strip per row exercises
/// strip tables longer than the RowsPerStrip grid.
pub fn orf_two_field12(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    assert_eq!(width % 8, 0);
    assert_eq!(height % 2, 1);
    assert_eq!(data.len(), width * height);
    let row_bytes = width * 3 / 2;
    let mut t = TiffBuilder::default();
    let mut strips = Vec::new();
    let order: Vec<usize> = (0..height).step_by(2).chain((1..height).step_by(2)).collect();
    for (stored_y, &y) in order.iter().enumerate() {
        let mut bytes = vec![0u8; row_bytes];
        for (x, &value) in data[y * width..(y + 1) * width].iter().enumerate() {
            assert!(value < 4096);
            for bit in 0..12 {
                let at = x * 12 + bit;
                bytes[at / 8] |= (((value >> (11 - bit)) & 1) as u8) << (7 - at % 8);
            }
        }
        if stored_y + 1 == height.div_ceil(2) {
            bytes.extend_from_slice(&[0xa5; 20]);
        }
        if stored_y + 1 == height {
            bytes.resize(bytes.len() + row_bytes, 0);
        }
        strips.push(t.blob(bytes));
    }
    let mut note = b"OLYMP\0\x01\0".to_vec();
    // The second values are unrelated metadata and must not be used as divisors.
    note.extend_from_slice(&relative_ifd(&[(0x1017, 3, 2, shorts(&[512, 62])), (0x1018, 3, 2, shorts(&[384, 64]))], 8));
    let exif = t.ifd(vec![(37500, Val::Undefined(note)), (41730, Val::Undefined(vec![2, 0, 2, 0, 2, 1, 1, 0]))]);
    let ifd = t.ifd(vec![
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![1])),
        (273, Val::Blobs(strips)),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![2])),
        (279, Val::Long(vec![row_bytes as u32; height])),
        (34665, Val::Ifds(vec![exif])),
    ]);
    t.chain = vec![ifd];
    let mut bytes = t.build();
    bytes[2..4].copy_from_slice(b"RO");
    bytes
}

fn orf_with_strip(width: usize, height: usize, data: Vec<u8>, bits: u16) -> Vec<u8> {
    let ip = relative_ifd(
        &[
            (0x0100, 3, 2, shorts(&[512, 384])),
            (0x011F, 3, 1, shorts(&[256])),
            (0x0600, 3, 4, shorts(&[64, 64, 64, 64])),
            (0x0611, 3, 2, shorts(&[12, 0])),
            (0x0612, 3, 2, shorts(&[2, 0])),
            (0x0613, 3, 2, shorts(&[2, 0])),
            (0x0614, 4, 1, ((width - 4) as u32).to_le_bytes().to_vec()),
            (0x0615, 4, 1, ((height - 4) as u32).to_le_bytes().to_vec()),
        ],
        64,
    );
    // CameraSettings with a preview JPEG (SOI, SOF0 16×8, EOI) right after it.
    let jpeg = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x08, 0x00, 0x10, 1, 1, 0x11, 0, 0xFF, 0xD9];
    let cs_at = 64 + ip.len().next_multiple_of(4);
    let cs_len = 2 + 12 * 3 + 4;
    let cs = relative_ifd(
        &[
            (0x0100, 4, 1, 1u32.to_le_bytes().to_vec()),
            (0x0101, 4, 1, ((cs_at + cs_len) as u32).to_le_bytes().to_vec()),
            (0x0102, 4, 1, (jpeg.len() as u32).to_le_bytes().to_vec()),
        ],
        cs_at,
    );
    let mut note = b"OLYMPUS\0II\x03\0".to_vec();
    note.extend_from_slice(&relative_ifd(&[(0x2020, 13, 1, (cs_at as u32).to_le_bytes().to_vec()), (0x2040, 13, 1, 64u32.to_le_bytes().to_vec())], 12));
    note.resize(64, 0);
    note.extend_from_slice(&ip);
    note.resize(cs_at, 0);
    note.extend_from_slice(&cs);
    note.extend_from_slice(&jpeg);
    let mut t = TiffBuilder::default();
    let byte_count = data.len() as u32;
    let strip = t.blob(data);
    let exif = t.ifd(vec![(37500, Val::Undefined(note)), (41730, Val::Undefined(vec![2, 0, 2, 0, 1, 0, 2, 1]))]);
    let ifd0 = t.ifd(vec![
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![bits])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![1])),
        (271, Val::Ascii("OLYMPUS IMAGING CORP.".into())),
        (272, Val::Ascii("E-Synthetic".into())),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![height as u32])),
        (279, Val::Long(vec![byte_count])),
        (34665, Val::Ifds(vec![exif])),
    ]);
    t.chain = vec![ifd0];
    let mut b = t.build();
    b[2..4].copy_from_slice(b"RO");
    b
}
