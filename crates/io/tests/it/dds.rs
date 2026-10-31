//! DDS import options (#8632): the alpha channel kept as the layer's transparency or routed to
//! an "Alpha 1" document channel (textures that carry data, such as specular maps, in alpha —
//! the NVIDIA/Intel plug-ins' "load transparency as alpha channel"), the mip chain opened as
//! one layer per level or the top level only, and the rows flipped on open (the GIMP plug-in's
//! "load flipped" toggle for the GL-vs-D3D origin).

use photocraft_io::*;

/// A 16×8 RGBA DDS with the full mip chain, uncompressed so every level is exact.
fn dds_with_mips() -> Vec<u8> {
    let ch = 4;
    let data: Vec<u8> = (0..16 * 8).flat_map(|i| [i as u8, 255 - i as u8, 40, 200 - i as u8]).collect();
    let img = photocraft_codecs::Image::from_u8(16, 8, photocraft_codecs::ChannelLayout::Rgba, data[..16 * 8 * ch].to_vec()).unwrap();
    photocraft_codecs::encode(
        &img,
        photocraft_codecs::Format::Dds,
        &photocraft_codecs::EncodeOptions { dds: photocraft_codecs::DdsOptions { compression: DdsCompression::None, mipmaps: true }, ..Default::default() },
    )
    .unwrap()
}

#[test]
fn the_default_import_reads_the_top_level_with_its_alpha() {
    let r = import("t.dds", &dds_with_mips()).unwrap();
    assert_eq!(r.document.size, photocraft_geom::Size::new(16, 8));
    assert_eq!(r.document.layers.len(), 1);
    let l = &r.document.layers[0];
    let px = l.surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 16, 8));
    assert!(px.chunks(4).any(|p| p[3] < 1.0), "alpha kept");
    assert_ne!(l.name, "Background", "a transparent image is not the background");
    assert!(r.document.channels.is_empty(), "no channel when the alpha is transparency");
}

#[test]
fn alpha_as_channel_opens_opaque_with_an_alpha_1_channel() {
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: true, mips_as_layers: false, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &dds_with_mips(), &opts).unwrap();
    assert_eq!(r.document.layers.len(), 1);
    let l = &r.document.layers[0];
    let px = l.surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 16, 8));
    assert!(px.chunks(4).all(|p| p[3] == 1.0), "the layer is opaque");
    // Opaque images open as the locked Background, like a 32-bit Targa's.
    assert_eq!(l.name, "Background");
    let [channel] = &r.document.channels[..] else { panic!("the texture's alpha becomes Alpha 1") };
    assert_eq!(channel.name, "Alpha 1");
    let a = channel.surface.read_region(photocraft_geom::Rect::new(0, 0, 16, 8));
    assert_eq!(a.len(), 16 * 8);
    // The texture's alpha (200 - i over 255) is the channel, nothing discarded.
    for (i, &v) in a.iter().enumerate().take(8) {
        assert!((v - (200 - i) as f32 / 255.0).abs() < 0.01, "alpha {i} kept in the channel");
    }
    // Round trip: the channel writes back as the texture's alpha.
    let out = export(&r.document, "t.dds", &ExportOptions::default()).unwrap();
    let again = photocraft_codecs::decode(&out.bytes).unwrap();
    let px = again.to_normalized();
    for i in [0, 4, 8] {
        assert!((px[i * 4 + 3] - (200 - i) as f32 / 255.0).abs() < 0.01, "alpha {i} round-trips");
    }
}

#[test]
fn alpha_as_channel_on_an_opaque_texture_makes_no_channel() {
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgba, [10, 20, 30, 255].repeat(16)).unwrap();
    let b = photocraft_codecs::encode(&img, photocraft_codecs::Format::Dds, &photocraft_codecs::EncodeOptions::default()).unwrap();
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: true, mips_as_layers: false, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &b, &opts).unwrap();
    assert!(r.document.channels.is_empty(), "an all-opaque alpha holds no data to keep");
}

#[test]
fn alpha_as_channel_with_mips_moves_only_the_top_levels_alpha() {
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: true, mips_as_layers: true, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &dds_with_mips(), &opts).unwrap();
    let names: Vec<&str> = r.document.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Mip 0", "Mip 1", "Mip 2", "Mip 3", "Mip 4"]);
    // The top level's alpha went to the channel: Mip 0 is opaque.
    let b = r.document.layers[0].surface().unwrap().content_bounds();
    let px = r.document.layers[0].surface().unwrap().read_region(b);
    assert!(px.chunks(4).all(|p| p[3] == 1.0), "Mip 0 is opaque");
    // The lower levels keep theirs as the layer's transparency rather than losing it.
    for l in &r.document.layers[1..] {
        let b = l.surface().unwrap().content_bounds();
        let px = l.surface().unwrap().read_region(b);
        assert!(px.chunks(4).any(|p| p[3] < 1.0), "{} keeps its alpha", l.name);
    }
    assert_eq!(r.document.channels.len(), 1);
}

#[test]
fn export_with_several_alpha_channels_writes_none() {
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: true, mips_as_layers: false, ..Default::default() }, ..Default::default() };
    let mut r = import_with_options("t.dds", &dds_with_mips(), &opts).unwrap();
    let extra = r.document.channels[0].clone();
    r.document.channels.push(extra);
    let out = export(
        &r.document,
        "t.dds",
        &ExportOptions {
            encode: photocraft_codecs::EncodeOptions {
                dds: photocraft_codecs::DdsOptions { compression: DdsCompression::Bc3, ..Default::default() },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("one alpha channel")), "{:?}", out.warnings);
}

#[test]
fn mipmaps_as_layers_opens_one_layer_per_level() {
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: false, mips_as_layers: true, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &dds_with_mips(), &opts).unwrap();
    // 16×8 → 8×4, 4×2, 2×1, 1×1.
    let names: Vec<&str> = r.document.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Mip 0", "Mip 1", "Mip 2", "Mip 3", "Mip 4"]);
    for (i, l) in r.document.layers.iter().enumerate() {
        let b = l.surface().unwrap().content_bounds();
        assert_eq!((b.width(), b.height()), ((16 >> i).max(1), (8 >> i).max(1)), "Mip {i}");
    }
    // The document stays the top level's size.
    assert_eq!(r.document.size, photocraft_geom::Size::new(16, 8));
}

#[test]
fn mip_layers_hide_below_the_top_level() {
    let opts = ImportOptions { dds: DdsImport { mips_as_layers: true, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &dds_with_mips(), &opts).unwrap();
    assert!(r.document.layers[0].visible);
    assert!(r.document.layers[1..].iter().all(|l| !l.visible), "lower mips don't cover Mip 0");
    // The composite a flattened export writes is the top level alone.
    let out = export(&r.document, "t.dds", &ExportOptions::default()).unwrap();
    let px = photocraft_codecs::decode(&out.bytes).unwrap().to_normalized();
    for i in [0usize, 4, 8] {
        assert!((px[i * 4] - i as f32 / 255.0).abs() < 0.01 && (px[i * 4 + 1] - (255 - i) as f32 / 255.0).abs() < 0.01, "pixel {i} is Mip 0's");
    }
}

#[test]
fn mips_as_layers_on_a_single_level_texture_is_one_layer() {
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgba, [10, 20, 30, 200].repeat(16)).unwrap();
    let b = photocraft_codecs::encode(&img, photocraft_codecs::Format::Dds, &photocraft_codecs::EncodeOptions::default()).unwrap();
    let opts = ImportOptions { dds: DdsImport { alpha_as_channel: false, mips_as_layers: true, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &b, &opts).unwrap();
    assert_eq!(r.document.layers.len(), 1);
    assert_ne!(r.document.layers[0].name, "Mip 0", "no chain, no rename");
}

/// A 2×2 RGBA DDS whose rows differ: red over blue.
fn two_tone_dds() -> Vec<u8> {
    let data = [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255].to_vec();
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgba, data).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Dds, &photocraft_codecs::EncodeOptions::default()).unwrap()
}

#[test]
fn flip_import_reads_the_rows_bottom_up() {
    let opts = ImportOptions { dds: DdsImport { flip: true, ..Default::default() }, ..Default::default() };
    let r = import_with_options("t.dds", &two_tone_dds(), &opts).unwrap();
    let px = r.document.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 2, 2));
    // Top row is the file's bottom row: blue over red.
    assert!((px[0] - 0.0).abs() < 0.01 && (px[2] - 1.0).abs() < 0.01, "top-left is blue: {:?}", &px[..4]);
    assert!((px[8] - 1.0).abs() < 0.01 && (px[10] - 0.0).abs() < 0.01, "bottom-left is red: {:?}", &px[8..12]);
    // The default import keeps the file's order.
    let r = import("t.dds", &two_tone_dds()).unwrap();
    let px = r.document.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 2, 2));
    assert!((px[0] - 1.0).abs() < 0.01, "top-left is red: {:?}", &px[..4]);
}
