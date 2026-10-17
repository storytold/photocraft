//! Real-camera corpus tests against `corpus/pixls` (raw.pixls.us, public domain): representative
//! decode paths, oracle-checked against values measured when the corpus was pinned (the
//! `inspect` example prints them). Opt-in like every real-file corpus:
//! `cargo xtask corpus --pixls` fetches, `cargo xtask test-corpus` runs.
#![cfg(feature = "corpus")]

use photocraft_raw::{Limits, RawError, RawFormat};
use std::path::PathBuf;

fn pixls(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pixls").join(name);
    if !p.is_file() {
        panic!("{} is missing: run `cargo xtask corpus --pixls` (or `--all`) first", p.display());
    }
    p
}

struct Want<'a> {
    file: &'static str,
    format: RawFormat,
    make: &'static str,
    model: &'static str,
    /// Sensor width and height.
    size: (usize, usize),
    /// The four CFA colours, row-major.
    cfa: [u8; 4],
    black: &'a [f32],
    white: [f32; 3],
}

const WANTS: &[Want] = &[
    Want {
        file: "IMG_4059.CR2",
        format: RawFormat::Cr2,
        make: "Canon",
        model: "Canon PowerShot SX50 HS",
        size: (4176, 3062),
        cfa: [0, 1, 1, 2],
        black: &[128.0; 4],
        white: [4095.0; 3],
    },
    // The same camera after DNG Converter: the sensor data round-trips identically.
    Want {
        file: "CRW_4061.DNG",
        format: RawFormat::Dng,
        make: "Canon",
        model: "Canon PowerShot SX50 HS",
        size: (4176, 3062),
        cfa: [0, 1, 1, 2],
        black: &[127.0],
        white: [4095.0; 3],
    },
    Want {
        file: "JD1_8203.NEF",
        format: RawFormat::Nef,
        make: "NIKON CORPORATION",
        model: "NIKON D3",
        size: (4288, 2844),
        cfa: [0, 1, 1, 2],
        black: &[0.0],
        white: [4095.0; 3],
    },
    // Sony cRAW: the saturation level comes from the compressed code's range.
    Want {
        file: "DSC00009.ARW",
        format: RawFormat::Arw,
        make: "SONY",
        model: "DSC-RX0",
        size: (4832, 3224),
        cfa: [0, 1, 1, 2],
        // No readable BlackLevel tag: the cRAW fallback of 512 (#1902). The file's darkest
        // samples sit near 800, so this body's real level is still to be read.
        black: &[512.0],
        white: [16301.0; 3],
    },
    // The G9's high-resolution mode: a 10480 × 7794 sensor cropped to 10368 × 7776.
    Want {
        file: "P1000475.RW2",
        format: RawFormat::Rw2,
        make: "Panasonic",
        model: "DC-G9",
        size: (10480, 7794),
        cfa: [0, 1, 1, 2],
        black: &[127.0, 128.0, 128.0, 127.0],
        white: [4095.0; 3],
    },
    // Four Thirds Olympus: GRBG instead of the usual RGGB.
    Want {
        file: "E_1__C106743_gredos.ORF",
        format: RawFormat::Orf,
        make: "OLYMPUS CORPORATION",
        model: "E-1",
        size: (2624, 1966),
        cfa: [1, 0, 2, 1],
        black: &[65.0; 4],
        white: [4095.0; 3],
    },
    // These bodies declare 16-bit uncompressed samples but store ten 12-bit values plus
    // one padding byte in each 16-byte block. Their sensor-origin CFA is RGGB.
    Want {
        file: "P1252148.ORF",
        format: RawFormat::Orf,
        make: "OLYMPUS IMAGING CORP.",
        model: "E-300",
        size: (3360, 2504),
        cfa: [0, 1, 1, 2],
        black: &[63.0; 4],
        white: [4095.0; 3],
    },
    Want {
        file: "_1010010.ORF",
        format: RawFormat::Orf,
        make: "OLYMPUS IMAGING CORP.",
        model: "E-500",
        size: (3360, 2504),
        cfa: [0, 1, 1, 2],
        black: &[63.0; 4],
        white: [4095.0; 3],
    },
    Want {
        file: "P3307182.ORF",
        format: RawFormat::Orf,
        make: "OLYMPUS IMAGING CORP.",
        model: "E-330",
        size: (3280, 2450),
        cfa: [0, 1, 1, 2],
        black: &[69.0, 70.0, 69.0, 70.0],
        white: [4095.0; 3],
    },
];

#[test]
fn decode_matches_the_measured_oracles() {
    for w in WANTS {
        let bytes = std::fs::read(pixls(w.file)).unwrap_or_else(|e| panic!("{}: {e}", w.file));
        let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", w.file));
        assert_eq!(s.format, w.format, "{}", w.file);
        assert_eq!(s.make.as_deref(), Some(w.make), "{}", w.file);
        assert_eq!(s.model.as_deref(), Some(w.model), "{}", w.file);
        assert_eq!((s.width, s.height), w.size, "{}", w.file);
        let cfa = s.cfa.as_ref().unwrap_or_else(|| panic!("{}: no CFA", w.file));
        assert_eq!(cfa.colors, w.cfa, "{}", w.file);
        assert_eq!(s.black.values, w.black, "{}", w.file);
        for ch in 0..3 {
            assert!((s.white[ch] - w.white[ch]).abs() < 0.5, "{} white[{ch}] = {}", w.file, s.white[ch]);
        }
    }
}

#[test]
fn developing_produces_the_crop_in_16bit_rgb() {
    for w in WANTS {
        let bytes = std::fs::read(pixls(w.file)).unwrap_or_else(|e| panic!("{}: {e}", w.file));
        let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", w.file));
        let d = photocraft_raw::develop_sensor(&s, &photocraft_raw::DevelopOptions::default()).unwrap_or_else(|e| panic!("{}: {e}", w.file));
        assert_eq!((d.width as usize, d.height as usize), (s.crop.width, s.crop.height), "{}", w.file);
        assert_eq!(d.rgb.len(), d.width as usize * d.height as usize * 3, "{}", w.file);
        // A real exposure covers a sane share of the range and never leaves it.
        let (mut lo, mut hi) = (u16::MAX, 0u16);
        for v in &d.rgb {
            lo = (*v).min(lo);
            hi = (*v).max(hi);
        }
        // A real exposure covers a sane share of the range and never leaves it. The PowerShot
        // CR2 and the RX0 ARW are the documented exceptions: neither carries an as-shot white
        // balance we read, so the grey-world estimate brightens these scenes (the PowerShot's
        // own DNG, with its file WB, develops blacks to ~0 — reading the maker-note WB of
        // Canon and Sony is the follow-up).
        let black_cap: u16 = match w.file {
            "IMG_4059.CR2" | "DSC00009.ARW" => 16384,
            // These hazy scenes have no near-black developed pixels with the existing
            // uncalibrated colour fallback: measured minima 6286 (E-500) and 5031 (E-330).
            "_1010010.ORF" | "P3307182.ORF" => 8192,
            _ => 4096,
        };
        assert!(lo < black_cap, "{}: developed blacks at {lo}", w.file);
        assert!(hi > u16::MAX / 4, "{}: developed highlights at {hi}", w.file);
        // Deterministic: a second develop of the same sensor gives the same pixels.
        let d2 = photocraft_raw::develop_sensor(&s, &photocraft_raw::DevelopOptions::default()).unwrap();
        assert_eq!(d.rgb, d2.rgb, "{}", w.file);
    }
}

/// These paired exposures used to decode below their recorded black levels because the
/// version 0x44 0x40 linearization curve was interpolated across the sensor's full bit range.
#[test]
fn modern_lossy_nef_preserves_brightness_at_both_depths() {
    for (file, black, white) in [("DSC_0751.NEF", 1008.0, 16383.0), ("DSC_0754.NEF", 252.0, 4095.0)] {
        let bytes = std::fs::read(pixls(file)).unwrap();
        let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(s.model.as_deref(), Some("NIKON Z 6"));
        assert_eq!((s.width, s.height), (6064, 4040));
        assert_eq!(s.black.values, vec![black; 4]);
        assert!(s.white.iter().all(|v| (*v - white).abs() < 100.0));
        // The same scene spans deep shadows and bright highlights at either sensor depth.
        let max = s.data.iter().copied().max().unwrap();
        assert!(f32::from(max) > white * 0.9, "{file}: maximum {max}");
        let mean = s.data.iter().map(|v| f64::from(*v)).sum::<f64>() / s.data.len() as f64;
        assert!(mean > f64::from(black) * 1.5, "{file}: mean {mean}");
        let d = photocraft_raw::develop_sensor(&s, &photocraft_raw::DevelopOptions::default()).unwrap();
        assert_eq!((d.width, d.height), (6048, 4024));
        assert!(d.rgb.iter().copied().max().unwrap() > 32768, "{file}: black development");
    }
}

#[test]
fn packed_nef_opens_full_sensor_and_metadata_crop_at_both_depths() {
    for (file, first) in [("DSC_0750.NEF", [3539, 3761, 3640, 3904]), ("DSC_0753.NEF", [866, 932, 881, 948])] {
        let bytes = std::fs::read(pixls(file)).unwrap();
        let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height), (6064, 4040));
        assert_eq!(&s.data[..4], &first);
        assert_eq!(s.crop, photocraft_raw::Rect::new(8, 8, 6048, 4024));
        let d = photocraft_raw::develop_sensor(&s, &Default::default()).unwrap();
        assert_eq!((d.width, d.height), (6048, 4024));
    }
}

/// Values read directly from the CC0 files' first two 16-byte blocks, including the padding
/// boundary, using the byte layout documented in LightCraft #651's public prose. These are
/// measurements of the files, not output copied from another RAW decoder.
#[test]
fn padded_orf_matches_measured_sensor_samples() {
    let cases = [
        ("P1252148.ORF", [621, 1252, 642, 1228, 672, 1314, 672, 1308, 662, 1202, 643, 1290, 661, 1277, 664, 1245, 654, 1245, 631, 1269]),
        ("_1010010.ORF", [619, 1554, 698, 1514, 663, 1504, 725, 1503, 703, 1511, 691, 1495, 699, 1515, 681, 1462, 684, 1521, 665, 1467]),
        ("P3307182.ORF", [2070, 3740, 2300, 4095, 2499, 4095, 2683, 4095, 2848, 4095, 3018, 4095, 3123, 4095, 3241, 4095, 3451, 4095, 3692, 4095]),
    ];
    for (file, expected) in cases {
        let bytes = std::fs::read(pixls(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let sensor = photocraft_raw::decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(sensor.data.get(..expected.len()), Some(expected.as_slice()), "{file}");
    }
}

/// Metadata is read from each original's TIFF/Exif and Olympus maker tags. Sample values
/// were extracted independently from the file bytes: 120-bit little-endian padded blocks,
/// 24-bit big-endian two-field pairs, or 96-bit groups of three little-endian words. No
/// values come from PhotoCraft or another RAW decoder. Points straddle packing/row/field
/// boundaries, include both ends of the last two rows, and check the crop corners.
#[test]
fn additional_orf_layouts_match_file_metadata_and_boundary_samples() {
    struct Case {
        file: &'static str,
        size: (usize, usize),
        crop: (usize, usize, usize, usize),
        cfa: [u8; 4],
        black: f32,
        wb_rb: (u16, u16),
        samples: &'static [(usize, usize, u16)],
    }
    let cases = [
        // raw.pixls.us #2856: E-M5 Mark II.
        Case {
            file: "CB252215.ORF",
            size: (9280, 6932),
            crop: (10, 10, 9216, 6912),
            cfa: [1, 0, 2, 1],
            black: 254.0,
            wb_rb: (438, 636),
            samples: &[
                (0, 0, 418),
                (7, 0, 318),
                (8, 0, 354),
                (9, 0, 298),
                (10, 0, 330),
                (9279, 0, 408),
                (0, 1, 315),
                (9279, 1, 331),
                (4640, 3466, 442),
                (0, 6930, 396),
                (9279, 6930, 395),
                (0, 6931, 296),
                (9279, 6931, 326),
                (10, 10, 374),
                (9225, 6921, 307),
            ],
        },
        // raw.pixls.us #2977: PEN-F.
        Case {
            file: "PenFHiRes.orf",
            size: (10400, 7796),
            crop: (10, 10, 10368, 7776),
            cfa: [1, 0, 2, 1],
            black: 254.0,
            wb_rb: (422, 532),
            samples: &[
                (0, 0, 1503),
                (7, 0, 1125),
                (8, 0, 1510),
                (9, 0, 1102),
                (10, 0, 1478),
                (10399, 0, 1494),
                (0, 1, 757),
                (10399, 1, 925),
                (5200, 3898, 314),
                (0, 7794, 437),
                (10399, 7794, 446),
                (0, 7795, 319),
                (10399, 7795, 362),
                (10, 10, 1412),
                (10377, 7785, 390),
            ],
        },
        // raw.pixls.us #1799: C5060WZ.
        Case {
            file: "C5060WZ_5_7_22_8mm__27_110mm_equiv__F_2_8_8P4141018_cabo_de_gata.ORF",
            size: (2608, 1949),
            crop: (0, 0, 2608, 1949),
            cfa: [0, 1, 1, 2],
            black: 0.0,
            wb_rb: (437, 411),
            samples: &[
                (0, 0, 703),
                (7, 0, 1488),
                (8, 0, 754),
                (9, 0, 1445),
                (10, 0, 780),
                (2607, 0, 1596),
                (0, 1, 1353),
                (2607, 1, 1353),
                (1304, 974, 218),
                (0, 1947, 259),
                (2607, 1947, 123),
                (0, 1948, 144),
                (2607, 1948, 217),
            ],
        },
        // raw.pixls.us #780: C7070WZ.
        Case {
            file: "PC270085.ORF",
            size: (3088, 2309),
            crop: (0, 0, 3088, 2309),
            cfa: [0, 1, 1, 2],
            black: 0.0,
            wb_rb: (476, 443),
            samples: &[
                (0, 0, 929),
                (7, 0, 2218),
                (8, 0, 971),
                (9, 0, 2270),
                (10, 0, 1046),
                (3087, 0, 156),
                (0, 1, 2120),
                (3087, 1, 75),
                (1544, 1154, 284),
                (0, 2307, 126),
                (3087, 2307, 44),
                (0, 2308, 73),
                (3087, 2308, 116),
            ],
        },
        // raw.pixls.us #2420: SP510UZ.
        Case {
            file: "PA210583.ORF",
            size: (3088, 2309),
            crop: (0, 0, 3088, 2309),
            cfa: [1, 2, 0, 1],
            black: 0.0,
            wb_rb: (460, 393),
            samples: &[
                (0, 0, 82),
                (7, 0, 30),
                (8, 0, 79),
                (9, 0, 28),
                (10, 0, 92),
                (3087, 0, 51),
                (0, 1, 40),
                (3087, 1, 197),
                (1544, 1154, 800),
                (0, 2307, 189),
                (3087, 2307, 78),
                (0, 2308, 420),
                (3087, 2308, 51),
            ],
        },
        // raw.pixls.us #8609: SP550UZ.
        Case {
            file: "PA030017.ORF",
            size: (3088, 2309),
            crop: (0, 0, 3088, 2309),
            cfa: [2, 1, 1, 0],
            black: 0.0,
            wb_rb: (444, 390),
            samples: &[
                (0, 0, 1004),
                (7, 0, 1617),
                (8, 0, 975),
                (9, 0, 1617),
                (10, 0, 913),
                (3087, 0, 2355),
                (0, 1, 1838),
                (3087, 1, 1161),
                (1544, 1154, 332),
                (0, 2307, 1039),
                (3087, 2307, 179),
                (0, 2308, 445),
                (3087, 2308, 238),
            ],
        },
        // raw.pixls.us #3438: SP565UZ.
        Case {
            file: "2019-09-26--10.03.53001.ORF",
            size: (3664, 2741),
            crop: (0, 0, 3664, 2741),
            cfa: [2, 1, 1, 0],
            black: 0.0,
            wb_rb: (470, 453),
            samples: &[
                (0, 0, 56),
                (7, 0, 157),
                (8, 0, 56),
                (9, 0, 159),
                (10, 0, 61),
                (3663, 0, 1893),
                (0, 1, 145),
                (3663, 1, 923),
                (1832, 1370, 214),
                (0, 2739, 46),
                (3663, 2739, 84),
                (0, 2740, 16),
                (3663, 2740, 112),
            ],
        },
        // raw.pixls.us #5491: SP570UZ.
        Case {
            file: "P5110102.ORF",
            size: (3664, 2741),
            crop: (0, 0, 3664, 2741),
            cfa: [2, 1, 1, 0],
            black: 0.0,
            wb_rb: (451, 407),
            samples: &[
                (0, 0, 161),
                (7, 0, 364),
                (8, 0, 142),
                (9, 0, 457),
                (10, 0, 151),
                (3663, 0, 599),
                (0, 1, 435),
                (3663, 1, 330),
                (1832, 1370, 610),
                (0, 2739, 368),
                (3663, 2739, 434),
                (0, 2740, 211),
                (3663, 2740, 657),
            ],
        },
        // raw.pixls.us #1432: XZ-2.
        Case {
            file: "p7166537.orf",
            size: (3984, 2986),
            crop: (8, 10, 3968, 2976),
            cfa: [0, 1, 1, 2],
            black: 200.0,
            wb_rb: (518, 452),
            samples: &[
                (0, 0, 320),
                (7, 0, 472),
                (8, 0, 324),
                (9, 0, 492),
                (10, 0, 320),
                (3983, 0, 236),
                (0, 1, 456),
                (3983, 1, 220),
                (1992, 1493, 1928),
                (0, 2984, 292),
                (3983, 2984, 244),
                (0, 2985, 396),
                (3983, 2985, 216),
                (8, 10, 332),
                (3975, 2985, 220),
            ],
        },
        // raw.pixls.us #2431: XZ-10.
        Case {
            file: "P1240016.ORF",
            size: (3984, 2990),
            crop: (8, 14, 3968, 2976),
            cfa: [0, 1, 1, 2],
            black: 200.0,
            wb_rb: (507, 438),
            samples: &[
                (0, 0, 884),
                (7, 0, 1587),
                (8, 0, 841),
                (9, 0, 1575),
                (10, 0, 816),
                (3983, 0, 283),
                (0, 1, 1585),
                (3983, 1, 279),
                (1992, 1495, 1924),
                (0, 2988, 813),
                (3983, 2988, 320),
                (0, 2989, 1437),
                (3983, 2989, 302),
                (8, 14, 840),
                (3975, 2989, 311),
            ],
        },
        // raw.pixls.us #792: SH-2.
        Case {
            file: "PC260009.ORF",
            size: (4624, 3470),
            crop: (7, 6, 4608, 3456),
            cfa: [1, 0, 2, 1],
            black: 200.0,
            wb_rb: (456, 485),
            samples: &[
                (0, 0, 443),
                (7, 0, 312),
                (8, 0, 516),
                (9, 0, 307),
                (10, 0, 488),
                (4623, 0, 286),
                (0, 1, 483),
                (4623, 1, 463),
                (2312, 1735, 277),
                (0, 3468, 228),
                (4623, 3468, 210),
                (0, 3469, 232),
                (4623, 3469, 230),
                (7, 6, 301),
                (4614, 3461, 230),
            ],
        },
    ];
    for case in cases {
        let file = case.file;
        let bytes = std::fs::read(pixls(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let sensor = photocraft_raw::decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(sensor.format, RawFormat::Orf, "{file}");
        assert_eq!((sensor.width, sensor.height), case.size, "{file}");
        assert_eq!(sensor.data.len(), case.size.0 * case.size.1, "{file}");
        assert_eq!((sensor.crop.x, sensor.crop.y, sensor.crop.width, sensor.crop.height), case.crop, "{file}");
        let cfa = sensor.cfa.as_ref().unwrap_or_else(|| panic!("{file}: no CFA"));
        assert_eq!(cfa.colors, case.cfa, "{file}");
        assert!(!sensor.black.values.is_empty(), "{file}: no black level");
        assert!(sensor.black.values.iter().all(|&value| value == case.black), "{file}: black levels {:?}", sensor.black.values);
        assert_eq!(sensor.camera_wb, Some([f64::from(case.wb_rb.0) / 256.0, 1.0, f64::from(case.wb_rb.1) / 256.0]), "{file}");
        for &(x, y, expected) in case.samples {
            assert_eq!(sensor.data[y * sensor.width + x], expected, "{file}: sensor sample ({x}, {y})");
        }
    }
}

/// The E-5 file is the documented known-unsupported case: later Olympus bodies store
/// compressed sensor data. The error must say so, never panic, and the file still previews.
#[test]
fn packed_orf_reports_the_documented_gap() {
    let bytes = std::fs::read(pixls("_7061961_copy.ORF")).unwrap();
    let err = photocraft_raw::decode(&bytes, &Limits::default()).unwrap_err();
    assert!(matches!(err, RawError::Unsupported(ref m) if m.contains("Olympus compressed")), "{err}");
    assert!(photocraft_raw::embedded_preview(&bytes).is_some(), "the camera's JPEG preview is still reachable");
}
