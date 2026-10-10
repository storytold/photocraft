//! Original synthetic profiles; no Adobe/vendor profile payload is stored in the repository.
use photocraft_raw::testgen::{DngSpec, mosaic, scene};
use photocraft_raw::{CameraProfile, DevelopOptions, Limits, camera_model_matches, decode, develop_sensor, develop_sensor_profile, picture_control};

fn dcp(big: bool) -> Vec<u8> {
    dcp_with_policy(big, None)
}

fn dcp_with_policy(big: bool, policy: Option<u32>) -> Vec<u8> {
    let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let floats = |a: &[f32]| a.iter().flat_map(|v| u32b(v.to_bits())).collect::<Vec<_>>();
    let matrix = [1i32, 0, 0, 0, 1, 0, 0, 0, 1].into_iter().flat_map(|v| [u32b(v as u32), u32b(1)].concat()).collect::<Vec<_>>();
    let mut tags = vec![
        (50708, 2, 10, b"Test Body\0".to_vec()),
        (50721, 10, 9, matrix),
        (50778, 3, 1, u16b(21).to_vec()),
        (50936, 2, 10, b"Test Look\0".to_vec()),
        (50940, 11, 6, floats(&[0.0, 0.0, 0.25, 0.5, 1.0, 1.0])),
        (51110, 4, 1, u32b(1).to_vec()),
    ];
    if let Some(policy) = policy {
        tags.push((50941, 4, 1, u32b(policy).to_vec()));
        let notice = b"Original synthetic test data\0";
        tags.push((50942, 2, notice.len() as u32, notice.to_vec()));
    }
    let mut out = if big { b"MMCR".to_vec() } else { b"IIRC".to_vec() };
    out.extend(u32b(8));
    out.extend(u16b(tags.len() as u16));
    let dir = out.len();
    out.resize(dir + tags.len() * 12 + 4, 0);
    for (i, (tag, typ, count, bytes)) in tags.into_iter().enumerate() {
        let e = dir + i * 12;
        out[e..e + 2].copy_from_slice(&u16b(tag));
        out[e + 2..e + 4].copy_from_slice(&u16b(typ));
        out[e + 4..e + 8].copy_from_slice(&u32b(count));
        if bytes.len() <= 4 {
            out[e + 8..e + 8 + bytes.len()].copy_from_slice(&bytes);
        } else {
            let at = out.len() as u32;
            out[e + 8..e + 12].copy_from_slice(&u32b(at));
            out.extend(bytes);
        }
    }
    out
}

#[test]
fn external_profile_calibrates_and_renders_both_byte_orders() {
    let mut spec = DngSpec::cfa(24, 16, mosaic(&scene(24, 16), 24, [0, 1, 1, 2], 0, 4095));
    spec.model = "Test Body".into();
    spec.bits = 12;
    spec.white = 4095;
    let sensor = decode(&spec.build(), &Limits::default()).unwrap();
    for big in [false, true] {
        let p = CameraProfile::from_dcp(&dcp(big)).unwrap();
        assert_eq!(p.name, "Test Look");
        p.validate(&sensor).unwrap();
        let neutral = develop_sensor(&sensor, &DevelopOptions::default()).unwrap();
        let rendered = develop_sensor_profile(&sensor, &DevelopOptions::default(), Some(&p)).unwrap();
        assert_ne!(rendered.rgb, neutral.rgb);
        assert_eq!((rendered.width, rendered.height), (24, 16));
        assert_eq!(rendered.info.wb_multipliers, neutral.info.wb_multipliers);
        let mut wrong = p.clone();
        wrong.camera_model = "Different Body".into();
        assert!(develop_sensor_profile(&sensor, &DevelopOptions::default(), Some(&wrong)).is_err());
        let mut bad = p.clone();
        bad.calibrations[0].color_matrix = [[0.0; 3]; 3];
        assert!(bad.validate(&sensor).is_err());
        bad = p.clone();
        bad.tone_curve[1][0] = 0.0;
        assert!(bad.validate(&sensor).is_err());
    }
}

#[test]
fn truncated_and_hostile_profiles_error_instead_of_panicking() {
    let bytes = dcp(false);
    for n in 0..bytes.len() {
        assert!(CameraProfile::from_dcp(&bytes[..n]).is_err(), "accepted truncated {n}");
    }
    let mut duplicate = bytes.clone();
    duplicate[22..24].copy_from_slice(&50708u16.to_le_bytes());
    assert!(CameraProfile::from_dcp(&duplicate).is_err());
    let mut huge = bytes;
    huge[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(CameraProfile::from_dcp(&huge).is_err());
}

#[test]
fn nikon_picture_control_versions_are_bounded_and_keep_auto_codes() {
    for (version, start, base, adjust, contrast, brightness, saturation, hue) in
        [("0100", 4, 24, 48, 51, 52, 53, 54), ("0200", 4, 24, 48, 55, 57, 59, 61), ("0310", 8, 28, 54, 63, 65, 67, 69)]
    {
        let mut b = vec![0; 80];
        b[..4].copy_from_slice(version.as_bytes());
        b[start..start + 4].copy_from_slice(b"AUTO");
        b[base..base + 4].copy_from_slice(b"AUTO");
        b[adjust] = 1;
        for p in [contrast, brightness, saturation, hue] {
            b[p] = 255;
        }
        let p = picture_control(&b).unwrap();
        assert_eq!(p.name, "AUTO");
        assert_eq!(p.contrast_code, 255);
        for n in 0..=hue {
            assert!(picture_control(&b[..n]).is_none());
        }
        b[..4].copy_from_slice(b"9999");
        assert!(picture_control(&b).is_none());
    }
}

#[test]
fn models_match_make_prefixes_but_not_neighbouring_models() {
    for (profile, make, model) in [
        ("Nikon Z f", "NIKON CORPORATION", "NIKON Z f"),
        ("Sony ILCE-7M3", "SONY", "ILCE-7M3"),
        ("Fujifilm X-T5", "FUJIFILM", "X-T5"),
        ("Canon EOS R5", "Canon", "Canon EOS R5"),
    ] {
        assert!(camera_model_matches(profile, make, model));
    }
    assert!(!camera_model_matches("Sony ILCE-7M4", "SONY", "ILCE-7M3"));
    assert!(!camera_model_matches("Nikon Z 6II", "NIKON CORPORATION", "NIKON Z 6"));
    assert!(!camera_model_matches("Sony X-1", "Canon", "X-1"));
    assert!(!camera_model_matches("Nikon Z f", "Nikon", ""));
}

#[test]
fn profile_usage_policy_is_retained_and_enforced_for_non_dng_input() {
    let mut spec = DngSpec::cfa(8, 8, mosaic(&scene(8, 8), 8, [0, 1, 1, 2], 0, 4095));
    spec.model = "Test Body".into();
    let mut sensor = decode(&spec.build(), &Limits::default()).unwrap();
    let mut profile = CameraProfile::from_dcp(&dcp(false)).unwrap();
    assert_eq!(profile.embed_policy, 0, "missing policy defaults to DNG-only");
    profile.validate(&sensor).unwrap();
    sensor.format = photocraft_raw::RawFormat::Nef;
    for policy in [0, 1] {
        profile.embed_policy = policy;
        assert!(profile.validate(&sensor).unwrap_err().to_string().contains("DNG input only"));
    }
    for policy in [2, 3] {
        profile.embed_policy = policy;
        profile.validate(&sensor).unwrap();
    }
    profile.embed_policy = 4;
    assert!(profile.validate(&sensor).is_err());
}

#[test]
fn policy_and_copyright_tags_parse_in_both_byte_orders() {
    for big in [false, true] {
        for policy in 0..=3 {
            let p = CameraProfile::from_dcp(&dcp_with_policy(big, Some(policy))).unwrap();
            assert_eq!(p.embed_policy, policy);
            assert_eq!(p.copyright, "Original synthetic test data");
        }
        assert!(CameraProfile::from_dcp(&dcp_with_policy(big, Some(4))).is_err());
    }
}
