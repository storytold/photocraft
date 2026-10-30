use super::bplist::{Obj, Plist, Value, write};
use super::*;
use photocraft_codecs::{ChannelLayout, Format, Image, encode};

fn png(w: u32, h: u32, layout: ChannelLayout, px: impl Fn(u32, u32) -> Vec<u8>) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend(px(x, y));
        }
    }
    encode(&Image::from_u8(w, h, layout, data).unwrap(), Format::Png, &Default::default()).unwrap()
}

/// 32 × 32, black with a white bar across rows 14..18, columns 4..28: Procreate's stitch shape.
fn bar() -> Vec<u8> {
    png(32, 32, ChannelLayout::Gray, |x, y| vec![if (4..28).contains(&x) && (14..18).contains(&y) { 255 } else { 0 }])
}

fn brush_zip(archive: &[u8], shape: Option<&[u8]>, grain: Option<&[u8]>) -> Vec<u8> {
    let mut z = ZipWriter::new();
    z.add("Brush.archive", archive).unwrap();
    if let Some(s) = shape {
        z.add("Shape.png", s).unwrap();
    }
    if let Some(g) = grain {
        z.add("Grain.png", g).unwrap();
    }
    z.add("QuickLook/Thumbnail.png", &bar()).unwrap();
    z.finish().unwrap()
}

fn text(s: &str) -> Field {
    Field::Text(s.into())
}

fn sampled(b: &BrushSettings) -> &GrayTile {
    match &b.tip {
        TipShape::Sampled(t) => t,
        other => panic!("sampled tip expected, got {other:?}"),
    }
}

fn paint_at(t: &GrayTile, x: u32, y: u32) -> f32 {
    f32::from(t.data[(y * t.width + x) as usize]) / 65535.0
}

#[test]
fn plist_round_trip() {
    let v = Value::Dict(vec![
        ("n".into(), Value::Int(-5)),
        ("r".into(), Value::Real(0.25)),
        ("b".into(), Value::Bool(true)),
        ("s".into(), Value::Str("Café brush".into())),
        ("long".into(), Value::Str("x".repeat(40))),
        ("u".into(), Value::Uid(7)),
        ("a".into(), Value::Array(vec![Value::Int(1), Value::Int(300)])),
    ]);
    let bytes = write(&v);
    let p = Plist::parse(&bytes).unwrap();
    let Obj::Dict(top) = p.object(p.top()).unwrap() else { panic!("dict expected") };
    let get = |k: &str| p.object(p.lookup(&top, k).unwrap()).unwrap();
    assert_eq!(get("n"), Obj::Int(-5));
    assert_eq!(get("r"), Obj::Real(0.25));
    assert_eq!(get("b"), Obj::Bool(true));
    assert_eq!(get("s"), Obj::Str("Café brush".into()));
    assert_eq!(get("long"), Obj::Str("x".repeat(40)));
    assert_eq!(get("u"), Obj::Uid(7));
    let Obj::Array(a) = get("a") else { panic!("array expected") };
    assert_eq!(a.iter().map(|&i| p.object(i).unwrap()).collect::<Vec<_>>(), [Obj::Int(1), Obj::Int(300)]);
    assert!(p.lookup(&top, "missing").is_none());
}

#[test]
fn plist_damage_is_an_error_not_a_panic() {
    let bytes = write(&Value::Array(vec![Value::Str("a".into()), Value::Dict(vec![("k".into(), Value::Real(1.0))])]));
    for n in 0..bytes.len() {
        if let Ok(p) = Plist::parse(&bytes[..n]) {
            for i in 0..4 {
                let _ = p.object(i);
            }
        }
    }
    for i in 0..bytes.len() {
        for flip in [0x01u8, 0x10, 0x80, 0xFF] {
            let mut b = bytes.clone();
            b[i] ^= flip;
            if let Ok(p) = Plist::parse(&b) {
                for k in 0..8 {
                    let _ = p.object(k);
                }
            }
        }
    }
    // A trailer claiming more objects than the file could hold, or a top past the end.
    let mut b = bytes.clone();
    let t = b.len() - 32;
    b[t + 8..t + 16].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(Plist::parse(&b).is_err());
    let mut b = bytes.clone();
    b[t + 16..t + 24].copy_from_slice(&99u64.to_be_bytes());
    assert!(Plist::parse(&b).is_err());
    assert!(Plist::parse(b"bplist00").is_err() && Plist::parse(b"not a plist at all, really not").is_err());
}

/// Arrays that each hold the one before twice: expanded, 40 levels would be 2^40 objects. Read by
/// index, every level is two numbers.
#[test]
fn shared_objects_are_not_expanded() {
    let mut b = b"bplist00".to_vec();
    let mut offsets = vec![b.len() as u64];
    b.push(0x09);
    for i in 0..40u32 {
        offsets.push(b.len() as u64);
        b.push(0xA2);
        b.extend_from_slice(&i.to_be_bytes());
        b.extend_from_slice(&i.to_be_bytes());
    }
    let table = b.len() as u64;
    b.extend(offsets.iter().flat_map(|o| o.to_be_bytes()));
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 8, 4]);
    b.extend_from_slice(&(offsets.len() as u64).to_be_bytes());
    b.extend_from_slice(&40u64.to_be_bytes());
    b.extend_from_slice(&table.to_be_bytes());
    let p = Plist::parse(&b).unwrap();
    assert_eq!(p.object(p.top()).unwrap(), Obj::Array(vec![39, 39]));
    // An array holding itself is just as harmless.
    let cyclic =
        [b"bplist00".as_slice(), &[0xA1, 0x00], &8u64.to_be_bytes(), &[0, 0, 0, 0, 0, 0, 8, 1], &1u64.to_be_bytes(), &0u64.to_be_bytes(), &10u64.to_be_bytes()]
            .concat();
    let p = Plist::parse(&cyclic).unwrap();
    assert_eq!(p.object(0).unwrap(), Obj::Array(vec![0]));
}

#[test]
fn reads_a_brush() {
    let archive = write_archive(&[
        ("name", text("  Plain Stitch ")),
        ("paintSize", Field::Num(0.24)),
        ("minSize", Field::Num(0.0086)),
        ("maxSize", Field::Num(0.49)),
        ("paintOpacity", Field::Num(0.9)),
        ("plotSpacing", Field::Num(0.55)),
        ("plotSmoothing", Field::Num(0.3)),
        ("shapeRotation", Field::Num(1.0)),
        ("shapeScatter", Field::Num(0.5)),
        ("dynamicsPressureSize", Field::Num(0.75)),
        ("dynamicsPressureSizeCurve", Field::Curve(vec![[0.0, 0.0], [1.0, 1.0], [0.6, 0.2]])),
        ("dynamicsJitterOpacity", Field::Num(0.4)),
        ("bundledGrainPath", text("Brush-Preset-Blank.png")),
    ]);
    let imp = read_procreate(&brush_zip(&archive, Some(&bar()), None), "Stitches").unwrap();
    assert_eq!(imp.warnings, Vec::<String>::new(), "a blank built-in grain is no grain");
    let [p] = &imp.presets[..] else { panic!("one preset expected") };
    assert_eq!((p.name.as_str(), p.group.as_str(), p.builtin), ("Plain Stitch", "Stitches", false));
    let b = &p.brush;
    assert!((b.size - 240.0).abs() < 1e-3 && (b.opacity - 0.9).abs() < 1e-6 && (b.spacing - 0.55).abs() < 1e-6, "{b:?}");
    assert!(!b.pressure_size && (b.smoothing.amount - 0.3).abs() < 1e-6);
    let sd = &b.shape_dynamics;
    assert!(sd.enabled && sd.angle.control == Control::Direction && (sd.angle.jitter - 0.25).abs() < 1e-6);
    assert_eq!(sd.size.control, Control::PenPressure);
    assert!((sd.size.minimum - 0.25).abs() < 1e-6);
    assert!(sd.size.curve.eval(0.6) < 0.3, "the pressure curve comes along: {:?}", sd.size.curve.points());
    assert!(b.transfer.enabled && (b.transfer.opacity.jitter - 0.4).abs() < 1e-6);
    assert!(!b.texture.enabled && !b.scattering.enabled);
    // The whole square is the tip, and white paints.
    let t = sampled(b);
    assert_eq!((t.width, t.height), (32, 32));
    assert!(paint_at(t, 16, 15) > 0.99 && paint_at(t, 1, 1) < 0.01);
}

#[test]
fn inverted_shapes_paint_their_dark_parts() {
    let archive = write_archive(&[("name", text("Sequin")), ("shapeInverted", Field::Flag(true))]);
    let imp = read_procreate(&brush_zip(&archive, Some(&bar()), None), "g").unwrap();
    let t = sampled(&imp.presets[0].brush);
    assert!(paint_at(t, 16, 15) < 0.01 && paint_at(t, 1, 1) > 0.99);
    // Transparent pixels never paint, inverted or not.
    let clear = png(4, 4, ChannelLayout::GrayA, |_, _| vec![0, 0]);
    let imp = read_procreate(&brush_zip(&archive, Some(&clear), None), "g").unwrap();
    assert!(sampled(&imp.presets[0].brush).data.iter().all(|&v| v == 0));
}

#[test]
fn grain_becomes_a_texture_on_every_stamp() {
    let grain = png(8, 8, ChannelLayout::Rgba, |x, _| vec![if x < 4 { 0 } else { 255 }, 0, 0, 255]);
    let archive = write_archive(&[
        ("textureInverted", Field::Flag(true)),
        ("grainDepth", Field::Num(0.6)),
        ("textureScale", Field::Num(0.5)),
        // Photoshop's Brightness -30, as Procreate keeps it when it converts an .abr brush.
        ("textureBrightness", Field::Num(-30.0 / 255.0)),
        ("textureContrast", Field::Num(0.4)),
    ]);
    let imp = read_procreate(&brush_zip(&archive, Some(&bar()), Some(&grain)), "g").unwrap();
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    let t = &imp.presets[0].brush.texture;
    assert!(t.enabled && t.invert && t.each_tip && (t.depth - 0.6).abs() < 1e-6 && (t.scale - 0.5).abs() < 1e-6, "{t:?}");
    assert!((t.brightness * 150.0 + 30.0).abs() < 1e-3 && (t.contrast - 0.4).abs() < 1e-6, "{t:?}");
    let Pattern::Tile(tile) = &t.pattern else { panic!("a tile pattern expected") };
    assert_eq!((tile.width, tile.height), (8, 8));
    // Luminance of pure red, then of black.
    assert!((paint_at(tile, 6, 0) - 0.299).abs() < 0.01 && paint_at(tile, 1, 0) < 0.01);
    // A grain blend mode other than Procreate's default is listed.
    let archive = write_archive(&[("grainBlendMode", Field::Num(27.0))]);
    let imp = read_procreate(&brush_zip(&archive, Some(&bar()), Some(&grain)), "g").unwrap();
    assert!(imp.warnings[0].ends_with("left out: Grain Blend Mode"), "{:?}", imp.warnings);
}

#[test]
fn transparent_grain_lets_paint_through_inverted_or_not() {
    let grain = png(4, 4, ChannelLayout::Rgba, |_, _| vec![0, 0, 0, 0]);
    for inverted in [false, true] {
        let archive = write_archive(&[("textureInverted", Field::Flag(inverted))]);
        let imp = read_procreate(&brush_zip(&archive, Some(&bar()), Some(&grain)), "g").unwrap();
        let t = &imp.presets[0].brush.texture;
        let Pattern::Tile(tile) = &t.pattern else { panic!("a tile pattern expected") };
        let level = if t.invert { 1.0 - paint_at(tile, 1, 1) } else { paint_at(tile, 1, 1) };
        assert!(level > 0.99, "inverted {inverted}: {level}");
    }
}

#[test]
fn sets_follow_their_plist_and_skip_reset_copies() {
    let a = |n: &str| write_archive(&[("name", text(n))]);
    let set = write_brushset("Cat's Basics & more", &[(a("One"), Some(bar()), None), (a("Two"), Some(bar()), None), (a("Three"), Some(bar()), None)]);
    let imp = read_procreate(&set, "cats").unwrap();
    assert_eq!(imp.set_name.as_deref(), Some("Cat's Basics & more"));
    assert_eq!(imp.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["One", "Two", "Three"]);

    // Without brushset.plist: every folder's top-level archive, never its Reset/ copy.
    let mut z = ZipWriter::new();
    for (dir, name) in [("B", "Bee"), ("A", "Ay")] {
        z.add(&format!("{dir}/Reset/Brush.archive"), &a("old")).unwrap();
        z.add(&format!("{dir}/Brush.archive"), &a(name)).unwrap();
        z.add(&format!("{dir}/Shape.png"), &bar()).unwrap();
    }
    let imp = read_procreate(&z.finish().unwrap(), "g").unwrap();
    assert_eq!(imp.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Bee", "Ay"]);
    assert!(imp.set_name.is_none());
}

#[test]
fn a_plist_declaring_entities_is_refused() {
    // Declared entities could expand far past the file's size, so any declaration is refused.
    let mut dtd = String::from("<!ENTITY e0 \"lol\">");
    for i in 1..10 {
        dtd += &format!("<!ENTITY e{i} \"{}\">", format!("&e{};", i - 1).repeat(10));
    }
    let plist = format!("<?xml version=\"1.0\"?><!DOCTYPE plist [{dtd}]><plist><dict><key>name</key><string>&e9;&e9;</string></dict></plist>");
    let mut z = ZipWriter::new();
    z.add("brushset.plist", plist.as_bytes()).unwrap();
    z.add("A/Brush.archive", &write_archive(&[("name", text("Ay"))])).unwrap();
    z.add("A/Shape.png", &bar()).unwrap();
    let imp = read_procreate(&z.finish().unwrap(), "g").unwrap();
    assert_eq!(imp.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Ay"]);
    assert!(imp.set_name.is_none());
    assert!(imp.warnings[0].contains("entity declarations are not allowed"), "{:?}", imp.warnings);
}

#[test]
fn missing_and_built_in_parts_are_reported() {
    let archive = write_archive(&[("name", text("Builtin")), ("bundledShapePath", text("Shape-Hard.png")), ("bundledGrainPath", text("Grain-Paper.png"))]);
    let imp = read_procreate(&brush_zip(&archive, None, None), "g").unwrap();
    assert!(matches!(imp.presets[0].brush.tip, TipShape::Round));
    assert!(!imp.presets[0].brush.texture.enabled);
    assert_eq!(imp.warnings.len(), 2, "{:?}", imp.warnings);
    assert!(imp.warnings[0].contains("built-in shape \"Shape-Hard.png\"") && imp.warnings[1].contains("built-in grain \"Grain-Paper.png\""));

    // A set listing a brush it doesn't hold, and one whose archive is damaged.
    let good = write_archive(&[("name", text("Good"))]);
    let mut z = ZipWriter::new();
    let plist = "<plist><dict><key>brushes</key><array><string>X</string><string>gone</string><string>Y</string></array></dict></plist>";
    z.add("brushset.plist", plist.as_bytes()).unwrap();
    z.add("X/Brush.archive", b"bplist00 broken").unwrap();
    z.add("Y/Brush.archive", &good).unwrap();
    let imp = read_procreate(&z.finish().unwrap(), "g").unwrap();
    assert_eq!(imp.presets.len(), 1);
    assert!(imp.warnings.iter().any(|w| w.contains("gone")) && imp.warnings.iter().any(|w| w.starts_with("brush 1:")), "{:?}", imp.warnings);
}

/// A set of many missing and damaged brushes reports them in a few lines, not one per brush.
#[test]
fn many_bad_brushes_are_summed_up() {
    let mut z = ZipWriter::new();
    let ids: String = (0..30).map(|i| format!("<string>b{i}</string>")).collect();
    z.add("brushset.plist", format!("<plist><dict><key>brushes</key><array>{ids}</array></dict></plist>").as_bytes()).unwrap();
    for i in 0..14 {
        z.add(&format!("b{i}/Brush.archive"), b"broken").unwrap();
    }
    z.add("b14/Brush.archive", &write_archive(&[])).unwrap();
    let imp = read_procreate(&z.finish().unwrap(), "g").unwrap();
    assert_eq!(imp.presets.len(), 1);
    assert_eq!(imp.warnings.len(), 13, "{:?}", imp.warnings);
    assert_eq!(imp.warnings[0], "15 listed brushes are not in the file; skipped");
    assert!(imp.warnings[1..11].iter().all(|w| w.starts_with("brush ")) && imp.warnings[11].contains("no shape image"));
    assert_eq!(imp.warnings[12], "4 more brushes could not be read; skipped");
}

#[test]
fn settings_without_an_equivalent_are_listed_once() {
    let archive = write_archive(&[
        ("pencilTaperStartLength", Field::Num(0.2)),
        ("pencilTaperSize", Field::Num(1.0)),
        ("dynamicsSpeedSize", Field::Num(0.5)),
        ("dynamicsMix", Field::Num(0.3)),
        ("blendMode", Field::Num(3.0)),
        ("shapeRotation", Field::Num(-1.0)),
        ("shapeRandomise", Field::Flag(true)),
        // A touch taper with no size or opacity change does nothing.
        ("taperEndLength", Field::Num(0.5)),
    ]);
    let set = write_brushset("s", &[(archive.clone(), Some(bar()), None), (archive, Some(bar()), None)]);
    let imp = read_procreate(&set, "g").unwrap();
    assert_eq!(
        imp.warnings,
        [
            "Procreate settings without a PhotoCraft equivalent were left out: Blend Mode, Pressure Taper, Randomized, Rotation against the stroke, Speed dynamics, Wet Mix"
        ]
    );
}

#[test]
fn sizes_and_opacities_keep_to_their_limits() {
    let read = |fields: &[(&str, Field)]| read_procreate(&brush_zip(&write_archive(fields), Some(&bar()), None), "g").unwrap().presets[0].brush.clone();
    // A size set before its limit was lowered: the limit wins.
    let b = read(&[("paintSize", Field::Num(0.0674)), ("maxSize", Field::Num(0.0399)), ("minOpacity", Field::Num(1.0)), ("paintOpacity", Field::Num(0.2))]);
    assert!((b.size - 39.9).abs() < 1e-3 && b.opacity == 1.0, "{} {}", b.size, b.opacity);
    // Swapped limits, absurd values and missing fields stay in range.
    let b = read(&[("paintSize", Field::Num(1e30)), ("minSize", Field::Num(0.9)), ("maxSize", Field::Num(0.1)), ("plotSpacing", Field::Num(f64::NAN))]);
    assert!((b.size - 900.0).abs() < 1e-3 && b.spacing == BrushSettings::default().spacing);
    let b = read(&[("shapeCount", Field::Num(0.53)), ("shapeCountJitter", Field::Num(0.7)), ("plotJitter", Field::Num(25.0))]);
    let sc = &b.scattering;
    assert!(sc.enabled && sc.count == 9 && sc.both_axes && sc.scatter.jitter == MAX_SCATTER && (sc.count_jitter.jitter - 0.7).abs() < 1e-6);
}

#[test]
fn bad_files_fail_cleanly() {
    for bytes in [&b""[..], b"PK\x03\x04 nope", b"just text"] {
        assert!(read_procreate(bytes, "g").is_err());
    }
    let mut z = ZipWriter::new();
    z.add("readme.txt", b"no brushes").unwrap();
    assert!(read_procreate(&z.finish().unwrap(), "g").unwrap_err().contains("no Procreate brushes"));
    // Not a keyed archive, and not a property list at all.
    let other = write(&Value::Dict(vec![("$archiver".into(), Value::Str("Other".into()))]));
    assert!(read_procreate(&brush_zip(&other, Some(&bar()), None), "g").is_err());
    assert!(read_procreate(&brush_zip(b"{}", Some(&bar()), None), "g").is_err());
    // A shape whose header claims 100000 × 100000 pixels is refused before decoding.
    let mut huge = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&[8, 0, 0, 0, 0]);
    let crc = photocraft_format::zip::crc32(&huge[12..]);
    huge.extend_from_slice(&crc.to_be_bytes());
    let err = read_procreate(&brush_zip(&write_archive(&[]), Some(&huge), None), "g").unwrap_err();
    assert!(err.contains("Shape.png"), "{err}");
}

#[test]
fn damaged_sets_never_panic() {
    let archive = write_archive(&[("name", text("N")), ("dynamicsPressureSizeCurve", Field::Curve(vec![[0.0, 0.0], [1.0, 1.0]]))]);
    let grain = png(4, 4, ChannelLayout::Gray, |x, y| vec![(x * 60 + y) as u8]);
    let set = write_brushset("s", &[(archive.clone(), Some(bar()), Some(grain)), (archive, Some(bar()), None)]);
    for n in (0..set.len()).step_by(7) {
        let _ = read_procreate(&set[..n], "g");
    }
    for i in (0..set.len()).step_by(3) {
        let mut b = set.clone();
        b[i] ^= 0x5A;
        let _ = read_procreate(&b, "g");
    }
}

#[test]
fn a_cancelled_import_stops() {
    let set = write_brushset("s", &[(write_archive(&[]), Some(bar()), None)]);
    let (cancel, progress) = (|| true, |_: f32| {});
    let err = read_procreate_with(&set, "g", &Interrupt::new(&cancel, &progress)).unwrap_err();
    assert_eq!(err, "cancelled");
}
