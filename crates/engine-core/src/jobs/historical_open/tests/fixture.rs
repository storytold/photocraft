//! Original one-pixel fixture shared with the existing forced-Open regression.
pub(crate) fn historical_rgba16() -> Vec<u8> {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn patch(out: &mut [u8], slot: usize, target: usize) {
        out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v010\0".to_vec();
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // no compression
    for value in [0, 0] {
        word(&mut out, value);
    }
    let layer = out.len();
    for value in [0, 0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, layer, at);
    for value in [1, 1, 1, 2] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"L\0");
    for value in [0, 0] {
        word(&mut out, value);
    }
    let hierarchy = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, hierarchy, at);
    for value in [1, 1, 8] {
        word(&mut out, value);
    }
    let level = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, level, at);
    for value in [1, 1] {
        word(&mut out, value);
    }
    let tile = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, tile, at);
    out.extend_from_slice(&[0x12, 0x34, 0x23, 0x45, 0x34, 0x56, 0xff, 0xff]);
    out
}
