//! A selection must reach shape filters as input geometry, while the photo remains behind them.
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_plugins::{Limits, Manifest, Plugin};
use photocraft_raster::Surface;
use serde_json::json;

fn manifest() -> serde_json::Value {
    json!({"id":"test.selection-shape","name":"Selection shape","kind":"filter","area":"canvas","overlap":4,
        "selectionShape":{"parameter":"effect","values":["shape"]},
        "params":{"effect":{"type":"choice","options":["shape","texture"],"default":"shape"}}})
}

fn plugin(filter: &str) -> Plugin {
    plugin_with_limits(filter, Limits::default())
}

fn plugin_with_limits(filter: &str, limits: Limits) -> Plugin {
    let manifest = manifest().to_string();
    let escaped = manifest.replace('\\', "\\\\").replace('"', "\\\"");
    let wasm = wat::parse_str(format!(
        r#"(module
        (memory (export "memory") 1)
        (data (i32.const 16) "{escaped}")
        (func (export "pc_abi_version") (result i32) (i32.const 1))
        (func (export "pc_manifest") (result i64) (i64.const {}))
        (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
            (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
            (i32.mul (local.get $old) (i32.const 65536)))
        (func (export "pc_filter") (param $buf i32) (param $len i32) (param $w i32) (param $h i32)
            (param $ch i32) (param $fmt i32) (param $pp i32) (param $pl i32) (result i32)
            {filter}))"#,
        ((manifest.len() as u64) << 32) | 16
    ))
    .unwrap();
    Plugin::load(&wasm, limits).unwrap()
}

fn input(fmt: PixelFormat, alpha: f32) -> Surface {
    let mut s = Surface::new(fmt);
    let mut pixel = vec![0.3; fmt.channels()];
    if fmt.alpha {
        *pixel.last_mut().unwrap() = alpha;
    }
    s.fill_rect(Rect::new(0, 0, 40, 32), &pixel);
    s
}

fn selection(coverage: f32) -> Surface {
    let mut s = Surface::new(PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 });
    s.fill_rect(Rect::new(10, 8, 24, 24), &[coverage]);
    s
}

#[test]
fn selection_shape_manifest_rejects_invalid_choice_contracts() {
    for contract in [
        json!({"parameter":"missing","values":["shape"]}),
        json!({"parameter":"effect","values":[]}),
        json!({"parameter":"effect","values":["missing"]}),
        json!({"parameter":"effect","values":["shape","shape"]}),
    ] {
        let mut m = manifest();
        m["selectionShape"] = contract;
        assert!(Manifest::parse(&serde_json::to_vec(&m).unwrap()).is_err(), "{m}");
    }
}

#[test]
fn opaque_rgb_selection_exposes_an_edge_to_the_filter() {
    // Red = absolute difference between own alpha and alpha immediately to the right.
    let p = plugin(
        r#"(local $i i32) (local $stride i32) (local $a i32)
        (local.set $stride (i32.mul (local.get $ch) (i32.const 4)))
        (local.set $a (i32.sub (local.get $stride) (i32.const 4)))
        (block $done (loop $loop
            (br_if $done (i32.ge_u (i32.add (local.get $i) (local.get $stride)) (local.get $len)))
            (f32.store (i32.add (local.get $buf) (local.get $i))
                (f32.abs (f32.sub
                    (f32.load (i32.add (i32.add (local.get $buf) (local.get $i)) (local.get $a)))
                    (f32.load (i32.add (i32.add (i32.add (local.get $buf) (local.get $i)) (local.get $a)) (local.get $stride))))))
            (local.set $i (i32.add (local.get $i) (local.get $stride))) (br $loop))) (i32.const 0)"#,
    );
    for sample in SampleType::ALL {
        for base in [PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, PixelFormat::RGBA8, PixelFormat::GRAY8, PixelFormat::GRAYA8] {
            let s = input(PixelFormat { sample, ..base }, 1.0);
            let out = p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(1.0)), &json!({})).unwrap();
            assert_eq!(out.format(), s.format());
            assert!(out.pixel(23, 16)[0] > 0.99, "selection edge: {sample:?} {base:?}");
            assert_eq!(out.pixel(30, 16), s.pixel(30, 16), "photo outside shape preserved");
        }
    }
}

#[test]
fn identity_shape_does_not_double_source_alpha_or_feather_coverage() {
    let p = plugin("(i32.const 0)");
    for sample in SampleType::ALL {
        for base in [PixelFormat::RGBA8, PixelFormat::GRAY8, PixelFormat::GRAYA8] {
            for alpha in [0.0, 0.2, 0.6, 1.0] {
                for coverage in [0.0, 0.25, 0.75, 1.0] {
                    let s = input(PixelFormat { sample, ..base }, alpha);
                    let out = p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(coverage)), &json!({})).unwrap();
                    for (a, b) in s.read_region(Rect::new(0, 0, 40, 32)).iter().zip(out.read_region(Rect::new(0, 0, 40, 32))) {
                        assert!((a - b).abs() < 0.000001, "{sample:?} {base:?} alpha {alpha}, coverage {coverage}: {a} -> {b}");
                    }
                }
            }
        }
    }
}

#[test]
fn disabled_scope_retains_normal_selection_blending() {
    let p = plugin(
        r#"(local $i i32)
        (block $done (loop $loop
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const 0.8))
            (local.set $i (i32.add (local.get $i) (i32.const 4))) (br $loop))) (i32.const 0)"#,
    );
    let s = input(PixelFormat::RGBA32F, 0.6);
    let out = p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(0.25)), &json!({"effect":"texture"})).unwrap();
    assert!((out.pixel(15, 15)[0] - 0.425).abs() < 0.000001);
    assert!((out.pixel(15, 15)[3] - 0.65).abs() < 0.000001);
    assert_eq!(out.pixel(25, 15), s.pixel(25, 15));
}

const SHIFT: &str = r#"(local $i i32) (local $stride i32) (local $a i32) (local $step i32)
    (local.set $stride (i32.mul (local.get $ch) (i32.const 4)))
    (local.set $a (i32.sub (local.get $stride) (i32.const 4)))
    (local.set $step (i32.mul (local.get $stride) (i32.const 3)))
    (local.set $i (i32.sub (local.get $len) (local.get $stride)))
    (block $done (loop $loop
        (br_if $done (i32.lt_s (local.get $i) (local.get $step)))
        (f32.store (i32.add (i32.add (local.get $buf) (local.get $i)) (local.get $a))
            (f32.load (i32.sub (i32.add (i32.add (local.get $buf) (local.get $i)) (local.get $a)) (local.get $step))))
        (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const 1))
        (local.set $i (i32.sub (local.get $i) (local.get $stride))) (br $loop))) (i32.const 0)"#;

#[test]
fn outer_shape_effect_survives_selection_and_leaves_distant_photo_untouched() {
    let p = plugin(SHIFT);
    for sample in SampleType::ALL {
        for base in [PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, PixelFormat::RGBA8, PixelFormat::GRAY8, PixelFormat::GRAYA8] {
            let s = input(PixelFormat { sample, ..base }, 1.0);
            let out = p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(1.0)), &json!({})).unwrap();
            assert!(out.pixel(25, 16)[0] > 0.99, "outer effect clipped: {sample:?} {base:?}");
            assert_eq!(out.pixel(30, 16), s.pixel(30, 16));
            assert_eq!(out.pixel(4, 16), s.pixel(4, 16));
        }
    }
}

#[test]
fn feathered_shape_effect_composites_once_over_the_remaining_photo() {
    let p = plugin(SHIFT);
    let s = input(PixelFormat::RGBA32F, 0.6);
    let out = p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(0.5)), &json!({})).unwrap();
    let px = out.pixel(25, 16);
    // Outer pixel: filtered coverage 0.6*0.5 over original coverage 0.6.
    let alpha = 0.3 + 0.6 * 0.7;
    let red = (0.3 + 0.3 * 0.6 * 0.7) / alpha;
    assert!((px[3] - alpha).abs() < 0.000001, "{px:?}");
    assert!((px[0] - red).abs() < 0.000001, "{px:?}");
    let interior = out.pixel(17, 16);
    assert!((interior[3] - 0.6).abs() < 0.000001, "interior alpha was doubled: {interior:?}");
    assert!((interior[0] - 0.65).abs() < 0.000001, "feather applied twice: {interior:?}");
}

#[test]
fn shape_bands_are_seamless_across_vertical_and_horizontal_boundaries() {
    let wide = plugin(SHIFT);
    let narrow = plugin_with_limits(SHIFT, Limits { band_bytes: 1, ..Limits::default() });
    let rect = Rect::new(0, 0, 610, 560);
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.fill_rect(rect, &[0.3, 0.3, 0.3, 0.7]);
    let mut sel = Surface::new(PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 });
    sel.fill_rect(Rect::new(47, 21, 515, 526), &[0.75]);
    assert_eq!(wide.apply(&s, rect, Some(&sel), &json!({})).unwrap(), narrow.apply(&s, rect, Some(&sel), &json!({})).unwrap());
}

#[test]
fn nonfinite_shape_output_keeps_masked_input_before_compositing() {
    let p = plugin(
        r#"(local $i i32)
        (block $done (loop $loop
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const nan))
            (local.set $i (i32.add (local.get $i) (i32.const 4))) (br $loop))) (i32.const 0)"#,
    );
    let s = input(PixelFormat::RGBA32F, 0.6);
    assert_eq!(p.apply(&s, Rect::new(0, 0, 40, 32), Some(&selection(0.5)), &json!({})).unwrap(), s);
}
