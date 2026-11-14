//! #13703: Contact Sheet II converts the thumbnails into the sheet's mode through
//! `photocraft-cms`, as Image › Mode does, not by per-pixel luma.

use super::tests::tmp;
use super::*;

const COLOURS: [(&str, [u8; 3]); 5] = [("red", [255, 0, 0]), ("green", [0, 255, 0]), ("blue", [0, 0, 255]), ("black", [0, 0, 0]), ("grey", [128, 128, 128])];

/// One flat 24×24 PNG per entry of [`COLOURS`] in a fresh folder.
fn colour_files(name: &str) -> String {
    let dir = tmp(name);
    for (stem, [r, g, b]) in COLOURS {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 24, "height": 24, "background": format!("#{r:02x}{g:02x}{b:02x}")})).unwrap();
        crate::file_cmds::save_doc(&s.active().unwrap().doc, &format!("{dir}/{stem}.png"), None).unwrap();
    }
    dir
}

/// Runs Contact Sheet II on `dir` and returns each thumbnail layer's centre pixel by name.
fn sheet_centres(s: &mut Session, dir: &str, mode: &str, depth: u32) -> Vec<(String, Vec<f32>)> {
    let p = json!({"input": dir, "units": "pixels", "width": 500, "height": 120, "resolution": 72, "columns": 5, "rows": 1, "caption": false, "mode": mode, "depth": depth});
    s.execute("file.automate.contactSheetII", p).unwrap();
    let d = &s.active().unwrap().doc;
    d.layers
        .iter()
        .filter_map(|l| {
            let surf = l.surface().filter(|_| !d.is_background(l.id))?;
            let b = surf.content_bounds();
            Some((l.name.clone(), surf.pixel((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2)))
        })
        .collect()
}

fn centre_of<'a>(centres: &'a [(String, Vec<f32>)], name: &str) -> &'a [f32] {
    &centres.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name} thumbnail in {centres:?}")).1
}

#[test]
fn a_grayscale_16_bit_sheet_gets_the_reference_grey_values() {
    let dir = colour_files("gray-reference");
    let mut s = Session::new();
    // The reference run's working grey, which its sheet carries.
    s.execute("edit.colorSettings", json!({"workingGray": "gray-gamma-2.2"})).unwrap();
    let centres = sheet_centres(&mut s, &dir, "gray", 16);
    for (name, k) in [("red", 49.5), ("green", 14.0), ("blue", 72.0), ("black", 100.0), ("grey", 50.2)] {
        let got = (1.0 - centre_of(&centres, name)[0]) * 100.0;
        assert!((got - k).abs() <= 1.0, "{name}: K {got:.1} %, reference {k} %");
    }
}

/// A 16-bit `mode` sheet holds what `image.mode.bits16` and then `command` make of each file.
fn assert_sheet_matches_image_mode(name: &str, mode: &str, command: &str) {
    let dir = colour_files(name);
    let mut s = Session::new();
    let centres = sheet_centres(&mut s, &dir, mode, 16);
    for (name, _) in COLOURS {
        let path = format!("{dir}/{name}.png");
        let mut t = Session::new();
        t.add_document(import(&file_name(&path), &read_file(&path).unwrap()).unwrap(), None);
        t.execute("image.mode.bits16", json!({})).unwrap();
        t.execute(command, json!({})).unwrap();
        let want = t.active().unwrap().doc.layers[0].surface().unwrap().pixel(12, 12);
        let got = centre_of(&centres, name);
        let n = got.len() - 1;
        assert!(got[..n].iter().zip(&want[..n]).all(|(g, w)| (g - w).abs() <= 0.005), "{mode} {name}: sheet {got:?}, {command} {want:?}");
        assert_eq!(got[n], 1.0, "{mode} {name}: opaque");
    }
}

#[test]
fn a_grayscale_sheet_matches_image_mode_grayscale() {
    assert_sheet_matches_image_mode("mode-gray", "gray", "image.mode.grayscale");
}

#[test]
fn a_cmyk_sheet_matches_image_mode_cmyk() {
    assert_sheet_matches_image_mode("mode-cmyk", "cmyk", "image.mode.cmyk");
}

#[test]
fn a_lab_sheet_matches_image_mode_lab() {
    assert_sheet_matches_image_mode("mode-lab", "lab", "image.mode.lab");
}

#[test]
fn an_rgb_sheet_keeps_the_file_colours() {
    let dir = colour_files("rgb-same");
    let mut s = Session::new();
    let centres = sheet_centres(&mut s, &dir, "rgb", 8);
    for (name, rgb) in COLOURS {
        let [r, g, b] = rgb.map(|v| f32::from(v) / 255.0);
        assert_eq!(centre_of(&centres, name), [r, g, b, 1.0], "{name}");
    }
}

#[test]
fn an_rgb_sheet_keeps_srgb_file_colours_whatever_the_working_rgb() {
    // The sheet is untagged, so it is read as sRGB; converting into Working RGB would shift it.
    let dir = colour_files("rgb-working");
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"workingRgb": "adobe-rgb-compat"})).unwrap();
    let centres = sheet_centres(&mut s, &dir, "rgb", 8);
    for (name, rgb) in COLOURS {
        let [r, g, b] = rgb.map(|v| f32::from(v) / 255.0);
        assert_eq!(centre_of(&centres, name), [r, g, b, 1.0], "{name}");
    }
}

#[test]
fn an_untagged_grey_file_is_taken_in_the_working_grey() {
    // As File › Open assigns Working Gray to an untagged file, a sheet in that profile copies it.
    let dir = tmp("gray-untagged");
    // A binary PGM: grey 102 with no room for a profile.
    let path = format!("{dir}/grey.pgm");
    std::fs::write(&path, [b"P5 24 24 255\n".as_slice(), &[102u8; 24 * 24]].concat()).unwrap();
    let file = import(&file_name(&path), &read_file(&path).unwrap()).unwrap();
    assert!(file.icc_profile.is_none(), "the test file must be untagged");
    let want = file.layers[0].surface().unwrap().pixel(12, 12)[0];
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"workingGray": "gray-gamma-2.2"})).unwrap();
    let centres = sheet_centres(&mut s, &dir, "gray", 16);
    let got = centre_of(&centres, "grey")[0];
    assert!((got - want).abs() <= 0.5 / 255.0, "sheet {got}, file {want}");
}
