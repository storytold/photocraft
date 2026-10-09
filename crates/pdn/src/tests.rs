use crate::error::Error;
use crate::model::{BlendMode, Limits, read};
use crate::synth::{SynthLayer, build_pdn};

#[test]
fn test_synthetic_single_layer_gzip() {
    let pixels = vec![
        255, 0, 0, 255,   0, 255, 0, 255,
        0, 0, 255, 255,   255, 255, 255, 128,
    ];
    let synth = SynthLayer {
        name: "Background".into(),
        visible: true,
        is_background: true,
        opacity: 255,
        blend_mode: BlendMode::Normal,
        rgba_pixels: pixels.clone(),
    };
    let bytes = build_pdn(2, 2, &[synth], true);
    let doc = read(&bytes, Limits::default()).expect("failed to parse synthetic PDN");

    assert_eq!(doc.width, 2);
    assert_eq!(doc.height, 2);
    assert_eq!(doc.layers.len(), 1);
    let l = &doc.layers[0];
    assert_eq!(l.name, "Background");
    assert!(l.visible);
    assert!(l.is_background);
    assert_eq!(l.opacity, 255);
    assert_eq!(l.blend_mode, BlendMode::Normal);
    assert_eq!(l.rgba_pixels, pixels);
}

#[test]
fn test_synthetic_single_layer_uncompressed() {
    let pixels = vec![
        10, 20, 30, 255, 40, 50, 60, 200,
        70, 80, 90, 150, 100, 110, 120, 100,
    ];
    let synth = SynthLayer {
        name: "Layer 1".into(),
        visible: true,
        is_background: false,
        opacity: 200,
        blend_mode: BlendMode::Multiply,
        rgba_pixels: pixels.clone(),
    };
    let bytes = build_pdn(2, 2, &[synth], false);
    let doc = read(&bytes, Limits::default()).expect("failed to parse uncompressed PDN");

    assert_eq!(doc.width, 2);
    assert_eq!(doc.height, 2);
    assert_eq!(doc.layers.len(), 1);
    let l = &doc.layers[0];
    assert_eq!(l.name, "Layer 1");
    assert_eq!(l.opacity, 200);
    assert_eq!(l.blend_mode, BlendMode::Multiply);
    assert_eq!(l.rgba_pixels, pixels);
}

#[test]
fn test_synthetic_multi_layer() {
    let p0 = vec![255; 16];
    let p1 = vec![128; 16];
    let layers = vec![
        SynthLayer {
            name: "Bottom".into(),
            visible: true,
            is_background: true,
            opacity: 255,
            blend_mode: BlendMode::Normal,
            rgba_pixels: p0.clone(),
        },
        SynthLayer {
            name: "Top".into(),
            visible: false,
            is_background: false,
            opacity: 180,
            blend_mode: BlendMode::Additive,
            rgba_pixels: p1.clone(),
        },
    ];
    let bytes = build_pdn(2, 2, &layers, true);
    let doc = read(&bytes, Limits::default()).expect("failed to parse multi-layer PDN");

    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.layers[0].name, "Bottom");
    assert_eq!(doc.layers[0].rgba_pixels, p0);
    assert_eq!(doc.layers[1].name, "Top");
    assert!(!doc.layers[1].visible);
    assert_eq!(doc.layers[1].opacity, 180);
    assert_eq!(doc.layers[1].blend_mode, BlendMode::Additive);
    assert_eq!(doc.layers[1].rgba_pixels, p1);
}

#[test]
fn test_adversarial_inputs_never_panic() {
    assert!(read(&[], Limits::default()).is_err());
    assert!(read(b"PDN", Limits::default()).is_err());
    assert!(read(b"PDN2\x00\x00\x00\x00\x01", Limits::default()).is_err());

    let limits_header = Limits { max_header_size: 10, ..Default::default() };
    let big_header = b"PDN3\xff\x00\x00<xml>some_long_xml_header</xml>\x00\x01";
    assert_eq!(read(big_header, limits_header), Err(Error::Limit("PDN XML header exceeds maximum allowed size")));

    let broken_indicator = b"PDN3\x07\x00\x00<xml />\x00\x02";
    assert!(read(broken_indicator, Limits::default()).is_err());

    // Corrupted payload after header
    let truncated_nrbf = b"PDN3\x07\x00\x00<xml />\x00\x01\x00\x01";
    assert!(read(truncated_nrbf, Limits::default()).is_err());

    // Limits enforcement: max file size
    let limits_file = Limits { max_file_size: 20, ..Default::default() };
    let synth = SynthLayer {
        name: "L".into(),
        visible: true,
        is_background: true,
        opacity: 255,
        blend_mode: BlendMode::Normal,
        rgba_pixels: vec![0; 4],
    };
    let bytes = build_pdn(1, 1, &[synth], false);
    assert_eq!(read(&bytes, limits_file), Err(Error::Limit("file size exceeds maximum allowed limit")));
}

#[test]
fn test_real_files_if_available() {
    for (name, expected_layers, w, h) in [
        ("/tmp/pdn_test/Untitled.pdn", 1, 800, 600),
        ("/tmp/pdn_test/Untitled2.pdn", 2, 800, 600),
        ("/tmp/pdn_test/Untitled3.pdn", 2, 800, 600),
        ("/tmp/pdn_test/oldPDN3510.pdn", 2, 800, 600),
        ("/tmp/pdn_test/FlattenBlendTest.pdn", 14, 800, 600),
    ] {
        if let Ok(bytes) = std::fs::read(name) {
            let doc = read(&bytes, Limits::default()).unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
            assert_eq!(doc.width, w);
            assert_eq!(doc.height, h);
            assert_eq!(doc.layers.len(), expected_layers);
            for (i, layer) in doc.layers.iter().enumerate() {
                println!("{name} layer {i}: {:?}", layer.name);
                assert_eq!(layer.width, w);
                assert_eq!(layer.height, h);
                assert_eq!(layer.rgba_pixels.len(), (w * h * 4) as usize);
            }
        }
    }
}
