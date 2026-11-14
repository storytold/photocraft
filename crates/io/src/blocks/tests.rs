use super::*;

fn noise_gradient_fill(min: [i32; 4], max: [i32; 4], add_transparency: bool) -> Descriptor {
    let values = |v: [i32; 4]| Value::List(v.into_iter().map(Value::Integer).collect());
    let gradient = Descriptor::new("Grdn")
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("ClNs") })
        .with("ClrS", Value::Enumerated { type_id: Id::new("ClrS"), value: Id::new("RGBC") })
        .with("ShTr", Value::Boolean(add_transparency))
        .with("VctC", Value::Boolean(true))
        .with("RndS", Value::Integer(9052369))
        .with("Smth", Value::Integer(2048))
        .with("Mnm ", values(min))
        .with("Mxm ", values(max));
    Descriptor::new("null").with("Grad", Value::Descriptor(gradient))
}

#[test]
fn constant_rgb_noise_fill_uses_its_exact_colour() {
    use photocraft_doc::Fill;
    let fill = fill_from_desc(b"GdFl", &noise_gradient_fill([50, 25, 0, 0], [50, 25, 0, 100], false));
    let Some(Fill::Gradient { stops, .. }) = fill else { panic!("constant noise gradient") };
    assert_eq!(stops, vec![(0.0, Color::rgb(0.5, 0.25, 0.0)), (1.0, Color::rgb(0.5, 0.25, 0.0))]);
}

#[test]
fn variable_or_transparent_noise_stays_a_fill_with_the_placeholder_ramp() {
    use photocraft_doc::Fill;
    // Photoshop's default Noise gradient (min 0, max 100) and a transparent variant: no
    // exact colour is provable, but the fill must parse — the layer keeps its type, its
    // stored pixels (`fill_cache`) and its raw `GdFl` block. Only the uncached preview
    // shows the placeholder ramp.
    for descriptor in [noise_gradient_fill([0, 0, 0, 0], [100, 100, 100, 100], false), noise_gradient_fill([50, 25, 0, 0], [50, 25, 0, 100], true)] {
        let Some(Fill::Gradient { stops, .. }) = fill_from_desc(b"GdFl", &descriptor) else { panic!("variable noise gradient must stay a fill layer") };
        assert_eq!(stops, vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
    }
}

/// psd-tools layers-minimal/gradient-fill.psd ("Color to Transparent", no stored pixels): a
/// gradient fill's opacity stops reach the model (as its opacity stops; written from stop
/// alpha) and survive a write; the ramp's alpha follows them.
#[test]
fn gradient_fill_opacity_stops_become_alpha() {
    use photocraft_color::Color;
    use photocraft_doc::{Fill, GradientStyle};
    let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
    let f = Fill::gradient(vec![(0.0, red), (1.0, Color { alpha: 0.0, ..red })], 90.0, 1.0, GradientStyle::Linear, false);
    let (k, data) = super::write_fill(&f);
    let back = super::parse_fill(&k, &data).unwrap();
    let Fill::Gradient { stops, opacity_stops, .. } = &back else { panic!("gradient") };
    assert_eq!(opacity_stops, &vec![(0.0, 1.0), (1.0, 0.0)]);
    assert!(stops.iter().all(|s| s.1.c[0] == 1.0 && s.1.c[1] == 0.0));
    let ramp = photocraft_compose::gradient_fill::Ramp::new(&back).unwrap();
    assert_eq!(ramp.sample(0.0)[3], 1.0);
    assert!(ramp.sample(1.0)[3] < 0.01);
    // Opaque gradients come back without opacity stops.
    let (k, data) = super::write_fill(&Fill::gradient(vec![(0.0, red), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false));
    let Some(Fill::Gradient { opacity_stops, .. }) = super::parse_fill(&k, &data) else { panic!("gradient") };
    assert!(opacity_stops.is_empty());
}

/// Photoshop 27.11 stores a gradient's method as the enum `gradientsInterpolationMethod`
/// (seen in gradient fill layers saved by scripted Photoshop): a black→white Linear gradient
/// is at 188 of 255 halfway (measured), where Classic gives 128. The key used to be read as
/// `gs99`, which Photoshop doesn't write, so every gradient opened as Classic.
#[test]
fn gradient_fill_reads_the_interpolation_method() {
    use craft_psd::descriptor::{Id, Value};
    use photocraft_color::Color;
    use photocraft_doc::{Fill, GradientStyle};
    let f = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false);
    let (k, data) = super::write_fill(&f);
    let d = super::parse_prefix_versioned(&data).unwrap();
    let mid = |code: &str| {
        let d = d.clone().with("gradientsInterpolationMethod", Value::Enumerated { type_id: Id::new("gradientInterpolationMethodType"), value: Id::new(code) });
        let back = super::fill_from_desc(&k, &d).unwrap();
        let ramp = photocraft_compose::gradient_fill::Ramp::new(&back).unwrap();
        (ramp.sample(0.5)[0] * 255.0).round()
    };
    assert_eq!(mid("Gcls"), 128.0);
    assert!((mid("Lnr ") - 188.0).abs() <= 1.0, "Linear: {}", mid("Lnr "));
    assert!((mid("Perc") - 99.0).abs() <= 2.0, "Perceptual: {}", mid("Perc"));
    assert!((mid("Smoo") - 99.0).abs() <= 2.0, "Smooth: {}", mid("Smoo"));
}

#[test]
fn lab_descriptor_colours_round_trip() {
    use craft_psd::descriptor::{Descriptor, Value};
    use photocraft_color::ColorMode;
    let d = Descriptor::new("LbCl").with("Lmnc", Value::Double(19.07)).with("A   ", Value::Double(52.29)).with("B   ", Value::Double(-85.08));
    let c = super::color_from_desc(&d).unwrap();
    assert_eq!(c.mode, ColorMode::Lab);
    assert!((c.c[0] - 0.1907).abs() < 1e-5 && (c.c[1] * 255.0 - 128.0 - 52.29).abs() < 1e-3 && (c.c[2] * 255.0 - 128.0 + 85.08).abs() < 1e-3);
    let back = super::color_from_desc(&super::color_to_desc(&c)).unwrap();
    assert_eq!(back.mode, ColorMode::Lab);
    for i in 0..3 {
        assert!((back.c[i] - c.c[i]).abs() < 1e-6);
    }
    // Missing components are rejected, not defaulted.
    assert!(super::color_from_desc(&Descriptor::new("LbCl").with("Lmnc", Value::Double(50.0))).is_none());
}

/// Colours stored as HSB (`HSBC`: `H   ` an angle in degrees, `Strt`/`Brgh` 0..100), as in
/// psd-tools `stroke-color-descriptors-hsb-with-rgb-mode.psd`, used to be dropped (#6774).
#[test]
fn hsb_descriptor_colours_parse() {
    use craft_psd::descriptor::{Descriptor, Value};
    use photocraft_doc::Fill;
    let hsb = |h: f64, s: f64, b: f64| {
        Descriptor::new("HSBC").with("H   ", Value::UnitFloat { unit: *b"#Ang", value: h }).with("Strt", Value::Double(s)).with("Brgh", Value::Double(b))
    };
    let close = |c: [f32; 3], e: [f32; 3]| c.iter().zip(e).all(|(a, b)| (a - b).abs() < 1e-5);
    let green = super::color_from_desc(&hsb(120.0, 100.0, 100.0)).unwrap().to_rgb();
    assert!(close(green, [0.0, 1.0, 0.0]), "{green:?}");
    let white = super::color_from_desc(&hsb(0.0, 0.0, 100.0)).unwrap().to_rgb();
    assert!(close(white, [1.0, 1.0, 1.0]), "{white:?}");
    let half = super::color_from_desc(&hsb(240.0, 50.0, 50.0)).unwrap().to_rgb();
    assert!(close(half, [0.25, 0.25, 0.5]), "{half:?}");
    // A solid fill (`SoCo`) with an HSB colour is read, not lost.
    let soco = Descriptor::new("null").with("Clr ", Value::Descriptor(hsb(0.0, 100.0, 100.0)));
    let Some(Fill::Solid(c)) = super::fill_from_desc(b"SoCo", &soco) else { panic!("solid fill") };
    assert!(close(c.to_rgb(), [1.0, 0.0, 0.0]), "{:?}", c.to_rgb());
    // Missing components are rejected, not defaulted.
    assert!(super::color_from_desc(&Descriptor::new("HSBC").with("Strt", Value::Double(50.0))).is_none());
}

#[test]
fn color_descriptor_keeps_photocraft_alpha() {
    use photocraft_color::Color;
    use photocraft_doc::Fill;
    let c = Color::rgba(0.2, 0.4, 0.6, 0.25);
    let (k, d) = super::fill_to_desc(&Fill::Solid(c));
    let Fill::Solid(back) = super::fill_from_desc(&k, &d).unwrap() else { panic!("solid") };
    assert!((back.alpha - 0.25).abs() < 1e-5);
    assert!((back.to_rgb()[0] - 0.2).abs() < 1e-3);
    let (k, d) = super::fill_to_desc(&Fill::Solid(Color::rgb(1.0, 0.0, 0.0)));
    let Fill::Solid(opaque) = super::fill_from_desc(&k, &d).unwrap() else { panic!("solid") };
    assert!((opaque.alpha - 1.0).abs() < 1e-6);
    assert!(super::color_to_desc(&c).get("pcAl").is_none(), "generic colour descriptors have no pcAl");
}

#[test]
fn non_finite_photocraft_alpha_is_ignored() {
    use photocraft_color::Color;
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let d = super::color_to_desc(&Color::rgb(0.2, 0.4, 0.6)).with("pcAl", Value::Double(bad));
        let c = super::color_from_desc(&d).unwrap();
        assert_eq!(c.alpha, 1.0, "pcAl {bad}");
    }
    let d = super::color_to_desc(&Color::rgb(0.2, 0.4, 0.6)).with("pcAl", Value::Double(0.5));
    assert!((super::color_from_desc(&d).unwrap().alpha - 0.5).abs() < 1e-6);
}

#[test]
fn gradient_stop_alpha_round_trips_once() {
    use photocraft_color::Color;
    use photocraft_doc::Fill;
    let fill = Fill::Gradient {
        stops: vec![(0.0, Color::rgba(1.0, 0.0, 0.0, 0.5)), (1.0, Color::WHITE)],
        angle: 90.0,
        scale: 1.0,
        style: photocraft_doc::GradientStyle::Linear,
        reverse: false,
        opacity_stops: vec![],
        midpoints: vec![],
        offset: (0.0, 0.0),
        dither: false,
        align: true,
        interpolation: None,
    };
    let (k, d) = super::fill_to_desc(&fill);
    let Fill::Gradient { stops, opacity_stops, .. } = super::fill_from_desc(&k, &d).unwrap() else { panic!("grad") };
    assert!((stops[0].1.alpha - 1.0).abs() < 1e-4, "colour alpha lives in Trns, not the stop colour");
    assert!(opacity_stops.iter().any(|(_, a)| (*a - 0.5).abs() < 1e-3), "{opacity_stops:?}");
}

#[test]
fn blending_ranges_map_to_blend_if() {
    use craft_psd::layer::BlendingRanges;
    use photocraft_doc::{BlendIf, BlendRange};
    // Full ranges (what every layer without Blend If carries) → the default setting.
    assert_eq!(super::blend_if_from_ranges(&BlendingRanges::full(3)), BlendIf::default());
    assert_eq!(super::blend_if_from_ranges(&BlendingRanges::default()), BlendIf::default());
    // Gray: This Layer black split 10/40; Blue (entry 3): Underlying white at 200.
    let mut data = BlendingRanges::full(3).data;
    data[..4].copy_from_slice(&[10, 40, 255, 255]);
    data[3 * 8 + 4..3 * 8 + 8].copy_from_slice(&[0, 0, 200, 200]);
    let b = super::blend_if_from_ranges(&BlendingRanges { data: data.clone() });
    assert_eq!(b.get(0), [BlendRange { black: [10, 40], white: [255, 255] }, BlendRange::FULL]);
    assert_eq!(b.get(3), [BlendRange::FULL, BlendRange { black: [0, 0], white: [200, 200] }]);
    assert_eq!(b.ranges.len(), 4);
    assert_eq!(super::ranges_from_blend_if(&b, 3).data, data);
    // The default writes full ranges for gray + each channel.
    assert_eq!(super::ranges_from_blend_if(&BlendIf::default(), 4), BlendingRanges::full(4));
    // A trimmed setting is padded to the document's channel count.
    let mut short = BlendIf::default();
    short.set(0, [BlendRange { black: [5, 5], white: [255, 255] }, BlendRange::FULL]);
    let r = super::ranges_from_blend_if(&short, 3);
    assert_eq!(r.data.len(), 4 * 8);
    assert_eq!(&r.data[..8], &[5, 5, 255, 255, 0, 0, 255, 255]);
    assert_eq!(&r.data[8..], &BlendingRanges::full(2).data[..]);
}

#[test]
fn advanced_blending_blocks_round_trip() {
    use photocraft_doc::{AdvancedBlending, Knockout};
    let a = AdvancedBlending {
        knockout: Knockout::Deep,
        blend_interior: true,
        blend_clipped: false,
        transparency_shapes: false,
        layer_mask_hides_effects: true,
        vector_mask_hides_effects: true,
    };
    let mut raw = Vec::new();
    super::put_advanced(&a, &mut raw);
    assert_eq!(raw.len(), 6);
    assert!(raw.contains(&(*b"knko", vec![2, 0, 0, 0])));
    assert!(raw.contains(&(*b"clbl", vec![0, 0, 0, 0])));
    let get = |k: &[u8; 4]| raw.iter().find(|(bk, _)| bk == k).map(|(_, d)| d.as_slice());
    assert_eq!(super::advanced_from_blocks(get), a);
    // Defaults: nothing added to a plain layer; existing blocks rewritten in place, in order.
    let mut plain = Vec::new();
    super::put_advanced(&AdvancedBlending::default(), &mut plain);
    assert!(plain.is_empty());
    let mut kept = vec![(*b"luni", vec![0; 4]), (*b"knko", vec![1, 0, 0, 0]), (*b"clbl", vec![1])];
    super::put_advanced(&AdvancedBlending::default(), &mut kept);
    assert_eq!(kept, vec![(*b"luni", vec![0; 4]), (*b"knko", vec![0, 0, 0, 0]), (*b"clbl", vec![1, 0, 0, 0])]);
    // Missing and empty blocks read as Photoshop's defaults.
    assert!(super::advanced_from_blocks(|_| None).is_default());
    assert!(super::advanced_from_blocks(|_| Some(&[][..])).is_default());
    assert_eq!(super::advanced_from_blocks(|k| (k == b"knko").then_some(&[1u8][..])).knockout, Knockout::Shallow);
}

#[test]
fn brst_round_trip() {
    assert_eq!(super::parse_brst(&[0, 0, 0, 2]), 0b100);
    assert_eq!(super::parse_brst(&[0, 0, 0, 0, 0, 0, 0, 1]), 0b11);
    assert_eq!(super::brst_data(0b101).unwrap(), vec![0, 0, 0, 0, 0, 0, 0, 2]);
    assert_eq!(super::brst_data(0), None);
}

#[test]
fn placed_layer_warp_is_read() {
    use craft_psd::descriptor::ObjectArray;
    use photocraft_geom::warp::WarpStyle;
    let e = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let bounds = Descriptor::new("Rctn")
        .with("Top ", Value::UnitFloat { unit: *b"#Pxl", value: 0.0 })
        .with("Left", Value::UnitFloat { unit: *b"#Pxl", value: 0.0 })
        .with("Btom", Value::UnitFloat { unit: *b"#Pxl", value: 30.0 })
        .with("Rght", Value::UnitFloat { unit: *b"#Pxl", value: 60.0 });
    let sold = |warp: Descriptor| {
        let mut v = b"soLD".to_vec();
        v.extend_from_slice(&4u32.to_be_bytes());
        v.extend(VersionedDescriptor::new(Descriptor::new("null").with("warp", Value::Descriptor(warp))).to_bytes());
        v
    };
    let arc = Descriptor::new("warp")
        .with("warpStyle", e("warpStyle", "warpArc"))
        .with("warpValue", Value::Double(40.0))
        .with("warpPerspective", Value::Double(0.0))
        .with("warpPerspectiveOther", Value::Double(0.0))
        .with("warpRotate", e("Ornt", "Hrzn"))
        .with("bounds", Value::Descriptor(bounds.clone()));
    let w = parse_placed_warp(b"SoLd", &sold(arc)).unwrap();
    assert_eq!((w.style, w.bend, w.bounds), (WarpStyle::Arc, 40.0, [0.0, 0.0, 60.0, 30.0]));
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for j in 0..4 {
        for i in 0..4 {
            xs.push(f64::from(i) * 20.0 + if i == 3 && j == 3 { 7.0 } else { 0.0 });
            ys.push(f64::from(j) * 10.0);
        }
    }
    let mesh =
        Descriptor::new("null").with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: xs }).with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: ys });
    let custom = Descriptor::new("warp").with("warpStyle", e("warpStyle", "warpCustom")).with("bounds", Value::Descriptor(bounds)).with(
        "customEnvelopeWarp",
        Value::Descriptor(Descriptor::new("customEnvelopeWarp").with("meshPoints", Value::ObjectArray(ObjectArray { prefix: 16, body: mesh }))),
    );
    let w = parse_placed_warp(b"SoLd", &sold(custom)).unwrap();
    assert_eq!(w.style, WarpStyle::Custom);
    assert_eq!(w.mesh.as_ref().unwrap().points[15], [67.0, 30.0]);
    assert!(parse_placed_warp(b"PlLd", &[0; 20]).is_none());
}

#[test]
fn locks_and_labels() {
    // Reference values round-trip byte for byte; Background layers carry 0x0D
    // (transparency + position + the bit-3 lock reference files write).
    for bits in [0u32, 1, 2, 4, 8, 0x0D, 0x8000_0000, 0x8000_000F] {
        assert_eq!(lspf_from_locks(&locks_from_lspf(bits)), bits);
    }
    // Older PhotoCraft files wrote the artboard lock at 0x10; it reads back as artboard
    // and is rewritten at the reference value 0x08.
    assert!(locks_from_lspf(0x10).artboard);
    assert_eq!(lspf_from_locks(&locks_from_lspf(0x10)), 0x08);
    for i in 0..8 {
        assert_eq!(label_index(label_from_index(i)), i);
    }
}

#[test]
fn fills_roundtrip() {
    for f in [
        Fill::Solid(Color::rgb(1.0, 0.5, 0.0)),
        Fill::Solid(Color { mode: ColorMode::Cmyk, c: [0.1, 0.2, 0.3, 0.4], alpha: 1.0 }),
        Fill::Solid(Color::gray(0.25)),
        Fill::gradient(vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))], 45.0, 1.0, GradientStyle::Reflected, true),
        // A live gradient (Gradient tool): canvas-aligned, offset, midpoints, opacity, dither.
        Fill::Gradient {
            stops: vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (0.5, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
            angle: -33.5,
            scale: 2.25,
            style: GradientStyle::Radial,
            reverse: false,
            opacity_stops: vec![(0.0, 1.0), (0.25, 0.5), (1.0, 0.0)],
            midpoints: vec![0.25, 0.75],
            offset: (0.125, -0.375),
            dither: true,
            align: false,
            interpolation: None,
        },
        Fill::Pattern { name: "Bubbles".into(), scale: 0.5, id: "abc".into(), angle: 30.0, link: false, phase: (3.0, -2.0) },
    ] {
        let (k, d) = write_fill(&f);
        let back = parse_fill(&k, &d).unwrap();
        match (&f, &back) {
            (Fill::Solid(a), Fill::Solid(b)) => {
                assert_eq!(a.mode, b.mode);
                for i in 0..4 {
                    assert!((a.c[i] - b.c[i]).abs() < 1e-5);
                }
            }
            _ => assert_eq!(back, f),
        }
    }
}

/// A Photoshop gradient fill (`GdFl`) descriptor: colour stops `(location 0..4096, rgb 0..1,
/// midpoint % of the segment it ends)`, opacity stops `(location, %, midpoint %)`, Smoothness
/// `intr` (0..4096) and the method code (`gradientsInterpolationMethod`, as Photoshop 27.11
/// writes it).
fn photoshop_gdfl(colours: &[(i32, [f64; 3], i32)], opacity: &[(i32, f64, i32)], intr: f64, method: &str) -> Descriptor {
    let clrs = colours
        .iter()
        .map(|(loc, c, mid)| {
            let rgb = Descriptor::new("RGBC")
                .with("Rd  ", Value::Double(c[0] * 255.0))
                .with("Grn ", Value::Double(c[1] * 255.0))
                .with("Bl  ", Value::Double(c[2] * 255.0));
            Value::Descriptor(
                Descriptor::new("Clrt")
                    .with("Clr ", Value::Descriptor(rgb))
                    .with("Type", Value::Enumerated { type_id: Id::new("Clry"), value: Id::new("UsrS") })
                    .with("Lctn", Value::Integer(*loc))
                    .with("Mdpn", Value::Integer(*mid)),
            )
        })
        .collect();
    let trns = opacity
        .iter()
        .map(|(loc, op, mid)| {
            Value::Descriptor(
                Descriptor::new("TrnS")
                    .with("Opct", Value::UnitFloat { unit: *b"#Prc", value: *op })
                    .with("Lctn", Value::Integer(*loc))
                    .with("Mdpn", Value::Integer(*mid)),
            )
        })
        .collect();
    let grad = Descriptor::new("Grdn")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Custom")))
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("CstS") })
        .with("Intr", Value::Double(intr))
        .with("Clrs", Value::List(clrs))
        .with("Trns", Value::List(trns));
    Descriptor::new("null")
        .with("Angl", Value::UnitFloat { unit: *b"#Ang", value: 90.0 })
        .with("Type", Value::Enumerated { type_id: Id::new("GrdT"), value: Id::new("Lnr ") })
        .with("Grad", Value::Descriptor(grad))
        .with("gradientsInterpolationMethod", Value::Enumerated { type_id: Id::new("gradientInterpolationMethodType"), value: Id::new(method) })
}

/// Red → green (midpoint 30 %) → blue (midpoint 50 %), opaque.
const THREE: [(i32, [f64; 3], i32); 3] = [(0, [1.0, 0.0, 0.0], 50), (2048, [0.0, 1.0, 0.0], 30), (4096, [0.0, 0.0, 1.0], 50)];
const OPAQUE: [(i32, f64, i32); 2] = [(0, 100.0, 50), (4096, 100.0, 50)];

/// Write then read a fill.
fn rewritten(f: &Fill) -> Fill {
    let (k, data) = write_fill(f);
    parse_fill(&k, &data).unwrap()
}

/// #13557: a gradient fill with Smoothness 100 (Photoshop's default) keeps Photoshop's three
/// stops, their midpoint, the Smoothness and the method next to the dense stops it renders, and
/// a save writes them back, so reading it again gives the same fill.
#[test]
fn gradient_fill_keeps_photoshops_stops_midpoints_and_smoothness() {
    use photocraft_compose::gradient_fill::Ramp;
    let f = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &OPAQUE, 4096.0, "Gcls")).unwrap();
    let Fill::Gradient { stops, midpoints, interpolation: Some(src), .. } = &f else { panic!("{f:?}") };
    assert!(stops.len() > 90, "rendered from the baked stops: {}", stops.len());
    assert!(midpoints.is_empty(), "the bake folds the midpoints in");
    assert_eq!(src.stops.len(), 3, "Photoshop's three stops are kept");
    assert_eq!((src.method.as_str(), src.smoothness), ("classic", 100.0));
    assert!((src.midpoints[0] - 0.3).abs() < 1e-6 && (src.midpoints[1] - 0.5).abs() < 1e-6, "{:?}", src.midpoints);
    assert_eq!(*stops, crate::gradient_bake::bake_interpolation(src), "the baked stops are the kept gradient baked");
    // The PSD writer writes Photoshop's gradient, not the 97 baked stops.
    let (_, data) = write_fill(&f);
    let d = parse_prefix_versioned(&data).unwrap();
    let Some(Value::List(clrs)) = get_desc(&d, "Grad").and_then(|g| g.get("Clrs")) else { panic!("Clrs") };
    assert_eq!(clrs.len(), 3);
    assert_eq!(num(get_desc(&d, "Grad").and_then(|g| g.get("Intr"))), Some(4096.0));
    assert_eq!(enum_of(&d, "gradientsInterpolationMethod"), Some(&b"Gcls"[..]));
    let back = rewritten(&f);
    assert_eq!(back, f, "read → write → read keeps the fill");
    // It renders as it did.
    let (a, b) = (Ramp::new(&f).unwrap(), Ramp::new(&back).unwrap());
    for k in 0..=20 {
        assert_eq!(a.sample(k as f32 / 20.0), b.sample(k as f32 / 20.0));
    }
}

/// #13557: Smoothness 0 with a method other than Classic (Perceptual here) is kept too; plain
/// Smoothness 0 Classic needs no baking and stays three stops with their midpoints.
#[test]
fn gradient_fill_source_by_method_and_smoothness() {
    let perceptual = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &OPAQUE, 0.0, "Perc")).unwrap();
    let Fill::Gradient { stops, interpolation: Some(src), .. } = &perceptual else { panic!("{perceptual:?}") };
    assert!(stops.len() > 3);
    assert_eq!((src.stops.len(), src.method.as_str(), src.smoothness), (3, "perceptual", 0.0));
    let back = rewritten(&perceptual);
    assert_eq!(back, perceptual);
    let classic = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &OPAQUE, 0.0, "Gcls")).unwrap();
    let Fill::Gradient { stops, midpoints, interpolation: None, .. } = &classic else { panic!("{classic:?}") };
    assert_eq!(stops.len(), 3, "a plain gradient keeps its stops as they are");
    assert_eq!(midpoints.len(), 2);
    assert_eq!(rewritten(&classic), classic);
}

/// #13557: Photoshop's own opacity stops (with their midpoint) are kept and written back with
/// the colours, so a Perceptual fade isn't baked a second time on the next read.
#[test]
fn gradient_fill_keeps_photoshops_opacity_stops() {
    let fade = [(0, 100.0, 50), (4096, 20.0, 35)];
    let f = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &fade, 4096.0, "Perc")).unwrap();
    let Fill::Gradient { opacity_stops, interpolation: Some(src), .. } = &f else { panic!("{f:?}") };
    assert!(opacity_stops.len() > 2, "the opacity renders baked");
    assert_eq!(src.opacity_stops, vec![(0.0, 1.0), (1.0, 0.2)]);
    assert!((src.opacity_midpoints[0] - 0.35).abs() < 1e-6);
    assert_eq!(rewritten(&f), f);
}

/// #13557: a record left behind by an edit that replaced the stops (not through it) is not
/// written: the save holds the stops the fill renders.
#[test]
fn gradient_fill_writes_its_stops_when_the_record_is_stale() {
    let mut f = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &OPAQUE, 4096.0, "Gcls")).unwrap();
    let Fill::Gradient { stops, .. } = &mut f else { panic!() };
    *stops = vec![(0.0, Color::WHITE), (1.0, Color::BLACK)];
    let back = rewritten(&f);
    let Fill::Gradient { stops, interpolation: None, .. } = &back else { panic!("{back:?}") };
    assert_eq!(stops, &vec![(0.0, Color::WHITE), (1.0, Color::BLACK)]);
}

/// #13557: hostile kept gradients (NaN and out-of-range values, too few stops, an unknown
/// method) neither panic nor get written.
#[test]
fn hostile_gradient_fill_records_are_safe() {
    use photocraft_doc::effects::GradientInterpolation;
    let mut src = GradientInterpolation::new("perceptual", f32::NAN, vec![(f32::NAN, Color::WHITE), (7.0, Color::BLACK), (-3.0, Color::WHITE)]);
    src.midpoints = vec![f32::INFINITY, -5.0, 9.0, 0.5];
    src.opacity_stops = vec![(f32::NAN, 2.0)];
    src.opacity_midpoints = vec![f32::NAN];
    let baked = crate::gradient_bake::bake_interpolation(&src);
    assert!(baked.iter().all(|(t, c)| t.is_finite() && c.c.iter().all(|v| v.is_finite())));
    let _ = crate::gradient_bake::bake_interpolation_opacity(&src);
    let mut f = Fill::gradient(baked, 0.0, 1.0, GradientStyle::Linear, false);
    if let Fill::Gradient { interpolation, .. } = &mut f {
        *interpolation = Some(Box::new(src.clone()));
    }
    let _ = rewritten(&f);
    for broken in
        [GradientInterpolation::new("wavy", 50.0, vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]), GradientInterpolation::new("classic", 50.0, vec![])]
    {
        assert!(!crate::gradient_bake::interpolation_matches(&broken, &[], &[]), "{broken:?}");
    }
}

/// #13557: a kept gradient with a single opacity stop below 100 % keeps that opacity when it is
/// written: one stop holds across the whole ramp, it is not saved as opaque.
#[test]
fn gradient_fill_keeps_a_single_translucent_opacity_stop() {
    use photocraft_compose::gradient_fill::Ramp;
    let f = fill_from_desc(b"GdFl", &photoshop_gdfl(&THREE, &[(2048, 50.0, 50)], 4096.0, "Gcls")).unwrap();
    let Fill::Gradient { interpolation: Some(src), .. } = &f else { panic!("{f:?}") };
    assert_eq!(src.opacity_stops, vec![(0.5, 0.5)]);
    let back = rewritten(&f);
    let (a, b) = (Ramp::new(&f).unwrap(), Ramp::new(&back).unwrap());
    for k in 0..=20 {
        let t = k as f32 / 20.0;
        assert!((a.sample(t)[3] - b.sample(t)[3]).abs() < 1e-4, "at {t}: {:?} vs {:?}", a.sample(t), b.sample(t));
        assert!((b.sample(t)[3] - 0.5).abs() < 1e-3, "at {t}: {:?}", b.sample(t));
    }
}
