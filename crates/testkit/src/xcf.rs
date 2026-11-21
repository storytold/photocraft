//! Independent XCF wire fixtures shared across PhotoCraft's host tests.

/// The historical one-pixel source used by engine, desktop and automation tests.
pub fn historical_pixel(little: bool) -> Vec<u8> {
    rgb16(7, little, b"a\0", &[2, 0, 8, 4, 1], [0x1234, 0x4567, 0x89ab, 0xffff])
}

/// A historical RGB16 source across all ambiguous versions, used by the CLI.
pub fn historical_rgb16(version: u32, little: bool) -> Vec<u8> {
    rgb16(version, little, b"Pixel\0", &[], [0x1234, 0x5678, 0x9abc, 0xffff])
}

/// A modern big-endian RGB16 source with a selectable red sample.
pub fn modern_pixel(red: u16) -> Vec<u8> {
    rgb16(12, false, b"m\0", &[], [red, 0x5678, 0x9abc, 0xffff])
}

fn rgb16(version: u32, little: bool, name: &[u8], properties: &[u32], samples: [u16; 4]) -> Vec<u8> {
    struct Wire {
        bytes: Vec<u8>,
        wide: bool,
    }
    impl Wire {
        fn word(&mut self, value: u32) {
            self.bytes.extend_from_slice(&value.to_be_bytes());
        }
        fn pointer(&mut self, value: usize) {
            if self.wide {
                self.bytes.extend_from_slice(&(value as u64).to_be_bytes());
            } else {
                self.word(value as u32);
            }
        }
        fn slot(&mut self) -> usize {
            let slot = self.bytes.len();
            self.pointer(0);
            slot
        }
        fn patch(&mut self, slot: usize) {
            let offset = self.bytes.len();
            if self.wide {
                self.bytes[slot..slot + 8].copy_from_slice(&(offset as u64).to_be_bytes());
            } else {
                self.bytes[slot..slot + 4].copy_from_slice(&(offset as u32).to_be_bytes());
            }
        }
    }
    let mut wire = Wire { bytes: format!("gimp xcf v{version:03}\0").into_bytes(), wide: version >= 11 };
    for value in [1, 1, 0, 250, 17, 1] {
        wire.word(value);
    }
    wire.bytes.push(0); // Uncompressed tile samples.
    wire.word(0);
    wire.word(0);
    let layer = wire.slot();
    wire.pointer(0);
    wire.pointer(0); // No saved channels.
    wire.patch(layer);
    for value in [1, 1, 1, name.len() as u32] {
        wire.word(value);
    }
    wire.bytes.extend_from_slice(name);
    for &property in properties {
        wire.word(property);
    }
    wire.word(0);
    wire.word(0);
    let hierarchy = wire.slot();
    wire.pointer(0); // No layer mask.
    wire.patch(hierarchy);
    for value in [1, 1, 8] {
        wire.word(value);
    }
    let level = wire.slot();
    wire.pointer(0);
    wire.patch(level);
    wire.word(1);
    wire.word(1);
    let tile = wire.slot();
    wire.pointer(0);
    wire.patch(tile);
    for sample in samples {
        wire.bytes.extend_from_slice(&if little { sample.to_le_bytes() } else { sample.to_be_bytes() });
    }
    wire.bytes
}

/// Wrap one block of uncompressed XCF bytes in an independently framed XZ stream.
pub fn xz(bytes: &[u8]) -> Vec<u8> {
    xz_with_dictionary(bytes, 0)
}

/// Use a selected LZMA2 dictionary property for memory admission tests.
pub fn xz_with_dictionary(bytes: &[u8], dictionary: u8) -> Vec<u8> {
    write_xz(&[bytes], dictionary, |_, _| (None, None), |_, _| {})
}

/// Wrap independently selected blocks and optionally change their Index records.
pub fn xz_blocks(blocks: &[&[u8]], change_record: impl Fn(usize, &mut (usize, usize))) -> Vec<u8> {
    write_xz(blocks, 0, |_, _| (None, None), change_record)
}

/// Wrap one block with caller-provided optional size declarations. Its Index
/// still describes the actual block.
pub fn xz_with_header_sizes(bytes: &[u8], sizes: impl FnOnce(usize, usize) -> (Option<usize>, Option<usize>)) -> Vec<u8> {
    let compressed = bytes.len() + bytes.len().div_ceil(65536) * 3 + 1;
    let declared = sizes(compressed, bytes.len());
    write_xz(&[bytes], 0, |_, _| declared, |_, _| {})
}

fn write_xz(
    blocks: &[&[u8]],
    dictionary: u8,
    declarations: impl Fn(usize, (usize, usize)) -> (Option<usize>, Option<usize>),
    change_record: impl Fn(usize, &mut (usize, usize)),
) -> Vec<u8> {
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 128 });
            if value == 0 {
                break;
            }
        }
    }
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend(crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut index = vec![0];
    vli(blocks.len(), &mut index);
    for (block, bytes) in blocks.iter().enumerate() {
        let mut data = Vec::new();
        for chunk in bytes.chunks(65536) {
            data.push(1);
            data.extend(((chunk.len() - 1) as u16).to_be_bytes());
            data.extend(chunk);
        }
        data.push(0);
        let (compressed, uncompressed) = declarations(block, (data.len(), bytes.len()));
        let flags = u8::from(compressed.is_some()) << 6 | u8::from(uncompressed.is_some()) << 7;
        let mut header = vec![0, flags];
        for size in [compressed, uncompressed].into_iter().flatten() {
            vli(size, &mut header);
        }
        header.extend([0x21, 1, dictionary]); // LZMA2 filter and its properties.
        // Preserve the original 12-byte test header, including its zero padding.
        header.resize(header.len().next_multiple_of(4).max(8), 0);
        header[0] = (header.len() / 4) as u8;
        header.extend(crc32fast::hash(&header).to_le_bytes());
        out.extend(&header);
        out.extend(&data);
        out.resize(out.len().next_multiple_of(4), 0);
        out.extend(crc32fast::hash(bytes).to_le_bytes());
        let mut record = (header.len() + data.len() + 4, bytes.len());
        change_record(block, &mut record);
        vli(record.0, &mut index);
        vli(record.1, &mut index);
    }
    index.resize(index.len().next_multiple_of(4), 0);
    index.extend(crc32fast::hash(&index).to_le_bytes());
    out.extend(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend([0, 1]);
    out.extend(crc32fast::hash(&footer).to_le_bytes());
    out.extend(footer);
    out.extend(b"YZ");
    out
}
