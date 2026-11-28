//! Layers to Files: Include ICC Profile (#13973). `iccProfile: false` writes no profile in PNG,
//! JPEG, TIFF or PSD files; `true` embeds the document's profile (a CMYK document's JPEG its CMYK
//! profile, its PNG none); unset keeps what the export wrote before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn session(mode: &str, depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 6, "mode": mode, "depth": depth, "background": "backgroundColor", "backgroundColor": [0.8, 0.1, 0.1]}))
        .unwrap();
    // One normal layer: Layers to Files refuses a Background-only document (#13968).
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
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

/// The pixels of each written file, decoded (RGBA, 0..1), and whether it carries a profile.
fn pixels(s: &mut Session, format: &str, extra: Value) -> Vec<(Vec<[f32; 4]>, bool)> {
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
            let doc = photocraft_io::import(path, &std::fs::read(path).unwrap()).unwrap().document;
            let icc = doc.icc_profile.is_some();
            (photocraft_compose::flatten(&doc).px.to_vec(), icc)
        })
        .collect()
}

#[test]
fn off_keeps_a_cmyk_documents_own_profile_for_its_colours() {
    // A CMYK document tagged with a profile other than the built-in coated one, whose layer
    // needs compositing (50 % opacity): its colours are converted through its own profile
    // whether or not the file carries it (CodeRabbit on #16653).
    for depth in [8, 16] {
        let mut s = session("cmyk", depth);
        s.execute("edit.assignProfile", json!({"profile": "coated-fogra39"})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
        let on = pixels(&mut s, "png", json!({"iccProfile": true}));
        let off = pixels(&mut s, "png", json!({"iccProfile": false}));
        let unset = pixels(&mut s, "png", json!({}));
        assert_eq!(off[0].0, on[0].0, "cmyk{depth}: the same pixels with iccProfile false and true");
        assert_eq!(off[0].0, unset[0].0, "cmyk{depth}: the same pixels with iccProfile false and unset");
        for format in FORMATS {
            assert!(pixels(&mut s, format, json!({"iccProfile": false})).iter().all(|(_, icc)| !icc), "cmyk{depth} {format}: no profile");
        }
        let layered = pixels(&mut s, "tif", json!({"iccProfile": false, "tiffLayers": true}));
        assert!(layered.iter().all(|(_, icc)| !icc), "cmyk{depth} layered tif: no profile");
        assert!(s.active().unwrap().doc.icc_profile.is_some(), "the document keeps its profile");
    }
}

/// An Indexed Color document with an RGB profile: `iccProfile: false` writes no profile in any file,
/// the PSD included, which is written over its colour table (CodeRabbit on #16653), and the
/// layered TIFF; `true` keeps it in the PSD.
#[test]
fn an_indexed_documents_files_follow_the_setting() {
    let mut s = session("rgb", 8);
    s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    // The conversion flattens: a normal layer again (#13968).
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.mode, ColorMode::Indexed);
    assert!(doc.icc_profile.is_some(), "the document keeps its profile");
    assert!(doc.layers.iter().any(|l| !doc.is_background(l.id)), "a layer to export");
    for format in ["png", "jpg", "tif", "psd", "bmp"] {
        assert!(profiles(&mut s, format, json!({"iccProfile": false})).iter().all(|(_, icc)| icc.is_none()), "indexed {format}: no profile");
    }
    let layered = profiles(&mut s, "tif", json!({"iccProfile": false, "tiffLayers": true}));
    assert!(layered.iter().all(|(_, icc)| icc.is_none()), "indexed layered tif: no profile");
    assert_eq!(one(&mut s, "psd", json!({"iccProfile": true})).1, Some("RGB".into()), "indexed psd: the profile");
}
