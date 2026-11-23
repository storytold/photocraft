//! Original public-spec source fixture, also used by existing CLI unit controls.
// Original one-pixel uncompressed wire fixture, following the owned XCF testkit
// recipe. No decoder/export round trip supplies the sample-order oracle.
pub(crate) fn rgb16(version: u32, little: bool) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn pointer(bytes: &mut Vec<u8>, wide: bool) -> usize {
        let slot = bytes.len();
        bytes.resize(slot + if wide { 8 } else { 4 }, 0);
        slot
    }
    fn patch(bytes: &mut [u8], slot: usize, wide: bool) {
        let offset = bytes.len();
        if wide {
            bytes[slot..slot + 8].copy_from_slice(&(offset as u64).to_be_bytes());
        } else {
            bytes[slot..slot + 4].copy_from_slice(&(offset as u32).to_be_bytes());
        }
    }
    let mut bytes = format!("gimp xcf v{version:03}\0").into_bytes();
    let wide = version >= 11;
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut bytes, value);
    }
    bytes.push(0); // PROP_COMPRESSION: uncompressed.
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let layer = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    pointer(&mut bytes, wide); // No channels.
    patch(&mut bytes, layer, wide);
    for value in [1, 1, 1, 6] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(b"Pixel\0");
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let hierarchy = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide); // No mask.
    patch(&mut bytes, hierarchy, wide);
    for value in [1, 1, 8] {
        word(&mut bytes, value);
    }
    let level = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    patch(&mut bytes, level, wide);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let tile = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    patch(&mut bytes, tile, wide);
    for value in [0x1234u16, 0x5678, 0x9abc, 0xffff] {
        bytes.extend_from_slice(&if little { value.to_le_bytes() } else { value.to_be_bytes() });
    }
    bytes
}
