use super::*;

#[test]
fn unsupported_vector_encoding_does_not_duplicate_unbounded_source_text() {
    let encoding = "unsupported-☃".repeat(50_000);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0u32.to_be_bytes()); // Solid paint.
    bytes.extend_from_slice(&1u32.to_be_bytes()); // Antialias.
    bytes.extend_from_slice(&((encoding.len() + 1) as u32).to_be_bytes());
    bytes.extend_from_slice(encoding.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&4u32.to_be_bytes());
    bytes.extend_from_slice(&[1, 2, 3, 255]);
    bytes.extend_from_slice(&0u32.to_be_bytes()); // No profile.
    bytes.extend_from_slice(&0u32.to_be_bytes()); // No resource.
    let mut cursor = Cursor { b: &bytes, pos: 0, wide: false };
    let mut notes = Vec::new();
    paint(&mut cursor, &mut notes).unwrap();
    assert!(notes.iter().map(String::len).sum::<usize>() <= 512, "one source encoding must not become retained diagnostic payload");
}
