//! The external decoder must preserve the public sensor/development contract.
use photocraft_raw::testgen::{DngSpec, DngStorage, mosaic, scene, tiff_ep};
use photocraft_raw::{DecodeBackend, Limits, RawError, Sensor, decode, decode_with_backend};
#[cfg(feature = "flashraw")]
use photocraft_raw::{Demosaic, DevelopOptions, develop_sensor};

fn same_sensor(a: &Sensor, b: &Sensor) {
    assert_eq!((a.format, a.width, a.height, a.samples), (b.format, b.width, b.height, b.samples));
    assert_eq!(a.data, b.data);
    assert_eq!(a.cfa, b.cfa);
    assert_eq!(a.linearization, b.linearization);
    assert_eq!(a.black, b.black);
    assert_eq!(a.white, b.white);
    assert_eq!((a.active, a.crop, a.orientation), (b.active, b.crop, b.orientation));
    assert_eq!(a.color, b.color);
    assert_eq!(a.camera_wb, b.camera_wb);
    assert_eq!(a.baseline_exposure, b.baseline_exposure);
    assert_eq!(a.gain_maps, b.gain_maps);
    assert_eq!(a.tone_curve, b.tone_curve);
    assert_eq!(a.black_point, b.black_point);
}

#[cfg(feature = "flashraw")]
#[test]
fn flashraw_and_builtin_develop_the_same_dng_at_each_depth_and_orientation() {
    for bits in [8u16, 12, 14, 16] {
        for orientation in 1..=8 {
            let (w, h) = (128, 96);
            let white = ((1u32 << bits) - 1) as u16;
            let mut s = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 16, white));
            s.bits = bits;
            s.white = u32::from(white);
            s.black = vec![16];
            s.orientation = orientation;
            s.as_shot_neutral = Some([0.5, 1.0, 0.75]);
            s.color_matrix1 = Some((21, [0.9, -0.3, -0.1, -0.4, 1.3, 0.1, -0.1, 0.2, 0.6]));
            let bytes = s.build();
            let builtin = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap();
            let flash = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw).unwrap();
            same_sensor(&builtin, &flash);
            let opts = DevelopOptions { demosaic: Demosaic::Mhc, ..Default::default() };
            let a = develop_sensor(&builtin, &opts).unwrap();
            let b = develop_sensor(&flash, &opts).unwrap();
            assert_eq!((a.width, a.height, a.rgb), (b.width, b.height, b.rgb));
        }
    }
}

#[cfg(feature = "flashraw")]
#[test]
fn lossless_jpeg_tiles_and_deflate_preserve_samples() {
    for storage in [DngStorage::Lj92Tiles { width: 32, height: 32 }, DngStorage::DeflateStrips { rows: 16, predictor: 1 }] {
        let mut s = DngSpec::cfa(128, 96, mosaic(&scene(128, 96), 128, [0, 1, 1, 2], 0, 65535));
        s.storage = storage;
        let bytes = s.build();
        same_sensor(
            &decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap(),
            &decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw).unwrap(),
        );
    }
}

#[cfg(feature = "flashraw")]
#[test]
fn full_size_uncompressed_sony_planes_keep_their_stored_precision() {
    for bits in [12u16, 14, 16] {
        let top = (1u32 << bits) - 1;
        let data: Vec<_> = (0..128 * 96).map(|i| ((i * 37 + 301) as u32 & top) as u16).collect();
        let bytes = tiff_ep("SONY", 128, 96, &data, [0, 1, 1, 2], bits, vec![]);
        same_sensor(
            &decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap(),
            &decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw).unwrap(),
        );
    }
}

#[test]
fn automatic_fallback_keeps_deferred_linearization_and_nef_containers() {
    let mut s = DngSpec::cfa(128, 96, mosaic(&scene(128, 96), 128, [0, 1, 1, 2], 0, 4095));
    s.bits = 12;
    s.white = 12000;
    s.linearization = Some((0..4096).map(|v| v * 3).collect());
    let bytes = s.build();
    same_sensor(&decode(&bytes, &Limits::default()).unwrap(), &decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap());
    assert!(matches!(decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw), Err(RawError::Unsupported(_))));
    let nef = tiff_ep("NIKON CORPORATION", 128, 96, &vec![1200; 128 * 96], [0, 1, 1, 2], 12, vec![]);
    same_sensor(&decode(&nef, &Limits::default()).unwrap(), &decode_with_backend(&nef, &Limits::default(), DecodeBackend::Builtin).unwrap());
}

#[test]
fn backend_selection_never_relaxes_asymmetric_limits() {
    let s = DngSpec::cfa(128, 96, vec![1000; 128 * 96]).build();
    for backend in [DecodeBackend::Auto, DecodeBackend::Builtin, DecodeBackend::FlashRaw] {
        if backend == DecodeBackend::FlashRaw && !cfg!(feature = "flashraw") {
            continue;
        }
        for limits in [
            Limits { max_width: 127, ..Limits::default() },
            Limits { max_height: 95, ..Limits::default() },
            Limits { max_pixels: 100, ..Limits::default() },
            Limits { max_alloc: 100, ..Limits::default() },
        ] {
            assert!(matches!(decode_with_backend(&s, &limits, backend), Err(RawError::LimitExceeded(_))));
        }
    }
}

#[test]
fn a_valid_tight_budget_can_use_builtin_without_relaxing_it() {
    let bytes = DngSpec::cfa(128, 96, vec![1000; 128 * 96]).build();
    let limits = Limits { max_alloc: 128 * 96 * 2, ..Limits::default() };
    let expected = decode_with_backend(&bytes, &limits, DecodeBackend::Builtin).unwrap();
    same_sensor(&decode(&bytes, &limits).unwrap(), &expected);
    assert!(matches!(decode_with_backend(&bytes, &limits, DecodeBackend::FlashRaw), Err(RawError::Unsupported(_))));
}

#[test]
fn corrupt_sensor_is_an_error_instead_of_a_preview_or_recovery() {
    let mut s = DngSpec::cfa(128, 96, vec![1000; 128 * 96]);
    s.storage = DngStorage::Lj92Tiles { width: 32, height: 32 };
    let mut bytes = s.build();
    bytes.truncate(bytes.len() / 2);
    assert!(decode(&bytes, &Limits::default()).is_err());
}

#[test]
fn hostile_dng_crop_is_rejected_before_entering_flashraw() {
    let mut s = DngSpec::cfa(16, 16, vec![500; 256]);
    s.default_crop_origin_double = Some([1e300, 0.0]);
    assert!(matches!(decode(&s.build(), &Limits::default()), Err(RawError::Malformed(_))));
}

#[test]
fn u32_crop_addition_cannot_overflow_the_external_parser() {
    let mut s = DngSpec::cfa(128, 96, vec![500; 128 * 96]);
    s.active_area = Some([4, 4, 92, 124]);
    s.default_crop = Some(([0, 0], [120, 88]));
    s.default_crop_origin_double = Some([f64::from(u32::MAX), 0.0]);
    let bytes = s.build();
    let got = decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(got.crop, got.active);
    assert!(got.warnings.iter().any(|w| w.contains("default crop")));
    assert!(matches!(decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw), Err(RawError::Unsupported(_))));
}

#[cfg(not(feature = "flashraw"))]
#[test]
fn explicitly_requesting_a_disabled_decoder_reports_unsupported() {
    let bytes = DngSpec::cfa(16, 16, vec![1000; 256]).build();
    assert!(matches!(decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw), Err(RawError::Unsupported(_))));
    assert!(decode(&bytes, &Limits::default()).is_ok());
}

#[cfg(feature = "flashraw")]
#[test]
fn rgbe_preserves_sensor_colour_codes_and_labels_rgb_reconstruction() {
    let bytes = DngSpec::cfa(128, 96, vec![1000; 128 * 96]).build();
    let mut sensor = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap();
    sensor.cfa.as_mut().unwrap().colors = vec![0, 1, 3, 2];
    let cfa = sensor.cfa.as_ref().unwrap();
    assert_eq!(cfa.sensor_color(0, 1), 3);
    assert_eq!(cfa.color(0, 1), 1);
    let developed = develop_sensor(&sensor, &DevelopOptions { demosaic: Demosaic::Mhc, ..Default::default() }).unwrap();
    assert!(developed.warnings.iter().any(|w| w.contains("RGBE") && w.contains("approximate")));
    assert_eq!(developed.info.cfa.as_deref(), Some("RGEB"));
    assert_eq!(sensor.cfa.as_ref().unwrap().colors, vec![0, 1, 3, 2]);
    assert_eq!(sensor.data, vec![1000; 128 * 96]);
}

#[cfg(feature = "flashraw")]
#[test]
fn flashraw_stored_dng_keeps_noninjective_linearization_tables_and_samples() {
    for bits in [12u16, 14, 16] {
        for components in [1, 3] {
            let (w, h) = (128, 96);
            let data: Vec<_> = (0..w * h * components).map(|i| ((i * 37 + i / w * 19) & ((1usize << bits) - 1)) as u16).collect();
            let mut spec = DngSpec::cfa(w, h, data.clone());
            spec.bits = bits;
            spec.samples = components;
            spec.storage = DngStorage::Lj92Tiles { width: 32, height: 32 };
            spec.linearization = Some((0..4096u32).map(|v| (v / 3 * 11).min(65535) as u16).collect());
            let bytes = spec.build();
            let builtin = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap();
            let external = decode_with_backend(&bytes, &Limits::default(), DecodeBackend::FlashRaw).unwrap();
            same_sensor(&builtin, &external);
            assert_eq!(external.data, data);
            assert_eq!(external.linearization.as_deref(), spec.linearization.as_deref());
        }
    }
}
