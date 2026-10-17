//! Font files for tests, made at run time from the bundled Inter, so no font file is committed.

/// Inter Regular (OFL, `assets/fonts`).
const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// Inter Regular renamed to `family` (style "Regular", PostScript name `<family>-Regular` without
/// spaces). The text engine's font database is one per process, so each test that registers a
/// font gives it a family no other test uses. `None` only if the bundled font has no `name` table.
pub fn renamed_font(family: &str) -> Option<Vec<u8>> {
    let mut bytes = INTER_REGULAR.to_vec();
    let tables = usize::from(u16::from_be_bytes(bytes.get(4..6)?.try_into().ok()?));
    let record = (0..tables).map(|i| 12 + i * 16).find(|r| bytes.get(*r..r + 4).is_some_and(|tag| tag == b"name"))?;
    let full = format!("{family} Regular");
    let postscript = format!("{}-Regular", family.replace(' ', ""));
    // Windows-platform English records, sorted by name id as the format requires.
    let names = [(1u16, family), (2, "Regular"), (4, full.as_str()), (6, postscript.as_str()), (16, family), (17, "Regular")];
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for v in [0, names.len(), 6 + names.len() * 12] {
        table.extend(u16::try_from(v).ok()?.to_be_bytes());
    }
    for (id, value) in names {
        let encoded: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, 0x409, id, u16::try_from(encoded.len()).ok()?, u16::try_from(strings.len()).ok()?] {
            table.extend(v.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    // The new table goes at the end, 4-byte aligned; its record points there.
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let offset = u32::try_from(bytes.len()).ok()?;
    let length = u32::try_from(table.len()).ok()?;
    bytes.get_mut(record + 8..record + 12)?.copy_from_slice(&offset.to_be_bytes());
    bytes.get_mut(record + 12..record + 16)?.copy_from_slice(&length.to_be_bytes());
    bytes.extend(table);
    Some(bytes)
}
