//! File › Open of an EXR brings in every part, view and channel group as layers in folders
//! (#14062): the measured trees, their pixels, and the files that keep today's open.

use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Image, ImageAttributes, IntegerBounds, Layer as ExrLayer, LayerAttributes, LineOrder,
    Text, WritableImage, f16,
};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_io::{ImportOptions, ImportResult, IoError, import, import_file};
use photocraft_raster::Interrupt;

/// ZIP16 scan lines: the layout File › Open takes into a layer tree (`Encoding::FAST_LOSSLESS`
/// writes tiles, which keep today's open).
const SCAN_LINES: Encoding = Encoding { compression: Compression::ZIP16, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };

/// A part's channel: its name and its samples (one value for every pixel, or one per pixel).
type Chan<'a> = (&'a str, Vec<f32>);

/// `n` pixels of `v`.
fn fill(v: f32, n: usize) -> Vec<f32> {
    vec![v; n]
}

/// One flat 32-bit float scan-line part of `w × h` at `origin`, with an optional `name` and
/// `view` attribute.
fn part(name: Option<&str>, view: Option<&str>, size: (usize, usize), origin: (i32, i32), channels: Vec<Chan>) -> ExrLayer<AnyChannels<FlatSamples>> {
    part_as(false, name, view, size, origin, channels)
}

/// [`part`] with 16-bit float samples when `half`.
fn part_as(
    half: bool,
    name: Option<&str>,
    view: Option<&str>,
    (w, h): (usize, usize),
    origin: (i32, i32),
    channels: Vec<Chan>,
) -> ExrLayer<AnyChannels<FlatSamples>> {
    let mut attrs = name.map_or_else(LayerAttributes::default, LayerAttributes::named);
    attrs.view_name = view.map(Text::from);
    attrs.layer_position = exr::math::Vec2(origin.0, origin.1);
    let samples = |v: Vec<f32>| if half { FlatSamples::F16(v.into_iter().map(f16::from_f32).collect()) } else { FlatSamples::F32(v) };
    let list = channels.into_iter().map(|(n, v)| AnyChannel::new(n, samples(v))).collect();
    ExrLayer::new((w, h), attrs, SCAN_LINES, AnyChannels::sort(list))
}

/// The file of these parts, its display window `w × h`.
fn file(parts: Vec<ExrLayer<AnyChannels<FlatSamples>>>, (w, h): (usize, usize)) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (w, h))), parts).write().to_buffered(&mut out).unwrap();
    out.into_inner()
}

/// A 2 × 2 file of one unnamed part holding these channels, each one value everywhere.
fn one_part(channels: &[(&str, f32)]) -> Vec<u8> {
    file(vec![part(None, None, (2, 2), (0, 0), channels.iter().map(|(n, v)| (*n, fill(*v, 4))).collect())], (2, 2))
}

/// A 2 × 2 file of named parts, each one value everywhere.
fn named_parts(parts: &[(&str, &[(&str, f32)])]) -> Vec<u8> {
    named_parts_as(false, parts)
}

/// [`named_parts`] with 16-bit float samples when `half`.
fn named_parts_as(half: bool, parts: &[(&str, &[(&str, f32)])]) -> Vec<u8> {
    file(parts.iter().map(|(name, ch)| part_as(half, Some(name), None, (2, 2), (0, 0), ch.iter().map(|(n, v)| (*n, fill(*v, 4))).collect())).collect(), (2, 2))
}

/// The layer tree, top of the Layers panel first: `[G] name > (children)` for a group,
/// `name` for a visible leaf and `name hidden` for a hidden one, siblings joined by ` · `.
fn tree(layers: &[Layer]) -> String {
    layers
        .iter()
        .rev()
        .map(|l| match l.children() {
            Some(children) => format!("[G] {} > ({})", l.name, tree(children)),
            None if l.visible => l.name.clone(),
            None => format!("{} hidden", l.name),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// File › Open of `bytes`, as the open job decodes a dropped or recent file.
fn open(bytes: &[u8]) -> ImportResult {
    import("render.exr", bytes).unwrap()
}

/// The leaf at `path` (names from the top level down; the first match at each level).
fn leaf<'a>(doc: &'a Document, path: &[&str]) -> &'a Layer {
    let mut layers = &doc.layers[..];
    let mut found = None;
    for name in path {
        let l = layers.iter().rev().find(|l| l.name == *name).unwrap_or_else(|| panic!("no {name} in {}", tree(layers)));
        layers = l.children().unwrap_or(&[]);
        found = Some(l);
    }
    found.unwrap()
}

/// The RGBA pixel at `(x, y)` of a pixel layer.
fn px(layer: &Layer, x: i32, y: i32) -> [f32; 4] {
    let LayerContent::Raster(s) = &layer.content else { panic!("{} is not a pixel layer", layer.name) };
    let p = s.pixel(x, y);
    [p[0], p[1], p[2], p[3]]
}

/// Every group, at any depth: visible, pass-through, opacity 100 %; every leaf normal, 100 %,
/// RGBA 32-bit float.
fn assert_layer_settings(layers: &[Layer]) {
    for l in layers {
        assert_eq!(l.opacity, 1.0, "{}", l.name);
        match l.children() {
            Some(children) => {
                assert!(l.visible, "group {} stays visible", l.name);
                assert_eq!(l.blend, BlendMode::PassThrough, "{}", l.name);
                assert_layer_settings(children);
            }
            None => {
                assert_eq!(l.blend, BlendMode::Normal, "{}", l.name);
                let fmt = l.surface().unwrap().format();
                assert_eq!((fmt.mode, fmt.sample, fmt.alpha), (ColorMode::Rgb, SampleType::F32, true), "{}", l.name);
            }
        }
    }
}

/// An open that gives a tree: one RGB 32-bit linear document, no background, no notices.
fn assert_tree_document(r: &ImportResult) {
    let d = &r.document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32));
    assert_eq!(d.icc_profile, Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes()));
    assert!(d.background().is_none());
    assert!(r.warnings.is_empty(), "no 'only one of N parts' notice: {:?}", r.warnings);
    assert_layer_settings(&d.layers);
}

const RGBA: [(&str, f32); 4] = [("R", 0.8), ("G", 0.4), ("B", 0.2), ("A", 0.5)];
const RGB: [(&str, f32); 3] = [("R", 0.3), ("G", 0.6), ("B", 0.9)];

#[test]
fn three_parts_open_as_part_folders_in_file_order() {
    let beauty_first = named_parts(&[("beauty", &RGBA), ("Z", &[("Z", 5.0)]), ("diffuse", &RGB)]);
    let r = open(&beauty_first);
    assert_tree_document(&r);
    assert_eq!(tree(&r.document.layers), "[G] beauty > (RGB) · [G] Z > (Z hidden) · [G] diffuse > (RGB hidden)");
    let d = &r.document;
    assert_eq!(d.size, photocraft_doc::Size::new(2, 2));
    // The part's A is the RGB layer's transparency; a solo channel is grey, opaque, above 1 kept.
    assert_eq!(px(leaf(d, &["beauty", "RGB"]), 1, 1), [0.8, 0.4, 0.2, 0.5]);
    assert_eq!(px(leaf(d, &["Z", "Z"]), 0, 1), [5.0, 5.0, 5.0, 1.0]);
    assert_eq!(px(leaf(d, &["diffuse", "RGB"]), 1, 0), [0.3, 0.6, 0.9, 1.0]);

    // A solo part first stays hidden: the first colour layer is the visible one.
    let z_first = named_parts(&[("Z", &[("Z", 5.0)]), ("beauty", &RGBA), ("diffuse", &RGB)]);
    assert_eq!(tree(&open(&z_first).document.layers), "[G] Z > (Z hidden) · [G] beauty > (RGB) · [G] diffuse > (RGB hidden)");
}

#[test]
fn channel_groups_open_as_folders_below_the_bare_channels() {
    let all = one_part(&[
        ("R", 0.9),
        ("G", 0.8),
        ("B", 0.7),
        ("A", 1.0),
        ("diffuse.R", 0.3),
        ("diffuse.G", 0.2),
        ("diffuse.B", 0.1),
        ("diffuse.A", 0.25),
        ("specular.R", 0.6),
        ("specular.G", 0.5),
        ("specular.B", 0.4),
    ]);
    let r = open(&all);
    assert_tree_document(&r);
    assert_eq!(tree(&r.document.layers), "RGB · [G] diffuse > (RGB hidden) · [G] specular > (RGB hidden)");
    let d = &r.document;
    assert_eq!(px(leaf(d, &["RGB"]), 0, 0), [0.9, 0.8, 0.7, 1.0]);
    assert_eq!(px(leaf(d, &["diffuse", "RGB"]), 0, 0), [0.3, 0.2, 0.1, 0.25], "the group's A is its own layer's");
    assert_eq!(px(leaf(d, &["specular", "RGB"]), 1, 1), [0.6, 0.5, 0.4, 1.0]);

    let groups_only = one_part(&[("diffuse.R", 0.3), ("diffuse.G", 0.2), ("diffuse.B", 0.1), ("specular.R", 0.6), ("specular.G", 0.5), ("specular.B", 0.4)]);
    assert_eq!(tree(&open(&groups_only).document.layers), "[G] diffuse > (RGB) · [G] specular > (RGB hidden)");

    let vectors = one_part(&[("R", 0.1), ("G", 0.2), ("B", 0.3), ("uv.U", 0.25), ("uv.V", 0.75), ("N.X", -1.0), ("N.Y", 0.5), ("N.Z", 2.0)]);
    let r = open(&vectors);
    assert_eq!(tree(&r.document.layers), "RGB · [G] N > (X hidden · Y hidden · Z hidden) · [G] uv > (U hidden · V hidden)");
    assert_eq!(px(leaf(&r.document, &["N", "X"]), 0, 0), [-1.0, -1.0, -1.0, 1.0], "negative values kept");
    assert_eq!(px(leaf(&r.document, &["N", "Z"]), 0, 0), [2.0, 2.0, 2.0, 1.0]);
    assert_eq!(px(leaf(&r.document, &["uv", "V"]), 1, 0), [0.75, 0.75, 0.75, 1.0]);
}

#[test]
fn lone_colour_channels_and_solo_channels_follow_the_file_s_channel_order() {
    let r = open(&one_part(&[("Z", 3.5), ("mask.R", 0.6), ("spec.G", 0.4), ("Y", 0.7)]));
    assert_tree_document(&r);
    // Stored order is Y, Z, mask.R, spec.G: the grey layers come first and stay hidden.
    assert_eq!(tree(&r.document.layers), "Y hidden · Z hidden · [G] mask > (RGB) · [G] spec > (RGB hidden)");
    let d = &r.document;
    assert_eq!(px(leaf(d, &["mask", "RGB"]), 0, 0), [0.6, 0.0, 0.0, 1.0], "a lone R reads R, 0, 0");
    assert_eq!(px(leaf(d, &["spec", "RGB"]), 0, 0), [0.0, 0.4, 0.0, 1.0], "a lone G reads 0, G, 0");
    assert_eq!(px(leaf(d, &["Y"]), 1, 1), [0.7, 0.7, 0.7, 1.0]);
    assert_eq!(px(leaf(d, &["Z"]), 1, 1), [3.5, 3.5, 3.5, 1.0]);

    let five = named_parts(&[("depth", &[("Z", 9.0)]), ("red", &[("R", 0.5)]), ("green", &[("G", 0.25)]), ("lum", &[("Y", 0.125)]), ("alpha", &[("A", 0.75)])]);
    let r = open(&five);
    assert_tree_document(&r);
    assert_eq!(tree(&r.document.layers), "[G] depth > (Z hidden) · [G] red > (RGB) · [G] green > (RGB hidden) · [G] lum > (Y hidden) · [G] alpha > (A hidden)");
    let d = &r.document;
    assert_eq!(px(leaf(d, &["green", "RGB"]), 0, 0), [0.0, 0.25, 0.0, 1.0]);
    assert_eq!(px(leaf(d, &["alpha", "A"]), 0, 0), [0.75, 0.75, 0.75, 1.0], "a lone A is a grey layer, not transparency");
}

#[test]
fn a_grey_layer_with_alpha_is_ya() {
    let r = open(&named_parts(&[("beauty", &RGB), ("lum", &[("Y", 0.5), ("A", 0.25)])]));
    assert_eq!(tree(&r.document.layers), "[G] beauty > (RGB) · [G] lum > (YA hidden)");
    assert_eq!(px(leaf(&r.document, &["lum", "YA"]), 0, 0), [0.5, 0.5, 0.5, 0.25]);
}

#[test]
fn half_float_files_open_as_the_same_tree() {
    // Values exact in half precision, so the half file's pixels equal the float file's.
    let parts: [(&str, &[(&str, f32)]); 3] =
        [("beauty", &[("R", 0.75), ("G", 0.5), ("B", 0.25), ("A", 0.5)]), ("Z", &[("Z", 6.5)]), ("diffuse", &[("R", 0.125), ("G", 0.375), ("B", 2.0)])];
    let (float, half) = (open(&named_parts_as(false, &parts)), open(&named_parts_as(true, &parts)));
    assert_eq!(tree(&half.document.layers), "[G] beauty > (RGB) · [G] Z > (Z hidden) · [G] diffuse > (RGB hidden)");
    assert_eq!(tree(&half.document.layers), tree(&float.document.layers));
    assert_eq!(half.warnings, ["16-bit float samples are stored as 32-bit float"], "as today's open of a half file says");
    assert_tree_document(&ImportResult { warnings: Vec::new(), ..half.clone() });
    let d = &half.document;
    assert_eq!(px(leaf(d, &["beauty", "RGB"]), 1, 1), [0.75, 0.5, 0.25, 0.5]);
    assert_eq!(px(leaf(d, &["Z", "Z"]), 0, 1), [6.5, 6.5, 6.5, 1.0], "above 1 kept");
    assert_eq!(px(leaf(d, &["diffuse", "RGB"]), 1, 0), [0.125, 0.375, 2.0, 1.0]);

    // A half single part with channel groups, opened from a path (the band decoder takes half too).
    let groups = file(
        vec![part_as(
            true,
            None,
            None,
            (2, 2),
            (0, 0),
            vec![("R", fill(0.5, 4)), ("G", fill(0.25, 4)), ("B", fill(0.125, 4)), ("uv.U", fill(0.75, 4)), ("uv.V", fill(1.5, 4))],
        )],
        (2, 2),
    );
    let path = TempExr::new("half-groups", &groups);
    let r = import_file("render.exr", &path.0, &ImportOptions::default(), &Interrupt::NONE).unwrap();
    assert_eq!(tree(&r.document.layers), "RGB · [G] uv > (U hidden · V hidden)");
    assert_eq!(px(leaf(&r.document, &["uv", "V"]), 1, 1), [1.5, 1.5, 1.5, 1.0]);
    assert_eq!(px(leaf(&r.document, &["RGB"]), 0, 0), [0.5, 0.25, 0.125, 1.0]);
}

#[test]
fn views_become_top_level_folders() {
    // One part whose `multiView` is left, right: the default view's channels carry no view name.
    let mut stereo = part(
        None,
        None,
        (2, 2),
        (0, 0),
        vec![("R", fill(0.1, 4)), ("G", fill(0.2, 4)), ("B", fill(0.3, 4)), ("right.R", fill(0.4, 4)), ("right.G", fill(0.5, 4)), ("right.B", fill(0.6, 4))],
    );
    stereo.attributes.multi_view_names = Some(vec![Text::from("left"), Text::from("right")]);
    let r = open(&file(vec![stereo], (2, 2)));
    assert_tree_document(&r);
    assert_eq!(tree(&r.document.layers), "[G] left > (RGB) · [G] right > (RGB hidden)");
    assert_eq!(px(leaf(&r.document, &["left", "RGB"]), 0, 0), [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(px(leaf(&r.document, &["right", "RGB"]), 0, 0), [0.4, 0.5, 0.6, 1.0]);

    // Layered channels of a view, and a dotted channel in no view (after the view folders).
    let mut layered = part(
        None,
        None,
        (2, 2),
        (0, 0),
        vec![("R", fill(0.1, 4)), ("Z", fill(4.0, 4)), ("right.R", fill(0.4, 4)), ("diffuse.right.G", fill(0.5, 4)), ("background.data", fill(2.0, 4))],
    );
    layered.attributes.multi_view_names = Some(vec![Text::from("left"), Text::from("right")]);
    let r = open(&file(vec![layered], (2, 2)));
    assert_eq!(
        tree(&r.document.layers),
        "[G] left > (RGB · Z hidden) · [G] right > (RGB hidden · [G] diffuse > (RGB hidden)) · [G] background > (data hidden)"
    );
    assert_eq!(px(leaf(&r.document, &["right", "diffuse", "RGB"]), 0, 0), [0.0, 0.5, 0.0, 1.0]);

    // Four parts that each name their view: a view folder holds its parts in file order.
    let views = ["left", "right", "left", "right"];
    let names = ["beauty.left", "beauty.right", "diffuse.left", "diffuse.right"];
    let parts = names
        .iter()
        .zip(views)
        .enumerate()
        .map(|(i, (n, v))| part(Some(n), Some(v), (2, 2), (0, 0), vec![("R", fill(i as f32, 4)), ("G", fill(0.0, 4)), ("B", fill(0.0, 4))]))
        .collect();
    let r = open(&file(parts, (2, 2)));
    assert_tree_document(&r);
    assert_eq!(
        tree(&r.document.layers),
        "[G] left > ([G] beauty.left > (RGB) · [G] diffuse.left > (RGB hidden)) · [G] right > ([G] beauty.right > (RGB hidden) · [G] diffuse.right > (RGB hidden))"
    );
    assert_eq!(px(leaf(&r.document, &["right", "diffuse.right", "RGB"]), 0, 0), [3.0, 0.0, 0.0, 1.0]);
}

#[test]
fn layers_keep_their_pixels_in_place() {
    // A 3 × 2 depth ramp, and a second part whose data window starts one pixel further right.
    let ramp: Vec<f32> = (0..6).map(|i| i as f32).collect();
    let parts = vec![
        part(Some("beauty"), None, (3, 2), (0, 0), vec![("R", ramp.clone()), ("G", fill(0.0, 6)), ("B", fill(0.0, 6))]),
        part(Some("depth"), None, (3, 2), (1, 0), vec![("Z", ramp)]),
    ];
    let r = open(&file(parts, (4, 2)));
    let d = &r.document;
    assert_eq!(d.size, photocraft_doc::Size::new(4, 2), "the canvas holds every part's data window");
    let beauty = leaf(d, &["beauty", "RGB"]);
    let depth = leaf(d, &["depth", "Z"]);
    for (x, y) in [(0, 0), (2, 0), (0, 1), (2, 1)] {
        let v = (y * 3 + x) as f32;
        assert_eq!(px(beauty, x, y), [v, 0.0, 0.0, 1.0], "beauty ({x}, {y})");
        assert_eq!(px(depth, x + 1, y), [v, v, v, 1.0], "depth ({x}, {y}) sits one pixel right");
    }
    assert_eq!(px(depth, 0, 0)[3], 0.0, "nothing left of the depth part's window");
}

/// Two 2 × 2 RGB parts, the second one's data window at `(far, far)`.
fn far_apart(far: i32) -> Vec<u8> {
    let rgb = |v: f32| vec![("R", fill(v, 4)), ("G", fill(0.0, 4)), ("B", fill(0.0, 4))];
    file(vec![part(Some("near"), None, (2, 2), (0, 0), rgb(0.25)), part(Some("far"), None, (2, 2), (far, far), rgb(0.75))], (2, 2))
}

#[test]
fn sparse_parts_are_charged_only_the_tiles_they_touch() {
    // A 32768 × 32768 canvas (2^30 pixels, the limit) holding two 2 × 2 parts: each layer stores
    // one tile, not the canvas's 16384 (32 GiB for the two layers at 16 bytes a pixel).
    assert!(!photocraft_raster::spill::enabled(), "the RAM tile budget applies");
    let r = open(&far_apart(32766));
    let d = &r.document;
    assert_eq!(d.size, photocraft_doc::Size::new(32768, 32768));
    assert_eq!(tree(&d.layers), "[G] near > (RGB) · [G] far > (RGB hidden)");
    assert_eq!(px(leaf(d, &["near", "RGB"]), 1, 1), [0.25, 0.0, 0.0, 1.0]);
    assert_eq!(px(leaf(d, &["far", "RGB"]), 32767, 32767), [0.75, 0.0, 0.0, 1.0]);
    for (path, tiles) in [(["near", "RGB"], 1), (["far", "RGB"], 1)] {
        let LayerContent::Raster(s) = &leaf(d, &path).content else { panic!() };
        assert_eq!(s.tile_count(), tiles, "{path:?}");
    }
}

#[test]
fn a_canvas_past_the_pixel_limit_is_refused() {
    // Each part is 2 × 2, but together they span 40002 × 40002, over 2^30 pixels.
    match import("render.exr", &far_apart(40000)) {
        Err(IoError::Codec(photocraft_codecs::CodecError::LimitExceeded(m))) => assert!(m.contains("40002x40002"), "{m}"),
        other => panic!("expected the canvas limit, got {:?}", other.map(|r| r.document.size)),
    }
}

#[test]
fn one_layer_files_keep_a_background_or_name_the_layer_after_its_channels() {
    let opaque = open(&one_part(&RGB));
    assert_eq!(opaque.document.layers.len(), 1);
    assert!(opaque.document.background().is_some(), "one opaque layer is a Background");
    for (channels, name, mode) in [(&RGBA[..], "RGB", ColorMode::Rgb), (&[("Y", 0.5), ("A", 0.25)][..], "YA", ColorMode::Grayscale)] {
        let r = open(&one_part(channels));
        assert_eq!(r.document.mode, mode);
        assert_eq!(r.document.layers.len(), 1);
        assert_eq!(r.document.layers[0].name, name);
        assert!(r.document.background().is_none());
    }
}

/// A file written to a temporary path, removed when dropped.
struct TempExr(std::path::PathBuf);

impl TempExr {
    /// Writes `bytes` to a fresh temporary `.exr` named after `tag`.
    fn new(tag: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("photocraft-exr-all-parts-{tag}-{}.exr", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        TempExr(path)
    }
}

impl Drop for TempExr {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The path open decodes a one-layer file block by block into an RGB document (#14069); the
/// layer is still named after the file's channels.
#[test]
fn a_one_layer_file_opened_from_a_path_is_named_after_its_channels() {
    for (tag, channels, name, pixel) in [
        ("one-ya", &[("Y", 0.5), ("A", 0.25)][..], "YA", [0.5, 0.5, 0.5, 0.25]),
        ("one-rgba", &RGBA[..], "RGB", [0.8, 0.4, 0.2, 0.5]),
        ("one-rgb", &RGB[..], "Background", [0.3, 0.6, 0.9, 1.0]),
    ] {
        let path = TempExr::new(tag, &one_part(channels));
        let r = import_file("render.exr", &path.0, &ImportOptions::default(), &Interrupt::NONE).unwrap();
        let d = &r.document;
        assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32), "{tag}");
        assert_eq!(d.layers.len(), 1, "{tag}");
        assert_eq!(d.layers[0].name, name, "{tag}");
        assert_eq!(d.background().is_some(), name == "Background", "{tag}");
        assert_eq!(px(&d.layers[0], 1, 1), pixel, "{tag}");
    }
}

#[test]
fn opening_from_a_path_gives_the_same_tree() {
    let cases = [
        ("parts", named_parts(&[("beauty", &RGBA), ("Z", &[("Z", 5.0)]), ("diffuse", &RGB)])),
        // One scan-line part: the band decoder would take its bare RGBA alone.
        ("groups", one_part(&[("R", 0.9), ("G", 0.8), ("B", 0.7), ("A", 1.0), ("diffuse.R", 0.3), ("diffuse.G", 0.2), ("diffuse.B", 0.1)])),
        ("rgba", one_part(&RGBA)),
    ];
    for (tag, bytes) in cases {
        let path = TempExr::new(tag, &bytes);
        let from_path = import_file("render.exr", &path.0, &ImportOptions::default(), &Interrupt::NONE).unwrap();
        let from_bytes = open(&bytes);
        assert_eq!(tree(&from_path.document.layers), tree(&from_bytes.document.layers), "{tag}");
        assert_eq!(from_path.warnings, from_bytes.warnings, "{tag}");
        assert_eq!(from_path.document.size, from_bytes.document.size, "{tag}");
    }
}

#[test]
fn files_the_reference_was_not_measured_on_keep_today_s_open() {
    // Tiled parts: today's open, the first colour part with its notice.
    let tiled = Encoding { compression: Compression::ZIP16, blocks: Blocks::Tiles(exr::math::Vec2(2, 2)), line_order: LineOrder::Increasing };
    let parts = ["beauty", "diffuse"]
        .iter()
        .map(|n| {
            ExrLayer::new(
                (2, 2),
                LayerAttributes::named(*n),
                tiled,
                AnyChannels::sort(["R", "G", "B"].iter().map(|c| AnyChannel::new(*c, FlatSamples::F32(fill(0.5, 4)))).collect()),
            )
        })
        .collect();
    let r = open(&file(parts, (2, 2)));
    assert_eq!(r.document.layers.len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("2")), "{:?}", r.warnings);
    // No RGB layer anywhere: today's grey open of the first colour channel.
    let r = open(&one_part(&[("Y", 0.5), ("Z", 2.0)]));
    assert_eq!((r.document.mode, r.document.layers.len()), (ColorMode::Grayscale, 1));
}

#[test]
fn a_cancelled_or_damaged_open_fails_without_a_panic() {
    let bytes = named_parts(&[("beauty", &RGBA), ("Z", &[("Z", 5.0)]), ("diffuse", &RGB)]);
    // Cancelled once the first part is in: the open stops before the next part.
    let stop = std::sync::atomic::AtomicBool::new(false);
    let cancel = || stop.load(std::sync::atomic::Ordering::SeqCst);
    let progress = |_: f32| stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let ctl = Interrupt::new(&cancel, &progress);
    assert!(matches!(photocraft_io::import_with("render.exr", &bytes, &ctl), Err(IoError::Cancelled)));
    // Every truncation and every byte overwritten opens or fails, never panics.
    for n in 0..bytes.len() {
        let _ = import("render.exr", &bytes[..n]);
    }
    for i in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[i] ^= 0xFF;
        if let Ok(r) = import("render.exr", &damaged) {
            assert!(r.document.size.width >= 1 && r.document.size.height >= 1);
        }
    }
    // A pixel budget too small for the parts refuses the open.
    let tight = ImportOptions { flat_max_pixel_bytes: 16, ..ImportOptions::default() };
    assert!(photocraft_io::import_with_options("render.exr", &bytes, &tight).is_err());
}
