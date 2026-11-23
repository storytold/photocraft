//! #13722: Export As's JPG Quality is the 1..7 scale. Each step writes a fixed encoder quality,
//! steps 1-4 keep 4:2:0 and steps 5-7 4:4:4, an old 1..100 quality loads as the nearest step, and
//! WebP keeps its own 1..100 quality.

use super::*;

/// A 32 x 24 document of solid colour blocks, so the encoded tables are all the file writes.
fn blocks_doc() -> Document {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_geom::Rect;
    let mut doc = Document::with_background("blocks", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let surf = doc.layers[0].surface_mut().unwrap();
    surf.fill_rect(Rect::new(0, 0, 16, 16), &[1.0, 0.0, 0.0, 1.0]);
    surf.fill_rect(Rect::new(16, 0, 32, 16), &[0.0, 1.0, 0.0, 1.0]);
    surf.fill_rect(Rect::new(0, 16, 16, 24), &[0.0, 0.0, 1.0, 1.0]);
    doc
}

/// The dialog's settings map for JPG at the quality `quality`.
fn jpg_fields(quality: u32) -> Map<String, Value> {
    json!({ "format": "jpg", "quality": quality, "scale": 100, "embedProfile": false }).as_object().unwrap().clone()
}

/// The bytes Export As writes for JPG at `quality`, through the dialog's own settings.
fn encode_jpg(doc: &Document, quality: u32) -> Vec<u8> {
    let f = jpg_fields(quality);
    let out = super::export_document(doc, &f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap();
    photocraft_io::export(&out, "x.jpg", &settings(&f).io_options()).unwrap().bytes
}

/// The zigzag position each quantisation-table byte sits in, so the tables read back in the order
/// a person reads a table: row by row.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 52, 45, 38, 31, 39, 46, 53, 54, 47, 55, 58, 59, 60, 61, 62, 63,
];

/// The DQT tables of `bytes` in the order a table is read: (table id, its values).
fn quantisation_tables(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut tables = Vec::new();
    let mut at = 2;
    while at + 1 < bytes.len() {
        if bytes[at] != 0xff {
            at += 1;
            continue;
        }
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = bytes.get(at).copied().expect("a marker");
        at += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
        let segment = bytes.get(at + 2..at + length).expect("a whole segment");
        if marker == 0xdb {
            let mut read = 0;
            while read + 65 <= segment.len() {
                let written = &segment[read + 1..read + 65];
                let mut raster = vec![0u8; 64];
                for (at_zigzag, value) in written.iter().enumerate() {
                    raster[ZIGZAG[at_zigzag]] = *value;
                }
                tables.push((segment[read] & 0x0f, raster));
                read += 65;
            }
        }
        at += length;
    }
    tables
}

/// The main frame's component sampling factors, skipping whole segments.
fn sampling_factors(bytes: &[u8]) -> [(u8, u8); 3] {
    let mut at = 2;
    loop {
        assert_eq!(bytes.get(at), Some(&0xff), "a marker prefix");
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = bytes.get(at).copied().expect("a marker");
        at += 1;
        assert!(!matches!(marker, 0xd9 | 0xda), "the frame comes before the scan");
        let length = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
        let segment = bytes.get(at + 2..at + length).expect("a whole segment");
        if matches!(marker, 0xc0..=0xc3) {
            assert_eq!(segment.get(5), Some(&3), "three components");
            return [
                (segment[6 + 1] >> 4, segment[6 + 1] & 0x0f),
                (segment[6 + 3 + 1] >> 4, segment[6 + 3 + 1] & 0x0f),
                (segment[6 + 6 + 1] >> 4, segment[6 + 6 + 1] & 0x0f),
            ];
        }
        at += length;
    }
}

/// The seven scale steps and the encoder quality each one writes. Step 6 holds the closest match to
/// the reference application's measured step-6 tables (#13722).
const STEPS: [(u32, u8); 7] = [(1, 20), (2, 35), (3, 50), (4, 65), (5, 75), (6, 93), (7, 100)];

#[test]
fn each_jpg_quality_step_writes_its_fixed_encoder_quality() {
    for (step, want) in STEPS {
        let s = settings(&jpg_fields(step));
        assert_eq!(s.jpeg_quality, Some(want), "step {step} writes the encoder quality {want}");
    }
    // Steps below and above the scale take its ends instead of failing or panicking.
    assert_eq!(jpg_encoder_quality(0), 20, "below the scale");
    assert_eq!(jpg_encoder_quality(9), 100, "above the scale");
    for step in 1..=7u8 {
        assert_eq!(jpg_encoder_quality(step), STEPS[usize::from(step) - 1].1, "step {step}");
    }
}

#[test]
fn step_six_writes_the_measured_quantisation_tables_as_close_as_this_encoder_gets() {
    // The reference application's step-6 tables, first eight entries each (#13722).
    let measured_luma = [2u8, 2, 2, 2, 3, 4, 5, 6];
    let measured_chroma = [3u8, 3, 5, 9, 13, 15, 15, 15];
    let tables = quantisation_tables(&encode_jpg(&blocks_doc(), 6));
    let luma = tables.iter().find(|(id, _)| *id == 0).expect("a luma table").1[..8].to_vec();
    let chroma = tables.iter().find(|(id, _)| *id == 1).expect("a chroma table").1[..8].to_vec();
    // Step 6 is encoder quality 93, whose tables are these; the encoder scales the Annex-K tables,
    // so the closest it comes to the reference application's own tables is three levels.
    assert_eq!(luma, [2, 2, 1, 2, 3, 6, 7, 9], "luma at step 6");
    assert_eq!(chroma, [2, 3, 3, 7, 14, 14, 14, 14], "chroma at step 6");
    for (name, written, measured) in [("luma", &luma, &measured_luma), ("chroma", &chroma, &measured_chroma)] {
        for (at, (written, measured)) in written.iter().zip(measured).enumerate() {
            assert!(written.abs_diff(*measured) <= 3, "{name} entry {at}: {written} against the measured {measured}");
        }
    }
}

#[test]
fn steps_one_to_four_write_420_and_steps_five_to_seven_write_444() {
    let doc = blocks_doc();
    let subsampled = [(2, 2), (1, 1), (1, 1)];
    let full = [(1, 1); 3];
    for (step, _) in STEPS {
        let want = if step <= 4 { subsampled } else { full };
        assert_eq!(sampling_factors(&encode_jpg(&doc, step)), want, "step {step}");
    }
}

#[test]
fn an_old_1_to_100_quality_loads_as_the_nearest_step() {
    // The value a preference, a saved preset or a recorded Export As step still carries.
    for (carried, want_step) in [(43u32, 3), (51, 3), (85, 6), (100, 7), (8, 1), (50, 3), (7, 7)] {
        let mut f = Map::new();
        f.insert("format".into(), json!("jpg"));
        f.insert("quality".into(), json!(carried));
        let o = options_of(&f);
        assert_eq!(o.quality, u8::try_from(want_step).unwrap(), "a carried {carried} opens as step {want_step}");
    }
    // A tie goes to the lower step, and the encode follows the step it loaded.
    assert_eq!(jpg_quality_step(84), 5, "84 sits between 75 and 93");
    assert_eq!(settings(&jpg_fields(43)).jpeg_quality, Some(50), "a carried 43 encodes at step 3");
    assert_eq!(settings(&jpg_fields(85)).jpeg_quality, Some(93), "the shipped preference default 85 encodes at step 6");
}

#[test]
fn a_quality_the_scale_cannot_hold_still_loads_as_a_step() {
    // 150 and 0 are outside what the settings accept, so the dialog keeps its default, which is
    // step 6: an old value never turns into an error.
    for carried in [json!(150), json!(0), json!(-4), json!("85")] {
        let mut f = Map::new();
        f.insert("format".into(), json!("jpg"));
        f.insert("quality".into(), carried.clone());
        let o = options_of(&f);
        assert_eq!(o.quality, JPG_QUALITY_DEFAULT, "a rejected {carried} keeps the default step");
    }
    for carried in [1u8, 7] {
        assert_eq!(jpg_quality_step(carried), carried, "a step stays itself");
    }
    assert_eq!(jpg_quality_step(0), 1, "below the scale is its first step");
}

#[test]
fn webp_keeps_its_own_1_to_100_quality() {
    for carried in [1u32, 43, 70, 100] {
        let f = json!({ "format": "webp", "quality": carried, "lossless": false }).as_object().unwrap().clone();
        let s = settings(&f);
        assert_eq!(s.webp_quality, Some(u8::try_from(carried).unwrap()), "WebP at {carried}");
        assert_eq!(s.jpeg_quality, None, "WebP writes no JPEG quality");
        let o = options_of(&f);
        assert_eq!(u32::from(o.quality), carried, "WebP's own quality is not put on the JPG scale");
    }
}

#[test]
fn the_formats_without_a_quality_control_are_untouched() {
    for format in ["png", "tif", "tga", "ico", "jxl"] {
        let f = json!({ "format": format, "quality": 43, "scale": 100, "transparency": true, "tgaBits": "24" }).as_object().unwrap().clone();
        let s = settings(&f);
        assert_eq!(s.jpeg_quality, None, "{format} writes no JPEG quality");
        assert!(!s.jpeg_chroma_subsampling, "{format} leaves the JPEG sampling default alone");
        assert_eq!(s.webp_quality, None, "{format} writes no WebP quality");
    }
    let tga = json!({ "format": "tga", "quality": 43, "tgaBits": "24" }).as_object().unwrap().clone();
    assert_eq!(settings(&tga).tga_bits, Some(photocraft_io::TgaBits::Bits24), "TGA bits are not a quality");
}

#[test]
fn export_as_opens_with_step_six_and_a_preference_uses_the_nearest_step() {
    let new_app = |jpeg_quality: u32| -> Box<PhotocraftApp> {
        let services =
            crate::Services { export: Some(Box::new(|_, _, _| Ok((Vec::new(), Vec::new())))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
        if jpeg_quality != 85 {
            app.session.prefs.edit(|p| p.export.jpeg_quality = jpeg_quality);
        }
        app
    };
    // The Export preferences ship 85, and 85 is nearest to step 6: the measured default.
    let mut app = new_app(85);
    open(app.as_mut()).unwrap();
    let opened = app.framework.dialogs.get::<HostedExportAs>().expect("Export As is open").options();
    assert_eq!(opened.quality, JPG_QUALITY_DEFAULT, "fresh preferences open JPG Quality on 6");
    let mut app = new_app(43);
    open(app.as_mut()).unwrap();
    let opened = app.framework.dialogs.get::<HostedExportAs>().expect("Export As is open").options();
    assert_eq!(opened.quality, 3, "a preference of 43 opens on the nearest step");
}
