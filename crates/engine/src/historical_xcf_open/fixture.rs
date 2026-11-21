//! Independent one-pixel XCF7/U16 framing for host tests; structural words are always big-endian.
pub(crate) fn historical_pixel(little: bool) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn point(bytes: &mut [u8], slot: usize, value: usize) {
        bytes[slot..slot + 4].copy_from_slice(&(value as u32).to_be_bytes());
    }
    let mut bytes = b"gimp xcf v007\0".to_vec();
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut bytes, value);
    }
    bytes.push(0); // No tile compression.
    for value in [0, 0] {
        word(&mut bytes, value);
    }
    let layer = bytes.len();
    for value in [0, 0, 0] {
        word(&mut bytes, value);
    }
    let offset = bytes.len();
    point(&mut bytes, layer, offset);
    for value in [1, 1, 1, 2] {
        word(&mut bytes, value);
    }
    bytes.extend(b"a\0");
    for value in [2, 0, 8, 4, 1, 0, 0] {
        word(&mut bytes, value);
    }
    let hierarchy = bytes.len();
    for value in [0, 0] {
        word(&mut bytes, value);
    }
    let offset = bytes.len();
    point(&mut bytes, hierarchy, offset);
    for value in [1, 1, 8] {
        word(&mut bytes, value);
    }
    let level = bytes.len();
    for value in [0, 0] {
        word(&mut bytes, value);
    }
    let offset = bytes.len();
    point(&mut bytes, level, offset);
    for value in [1, 1] {
        word(&mut bytes, value);
    }
    let tile = bytes.len();
    for value in [0, 0] {
        word(&mut bytes, value);
    }
    let offset = bytes.len();
    point(&mut bytes, tile, offset);
    for value in [0x1234u16, 0x4567, 0x89ab, 0xffff] {
        bytes.extend(if little { value.to_le_bytes() } else { value.to_be_bytes() });
    }
    bytes
}

/// A modern big-endian U16 XCF with a selectable red sample.
pub(crate) fn modern_pixel(red: u16) -> Vec<u8> {
    fn words(bytes: &mut Vec<u8>, values: &[u32]) {
        bytes.extend(values.iter().flat_map(|word| word.to_be_bytes()));
    }
    fn point(bytes: &mut [u8], slot: usize, address: usize) {
        bytes[slot..slot + 8].copy_from_slice(&(address as u64).to_be_bytes());
    }
    let mut bytes = b"gimp xcf v012\0".to_vec();
    words(&mut bytes, &[1, 1, 0, 250, 17, 1]);
    bytes.push(0);
    words(&mut bytes, &[0, 0]);
    let slot = bytes.len();
    bytes.extend([0; 24]); // Layer pointer, end layers, end channels.
    let layer = bytes.len();
    point(&mut bytes, slot, layer);
    words(&mut bytes, &[1, 1, 1, 2]);
    bytes.extend(b"m\0");
    words(&mut bytes, &[0, 0]);
    let slot = bytes.len();
    bytes.extend([0; 16]); // Hierarchy pointer and no mask.
    let hierarchy = bytes.len();
    point(&mut bytes, slot, hierarchy);
    words(&mut bytes, &[1, 1, 8]);
    let slot = bytes.len();
    bytes.extend([0; 16]);
    let level = bytes.len();
    point(&mut bytes, slot, level);
    words(&mut bytes, &[1, 1]);
    let slot = bytes.len();
    bytes.extend([0; 16]);
    let tile = bytes.len();
    point(&mut bytes, slot, tile);
    bytes.extend([red, 0x5678u16, 0x9abc, 0xffff].into_iter().flat_map(u16::to_be_bytes));
    bytes
}
