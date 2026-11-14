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
