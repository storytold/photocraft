//! #13967: Layers to Files flattens TIFF, BMP and Targa files on white (no transparent pixels;
//! Targa 32 bit alpha 255, BMP no alpha content); PNG and PSD keep transparency.

use super::*;

/// 20 x 15 RGB, top to bottom: `Blue Half` (30,30,200 at 50 % opacity over 4,4-16,11) and `Red`
/// (200,30,30 over 2,2-18,13, so its corners are transparent), on a white `Background`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 15, "background": "white"})).unwrap();
    for (name, rect, color) in [("Red", [2, 2, 16, 11], "#C81E1E"), ("Blue Half", [4, 4, 12, 7], "#1E1EC8")] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        s.execute("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3], "antiAlias": false})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s
}

/// The files of one export with `params`, by name.
fn export(s: &Session, params: Value) -> Vec<(String, Vec<u8>)> {
    let mut written = Vec::new();
    let mut p = json!({"prefix": "l2f"});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    export_layers_report_with(s, &p, |name, bytes| {
        written.push((name.to_string(), bytes.to_vec()));
        Ok(())
    })
    .unwrap();
    written
}

/// The decoded file as 8-bit RGBA rows (alpha 255 where the file has none).
fn rgba(bytes: &[u8]) -> (usize, Vec<[u8; 4]>, usize) {
    let img = photocraft_codecs::decode(bytes).unwrap();
    let n = img.layout().channels();
    let px = img.as_u8().unwrap();
    let pixels = px.chunks(n).map(|c| if n == 4 { [c[0], c[1], c[2], c[3]] } else { [c[0], c[1], c[2], 255] }).collect();
    (img.width() as usize, pixels, n)
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

#[test]
fn tiff_bmp_and_targa_are_flattened_on_white() {
    let s = session();
    for (format, extra) in [("tiff", json!({})), ("bmp", json!({})), ("tga", json!({"tgaBits": 32}))] {
        let mut params = json!({"format": format});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let files = export(&s, params);
        let (_, blue) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
        let (w, px, _) = rgba(blue);
        assert!(px.iter().all(|p| p[3] == 255), "{format}: a transparent pixel");
        assert!(near(px[7 * w + 8], [142, 142, 227, 255]), "{format}: {:?}", px[7 * w + 8]);
        assert_eq!(px[0], [255, 255, 255, 255], "{format}: white outside the layer");
        let (_, red) = files.iter().find(|(n, _)| n.contains("Red")).unwrap();
        let (w, px, _) = rgba(red);
        assert_eq!(px[0], [255, 255, 255, 255], "{format}: Red's transparent corner is white");
        assert!(near(px[5 * w + 5], [200, 30, 30, 255]), "{format}");
    }
}

#[test]
fn targa_32_has_alpha_255_and_bmp_no_alpha_content() {
    let s = session();
    let files = export(&s, json!({"format": "tga", "tgaBits": 32}));
    let (_, tga) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
    assert_eq!(tga[16], 32, "32 bits per pixel");
    let (_, px, n) = rgba(tga);
    assert_eq!(n, 4);
    assert!(px.iter().all(|p| p[3] == 255));
    let files = export(&s, json!({"format": "bmp"}));
    let (_, bmp) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
    assert_eq!(&bmp[..2], b"BM");
    let (_, px, _) = rgba(bmp);
    assert!(px.iter().all(|p| p[3] == 255), "no alpha content");
    // The TIFF is a single flat image without an alpha channel.
    let files = export(&s, json!({"format": "tiff"}));
    let (_, tif) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
    assert_eq!(rgba(tif).2, 3, "RGB only");
}

#[test]
fn png_and_psd_keep_their_transparency() {
    let s = session();
    let files = export(&s, json!({"format": "png"}));
    let (_, png) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
    let (w, px, _) = rgba(png);
    assert_eq!(px[0][3], 0, "transparent outside the layer");
    assert!(px[7 * w + 8][3].abs_diff(128) <= 1, "{:?}", px[7 * w + 8]);
    let files = export(&s, json!({"format": "psd"}));
    let (name, psd) = files.iter().find(|(n, _)| n.contains("Blue Half")).unwrap();
    let doc = photocraft_io::import(name, psd).unwrap().document;
    let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Blue Half"], "no white layer added");
    assert!((doc.layers[0].opacity - 0.5).abs() < 0.01);
}
