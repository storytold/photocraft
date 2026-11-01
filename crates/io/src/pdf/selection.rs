//! An embedded, empty Type 3 font for the selection plane. Visible glyphs remain paths.
//! No source font programs are copied into this font; each code has a Unicode mapping.
use std::collections::BTreeSet;
use std::fmt::Write as _;

#[derive(Default)]
pub(super) struct SelectionFont {
    chars: BTreeSet<char>,
}

impl SelectionFont {
    pub(super) fn new<'a>(texts: impl Iterator<Item = &'a str>) -> Self {
        Self { chars: texts.flat_map(str::chars).take(65536).collect() }
    }

    pub(super) fn encode(&self, text: &str) -> Option<String> {
        if self.chars.len() > 255 {
            return None;
        }
        let mut encoded = String::new();
        for ch in text.chars() {
            let code = self.chars.iter().position(|c| *c == ch)?.checked_add(1)?;
            write!(encoded, "{code:02X}").ok()?;
        }
        Some(encoded)
    }

    // Object numbers are assigned by the caller, after the optional ICC object.
    pub(super) fn objects(&self, base: usize) -> Vec<Vec<u8>> {
        let count = self.chars.len().min(255);
        let mut differences = String::new();
        let mut procs = String::new();
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /PhotoCraftUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
        );
        // PDF permits at most 100 entries in each bfchar block.
        let chars: Vec<_> = self.chars.iter().take(count).collect();
        for (block, chunk) in chars.chunks(100).enumerate() {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for (i, ch) in chunk.iter().enumerate() {
                let code = block * 100 + i + 1;
                let mut buf = [0; 2];
                let unicode = ch.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect::<String>();
                let _ = writeln!(cmap, "<{code:02X}> <{unicode}>");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        for code in 1..=count {
            let _ = write!(differences, " /c{code}");
            let _ = write!(procs, " /c{code} {} 0 R", base + 1);
        }
        let widths = "1000 ".repeat(count.max(1));
        let font = format!(
            "<< /Type /Font /Subtype /Type3 /Name /FSelect /FontBBox [0 -250 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs <<{procs} >> /Encoding << /Type /Encoding /Differences [1{differences}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {} 0 R >>",
            count.max(1),
            base + 2
        );
        let stream = |data: &str| format!("<< /Length {} >>\nstream\n{data}endstream", data.len()).into_bytes();
        vec![font.into_bytes(), stream("1000 0 0 -250 1000 1000 d1\n"), stream(&cmap)]
    }
}
