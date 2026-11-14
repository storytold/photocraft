use super::*;
use craft_psd::descriptor::UnicodeString;
use craft_psd::resources::VersionInfo;
use craft_psd::{Compression, ImageData, ImageResource, PsdError, Version, testgen};

/// #5200: the Multichannel fallback checks its RGBA output against the budget before
/// allocating it (here a 2x2 image, 16 bytes, against caps of 15 and 16 bytes).
#[test]
fn flat_fallback_refuses_an_output_over_the_cap() {
    let file = testgen::merged_only(Version::Psd, PsdMode::Multichannel, 8, Compression::Raw, 2, 2);
    assert!(file.composite_rgba8().is_err(), "Multichannel goes through the fallback");
    let merged = file.decode_merged();
    assert!(matches!(flat_composite(&file, Some(&merged), 15), Err(PsdError::LimitExceeded(_))));
    let img = flat_composite(&file, Some(&merged), 16).unwrap();
    assert_eq!(img.data.len(), 16);
}

/// #5200: a composite over the budget is not retried by the fallback (which allocated the
/// refused buffer: 14.4 GB for this 60000x60000 Bitmap header).
#[test]
fn flat_fallback_does_not_retry_an_over_budget_composite() {
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    file.header.color_mode = PsdMode::Bitmap;
    file.header.depth = 1;
    file.header.channels = 1;
    file.header.width = 60_000;
    file.header.height = 60_000;
    let merged = Ok(vec![0u8; 16]);
    let cap = craft_psd::compression::MAX_DECODED_BYTES;
    assert!(matches!(flat_composite(&file, Some(&merged), cap), Err(PsdError::LimitExceeded(_))));
}

/// #5202: a hostile name length ends the list instead of overflowing.
#[test]
fn unicode_names_stop_at_a_hostile_length() {
    let mut data = 1u32.to_be_bytes().to_vec();
    data.extend([0, b'A']);
    data.extend(u32::MAX.to_be_bytes());
    data.extend([0, b'B']);
    assert_eq!(unicode_names(&data), ["A"]);
}

/// Verifies an unreadable layer channel sets the structured damage indicator.
#[test]
fn unreadable_layer_channel_marks_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == 0).unwrap();
    channel.data.clear();
    assert!(file.layers()[0].decode_channel(0, file.header.depth, file.header.version).is_err());

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("could not be decoded")));
}

/// Verifies an unreadable user-mask channel sets the structured damage indicator.
#[test]
fn unreadable_mask_channel_marks_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == CHANNEL_USER_MASK).unwrap();
    channel.data.clear();

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("could not be decoded")));
}

/// A layer whose bounds hold pixels but whose colour and transparency channels are missing
/// lost its pixel data: the open asks before continuing.
#[test]
fn layer_bounds_without_channel_data_mark_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    file.layers_mut()[0].channels.retain(|c| c.id == CHANNEL_USER_MASK);

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("no channel data")));
}

/// A layer without pixels (an empty layer, or a text or shape layer written without a
/// pixel cache) has empty bounds: nothing is missing, so the file is not damaged even when
/// it lists no colour or transparency channel.
#[test]
fn empty_layer_without_channel_data_is_not_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    file.layers_mut()[0].channels.retain(|c| c.id == CHANNEL_USER_MASK);
    file.layers_mut()[0].rect = Default::default();

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(!damaged, "an empty layer marked the PSD damaged: {warnings:?}");
}

/// Verifies healthy layer channels do not mark the PSD as damaged.
#[test]
fn valid_layer_channels_keep_conversion_editable() {
    let file = testgen::small(Version::Psd, Compression::Raw);
    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(!damaged, "unexpectedly marked good PSD damaged: {warnings:?}");
}

#[test]
fn merged_plane_checks_offsets() {
    let all = [1u8, 2, 3, 4];
    assert_eq!(merged_plane(&all, 1, 2), Some(&[3u8, 4][..]));
    assert_eq!(merged_plane(&all, 2, 2), None);
    assert_eq!(merged_plane(&all, usize::MAX, 2), None);
}

/// Resource 1057's data.
fn version_info(writer: UnicodeString, reader: UnicodeString, has_real_merged_data: bool) -> Vec<u8> {
    VersionInfo { version: 1, has_real_merged_data, writer, reader, file_version: 1 }.to_bytes()
}

/// The 1057 PhotoCraft 0.1.0 to 0.7.0 wrote: Photoshop CS6's names, NUL-terminated.
fn version_info_cs6(has_real_merged_data: bool) -> Vec<u8> {
    version_info(UnicodeString::new_nul("Adobe Photoshop"), UnicodeString::new_nul("Adobe Photoshop CS6"), has_real_merged_data)
}

/// The 1057 PhotoCraft writes since 0.8.2: Photoshop 2026's bytes, no NUL.
fn version_info_2026(has_real_merged_data: bool) -> Vec<u8> {
    craft_psd::resources::version_info_resource(has_real_merged_data).data
}

/// An opaque 4 × 1 RGB document with the Photoshop spot oracles' channels in their order:
/// "Spot A", "Alpha 1" and "Spot B", each holding 100, 25, 75 and 0 % in four columns.
fn channels_document(depth: SampleType) -> Document {
    let mut doc = Document::with_background("old", Size::new(4, 1), ColorMode::Rgb, depth, photocraft_color::Color::rgb(1.0, 1.0, 1.0));
    let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let canvas = doc.bounds();
    let channel = |name: &str, spot: Option<(photocraft_color::Color, f32)>| {
        let mut s = Surface::new(gray);
        s.write_region(canvas, &[1.0, 0.25, 0.75, 0.0]);
        AlphaChannel { spot, ..AlphaChannel::new(name, s) }
    };
    doc.channels.push(channel("Spot A", Some((photocraft_color::Color::rgb(0.0, 0.627, 1.0), 0.0))));
    doc.channels.push(channel("Alpha 1", None));
    doc.channels.push(channel("Spot B", Some((photocraft_color::Color::rgb(1.0, 0.0, 0.627), 0.5))));
    doc
}

/// Each channel's values on an 8-bit scale.
fn ink(doc: &Document) -> Vec<Vec<u8>> {
    doc.channels.iter().map(|c| c.surface.read_region(doc.bounds()).iter().map(|v| (v * 255.0).round() as u8).collect()).collect()
}

fn spots(doc: &Document) -> Vec<bool> {
    doc.channels.iter().map(|c| c.spot.is_some()).collect()
}

/// `doc` (opaque, 8 or 16-bit) saved the way PhotoCraft 0.1.0 to 0.16.0 saved it: spot planes
/// straight (the maximum = solid ink), no 1053, and `version_info` as resource 1057. `padded`
/// lays DisplayInfo out in the 14-byte records of 0.1.0 to 0.7.0.
fn as_earlier_photocraft_saved(doc: &Document, version_info: Vec<u8>, padded: bool) -> Vec<u8> {
    let mut file = crate::psd_export::document_to_psd(doc).unwrap();
    assert!(!file.merged_has_alpha());
    file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS);
    for r in &mut file.resources {
        match r.id {
            ids::VERSION_INFO => r.data = version_info.clone(),
            crate::channel_map::DISPLAY_INFO if padded => {
                let (version, records) = r.data.split_at(4);
                r.data = version.iter().copied().chain(records.chunks(13).flat_map(|rec| rec.iter().copied().chain([0]))).collect();
            }
            _ => {}
        }
    }
    let mut planes = file.decode_merged().unwrap();
    let plane = planes.len() / usize::from(file.header.channels);
    for (k, c) in doc.channels.iter().enumerate() {
        if c.spot.is_some() {
            // At 8 and 16 bits the complement of a sample is that of each of its bytes.
            planes[(3 + k) * plane..(4 + k) * plane].iter_mut().for_each(|b| *b = !*b);
        }
    }
    file.image_data = ImageData::encode(Compression::Raw, &planes, &file.header).unwrap();
    file.to_bytes().unwrap()
}

/// PSDs that PhotoCraft 0.1.0 to 0.16.0 saved with spot channels store them straight. They
/// open with the ink they had in those versions (which read every plane straight), whichever
/// 1057 they carry. Saved again, they get Photoshop's polarity and a 1053, and reopen with the
/// same ink. (A 0.1.0 to 0.7.0 file's padded records misread every channel's options after
/// the first, here Spot B's kind, as those versions' successors always have; its ink is kept.)
#[test]
fn spot_planes_from_earlier_photocraft_keep_their_ink() {
    for depth in [SampleType::U8, SampleType::U16] {
        let doc = channels_document(depth);
        let cases = [
            ("0.1.0 to 0.7.0", version_info_cs6(true), true),
            ("0.8.2 on", version_info_2026(true), false),
            ("0.8.2 on, Maximize Compatibility off", version_info_2026(false), false),
            ("0.1.0 to 0.7.0, 13-byte records", version_info_cs6(true), false),
        ];
        for (what, version_info, padded) in cases {
            let bytes = as_earlier_photocraft_saved(&doc, version_info, padded);
            assert!(earlier_photocraft_spot_planes(&PsdFile::from_bytes(&bytes).unwrap()), "{what}");
            let opened = crate::import("old.psd", &bytes).unwrap().document;
            assert_eq!(ink(&opened), ink(&doc), "{depth:?} {what}: the ink earlier versions opened");
            assert!(spots(&opened)[0], "{depth:?} {what}");
            if !padded {
                assert_eq!(spots(&opened), spots(&doc), "{depth:?} {what}");
            }
            let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
            let file = PsdFile::from_bytes(&saved.bytes).unwrap();
            assert!(file.resource(crate::channel_map::ALPHA_IDENTIFIERS).is_some() && !earlier_photocraft_spot_planes(&file), "{what}");
            let again = crate::import("again.psd", &saved.bytes).unwrap().document;
            assert_eq!(ink(&again), ink(&doc), "{depth:?} {what}: saved again");
            assert_eq!(spots(&again), spots(&opened), "{depth:?} {what}: saved again");
        }
    }
}

/// A 0.1.0 to 0.7.0 file whose first extra channel is an alpha channel, spots after it. In its
/// 14-byte DisplayInfo records the spots' kind bytes sit where a 13-byte reader (main's, and
/// this one's) doesn't look, so only the padded reading finds a spot for the rule. Every plane
/// is then read straight, as those versions read it, and keeps its ink. The 13-byte reader
/// opens each channel's options from shifted bytes, as on main: here every channel opens as
/// an alpha channel. A spot of 2 % solidity puts a 2 where that reader looks for the kind, so
/// that channel opens as a spot: without the rule its plane would be inverted. Saved again and
/// reopened, every channel keeps its ink and kind.
#[test]
fn an_early_file_with_an_alpha_channel_first_keeps_its_ink() {
    for solidity in [0.0, 0.02] {
        let mut doc = channels_document(SampleType::U8);
        doc.channels.swap(0, 1);
        doc.channels[1].spot = Some((photocraft_color::Color::rgb(0.0, 0.627, 1.0), solidity));
        let names: Vec<&str> = doc.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Alpha 1", "Spot A", "Spot B"]);
        let bytes = as_earlier_photocraft_saved(&doc, version_info_cs6(true), true);
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let info = &file.resource(crate::channel_map::DISPLAY_INFO).unwrap().data;
        let misread = solidity > 0.0;
        assert_eq!(crate::channel_map::spot_flags(info, true, 0), [false, misread, false], "{solidity}: 13-byte records");
        assert!(earlier_photocraft_spot_planes(&file), "{solidity}: found in the 14-byte records");
        let opened = crate::import("old.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{solidity}: the ink 0.7.0 opened");
        assert_eq!(spots(&opened), [false, misread, false], "{solidity}: kinds as main reads them");
        let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
        let again = crate::import("again.psd", &saved.bytes).unwrap().document;
        assert_eq!(ink(&again), ink(&doc), "{solidity}: saved again");
        assert_eq!(spots(&again), spots(&opened), "{solidity}: saved again");
    }
}

/// A file with spots and no 1053 that no earlier PhotoCraft wrote is read in Photoshop's
/// polarity (0 = solid ink). psd-tools' `cmyk-spot.psd` is one: its 1057 names psd-tools as
/// writer and reader. So is a file with no 1057, with Photoshop CS6's names without the NUL,
/// or with bytes after PhotoCraft's.
#[test]
fn other_files_without_alpha_identifiers_use_photoshop_polarity() {
    let doc = channels_document(SampleType::U8);
    let mut trailing = version_info_2026(true);
    trailing.extend([0, 0]);
    let cases = [
        ("psd-tools", Some(version_info(UnicodeString::new("psd-tools 1.10.0"), UnicodeString::new("psd-tools 1.10.0"), true))),
        ("no 1057", None),
        ("CS6 without a NUL", Some(version_info(UnicodeString::new("Adobe Photoshop"), UnicodeString::new("Adobe Photoshop CS6"), true))),
        ("bytes after PhotoCraft's 1057", Some(trailing)),
    ];
    for (what, info) in cases {
        // Photoshop's polarity, as this version writes it, without 1053.
        let mut file = crate::psd_export::document_to_psd(&doc).unwrap();
        file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS && r.id != ids::VERSION_INFO);
        file.resources.extend(info.map(|d| ImageResource::new(ids::VERSION_INFO, d)));
        let bytes = file.to_bytes().unwrap();
        assert!(!earlier_photocraft_spot_planes(&PsdFile::from_bytes(&bytes).unwrap()), "{what}");
        let opened = crate::import("x.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{what}");
        assert_eq!(spots(&opened), spots(&doc), "{what}");
    }
}

/// The rule needs a spot in DisplayInfo and no 1053, besides PhotoCraft's 1057.
#[test]
fn the_earlier_photocraft_rule_needs_a_spot_and_no_alpha_identifiers() {
    let mut doc = channels_document(SampleType::U8);
    let old = PsdFile::from_bytes(&as_earlier_photocraft_saved(&doc, version_info_2026(true), false)).unwrap();
    assert!(earlier_photocraft_spot_planes(&old));
    let mut with_ids = old.clone();
    with_ids.resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, crate::channel_map::alpha_identifiers(false, 3)));
    assert!(!earlier_photocraft_spot_planes(&with_ids), "a 1053");
    let mut no_info = old.clone();
    no_info.resources.retain(|r| r.id != crate::channel_map::DISPLAY_INFO);
    assert!(!earlier_photocraft_spot_planes(&no_info), "no DisplayInfo");
    doc.channels.retain(|c| c.spot.is_none());
    let alpha_only = PsdFile::from_bytes(&as_earlier_photocraft_saved(&doc, version_info_2026(true), false)).unwrap();
    assert!(!earlier_photocraft_spot_planes(&alpha_only), "alpha channels only");
}

/// `doc` as an earlier PhotoCraft saved a Photoshop file it had opened with the first `kept`
/// of `doc`'s channels: it wrote back the Photoshop file's 1053, `stale_ids`, unchanged and
/// those channels' planes as Photoshop stored them (it had read and written them straight),
/// and stored the channels added after them straight.
fn as_earlier_photocraft_resaved(doc: &Document, stale_ids: &[u32], kept: usize) -> Vec<u8> {
    let mut file = crate::psd_export::document_to_psd(doc).unwrap();
    file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS && r.id != ids::VERSION_INFO);
    file.resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, stale_ids.iter().flat_map(|id| id.to_be_bytes()).collect()));
    file.resources.push(ImageResource::new(ids::VERSION_INFO, version_info_2026(true)));
    let mut planes = file.decode_merged().unwrap();
    let plane = planes.len() / usize::from(file.header.channels);
    let first = usize::from(file.header.channels) - doc.channels.len();
    for (k, c) in doc.channels.iter().enumerate().skip(kept) {
        if c.spot.is_some() {
            planes[(first + k) * plane..(first + k + 1) * plane].iter_mut().for_each(|b| *b = !*b);
        }
    }
    file.image_data = ImageData::encode(Compression::Raw, &planes, &file.header).unwrap();
    file.to_bytes().unwrap()
}

/// An earlier PhotoCraft kept the 1053 of a Photoshop file it opened and wrote it back
/// unchanged, so a spot channel added there sits behind a 1053 that doesn't list it: a
/// transparent Photoshop document's `[0]` (the white plate of a DTF print, added before
/// saving), the `[0, 3]` of one with a spot channel already, or a `[0]` kept through a
/// flatten that removed the transparency. The added spot is stored straight and keeps its
/// ink; the Photoshop file's own spot keeps Photoshop's polarity. Saved again, both reopen
/// the same.
#[test]
fn a_spot_added_behind_a_kept_alpha_identifiers_resource_keeps_its_ink() {
    let transparent = |names: &[&str]| {
        let mut doc = channels_document(SampleType::U8);
        doc.layers = vec![Layer::raster("Layer 1", doc.pixel_format())];
        doc.channels.retain(|c| names.contains(&c.name.as_str()));
        doc
    };
    let mut flattened = channels_document(SampleType::U8);
    flattened.channels.retain(|c| c.name == "Spot A");
    let cases = [
        ("transparent, one spot added", transparent(&["Spot A"]), vec![0], 0),
        ("transparent, a Photoshop spot and one added", transparent(&["Spot A", "Spot B"]), vec![0, 3], 1),
        ("flattened, one spot added", flattened, vec![0], 0),
    ];
    for (what, doc, stale_ids, kept) in cases {
        let bytes = as_earlier_photocraft_resaved(&doc, &stale_ids, kept);
        let file = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.merged_has_alpha(), what.starts_with("transparent"), "{what}");
        assert!(earlier_photocraft_spot_planes(&file), "{what}");
        let opened = crate::import("old.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{what}: the ink earlier versions opened");
        assert_eq!(spots(&opened), spots(&doc), "{what}");
        let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
        let again = crate::import("again.psd", &saved.bytes).unwrap().document;
        assert_eq!(ink(&again), ink(&doc), "{what}: saved again");
    }
}

/// A 1053 that lists every extra channel, as Photoshop and this version write it, keeps the
/// file in Photoshop's polarity, with PhotoCraft's (Photoshop 2026's) 1057 too: a transparent
/// file with a spot, and Photoshop's own `[0]` in a file without a transparency plane.
#[test]
fn a_complete_alpha_identifiers_resource_keeps_photoshop_polarity() {
    let mut doc = channels_document(SampleType::U8);
    doc.layers = vec![Layer::raster("Layer 1", doc.pixel_format())];
    let bytes = as_earlier_photocraft_resaved(&doc, &[0, 3, 4, 5], 3);
    let file = PsdFile::from_bytes(&bytes).unwrap();
    assert!(file.merged_has_alpha());
    assert!(!earlier_photocraft_spot_planes(&file));
    assert_eq!(ink(&crate::import("ps.psd", &bytes).unwrap().document), ink(&doc));
    let mut opaque = channels_document(SampleType::U8);
    opaque.channels.clear();
    let mut file = PsdFile::from_bytes(&as_earlier_photocraft_resaved(&opaque, &[0], 0)).unwrap();
    file.resources.retain(|r| r.id != crate::channel_map::DISPLAY_INFO);
    file.resources.push(ImageResource::new(
        crate::channel_map::DISPLAY_INFO,
        crate::channel_map::display_info(&channels_document(SampleType::U8).channels.iter().collect::<Vec<_>>(), false),
    ));
    assert_eq!(photoshop_identified_channels(&file), 0, "0 is the transparency's");
    assert!(!earlier_photocraft_spot_planes(&file), "no channel left unlisted");
}

#[test]
fn layer_bands_end_on_tile_rows_and_cover_the_layer() {
    let t = TILE_SIZE as usize;
    for (top, h, row_bytes) in [(0, 2000, 8000), (-37, 15000, 80_000), (100, 5, 4), (255, 1, 4), (-256, 40_000, 1 << 20), (7, 0, 4)] {
        let bands = band_rows(top, h, row_bytes);
        let mut next = 0;
        for &(a, b) in &bands {
            assert_eq!(a, next);
            assert!(b > a);
            if b < h {
                assert_eq!((top + b as i32).rem_euclid(TILE_SIZE), 0, "{top} {h}: band ends inside a tile row");
                assert!((b - a) * row_bytes <= BAND_BYTES + t * row_bytes || b - a <= t);
            }
            next = b;
        }
        assert_eq!(next, h);
    }
    // A small layer is one band.
    assert_eq!(band_rows(0, 2000, 8000), vec![(0, 2000)]);
}

// Tall, narrow masks cross tile bands while keeping these cases only a few KiB.
fn banded_mask_file(depth: u16, compression: Compression, version: Version, real: bool) -> (PsdFile, Vec<u8>, craft_psd::Rect) {
    use craft_psd::{ChannelData, LayerMask as PsdMask, MaskData, MaskParameters, RealMask};
    let mut file = testgen::layered(version, PsdMode::Rgb, depth, compression);
    file.layer_info.as_mut().unwrap().layers.truncate(1);
    let rect = craft_psd::Rect::from_xywh(-3, -17, 5, 600);
    let mut plane = Vec::new();
    for i in 0..3000 {
        match depth {
            8 => plane.push((i % 251) as u8),
            16 => plane.extend(((i * 73) as u16).to_be_bytes()),
            32 => plane.extend(((i % 251) as f32 / 125.0 - 0.5).to_be_bytes()),
            _ => panic!("unexpected test case"),
        }
    }
    let mut mask = PsdMask::new(rect, 0, 3 | 16);
    mask.flags |= PsdMask::FLAG_PARAMETERS;
    mask.parameters = Some(MaskParameters { flags: 3, user_density: Some(123), user_feather: Some(2.5), ..Default::default() });
    let record = &mut file.layers_mut()[0];
    record.channels.retain(|c| c.id != CHANNEL_USER_MASK && c.id != CHANNEL_REAL_USER_MASK);
    let id = if real {
        // Different synthetic bounds/defaults ensure -3's own metadata is selected.
        mask.rect = craft_psd::Rect::from_xywh(1, 1, 1, 1);
        mask.real = Some(RealMask { flags: 3, background: 255, rect });
        record.channels.push(ChannelData::encode(CHANNEL_USER_MASK, compression, &vec![0; usize::from(depth / 8)], 1, 1, depth, version).unwrap());
        CHANNEL_REAL_USER_MASK
    } else {
        CHANNEL_USER_MASK
    };
    record.mask = MaskData::Mask(mask);
    record.channels.push(ChannelData::encode(id, compression, &plane, 5, 600, depth, version).unwrap());
    (file, plane, rect)
}

fn assert_banded_mask(file: &PsdFile, plane: &[u8], rect: craft_psd::Rect, real: bool) {
    let (doc, warnings, damaged) = psd_to_document_decoded_integrity(file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(!damaged, "{warnings:?}");
    let mask = doc.layers[0].mask.as_ref().unwrap();
    let sample = sample_for_depth(file.header.depth);
    let expected = crate::pixels::interleave(&[Some(plane)], &[zero_sample(sample)], 3000, sample, &[false]);
    assert_eq!(mask.surface.to_interleaved(Rect::new(rect.left, rect.top, rect.right, rect.bottom)), expected);
    assert_eq!(mask.surface.pixel(-4, -18), vec![if real { 1.0 } else { 0.0 }]);
    assert!(!mask.enabled);
    assert!(!mask.linked);
    assert_eq!(mask.density, 123.0 / 255.0);
    assert_eq!(mask.feather, 2.5);
}

#[test]
fn mask_bands_preserve_samples_bounds_defaults_and_parameters() {
    for depth in [8, 16, 32] {
        for compression in Compression::ALL {
            for version in [Version::Psd, Version::Psb] {
                for real in [false, true] {
                    let (file, plane, rect) = banded_mask_file(depth, compression, version, real);
                    assert_banded_mask(&file, &plane, rect, real);
                    let bytes = file.to_bytes().unwrap();
                    let eager = PsdFile::from_bytes(&bytes).unwrap();
                    let lazy = PsdFile::from_source(craft_psd::Source::new(bytes)).unwrap();
                    assert_banded_mask(&eager, &plane, rect, real);
                    assert_banded_mask(&lazy, &plane, rect, real);
                }
            }
        }
    }
}

#[test]
fn raw_and_rle_mask_sources_are_read_in_bands() {
    use craft_psd::{ReadAt, Source, Stored};
    use std::sync::{Arc, Mutex};
    struct Reads {
        bytes: Vec<u8>,
        sizes: Arc<Mutex<Vec<usize>>>,
    }
    impl ReadAt for Reads {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> craft_psd::Result<()> {
            self.sizes.lock().unwrap().push(buf.len());
            self.bytes.read_at(offset, buf)
        }
    }
    for depth in [8, 16, 32] {
        for compression in [Compression::Raw, Compression::Rle] {
            for real in [false, true] {
                let (mut file, plane, rect) = banded_mask_file(depth, compression, Version::Psb, real);
                let id = if real { CHANNEL_REAL_USER_MASK } else { CHANNEL_USER_MASK };
                let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == id).unwrap();
                let bytes = std::mem::take(&mut channel.data);
                let len = bytes.len();
                let sizes = Arc::new(Mutex::new(Vec::new()));
                channel.stored = Some(Stored { source: Source::new(Reads { bytes, sizes: sizes.clone() }), offset: 0, len: len as u64 });
                assert_banded_mask(&file, &plane, rect, real);
                let reads = sizes.lock().unwrap();
                match compression {
                    Compression::Raw => {
                        assert_eq!(*reads, [17, 256, 256, 71].map(|rows| 5 * rows * usize::from(depth / 8)).to_vec())
                    }
                    Compression::Rle => {
                        assert_eq!(reads.len(), 5, "one index read and four payload bands: {reads:?}");
                        assert_eq!(reads[0], 600 * 4, "PSB row index");
                    }
                    _ => panic!("unexpected test case"),
                }
                assert!(reads.iter().all(|&n| n < len), "read whole {len}-byte mask: {reads:?}");
            }
        }
    }
}

#[test]
fn a_damaged_later_mask_band_discards_earlier_bands() {
    let (mut file, _, rect) = banded_mask_file(8, Compression::Rle, Version::Psd, false);
    let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == CHANNEL_USER_MASK).unwrap();
    channel.data = [0, 2].repeat(600);
    for y in 0..600 {
        // Each valid row repeats five bytes; row 400 promises two literal bytes but has one.
        channel.data.extend(if y == 400 { [1, 9] } else { [0xFC, 7] });
    }
    assert!(channel.row_decoder(5, 600, 8, Version::Psd).unwrap().rows(0, 17).is_ok());
    let (doc, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("channel -2 could not be decoded")));
    let mask = doc.layers[0].mask.as_ref().unwrap();
    assert_eq!(mask.surface.tile_count(), 0);
    assert_eq!(mask.surface.pixel(rect.left, rect.top), vec![0.0]);
}

/// An allocation or read that fails while the layers are built fails the open, and the merged
/// image's extra channels are not converted on the way out: that is another full plane per
/// channel, asked for after the import was already reported as failing.
#[test]
fn a_fatal_layer_error_skips_the_extra_channel_conversion() {
    use craft_psd::ReadAt;
    struct Failing {
        bytes: Vec<u8>,
        data: Vec<(u64, u64)>,
        armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl ReadAt for Failing {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> craft_psd::Result<()> {
            let end = offset + buf.len() as u64;
            if self.armed.load(std::sync::atomic::Ordering::SeqCst) && self.data.iter().any(|&(a, b)| offset < b && end > a) {
                return Err(PsdError::Read("the drive went away".into()));
            }
            self.bytes.as_slice().read_at(offset, buf)
        }
    }
    let bytes = crate::psd_export::document_to_psd(&channels_document(SampleType::U8)).unwrap().to_bytes().unwrap();
    let whole = PsdFile::from_bytes(&bytes).unwrap();
    assert!(merged_needed(&whole), "the extra channels come from the merged image");
    let converted = psd_to_document_global(&whole, &photocraft_raster::Interrupt::NONE, None, Vec::new(), false).unwrap();
    assert!(converted.3.is_none() && converted.0.channels.len() == 3, "the control file converts its channels");

    let lazy = PsdFile::from_source(craft_psd::Source::new(bytes.clone())).unwrap();
    let data: Vec<(u64, u64)> =
        lazy.layers().iter().flat_map(|l| &l.channels).filter_map(|c| c.stored.as_ref()).filter(|s| s.len > 0).map(|s| (s.offset, s.offset + s.len)).collect();
    assert!(!data.is_empty());
    // Reads fail only once the file is parsed: the layers decode lazily from the source.
    let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let file = PsdFile::from_source(craft_psd::Source::new(Failing { bytes, data, armed: armed.clone() })).unwrap();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let (doc, _, _, fatal) = psd_to_document_global(&file, &photocraft_raster::Interrupt::NONE, None, Vec::new(), false).unwrap();
    assert!(matches!(fatal, Some(crate::IoError::Read(_))), "{fatal:?}");
    assert!(doc.channels.is_empty(), "no extra channel was converted after the failure");
}

/// Only a failed allocation fails the open; a size limit or damaged compressed data still
/// leaves the layer empty with a damage warning, and the rest of the file opens. A failed
/// allocation names the layer and its channel.
#[test]
fn only_allocation_failures_are_fatal_to_a_layer_channel() {
    let file = testgen::small(Version::Psd, Compression::Raw);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut cx = Ctx {
        file: &file,
        global: Vec::new(),
        fmt,
        mask_fmt: PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false),
        cc: 3,
        cmyk: false,
        warnings: Vec::new(),
        damaged: false,
        fatal_error: None,
        dpi: 72.0,
        txt2: None,
        filter_effects: Vec::new(),
        filter_masks: Default::default(),
        unfiltered: Default::default(),
        smart_filter_budget: Default::default(),
        placed_uses: Default::default(),
        smart_contents: Default::default(),
        plld_sizes: Default::default(),
        vector_previews: Default::default(),
        ctl: photocraft_raster::Interrupt::NONE,
        done: 0,
        uniform: Default::default(),
        total: 0,
    };
    assert!(!cx.channel_failed("Layer 1", 0, &PsdError::LimitExceeded("decoded channel data exceeds MAX_DECODED_BYTES")));
    assert!(cx.fatal_error.is_none() && cx.damaged);
    assert!(cx.channel_failed("Layer 1", 2, &PsdError::OutOfMemory("not enough memory for the decoded channel data")));
    match cx.fatal_error {
        Some(crate::IoError::OutOfMemory(message)) => assert!(message.contains("layer \"Layer 1\" channel 2"), "{message}"),
        other => panic!("{other:?}"),
    }
}

/// A standalone legacy conversion that is over budget is refused *before* it waits for the
/// decode gate, so an ineligible file never blocks behind an active decode. The legacy API keeps
/// its warning (the ordinary import path still receives the fatal error).
#[test]
fn legacy_conversion_refuses_before_waiting_for_the_decode_gate() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    // 30,000² tiles are several GiB: far past the hard budget of the 4 GiB machine below.
    file.layers_mut()[0].rect = craft_psd::Rect::from_xywh(0, 0, 30_000, 30_000);
    let snapshot = crate::memory::MemorySnapshot { available_ram: Some(4 << 30), swap_free: None, commit_headroom: None };
    let ctl = photocraft_raster::Interrupt::NONE;
    let _guard = crate::psd_gate::acquire_decode_admission(&ctl).expect("active decode");

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let (doc, warnings) = crate::memory::with_mock_memory(snapshot, || crate::psd_to_document(&file));
        tx.send((doc.layers.len(), warnings)).ok();
    });

    // The refusal arrives while this thread still holds the gate: the legacy call never waited.
    let (layers, warnings) = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("a refused legacy conversion must not wait for the decode gate");
    assert_eq!(layers, 0, "a refused conversion returns an empty document");
    assert!(warnings.iter().any(|w| w.contains("pixel memory")), "{warnings:?}");

    drop(_guard);
    handle.join().expect("worker thread");
}

fn test_ctx(file: &PsdFile) -> Ctx<'_> {
    Ctx {
        file,
        global: Vec::new(),
        fmt: PixelFormat::new(ColorMode::Rgb, SampleType::U8, true),
        mask_fmt: PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false),
        cc: 3,
        cmyk: false,
        warnings: Vec::new(),
        damaged: false,
        fatal_error: None,
        dpi: 72.0,
        txt2: None,
        filter_effects: Vec::new(),
        filter_masks: Default::default(),
        unfiltered: Default::default(),
        smart_filter_budget: Default::default(),
        placed_uses: Default::default(),
        smart_contents: Default::default(),
        plld_sizes: Default::default(),
        vector_previews: Default::default(),
        ctl: photocraft_raster::Interrupt::NONE,
        done: 0,
        uniform: Default::default(),
        total: 0,
    }
}

/// An inverted, solid-filled shape record without pixels (`vsms` with only the initial fill).
fn bare_shape_record(extra: Vec<TaggedBlock>) -> LayerRecord {
    use craft_psd::path::{PathData, VectorMaskBlock};
    let vsms = VectorMaskBlock { version: 3, flags: 0, path: PathData::from_subpaths(&[], true) }.to_bytes();
    let (key, data) = blocks::write_fill(&photocraft_doc::Fill::Solid(photocraft_color::Color::BLACK));
    let mut all = vec![TaggedBlock::new(*b"vsms", vsms), TaggedBlock::new(key, data)];
    all.extend(extra);
    LayerRecord { name: b"shape".to_vec(), blocks: all, ..Default::default() }
}

/// The budget charges exactly the shapes the importer renders: a shape record that is really
/// another kind of layer (adjustment, type, placed), or keeps its stored pixels, is not charged.
#[test]
fn shape_render_area_follows_the_importer() {
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    file.header.width = 1000;
    file.header.height = 600;
    let canvas = Rect::new(0, 0, 1000, 600);
    let shape = bare_shape_record(Vec::new());
    assert_eq!(shape_render_area(&file, &shape, 72.0).map(|(area, _)| area), Some(canvas));
    let other = |key: &[u8; 4]| bare_shape_record(vec![TaggedBlock::new(*key, vec![0; 4])]);
    for key in [b"levl", b"TySh", b"SoLd", b"SoLE", b"PlLd"] {
        assert_eq!(shape_render_area(&file, &other(key), 72.0).map(|(area, _)| area), None, "{}", String::from_utf8_lossy(key));
    }
    // Stored pixels stay the rendering of a hard shape.
    let mut stored = shape.clone();
    stored.rect = craft_psd::Rect::from_xywh(0, 0, 4, 3);
    assert_eq!(shape_render_area(&file, &stored, 72.0).map(|(area, _)| area), None);
    // The importer makes each of these the content the area says.
    for (rec, rendered) in [(&shape, true), (&other(b"TySh"), false), (&stored, false)] {
        let layer = test_ctx(&file).layer_from_record(rec);
        let shape_cache = match &layer.content {
            LayerContent::Shape(sh) => sh.cache.as_ref().map(Surface::content_bounds),
            _ => None,
        };
        assert_eq!(shape_cache == Some(canvas), rendered, "{:?}", layer.content);
    }
}

/// The estimate reads the resolution the import gives the document (stroke widths in points
/// depend on it).
#[test]
fn the_estimate_reads_the_document_resolution() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    assert_eq!(resolution_dpi(&file), psd_to_document(&file).0.resolution_dpi);
    let info = craft_psd::ResolutionInfo::from_dpi(300.0);
    file.resources.push(ImageResource::new(ids::RESOLUTION_INFO, info.to_bytes()));
    let (doc, _) = psd_to_document(&file);
    assert_eq!(doc.resolution_dpi, 300.0);
    assert_eq!(resolution_dpi(&file), doc.resolution_dpi);
}

/// A smart object over testgen's first layer (4x3 stored pixels at the origin) with a Gaussian
/// Blur, and its `FEid` item: green unfiltered pixels over `source`, with a filter mask that
/// hides the filter over `hidden` (where the mix shows the green).
fn filtered_smart_object(file: &PsdFile, source: Rect, hidden: Rect) -> (LayerRecord, craft_psd::filter_effects::FilterEffectsItem) {
    masked_smart_object(file, "placed-1", source, hidden, 0.0)
}

/// [`filtered_smart_object`] with the given `placed` id and a mask of `level` over `hidden`.
fn masked_smart_object(file: &PsdFile, placed: &str, source: Rect, hidden: Rect, level: f32) -> (LayerRecord, craft_psd::filter_effects::FilterEffectsItem) {
    use crate::smart_map::{FilterStack, PlacedSpec, feid_item, filter_fx, sold_bytes};
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut unfiltered = Surface::new(fmt);
    unfiltered.fill_rect(source, &[0.0, 1.0, 0.0, 1.0]);
    let mut hide = Surface::with_default(mask_fmt, &[1.0]);
    hide.fill_rect(hidden, &[level]);
    let mask = LayerMask { surface: hide, enabled: true, linked: false, density: 1.0, feather: 0.0 };
    let item = feid_item(placed, &unfiltered, Some(&mask), source, fmt).expect("item");
    let blur = photocraft_doc::SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({ "radius": 1.0 }),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    };
    let stack = FilterStack { enabled: true, filters: vec![blur], mask_enabled: true, mask_linked: false };
    let spec = PlacedSpec {
        idnt: "source",
        placed,
        transform: photocraft_geom::Affine::IDENTITY,
        perspective: None,
        size: (4.0, 3.0),
        dpi: 72.0,
        warp: None,
        filter_fx: Some(filter_fx(&stack, "s", &mut Vec::new())),
    };
    let mut rec = file.layers()[0].clone();
    rec.blocks.push(TaggedBlock::new(*b"SoLd", sold_bytes(None, &spec, &mut Vec::new())));
    (rec, item)
}

/// Smart objects sharing one `FEid` source each mix their own copy of it through the filter
/// mask: the copies are charged to the import's smart-filter cache budget, and past it a layer
/// keeps its stored pixels with a warning, however many layers name the source.
#[test]
fn filter_mask_mixes_are_charged_per_layer() {
    use crate::smart_map::FilterCacheBudget;
    let file = testgen::small(Version::Psd, Compression::Raw);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    // The unfiltered source sits a tile right of the stored pixels, its left half where the
    // mask hides the filter (green once mixed): every mixed copy adds a tile.
    let (rec, item) = filtered_smart_object(&file, Rect::new(256, 0, 260, 3), Rect::new(256, 0, 258, 3));
    let tile = photocraft_raster::stored_tiles_bytes(1, &fmt);
    let mask_tile = photocraft_raster::stored_tiles_bytes(1, &mask_fmt);

    let mixed_layers = |cap: u64, layers: usize| {
        let mut cx = test_ctx(&file);
        cx.filter_effects = vec![item.clone()];
        cx.smart_filter_budget = FilterCacheBudget::new(cap);
        let mixed = (0..layers)
            .filter(|_| match cx.layer_from_record(&rec).content {
                LayerContent::Smart(sm) => {
                    let mixed = sm.cache.is_some_and(|c| c.pixel(256, 0) == vec![0.0, 1.0, 0.0, 1.0]);
                    // A mask is kept only over pixels it was mixed into (saving unmixes them).
                    assert_eq!(sm.filter_mask.is_some(), mixed);
                    mixed
                }
                other => panic!("{other:?}"),
            })
            .count();
        assert!(cx.smart_filter_budget.used() <= cap);
        assert!(cx.fatal_error.is_none());
        (mixed, cx.warnings)
    };
    // The mask, the source and every layer's copy fit: all are mixed, without a word.
    let (mixed, warnings) = mixed_layers(mask_tile + tile + 8 * tile, 8);
    assert_eq!(mixed, 8, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    // Room for three copies: three layers are mixed, the others keep their stored pixels.
    let (mixed, warnings) = mixed_layers(mask_tile + tile + 3 * tile, 8);
    assert_eq!(mixed, 3);
    assert_eq!(warnings.iter().filter(|w| w.contains("filter mask was ignored")).count(), 5, "{warnings:?}");
    // No room for the mask: it is ignored before it is decoded, so nothing is mixed.
    let (mixed, warnings) = mixed_layers(mask_tile - 1, 2);
    assert_eq!(mixed, 0);
    assert!(warnings.iter().any(|w| w.contains("too large") && w.contains("filter mask was ignored")), "{warnings:?}");
}

/// Mixing a smart object's stored pixels with its unfiltered source visits only the tiles the
/// two hold, not every tile of the rectangle between them: a source a million pixels away from
/// the stored layer (a few bytes in the file) costs two tiles, not fifteen million.
#[test]
fn a_far_filter_source_mixes_without_walking_the_tiles_between() {
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    let far = Rect::new(1_000_000, 1_000_000, 1_000_004, 1_000_003);
    let (rec, item) = filtered_smart_object(&file, far, far);
    file.layers_mut().clear();
    file.layers_mut().push(rec);
    let mut fx = craft_psd::filter_effects::FilterEffects { version: 3, items: vec![item] }.to_bytes();
    fx.resize(fx.len().next_multiple_of(4), 0);
    file.global_blocks.push(TaggedBlock::new(*b"FEid", fx));
    let bytes = file.to_bytes().unwrap();

    let before = photocraft_compose::filter_mask::tiles_visited();
    let imported = crate::import("far.psb", &bytes).expect("import");
    let visited = photocraft_compose::filter_mask::tiles_visited() - before;
    assert!(visited <= 4, "{visited} tiles visited");
    let LayerContent::Smart(sm) = &imported.document.layers[0].content else { panic!("{:?}", imported.document.layers[0].content) };
    let cache = sm.cache.as_ref().expect("pixels");
    assert_eq!(cache.pixel(1_000_000, 1_000_000), vec![0.0, 1.0, 0.0, 1.0], "the mask shows the unfiltered source there");
    let stored = test_ctx(&file).record_surface(&file.layers()[0], "a");
    assert_eq!(cache.pixel(1, 1), stored.pixel(1, 1), "and the filtered pixels elsewhere");
}

/// A PSB of testgen's first layer as smart objects, one per `(placed id, mask level)`, each with
/// its own `FEid` item (green unfiltered pixels, a mask of that level over the layer).
fn masked_smart_psb(objects: &[(&str, f32)]) -> (Vec<u8>, Vec<Vec<u8>>) {
    masked_smart_psb_over(&objects.iter().map(|(p, l)| (*p, *l, Rect::new(0, 0, 4, 3))).collect::<Vec<_>>())
}

/// [`masked_smart_psb`] with each object's unfiltered source (and mask) over its own rect.
fn masked_smart_psb_over(objects: &[(&str, f32, Rect)]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    let (mut records, mut items) = (Vec::new(), Vec::new());
    for (placed, level, source) in objects {
        let (rec, item) = masked_smart_object(&file, placed, *source, *source, *level);
        records.push(rec);
        items.push(item);
    }
    let solds = records.iter().map(|r| r.block(b"SoLd").expect("SoLd").data.clone()).collect();
    file.layers_mut().clear();
    file.layers_mut().extend(records);
    let mut fx = craft_psd::filter_effects::FilterEffects { version: 3, items }.to_bytes();
    fx.resize(fx.len().next_multiple_of(4), 0);
    file.global_blocks.push(TaggedBlock::new(*b"FEid", fx));
    (file.to_bytes().unwrap(), solds)
}

fn smart(doc: &Document, i: usize) -> &photocraft_doc::SmartObject {
    match &doc.layers[i].content {
        LayerContent::Smart(sm) => sm,
        other => panic!("{other:?}"),
    }
}

/// The placed ids of the `FEid` items a saved file keeps.
fn saved_fx_ids(file: &PsdFile) -> Vec<String> {
    file.global_blocks
        .iter()
        .filter(|b| &b.key == b"FEid")
        .filter_map(|b| craft_psd::filter_effects::FilterEffects::parse(&b.data).ok())
        .flat_map(|fx| fx.items.into_iter().map(|i| i.id))
        .collect()
}

/// A smart object whose filter mask is kept on save holds its pixels without the mask, as
/// Photoshop's do: saving unmixes them with the unfiltered pixels. When the save's smart-filter
/// cache budget can't hold those, the cache is left out with a warning rather than written over
/// mixed pixels, which would apply a partial mask twice when the file is opened again.
#[test]
fn a_mask_the_save_cannot_unmix_is_left_out_not_applied_twice() {
    let (bytes, _) = masked_smart_psb(&[("placed-1", 0.5)]);
    let doc = crate::import("half.psb", &bytes).unwrap().document;
    assert!(smart(&doc, 0).filter_mask.is_some());
    let mixed = smart(&doc, 0).cache.as_ref().unwrap().pixel(1, 1);

    // Room for the mask, not for the unfiltered pixels (one RGBA tile).
    let mask_tile = photocraft_raster::stored_tiles_bytes(1, &PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    let out = crate::smart_map::with_test_cache_cap(mask_tile, || crate::export(&doc, "half.psb", &crate::ExportOptions::default())).unwrap();
    let back = crate::import("half.psb", &out.bytes).unwrap().document;
    assert_eq!(back.layers.len(), 1);
    assert_eq!(smart(&back, 0).cache.as_ref().unwrap().pixel(1, 1), mixed, "the mask is applied once");
    assert!(out.warnings.iter().any(|w| w.contains("smart filter cache was left out")), "{:?}", out.warnings);
    assert!(saved_fx_ids(&PsdFile::from_bytes(&out.bytes).unwrap()).is_empty());

    // With room for both, the cache is kept and the pixels round-trip.
    let out = crate::export(&doc, "half.psb", &crate::ExportOptions::default()).unwrap();
    assert_eq!(saved_fx_ids(&PsdFile::from_bytes(&out.bytes).unwrap()), ["placed-1"]);
    let back = crate::import("half.psb", &out.bytes).unwrap().document;
    assert!(close(&smart(&back, 0).cache.as_ref().unwrap().pixel(1, 1), &mixed), "unmixed on save, mixed again on open");
}

/// Pixels equal within a level of 8-bit rounding.
fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.5 / 255.0)
}

/// A save keeps an unchanged smart object's placement as it was read, and when that object's own
/// caches don't fit the save's smart-filter cache budget, it leaves its cache out with a
/// warning: it doesn't treat it as changed and redraw the cache (over the source's bounds and
/// the canvas). The caches of the objects before it were given back once their records were
/// written, so they don't count.
#[test]
fn a_save_keeps_unchanged_placements_and_leaves_out_a_cache_too_large_for_its_budget() {
    // The second object's unfiltered pixels take two tiles.
    let (bytes, solds) = masked_smart_psb_over(&[("placed-1", 0.5, Rect::new(0, 0, 4, 3)), ("placed-2", 0.5, Rect::new(0, 0, 300, 3))]);
    let doc = crate::import("two.psb", &bytes).unwrap().document;
    // Room for a mask and one tile of unfiltered pixels.
    let fits_one = photocraft_raster::stored_tiles_bytes(1, &PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false))
        + photocraft_raster::stored_tiles_bytes(1, &PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    let out = crate::smart_map::with_test_cache_cap(fits_one, || crate::export(&doc, "two.psb", &crate::ExportOptions::default())).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("smart filter cache was left out")), "{:?}", out.warnings);
    let saved = PsdFile::from_bytes(&out.bytes).unwrap();
    assert_eq!(saved_fx_ids(&saved), ["placed-1"]);
    for (rec, sold) in saved.layers().iter().zip(&solds) {
        assert_eq!(&rec.block(b"SoLd").expect("SoLd").data, sold, "the placement is written as it was read");
    }
    let back = crate::import("two.psb", &out.bytes).unwrap().document;
    for i in 0..2 {
        assert!(close(&smart(&back, i).cache.as_ref().unwrap().pixel(1, 1), &smart(&doc, i).cache.as_ref().unwrap().pixel(1, 1)), "object {i}");
    }
}

/// Opening holds a smart object's unfiltered source only while layers naming it are built, and
/// charges mixing only for the tiles it adds: many masked smart objects whose sources together
/// exceed the smart-filter cache budget, each fitting it, all open with their masks applied.
#[test]
fn masked_smart_objects_open_with_their_masks_however_many_their_sources_add_up_to() {
    let objects = [("p1", 0.5), ("p2", 0.5), ("p3", 0.5), ("p4", 0.5)];
    let (bytes, _) = masked_smart_psb(&objects);
    // Room for the four masks (kept with the document) and one source (one tile) at a time.
    let mask_tile = photocraft_raster::stored_tiles_bytes(1, &PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    let cap = 4 * mask_tile + photocraft_raster::stored_tiles_bytes(1, &PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    let imported = crate::smart_map::with_test_cache_cap(cap, || crate::import("four.psb", &bytes)).unwrap();
    assert!(!imported.warnings.iter().any(|w| w.contains("filter mask was ignored")), "{:?}", imported.warnings);
    let unlimited = crate::import("four.psb", &bytes).unwrap().document;
    for i in 0..objects.len() {
        assert!(smart(&imported.document, i).filter_mask.is_some(), "object {i} keeps its mask");
        assert_eq!(
            smart(&imported.document, i).cache.as_ref().unwrap().pixel(1, 1),
            smart(&unlimited, i).cache.as_ref().unwrap().pixel(1, 1),
            "object {i} mixed"
        );
    }
}
