//! Layers to Files: Include ICC Profile (#13973). `iccProfile: false` writes no profile in PNG,
//! JPEG, TIFF or PSD files; `true` embeds the document's profile (a CMYK document's JPEG its CMYK
//! profile, its PNG none); unset keeps what the export wrote before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn session(mode: &str, depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 6, "mode": mode, "depth": depth, "background": "backgroundColor", "backgroundColor": [0.8, 0.1, 0.1]}))
        .unwrap();
    s
}

/// The colour space of each written file's embedded profile (`None`: no profile), by extension.
fn profiles(s: &mut Session, format: &str, extra: Value) -> Vec<(String, Option<String>)> {
    let dir = tempfile::tempdir().unwrap();
    let mut p = json!({"dir": dir.path().to_string_lossy(), "prefix": "t", "format": format});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute("file.export.layersToFiles", p).unwrap();
    r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let path = f.as_str().unwrap();
            let bytes = std::fs::read(path).unwrap();
            let doc = photocraft_io::import(path, &bytes).unwrap().document;
            let space = doc.icc_profile.map(|icc| String::from_utf8_lossy(icc.get(16..20).unwrap()).trim().to_string());
            (format!("{:?}", doc.mode), space)
        })
        .collect()
}

const FORMATS: [&str; 4] = ["png", "jpg", "tif", "psd"];

/// RGB and CMYK documents at 8 and 16 bits.
const DOCS: [(&str, u32); 4] = [("rgb", 8), ("rgb", 16), ("cmyk", 8), ("cmyk", 16)];

fn one(s: &mut Session, format: &str, extra: Value) -> (String, Option<String>) {
    let files = profiles(s, format, extra);
    assert_eq!(files.len(), 1, "{format}: one layer, one file");
    files.into_iter().next().unwrap()
}

#[test]
fn off_writes_no_profile() {
    for (mode, depth) in DOCS {
        let mut s = session(mode, depth);
        for format in FORMATS {
            assert_eq!(one(&mut s, format, json!({"iccProfile": false})).1, None, "{mode}{depth} {format}");
        }
    }
}

#[test]
fn on_embeds_the_documents_profile() {
    for depth in [8, 16] {
        let mut s = session("rgb", depth);
        for format in FORMATS {
            assert_eq!(one(&mut s, format, json!({"iccProfile": true})), ("Rgb".into(), Some("RGB".into())), "rgb{depth} {format}");
        }
    }
    // The document's own profile, not a stand-in, when it has one.
    let mut s = session("rgb", 8);
    let own = crate::color_cmds::working_profile(ColorMode::Rgb).to_bytes();
    let dir = tempfile::tempdir().unwrap();
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir.path().to_string_lossy(), "format": "jpg", "iccProfile": true})).unwrap();
    let path = r["files"][0].as_str().unwrap();
    let embedded = photocraft_io::import(path, &std::fs::read(path).unwrap()).unwrap().document.icc_profile;
    assert_eq!(embedded.as_deref(), Some(&*own));
}

#[test]
fn a_cmyk_documents_jpeg_keeps_its_cmyk_profile_and_its_png_has_none() {
    for depth in [8, 16] {
        let mut s = session("cmyk", depth);
        let on = json!({"iccProfile": true});
        assert_eq!(one(&mut s, "jpg", on.clone()), ("Cmyk".into(), Some("CMYK".into())), "cmyk{depth} jpg");
        assert_eq!(one(&mut s, "tif", on.clone()), ("Cmyk".into(), Some("CMYK".into())), "cmyk{depth} tif");
        assert_eq!(one(&mut s, "psd", on.clone()), ("Cmyk".into(), Some("CMYK".into())), "cmyk{depth} psd");
        assert_eq!(one(&mut s, "png", on), ("Rgb".into(), None), "cmyk{depth} png");
    }
}

#[test]
fn unset_keeps_what_the_export_wrote_before() {
    // As main wrote them before the param: flat RGB files and a CMYK document's PNG carry the
    // RGB working profile; PSD files and CMYK JPEG and TIFF files carry none.
    let rgb = Some("RGB".to_string());
    for depth in [8, 16] {
        let mut s = session("rgb", depth);
        let got: Vec<_> = FORMATS.iter().map(|f| one(&mut s, f, json!({})).1).collect();
        assert_eq!(got, vec![rgb.clone(), rgb.clone(), rgb.clone(), None], "rgb{depth}");
        let mut s = session("cmyk", depth);
        let got: Vec<_> = FORMATS.iter().map(|f| one(&mut s, f, json!({})).1).collect();
        assert_eq!(got, vec![rgb.clone(), None, None, None], "cmyk{depth}");
        // `null` is unset too.
        assert_eq!(one(&mut s, "png", json!({"iccProfile": null})).1, rgb, "cmyk{depth} null");
    }
}

#[test]
fn a_bad_value_is_an_error_and_writes_nothing() {
    let mut s = session("rgb", 8);
    for bad in [json!("no"), json!(0), json!([false]), json!({})] {
        let dir = tempfile::tempdir().unwrap();
        let r = s.execute("file.export.layersToFiles", json!({"dir": dir.path().to_string_lossy(), "format": "png", "iccProfile": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "{bad}: nothing written");
    }
    assert_eq!(s.active().unwrap().doc.icc_profile, None, "the document is untouched");
}
