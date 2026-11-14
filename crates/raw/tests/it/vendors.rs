//! Synthetic CR2 and TIFF/EP (NEF / ARW-like) files.

use photocraft_raw::testgen::{Cr2Spec, Val, mosaic, scene, tiff_ep};
use photocraft_raw::*;

fn cr2_spec(w: usize, h: usize, comps: usize, slices: Vec<usize>) -> Cr2Spec {
    // Left / top borders of 16 / 4 masked pixels at black level 512.
    let (l, t) = (16, 4);
    let rgb = scene(w - l, h - t);
    let img = mosaic(&rgb, w - l, [0, 1, 1, 2], 512, 13000);
    let mut data = vec![512u16; w * h];
    for y in t..h {
        for x in l..w {
            data[y * w + x] = img[(y - t) * (w - l) + (x - l)];
        }
    }
    Cr2Spec {
        width: w,
        height: h,
        data,
        precision: 14,
        components: comps,
        slices,
        borders: Some([l as u16, t as u16, w as u16 - 1, h as u16 - 1]),
        wb_rggb: Some([2048, 1024, 1024, 1536]),
        orientation: 1,
        model_id: None,
    }
}

#[test]
fn cr2_slices_decode_exactly() {
    for (comps, slices) in [(2, vec![32, 32, 24]), (4, vec![40, 48]), (2, vec![]), (4, vec![88])] {
        let spec = cr2_spec(88, 30, comps, slices.clone());
        let b = spec.build();
        assert_eq!(identify(&b), Some(RawFormat::Cr2));
        let s = decode(&b, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height), (88, 30), "{comps} {slices:?}");
        assert_eq!(s.data, spec.data, "{comps} components, slices {slices:?}");
        assert_eq!(s.active, Rect::new(16, 4, 72, 26));
        assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
        assert!(s.black.values.iter().all(|&v| (v - 512.0).abs() < 1e-3), "{:?}", s.black.values);
        assert_eq!(s.cfa.as_ref().unwrap().phase(16, 4), [0, 1, 1, 2]);
    }
}

#[test]
fn cr2_develops() {
    let b = cr2_spec(88, 30, 2, vec![32, 32, 24]).build();
    let d = develop(&b, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width, d.height), (72, 26));
    assert_eq!(d.info.format, RawFormat::Cr2);
    assert_eq!(d.info.wb_multipliers, [2.0, 1.0, 1.5]);
    assert!(d.warnings.iter().any(|w| w.contains("sRGB primaries")));
}

#[test]
fn cr2_srgb_raw_is_unsupported() {
    // A subsampled frame in a layout Canon sRAW does not use (two components, Y 2×1): patch the
    // first component's sampling factors. Real sRAW / mRAW decodes (see `sraw.rs`).
    let mut b = cr2_spec(32, 8, 2, vec![]).build();
    let sof = b.windows(2).position(|w| w == [0xFF, 0xC3]).unwrap();
    b[sof + 11] = 0x21;
    match decode(&b, &Limits::default()) {
        Err(RawError::Unsupported(m)) => assert!(m.contains("subsampled"), "{m}"),
        other => panic!("expected unsupported sRAW, got {other:?}"),
    }
}

#[test]
fn nef_like_with_maker_tags() {
    let (w, h) = (24, 16);
    let data = mosaic(&scene(w, h), w, [2, 1, 1, 0], 0, 4095);
    let b = tiff_ep("NIKON CORPORATION", w, h, &data, [2, 1, 1, 0], 12, vec![]);
    assert_eq!(identify(&b), Some(RawFormat::Nef));
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.data, data);
    assert_eq!(s.cfa.as_ref().unwrap().phase(0, 0), [2, 1, 1, 0]);
    assert!(develop_sensor(&s, &DevelopOptions::default()).is_ok());
}

#[test]
fn arw_like_reads_sony_levels_and_crop() {
    let (w, h) = (24, 16);
    let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 512, 16383);
    let extra = vec![
        (0x7310, Val::Short(vec![512, 512, 512, 512])),
        (0x7313, Val::Short(vec![2048, 1024, 1024, 1536])),
        (50717, Val::Short(vec![16383])),
        (50719, Val::Long(vec![2, 2])),
        (50720, Val::Long(vec![20, 12])),
    ];
    let b = tiff_ep("SONY", w, h, &data, [0, 1, 1, 2], 14, extra);
    assert_eq!(identify(&b), Some(RawFormat::Arw));
    let s = decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.black.values, vec![512.0; 4]);
    assert_eq!(s.white, [16383.0; 3]);
    assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
    assert_eq!(s.crop, Rect::new(2, 2, 20, 12));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width, d.height), (20, 12));
}

#[test]
fn other_raw_containers_are_recognised_but_unsupported() {
    let mut cr3 = vec![0, 0, 0, 24];
    cr3.extend_from_slice(b"ftypcrx ");
    cr3.extend_from_slice(&[0; 12]);
    assert_eq!(identify(&cr3), Some(RawFormat::Cr3));
    assert!(matches!(decode(&cr3, &Limits::default()), Err(RawError::Unsupported(_))));
    // A bare RAF header has no CFA TIFF (like early FinePix files): recognised, unsupported.
    let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
    raf.resize(200, 0);
    assert_eq!(identify(&raf), Some(RawFormat::Raf));
    assert!(decode(&raf, &Limits::default()).is_err());
    assert_eq!(decode(b"not a raw file", &Limits::default()).unwrap_err(), RawError::NotRaw);
}

#[test]
fn embedded_preview_is_found() {
    // A RAF header pointing at a minimal baseline JPEG (SOI, SOF0, EOI).
    let jpeg = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x20, 0x00, 0x30, 1, 1, 0x11, 0, 0xFF, 0xD9];
    let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
    raf.resize(100, 0);
    raf[84..88].copy_from_slice(&100u32.to_be_bytes());
    raf[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
    raf.extend_from_slice(&jpeg);
    assert_eq!(embedded_preview(&raf).unwrap(), jpeg);
    // The lossless-JPEG raw data of a CR2 is not mistaken for a preview.
    assert!(embedded_preview(&cr2_spec(32, 8, 2, vec![]).build()).is_none());
}
