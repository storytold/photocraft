//! Macintosh PICT pictures open through the shared codec: indexed pixel images in Indexed Color
//! with their colour table, direct 32-bit ones as RGB, and pictures without a pixel image are
//! refused with an error. The pictures are built here from the published QuickDraw opcodes.

use photocraft_color::{ColorMode, SampleType};
use photocraft_io::import;

fn be(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn rect(v: &mut Vec<u8>, w: u16, h: u16) {
    for x in [0, 0, h, w] {
        be(v, x);
    }
}

/// An extended version 2 picture header at 72 ppi, then `ops`, then opEndPic.
fn picture(w: u16, h: u16, ops: &[u8]) -> Vec<u8> {
    let mut v = vec![0, 0];
    rect(&mut v, w, h);
    v.extend_from_slice(&[0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00, 0xFF, 0xFE, 0, 0, 0, 72, 0, 0, 0, 72, 0, 0]);
    rect(&mut v, w, h);
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(ops);
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v.extend_from_slice(&[0x00, 0xFF]);
    v
}

/// A pixel map's fields after its bounds: pack type, 72 ppi, pixel type, size and components.
fn pixmap_fields(v: &mut Vec<u8>, pack: u16, pixel_type: u16, size: u16, cmps: u16) {
    for x in [0, pack, 0, 0, 72, 0, 72, 0, pixel_type, size, cmps, size.min(8), 0, 0, 0, 0, 0, 0] {
        be(v, x);
    }
}

/// A BitsRect of an 8-bit indexed pixel map with `palette`, stored rows of `indices`.
fn indexed_pict(w: u16, h: u16, palette: &[[u8; 3]], indices: &[u8]) -> Vec<u8> {
    let row_bytes = (w + 1) & !1;
    let mut op = vec![0x00, 0x90];
    be(&mut op, 0x8000 | row_bytes);
    rect(&mut op, w, h);
    pixmap_fields(&mut op, 0, 0, 8, 1);
    op.extend_from_slice(&[0; 6]); // ctSeed, ctFlags
    be(&mut op, palette.len() as u16 - 1);
    for (i, c) in palette.iter().enumerate() {
        be(&mut op, i as u16);
        for x in c {
            be(&mut op, u16::from(*x) * 257);
        }
    }
    rect(&mut op, w, h);
    rect(&mut op, w, h);
    be(&mut op, 0);
    for row in indices.chunks(w as usize) {
        op.extend_from_slice(row);
        op.resize(op.len() + (row_bytes - w) as usize, 0);
    }
    picture(w, h, &op)
}

/// A DirectBitsRect of 32-bit pixels (alpha, red, green, blue), stored rows, with `cmps`
/// components, after a line (a drawing operation PhotoCraft skips) when `line`.
fn direct_pict(w: u16, h: u16, argb: &[[u8; 4]], cmps: u16, line: bool) -> Vec<u8> {
    let mut op = Vec::new();
    if line {
        op.extend_from_slice(&[0x00, 0x20, 0, 0, 0, 0, 0, 1, 0, 1]);
    }
    op.extend_from_slice(&[0x00, 0x9A, 0, 0, 0, 0xFF]);
    be(&mut op, 0x8000 | (w * 4));
    rect(&mut op, w, h);
    pixmap_fields(&mut op, 1, 16, 32, cmps);
    rect(&mut op, w, h);
    rect(&mut op, w, h);
    be(&mut op, 0);
    argb.iter().for_each(|p| op.extend_from_slice(p));
    picture(w, h, &op)
}

fn close(a: f32, b: u8) -> bool {
    (a - f32::from(b) / 255.0).abs() < 1e-6
}

#[test]
fn an_indexed_pict_opens_in_indexed_color_with_its_table() {
    let palette = [[10, 20, 30], [200, 100, 50], [0, 255, 0]];
    let indices = [0, 1, 2, 1, 0, 2];
    let r = import("scan.pct", &indexed_pict(3, 2, &palette, &indices)).unwrap();
    let doc = r.document;
    assert_eq!((doc.size.width, doc.size.height), (3, 2));
    assert_eq!((doc.mode, doc.depth), (ColorMode::Indexed, SampleType::U8));
    assert_eq!(doc.color_table.as_ref().unwrap().colors, palette);
    let s = doc.layers[0].surface().unwrap();
    for (i, &index) in indices.iter().enumerate() {
        let px = s.rgba(i as i32 % 3, i as i32 / 3);
        let [r, g, b] = palette[usize::from(index)];
        assert!(close(px[0], r) && close(px[1], g) && close(px[2], b) && px[3] == 1.0, "pixel {i}: {px:?}");
    }
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_32_bit_pict_opens_as_8_bit_rgb_and_says_what_it_skipped() {
    let px: Vec<[u8; 4]> = (0..8u8).map(|i| [0, i * 30, 255 - i * 20, i * 7]).collect();
    let r = import("photo.pict", &direct_pict(4, 2, &px, 3, true)).unwrap();
    let doc = &r.document;
    assert_eq!((doc.size.width, doc.size.height, doc.mode, doc.depth), (4, 2, ColorMode::Rgb, SampleType::U8));
    let s = doc.layers[0].surface().unwrap();
    for (i, p) in px.iter().enumerate() {
        let got = s.rgba(i as i32 % 4, i as i32 / 4);
        assert!(close(got[0], p[1]) && close(got[1], p[2]) && close(got[2], p[3]) && got[3] == 1.0, "pixel {i}: {got:?}");
    }
    assert!(r.warnings.iter().any(|w| w.contains("1 PICT drawing operation")), "{:?}", r.warnings);
}

#[test]
fn a_32_bit_pict_with_an_alpha_plane_opens_with_transparency() {
    let px: Vec<[u8; 4]> = (0..4u8).map(|i| [i * 80, 200, 100, 50]).collect();
    let doc = import("alpha.pct", &direct_pict(2, 2, &px, 4, false)).unwrap().document;
    assert_eq!((doc.mode, doc.depth), (ColorMode::Rgb, SampleType::U8));
    let s = doc.layers[0].surface().unwrap();
    for (i, p) in px.iter().enumerate() {
        assert!(close(s.rgba(i as i32 % 2, i as i32 / 2)[3], p[0]), "alpha {i}");
    }
}

#[test]
fn a_pict_without_a_pixel_image_is_refused() {
    let vector = picture(10, 10, &[0x00, 0x20, 0, 0, 0, 0, 0, 5, 0, 5, 0x00, 0x31, 0, 0, 0, 0, 0, 2, 0, 2]);
    let e = import("drawing.pct", &vector).unwrap_err().to_string();
    assert!(e.contains("QuickDraw drawing"), "{e}");
    let mut damaged = indexed_pict(3, 2, &[[0, 0, 0], [255, 255, 255]], &[0, 1, 0, 1, 0, 1]);
    damaged.truncate(damaged.len() - 5);
    assert!(import("damaged.pct", &damaged).is_err());
}
