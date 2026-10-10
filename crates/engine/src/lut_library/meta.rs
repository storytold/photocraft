//! What a LUT file says about itself, read from its first few kilobytes (no table parsing), for
//! labels and tooltips.

use std::fs;
use std::io::Read;
use std::path::Path;

/// The header is looked for in this many leading bytes.
const HEADER_BYTES: u64 = 16 << 10;

/// The kind of LUT file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cube3d,
    /// A 1D `.cube` (expanded to a 3D table when loaded).
    Cube1d,
    ThreeDl,
    Look,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub kind: Kind,
    pub title: String,
    /// Edge length of the table (0 when the header does not say).
    pub size: u32,
    /// The file declares a `DOMAIN_MIN` / `DOMAIN_MAX` other than 0..1.
    pub custom_domain: bool,
}

/// Read the header of the LUT file at `path`; `None` when it cannot be read.
pub fn read_header(path: &Path) -> Option<Header> {
    let mut buf = Vec::new();
    fs::File::open(path).ok()?.take(HEADER_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if ext == "look" {
        return Some(Header { kind: Kind::Look, title: String::new(), size: 0, custom_domain: false });
    }
    if ext == "3dl" {
        // The first line of numbers lists the shaper stops; their count is the table's edge.
        let size = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#') && l.split_whitespace().all(|w| w.parse::<f32>().is_ok()))
            .map_or(0, |l| l.split_whitespace().count() as u32);
        return Some(Header { kind: Kind::ThreeDl, title: String::new(), size, custom_domain: false });
    }
    let mut h = Header { kind: Kind::Cube3d, title: String::new(), size: 0, custom_domain: false };
    for line in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()) {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("TITLE") => h.title = line.trim_start_matches("TITLE").trim().trim_matches('"').to_string(),
            Some("LUT_3D_SIZE") => h.size = words.next().and_then(|w| w.parse().ok()).unwrap_or(0),
            Some("LUT_1D_SIZE") => {
                h.kind = Kind::Cube1d;
                h.size = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
            }
            Some(k @ ("DOMAIN_MIN" | "DOMAIN_MAX")) => {
                let v: Vec<f32> = words.filter_map(|w| w.parse().ok()).collect();
                let want = if k == "DOMAIN_MIN" { 0.0 } else { 1.0 };
                h.custom_domain |= v.iter().any(|x| (x - want).abs() > 1e-6);
            }
            Some(first) if first.parse::<f32>().is_ok() => break,
            _ => {}
        }
    }
    Some(h)
}

/// Whether the name suggests a technical conversion (log to display, colour space to colour
/// space) rather than a creative look. Only a hint, for the tooltip.
pub fn looks_like_conversion(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "log",
        "rec709",
        "rec.709",
        "rec2020",
        "rec.2020",
        "aces",
        "slog",
        "s-log",
        "clog",
        "c-log",
        "vlog",
        "v-log",
        "gamma",
        "_to_",
        " to ",
        "linear",
        "conversion",
    ]
    .iter()
    .any(|k| n.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("photocraft-lutmeta-{}-{name}", std::process::id()));
        fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn cube_headers_are_read_without_the_table() {
        let p = file("a.cube", "# made by hand\nTITLE \"Warm Fade\"\nLUT_3D_SIZE 33\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n");
        let h = read_header(&p).unwrap();
        assert_eq!((h.kind, h.title.as_str(), h.size, h.custom_domain), (Kind::Cube3d, "Warm Fade", 33, false));
        let p = file("b.cube", "LUT_1D_SIZE 1024\nDOMAIN_MAX 2 2 2\n0 0 0\n");
        let h = read_header(&p).unwrap();
        assert_eq!((h.kind, h.size, h.custom_domain), (Kind::Cube1d, 1024, true));
    }

    #[test]
    fn three_dl_size_is_the_shaper_length_and_unreadable_files_are_none() {
        let p = file("c.3dl", "0 64 128 192 256 320 384 448 512 576 640 704 768 832 896 960 1023\n0 0 0\n");
        assert_eq!(read_header(&p).unwrap().size, 17);
        assert!(read_header(&std::env::temp_dir().join("photocraft-lutmeta-missing.cube")).is_none());
    }

    #[test]
    fn conversion_names_are_hinted() {
        assert!(looks_like_conversion("LC3DL_Kodak2393_log2hd_ConstLclip"));
        assert!(looks_like_conversion("Rec709 to P3"));
        assert!(!looks_like_conversion("Caramel 3"));
    }
}
