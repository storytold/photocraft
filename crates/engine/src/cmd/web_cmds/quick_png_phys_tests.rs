use serde_json::json;

use crate::Session;

fn session(background: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 12, "height": 8, "resolution": 300, "background": background})).unwrap();
    s
}

fn png_chunks(bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut remaining = bytes.strip_prefix(b"\x89PNG\r\n\x1a\n").expect("PNG signature");
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let length = u32::from_be_bytes(remaining[..4].try_into().unwrap()) as usize;
        let name = remaining[4..8].try_into().unwrap();
        chunks.push((name, &remaining[8..8 + length]));
        remaining = &remaining[12 + length..];
    }
    chunks
}

fn assert_300_ppi(chunks: &[([u8; 4], &[u8])]) {
    let mut phys = chunks.iter().filter(|(name, _)| name == b"pHYs").map(|(_, data)| *data);
    let payload = phys.next().expect("one pHYs chunk");
    assert_eq!(payload.len(), 9);
    assert_eq!(payload[..4], 11_811_u32.to_be_bytes());
    assert_eq!(payload[4..8], 11_811_u32.to_be_bytes());
    assert_eq!(payload[8], 1, "pixels per metre");
    assert_eq!(phys.next(), None, "only one pHYs chunk");
}

#[test]
fn quick_export_png_preserves_resolution_for_every_metadata_choice() {
    let dir = tempfile::tempdir().unwrap();
    for (background, tagged) in [("white", false), ("transparent", true)] {
        let mut s = session(background);
        if tagged {
            s.execute("file.fileInfo", json!({"copyright": "Example Copyright", "description": "Private description"})).unwrap();
        }
        for choice in ["none", "copyright", "all"] {
            let path = dir.path().join(format!("{background}-{choice}.png"));
            s.execute("file.export.exportPreferences", json!({"quickExportFormat": "png", "metadata": choice})).unwrap();
            s.execute("file.export.quickExport", json!({"path": path})).unwrap();
            let bytes = std::fs::read(path).unwrap();
            let chunks = png_chunks(&bytes);
            assert_300_ppi(&chunks);

            if choice == "none" || !tagged {
                assert!(!chunks.iter().any(|(name, _)| matches!(name, b"iTXt" | b"zTXt" | b"tEXt" | b"eXIf")), "{background}, {choice}");
            } else {
                let xmp = photocraft_codecs::decode(&bytes).unwrap().meta.xmp.expect("retained XMP");
                assert!(xmp.contains("Example Copyright"), "{choice}");
            }
        }
    }
}

#[test]
fn save_for_web_keeps_its_png_resolution_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session("white");
    s.execute("file.fileInfo", json!({"copyright": "Example Copyright"})).unwrap();
    for (format, choice, has_resolution) in [("png24", "none", false), ("png24", "all", true), ("png8", "all", false)] {
        let path = dir.path().join(format!("{format}-{choice}.png"));
        s.execute("file.export.saveForWebLegacy", json!({"format": format, "metadata": choice, "path": path})).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let chunks = png_chunks(&bytes);
        if has_resolution {
            assert_300_ppi(&chunks);
        } else {
            assert!(!chunks.iter().any(|(name, _)| name == b"pHYs"), "{format}, {choice}");
        }
    }
}
