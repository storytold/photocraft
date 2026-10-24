//! Bundled palette catalog and bounded GPL/ACT importers.
//!
//! The bundled GPL files are byte-for-byte copies of GIMP's `data/palettes`
//! directory at commit `e101dd19b165f927d3ba0a74658a71537c5661b9` (GIMP 3.2.6).
//! GIMP's `LICENSE` is embedded below as [`GIMP_LICENSE`].  It says that new data
//! files are expected to be CC0, while the copyright status of historical data is
//! not clear; this catalog intentionally does not make a stronger licensing claim.

use std::str;
use std::sync::OnceLock;
#[path = "palettes_data.rs"]
mod palettes_data;

use super::palette_map::Palette;
use palettes_data::BUILTIN_DATA;

const MAX_INPUT_BYTES: usize = 1 << 20;
const MAX_COLORS: usize = 4096;
const MAX_NAME_BYTES: usize = 4096;
const MAX_COLUMNS: u16 = 255;
const ACT_TABLE_BYTES: usize = 256 * 3;
const ACT_EXTENDED_BYTES: usize = ACT_TABLE_BYTES + 4;

/// The exact upstream policy text shipped with the bundled palette collection.
pub const GIMP_LICENSE: &str = include_str!("../../../assets/palettes/gimp/LICENSE");

/// A built-in palette and its source metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinPalette {
    pub id: &'static str,
    pub source: &'static str,
    pub palette: Palette,
    pub columns: u16,
    pub color_names: Vec<String>,
}

fn error(line: usize, message: &str) -> String {
    format!("GPL line {line}: {message}")
}

fn next_field<'a>(line: &'a str, offset: &mut usize) -> Option<&'a str> {
    let bytes = line.as_bytes();
    while *offset < bytes.len() && bytes[*offset].is_ascii_whitespace() {
        *offset += 1;
    }
    let start = *offset;
    while *offset < bytes.len() && !bytes[*offset].is_ascii_whitespace() {
        *offset += 1;
    }
    (start != *offset).then(|| &line[start..*offset])
}

fn parse_rgb_line(line: &str, line_number: usize) -> Result<[f32; 3], String> {
    let mut offset = 0;
    let fields = [next_field(line, &mut offset), next_field(line, &mut offset), next_field(line, &mut offset)];
    let mut rgb = [0.0; 3];
    for (channel, field) in fields.into_iter().enumerate() {
        let field = field.ok_or_else(|| error(line_number, "palette row needs three channels"))?;
        let value = field.parse::<u16>().map_err(|_| error(line_number, "palette channel is not an integer"))?;
        if value > u16::from(u8::MAX) {
            return Err(error(line_number, "palette channel is outside 0..=255"));
        }
        rgb[channel] = f32::from(value) / 255.0;
    }
    let label = line[offset..].trim();
    if label.len() > MAX_NAME_BYTES {
        return Err(error(line_number, "palette color name is too long"));
    }
    Ok(rgb)
}

/// Parse a UTF-8 GIMP Palette (`.gpl`) file.
///
/// Comments and blank lines are accepted, while every other non-empty line must
/// be metadata or a complete RGB row.  Color order and duplicates are retained.
pub fn parse_gpl(bytes: &[u8]) -> Result<Palette, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("GPL input exceeds {MAX_INPUT_BYTES} bytes"));
    }
    let text = str::from_utf8(bytes).map_err(|_| "GPL input is not UTF-8".to_owned())?;
    if text.as_bytes().contains(&0) {
        return Err("GPL input contains a NUL byte".to_owned());
    }

    let mut lines = text.split('\n');
    let first = lines.next().ok_or_else(|| "GPL input is empty".to_owned())?;
    let first = first.strip_suffix('\r').unwrap_or(first);
    let first = first.strip_prefix('\u{feff}').unwrap_or(first);
    if first != "GIMP Palette" {
        return Err("GPL header must be exactly `GIMP Palette`".to_owned());
    }

    let mut name: Option<String> = None;
    let mut columns: Option<u16> = None;
    let mut colors = Vec::new();

    for (line_index, raw_line) in lines.enumerate() {
        let line_number = line_index + 2;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix("Name:") {
            if name.is_some() {
                return Err(error(line_number, "duplicate Name metadata"));
            }
            let value = value.trim();
            if value.len() > MAX_NAME_BYTES {
                return Err(error(line_number, "palette name is too long"));
            }
            name = Some(value.to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("Columns:") {
            if columns.is_some() {
                return Err(error(line_number, "duplicate Columns metadata"));
            }
            let value = value.trim();
            let parsed = value.parse::<u16>().map_err(|_| error(line_number, "Columns metadata is not an integer"))?;
            if parsed > MAX_COLUMNS {
                return Err(error(line_number, "Columns metadata is outside 0..=255"));
            }
            columns = Some(parsed);
            continue;
        }
        if colors.len() >= MAX_COLORS {
            return Err(error(line_number, "palette has more than 4096 colors"));
        }
        let rgb = parse_rgb_line(line, line_number)?;
        colors.push(rgb);
    }

    if colors.is_empty() {
        return Err("GPL palette has no color rows".to_owned());
    }
    let palette_name = name.unwrap_or_else(|| "Imported GPL".to_owned());
    Ok(Palette { name: palette_name, colors })
}

/// Parse an Adobe Color Table (`.act`) file.
///
/// The classic 768-byte form contains 256 RGB entries.  The extended 772-byte
/// form appends a big-endian entry count (zero means 256) and a big-endian
/// transparency index (`0xffff` means no transparency index).
pub fn parse_act(bytes: &[u8]) -> Result<Palette, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("ACT input exceeds {MAX_INPUT_BYTES} bytes"));
    }
    let count = match bytes.len() {
        ACT_TABLE_BYTES => 256usize,
        ACT_EXTENDED_BYTES => {
            let declared = u16::from_be_bytes([bytes[ACT_TABLE_BYTES], bytes[ACT_TABLE_BYTES + 1]]);
            let count = if declared == 0 { 256 } else { usize::from(declared) };
            if count == 0 || count > 256 {
                return Err("ACT color count is outside 1..=256".to_owned());
            }
            let transparent = u16::from_be_bytes([bytes[ACT_TABLE_BYTES + 2], bytes[ACT_TABLE_BYTES + 3]]);
            if transparent != u16::MAX && usize::from(transparent) >= count {
                return Err("ACT transparency index is outside the declared palette".to_owned());
            }
            count
        }
        actual => {
            return Err(format!("ACT input must be exactly {ACT_TABLE_BYTES} or {ACT_EXTENDED_BYTES} bytes (got {actual})"));
        }
    };

    let mut colors = Vec::with_capacity(count);
    for index in 0..count {
        let offset = index * 3;
        colors.push([f32::from(bytes[offset]) / 255.0, f32::from(bytes[offset + 1]) / 255.0, f32::from(bytes[offset + 2]) / 255.0]);
    }
    Ok(Palette { name: "Imported ACT".to_owned(), colors })
}

#[cfg(test)]
struct BuiltinSpec {
    bytes: &'static [u8],
}

#[cfg(test)]
macro_rules! builtin_spec {
    ($stem:literal) => {
        BuiltinSpec { bytes: include_bytes!(concat!("../../../assets/palettes/gimp/", $stem, ".gpl")) }
    };
}

#[cfg(test)]
const BUILTIN_SPECS: &[BuiltinSpec] = &[
    builtin_spec!("Bears"),
    builtin_spec!("Bgold"),
    builtin_spec!("Blues"),
    builtin_spec!("Borders"),
    builtin_spec!("Browns_And_Yellows"),
    builtin_spec!("Caramel"),
    builtin_spec!("Cascade"),
    builtin_spec!("China"),
    builtin_spec!("Coldfire"),
    builtin_spec!("Cool_Colors"),
    builtin_spec!("Cranes"),
    builtin_spec!("Dark_pastels"),
    builtin_spec!("Default"),
    builtin_spec!("Ega"),
    builtin_spec!("Firecode"),
    builtin_spec!("Gold"),
    builtin_spec!("GrayViolet"),
    builtin_spec!("Grayblue"),
    builtin_spec!("Grays"),
    builtin_spec!("Greens"),
    builtin_spec!("Hilite"),
    builtin_spec!("Khaki"),
    builtin_spec!("Lights"),
    builtin_spec!("Muted"),
    builtin_spec!("Named_Colors"),
    builtin_spec!("News3"),
    builtin_spec!("Op2"),
    builtin_spec!("Paintjet"),
    builtin_spec!("Pastels"),
    builtin_spec!("Plasma"),
    builtin_spec!("Reds"),
    builtin_spec!("Reds_And_Purples"),
    builtin_spec!("Royal"),
    builtin_spec!("Tango"),
    builtin_spec!("Topographic"),
    builtin_spec!("Visibone"),
    builtin_spec!("Visibone_2"),
    builtin_spec!("Volcano"),
    builtin_spec!("Warm_Colors"),
    builtin_spec!("Web"),
];
fn materialize_builtin(data: &palettes_data::StaticBuiltin) -> BuiltinPalette {
    BuiltinPalette {
        id: data.id,
        source: data.source,
        palette: Palette {
            name: data.name.to_owned(),
            colors: data.colors.iter().map(|rgb| [f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0]).collect(),
        },
        columns: data.columns,
        color_names: data.color_names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

/// Return all forty pinned GIMP palettes in source-directory order.
pub fn builtins() -> &'static [BuiltinPalette] {
    static BUILTINS: OnceLock<Vec<BuiltinPalette>> = OnceLock::new();
    BUILTINS.get_or_init(|| BUILTIN_DATA.iter().map(materialize_builtin).collect()).as_slice()
}

/// Find one pinned palette by ID, such as `gimp:Blues` or `gimp:Ega`.
pub fn builtin(id: &str) -> Option<&'static BuiltinPalette> {
    builtins().iter().find(|palette| palette.id == id)
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_SPECS, MAX_COLORS, MAX_INPUT_BYTES, builtin, builtins, parse_act, parse_gpl};
    use std::fmt::Write as _;

    #[test]
    fn builtins_include_all_pinned_files_and_large_named_palettes() {
        assert_eq!(builtins().len(), 40);
        assert_eq!(builtin("gimp:Named_Colors").map(|p| p.palette.colors.len()), Some(448));
        assert_eq!(builtin("gimp:Visibone").map(|p| p.palette.colors.len()), Some(336));
        assert_eq!(builtin("gimp:Blues").map(|p| p.palette.name.as_str()), Some("Blues"));
    }

    #[test]
    fn catalog_entries_are_valid_complete_and_ordered() {
        let catalog = builtins();
        assert_eq!(catalog.len(), 40);
        assert_eq!(catalog.first().map(|entry| entry.id), Some("gimp:Bears"));
        assert_eq!(catalog.last().map(|entry| entry.id), Some("gimp:Web"));
        assert!(catalog.iter().all(|entry| entry.palette.validate().is_ok()));
        assert!(catalog.iter().all(|entry| entry.color_names.len() == entry.palette.colors.len()));

        let ega = builtin("gimp:Ega");
        assert!(ega.is_some());
        if let Some(ega) = ega {
            assert_eq!(ega.columns, 16);
            assert_eq!(ega.palette.colors.len(), 240);
            assert_eq!(ega.color_names.first().map(String::as_str), Some("grey0"));
            assert_eq!(ega.palette.colors[0], [0.0, 0.0, 0.0]);
            assert_eq!(ega.palette.colors[1], [168.0 / 255.0, 0.0, 168.0 / 255.0]);
            assert_eq!(ega.palette.colors[0], ega.palette.colors[16]);
            assert_eq!(ega.palette.colors[1], ega.palette.colors[17]);
        }
    }

    #[test]
    fn pinned_raw_files_are_parseable() {
        assert_eq!(BUILTIN_SPECS.len(), 40);
        assert!(BUILTIN_SPECS.iter().all(|spec| parse_gpl(spec.bytes).is_ok()));
    }

    #[test]
    fn gpl_preserves_order_and_duplicates() {
        let input = b"GIMP Palette\nName: Keep me\nColumns: 2\n# comment\n1 2 3 first label\n1 2 3 second label\n4 5 6\n";
        let parsed = parse_gpl(input).unwrap();
        assert_eq!(parsed.name, "Keep me");
        let first = [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0];
        let second = [4.0 / 255.0, 5.0 / 255.0, 6.0 / 255.0];
        assert_eq!(parsed.colors, vec![first, first, second]);
        assert_eq!(parse_gpl(b"GIMP Palette\nColumns: 0\n1 2 3\n").unwrap().colors, vec![first]);
    }

    #[test]
    fn gpl_rejects_bad_rows_headers_metadata_and_encoding() {
        for input in [
            &b""[..],
            b"not a palette\n1 2 3\n",
            b"GIMP Palette\n1 2\n",
            b"GIMP Palette\n256 0 0\n",
            b"GIMP Palette\nName: x\nColumns: nope\n1 2 3\n",
            b"GIMP Palette\nName: x\nColumns: 256\n1 2 3\n",
            b"GIMP Palette\nunknown metadata\n1 2 3\n",
            b"GIMP Palette\n1 2 3\nGIMP Palette\n0 0 0\n",
        ] {
            assert!(parse_gpl(input).is_err());
        }
        assert!(parse_gpl(b"GIMP Palette\n1 2 3\n\xff").is_err());
    }

    #[test]
    fn gpl_accepts_the_maximum_palette_but_not_more() {
        let mut input = String::from("GIMP Palette\n");
        for index in 0..MAX_COLORS {
            let _ = writeln!(input, "{} 0 0", index % 256);
        }
        assert_eq!(parse_gpl(input.as_bytes()).map(|p| p.colors.len()), Ok(MAX_COLORS));
        let _ = writeln!(input, "0 0 0");
        assert!(parse_gpl(input.as_bytes()).is_err());
    }

    #[test]
    fn gpl_rejects_oversized_input_without_truncating() {
        let mut input = vec![b' '; MAX_INPUT_BYTES + 1];
        input[0..13].copy_from_slice(b"GIMP Palette\n");
        assert!(parse_gpl(&input).is_err());
    }

    #[test]
    fn act_requires_exact_bounded_sizes_and_preserves_duplicates() {
        let mut bytes = vec![0u8; 768];
        bytes[0..3].copy_from_slice(&[1, 2, 3]);
        bytes[3..6].copy_from_slice(&[1, 2, 3]);
        let parsed = parse_act(&bytes);
        assert_eq!(parsed.as_ref().map(|p| p.colors.len()), Ok(256));
        if let Ok(palette) = parsed {
            assert_eq!(palette.colors[0], palette.colors[1]);
            assert_eq!(palette.colors[0], [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0]);
        }

        let mut extended = vec![0u8; 772];
        extended[768..770].copy_from_slice(&2u16.to_be_bytes());
        extended[770..772].copy_from_slice(&u16::MAX.to_be_bytes());
        assert_eq!(parse_act(&extended).map(|p| p.colors.len()), Ok(2));
        for size in [0usize, 767, 769, 771, 773, 1 << 20] {
            assert!(parse_act(&vec![0; size]).is_err());
        }
        let mut bad_count = vec![0u8; 772];
        bad_count[768..770].copy_from_slice(&257u16.to_be_bytes());
        assert!(parse_act(&bad_count).is_err());
        let mut bad_transparency = vec![0u8; 772];
        bad_transparency[768..770].copy_from_slice(&2u16.to_be_bytes());
        bad_transparency[770..772].copy_from_slice(&2u16.to_be_bytes());
        assert!(parse_act(&bad_transparency).is_err());
    }
}
