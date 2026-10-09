//! Paint.NET documents (.pdn): native import and export rejection tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_io::{ExportOptions, export, import};
use photocraft_pdn::synth::{SynthLayer, build_pdn};

#[test]
fn test_pdn_import_synthetic() {
    let p0 = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let p1 = vec![0, 0, 0, 128, 50, 50, 50, 128, 100, 100, 100, 128, 150, 150, 150, 128];
    let layers = vec![
        SynthLayer {
            name: "Background".into(),
            visible: true,
            is_background: true,
            opacity: 255,
            blend_mode: photocraft_pdn::BlendMode::Normal,
            rgba_pixels: p0.clone(),
        },
        SynthLayer {
            name: "Layer 2".into(),
            visible: true,
            is_background: false,
            opacity: 180,
            blend_mode: photocraft_pdn::BlendMode::Additive,
            rgba_pixels: p1.clone(),
        },
    ];

    let bytes = build_pdn(2, 2, &layers, true);
    let r = import("test.pdn", &bytes).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (2, 2));
    assert_eq!(r.document.layers.len(), 2);
    assert!(r.source_read_only);
    assert!(!r.preview_only);

    let l0 = &r.document.layers[0];
    assert_eq!(l0.name, "Background");
    assert!(l0.visible);
    assert!(l0.locks.transparency);
    assert_eq!(l0.blend, BlendMode::Normal);

    let l1 = &r.document.layers[1];
    assert_eq!(l1.name, "Layer 2");
    assert!(l1.visible);
    assert!(!l1.locks.transparency);
    assert_eq!(l1.blend, BlendMode::LinearDodge); // Additive -> LinearDodge
    assert!((l1.opacity - 180.0 / 255.0).abs() < 1e-4);
}

#[test]
fn test_pdn_export_unsupported() {
    let doc = Document::new("test", Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
    let err = export(&doc, "test.pdn", &ExportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("Paint.NET (.pdn) export is not implemented"));
}

#[test]
fn test_real_pdn_import_if_available() {
    if let Ok(bytes) = std::fs::read("/tmp/pdn_test/Untitled.pdn") {
        let r = import("Untitled.pdn", &bytes).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (800, 600));
        assert_eq!(r.document.layers.len(), 1);
        assert_eq!(r.document.layers[0].name, "Background");
    }
}
