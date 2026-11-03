//! `.tx` textures (OpenImageIO's `maketx`, read by V-Ray, Arnold and RenderMan): a tiled TIFF with
//! one directory per MIP level, full resolution first. File › Open shows the first level, read-only
//! (Save asks for a new name, so the texture is never replaced by a plain TIFF), without the
//! "more pages" warning a MIP chain doesn't deserve. Every file here is hand-built by `tiffgen`.

#[path = "../../../codecs/tests/it/common/tiffgen.rs"]
mod tiffgen;
use tiffgen::*;

const ORDERS: [(bool, bool); 4] = [(true, false), (false, false), (true, true), (false, true)];

/// A tiled RGB 8-bit directory of `w`×`h` filled with `rgb`, in `tile`×`tile` tiles, as `maketx`
/// writes them (no NewSubfileType).
fn level(w: u32, h: u32, rgb: [u8; 3], tile: usize) -> Dir {
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
    let d = Dir { entries: base(w, h, 8, 3, 2), chunks: tile_split(&px, w as usize, h as usize, 3, 1, tile, tile, false), tiled: true };
    d.tag(322, SHORT, &[tile as u64]).tag(323, SHORT, &[tile as u64])
}

/// A `maketx`-style pyramid from `w`×`h` down to 1×1 (sizes rounded down), each level its own colour.
fn pyramid(little: bool, big: bool, w: u32, h: u32) -> Vec<u8> {
    let mut dirs = Vec::new();
    let (mut lw, mut lh) = (w, h);
    loop {
        let shade = u8::try_from(dirs.len() * 20).unwrap_or(u8::MAX);
        dirs.push(level(lw, lh, [200, 100, shade], 16));
        if lw == 1 && lh == 1 {
            break;
        }
        (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
    }
    let chain: Vec<usize> = (0..dirs.len()).collect();
    build(little, big, &dirs, &chain).bytes
}

fn first_pixel(doc: &photocraft_doc::Document) -> Vec<u8> {
    let img = photocraft_io::document_to_image(doc, &mut Vec::new()).unwrap();
    img.data()[..3].to_vec()
}

#[test]
fn a_tx_opens_its_full_resolution_level_read_only_without_a_pages_warning() {
    for (little, big) in ORDERS {
        let bytes = pyramid(little, big, 37, 20);
        // 37×20, 18×10, 9×5, 4×2, 2×1, 1×1.
        assert_eq!(photocraft_codecs::tiff_info(&bytes).unwrap().pages.len(), 6);
        let r = photocraft_io::import("wall_diffuse.tx", &bytes).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (37, 20), "little {little} big {big}");
        assert_eq!(first_pixel(&r.document), [200, 100, 0], "the full-resolution level, not a smaller one");
        assert!(r.source_read_only, "Save must not write a plain TIFF over the texture");
        assert!(r.warnings.iter().all(|w| !w.contains("page")), "{:?}", r.warnings);
        // Upper-case extensions are textures too.
        assert!(photocraft_io::import("WALL.TX", &bytes).unwrap().source_read_only);
    }
}

#[test]
fn the_same_file_named_tif_still_opens_as_an_ordinary_tiff() {
    let bytes = pyramid(true, false, 16, 16);
    let r = photocraft_io::import("pyramid.tif", &bytes).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (16, 16));
    assert!(!r.source_read_only);
    assert!(r.warnings.iter().any(|w| w == "only the first of 5 pages was imported"), "{:?}", r.warnings);
}

#[test]
fn a_tx_whose_pages_are_not_a_mip_chain_keeps_the_pages_warning() {
    // Two full-size pages: the second one is real content, so leaving it out is worth a warning.
    let dirs = [level(16, 16, [10, 20, 30], 16), level(16, 16, [40, 50, 60], 16)];
    let bytes = build(true, false, &dirs, &[0, 1]).bytes;
    let r = photocraft_io::import("pages.tx", &bytes).unwrap();
    assert!(r.source_read_only);
    assert!(r.warnings.iter().any(|w| w == "only the first of 2 pages was imported"), "{:?}", r.warnings);
}

#[test]
fn a_thumbnail_in_front_of_the_pyramid_is_skipped() {
    // NewSubfileType 1 marks a reduced copy: the full-resolution level after it opens, and the
    // pyramid that follows still counts as one.
    let mut dirs = vec![level(4, 4, [9, 9, 9], 16).tag(254, LONG, &[1])];
    for (s, shade) in [(8, 0), (4, 20), (2, 40), (1, 60)] {
        dirs.push(level(s, s, [200, 100, shade], 16));
    }
    let bytes = build(true, false, &dirs, &[0, 1, 2, 3, 4]).bytes;
    let r = photocraft_io::import("thumb.tx", &bytes).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (8, 8));
    assert_eq!(first_pixel(&r.document), [200, 100, 0]);
    assert!(r.warnings.iter().all(|w| !w.contains("page")), "{:?}", r.warnings);
}

#[test]
fn halving_pages_that_are_not_a_full_tiled_pyramid_keep_the_pages_warning() {
    let pages_warning = |r: &photocraft_io::ImportResult| r.warnings.iter().any(|w| w.starts_with("only the first of"));
    // Halving sizes, but stored in strips: an ordinary multi-page TIFF.
    let strips_only: Vec<Dir> = [(8u32, 1u8), (4, 2), (2, 3), (1, 4)].iter().map(|&(s, v)| strips(s, s, 8, 1, 1, s, &vec![v; (s * s) as usize])).collect();
    // Tiled and halving, but stopping before 1×1.
    let short: Vec<Dir> = [16u32, 8, 4].iter().map(|&s| level(s, s, [1, 2, 3], 16)).collect();
    // Down to 1×1, then another 1×1 page with other content.
    let repeated: Vec<Dir> = [(4u32, 0u8), (2, 1), (1, 2), (1, 3)].iter().map(|&(s, v)| level(s, s, [v, v, v], 16)).collect();
    for (what, dirs) in [("strips", strips_only), ("short", short), ("repeated 1×1", repeated)] {
        let chain: Vec<usize> = (0..dirs.len()).collect();
        let r = photocraft_io::import("pages.tx", &build(true, false, &dirs, &chain).bytes).unwrap();
        assert!(r.source_read_only, "{what}");
        assert!(pages_warning(&r), "{what}: {:?}", r.warnings);
    }
}

/// Every page the warning counts must be a level of the chain: a directory without a size, after
/// the pyramid or in front of it, is still a page the import leaves out.
#[test]
fn a_page_without_a_size_beside_the_pyramid_keeps_the_pages_warning() {
    let empty = || Dir { entries: base(0, 0, 8, 3, 2), chunks: Vec::new(), tiled: true }.tag(322, SHORT, &[16]).tag(323, SHORT, &[16]);
    let pyramid_dirs = || [(8u32, 0u8), (4, 20), (2, 40), (1, 60)].map(|(s, v)| level(s, s, [200, 100, v], 16)).to_vec();
    let mut after = pyramid_dirs();
    after.push(empty());
    let mut before = vec![empty()];
    before.extend(pyramid_dirs());
    for (what, dirs) in [("after", after), ("before", before)] {
        let chain: Vec<usize> = (0..dirs.len()).collect();
        let r = photocraft_io::import("pages.tx", &build(true, false, &dirs, &chain).bytes).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (8, 8), "{what}");
        assert!(r.warnings.iter().any(|w| w.starts_with("only the first of")), "{what}: {:?}", r.warnings);
    }
}

#[test]
fn a_single_level_tx_opens_read_only() {
    let bytes = build(false, false, &[level(8, 8, [1, 2, 3], 16)], &[0]).bytes;
    let r = photocraft_io::import("flat.tx", &bytes).unwrap();
    assert_eq!(first_pixel(&r.document), [1, 2, 3]);
    assert!(r.source_read_only && r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_tx_that_is_not_a_tiff_is_left_to_the_other_readers() {
    // Only a TIFF signature makes a `.tx` a texture; anything else goes the usual way.
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let img = Image::new(2, 2, ChannelLayout::Rgb, SampleType::U8).unwrap();
    let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let r = photocraft_io::import("not_a_texture.tx", &png).unwrap();
    assert!(!r.source_read_only);
}

#[test]
fn saving_as_tx_is_refused_rather_than_writing_a_plain_tiff() {
    let bytes = pyramid(true, false, 8, 8);
    let doc = photocraft_io::import("t.tx", &bytes).unwrap().document;
    assert!(photocraft_io::export(&doc, "t.tx", &photocraft_io::ExportOptions::default()).is_err());
}
