//! Optional ABI-v1 overlap negotiation must govern both input context and output bounds.
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_plugins::{Error, Limits, Manifest, Plugin};
use photocraft_raster::Surface;
use serde_json::{Value, json};

fn module(manifest: &Value, filter: &str, extra: &str) -> Vec<u8> {
    let manifest = manifest.to_string();
    let escaped = manifest.replace('\\', "\\\\").replace('"', "\\\"");
    wat::parse_str(format!(
        r#"(module
        (memory (export "memory") 1)
        (data (i32.const 16) "{escaped}")
        {extra}
        (func (export "pc_abi_version") (result i32) (i32.const 1))
        (func (export "pc_manifest") (result i64) (i64.const {}))
        (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
            (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
            (if (result i32) (i32.eq (local.get $old) (i32.const -1))
                (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
        (func (export "pc_filter") (param $buf i32) (param $len i32) (param $w i32) (param $h i32)
            (param $ch i32) (param $fmt i32) (param $pp i32) (param $pl i32) (result i32)
            {filter}))"#,
        ((manifest.len() as u64) << 32) | 16
    ))
    .unwrap()
}

fn manifest() -> Value {
    json!({"id":"test.dynamic-overlap","name":"Dynamic overlap","kind":"filter","area":"canvas","overlap":2})
}

fn overlap(value: i32) -> String {
    format!("(func (export \"pc_overlap\") (param i32 i32) (result i32) (i32.const {value}))")
}

fn json_contains(fragment: &str) -> String {
    let escaped = fragment.replace('\\', "\\\\").replace('"', "\\\"");
    let len = fragment.len();
    format!(
        r#"(data (i32.const 4096) "{escaped}")
        (func $contains (param $pp i32) (param $pl i32) (result i32) (local $i i32) (local $j i32)
            (block $done (loop $next
                (br_if $done (i32.gt_u (i32.add (local.get $i) (i32.const {len})) (local.get $pl)))
                (local.set $j (i32.const 0))
                (block $different (loop $byte
                    (br_if $different (i32.ne
                        (i32.load8_u (i32.add (local.get $pp) (i32.add (local.get $i) (local.get $j))))
                        (i32.load8_u (i32.add (i32.const 4096) (local.get $j)))))
                    (local.set $j (i32.add (local.get $j) (i32.const 1)))
                    (if (i32.eq (local.get $j) (i32.const {len})) (then (return (i32.const 1))))
                    (br $byte)))
                (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $next)))
            (i32.const 0))"#
    )
}

fn input() -> Surface {
    let mut surface = Surface::new(PixelFormat::RGBA32F);
    surface.fill_rect(Rect::new(0, 0, 64, 64), &[0.25, 0.5, 0.75, 1.0]);
    surface
}

const WHITE: &str = r#"(local $i i32)
    (block $done (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 4))) (br $loop))) (i32.const 0)"#;

#[test]
fn old_plugins_keep_static_overlap_and_optional_export_can_reduce_it() {
    for (extra, expected) in [(String::new(), 68), (overlap(0), 64), (overlap(13), 90)] {
        let filter = format!(
            "(if (result i32) (i32.and (i32.eq (local.get $w) (i32.const {expected})) \
             (i32.eq (local.get $h) (i32.const {expected}))) (then (i32.const 0)) (else (i32.const 7)))"
        );
        let plugin = Plugin::load(&module(&manifest(), &filter, &extra), Limits::default()).unwrap();
        assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})).unwrap(), input());
    }
}

#[test]
fn negotiation_receives_resolved_defaults_and_filter_receives_exact_context() {
    let mut manifest = manifest();
    manifest["params"] = json!({"reach":{"type":"int","min":0,"max":99,"default":13}});
    // With one declared parameter the resolved JSON is exactly {"reach":13}; no _image yet.
    let extra = r#"(func (export "pc_overlap") (param $pp i32) (param $pl i32) (result i32)
        (if (i32.ne (local.get $pl) (i32.const 12)) (then (return (i32.const -1))))
        (i32.add
            (i32.mul (i32.sub (i32.load8_u offset=9 (local.get $pp)) (i32.const 48)) (i32.const 10))
            (i32.sub (i32.load8_u offset=10 (local.get $pp)) (i32.const 48))))
        (data (i32.const 4096) "\22overlap\22:13,")
        (func $contains (param $pp i32) (param $pl i32) (result i32) (local $i i32) (local $j i32)
            (block $done (loop $next
                (br_if $done (i32.gt_u (i32.add (local.get $i) (i32.const 13)) (local.get $pl)))
                (local.set $j (i32.const 0))
                (block $different (loop $byte
                    (br_if $different (i32.ne
                        (i32.load8_u (i32.add (local.get $pp) (i32.add (local.get $i) (local.get $j))))
                        (i32.load8_u (i32.add (i32.const 4096) (local.get $j)))))
                    (local.set $j (i32.add (local.get $j) (i32.const 1)))
                    (if (i32.eq (local.get $j) (i32.const 13)) (then (return (i32.const 1))))
                    (br $byte)))
                (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $next)))
            (i32.const 0))"#;
    let filter = r#"(if (result i32)
        (i32.and (call $contains (local.get $pp) (local.get $pl))
            (i32.and (i32.eq (local.get $w) (i32.const 90)) (i32.eq (local.get $h) (i32.const 90))))
        (then (i32.const 0)) (else (i32.const 8)))"#;
    let plugin = Plugin::load(&module(&manifest, filter, extra), Limits::default()).unwrap();
    assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})).unwrap(), input());
}

#[test]
fn effective_overlap_expands_selection_shapes_and_content() {
    let mut manifest = manifest();
    manifest["params"] = json!({"effect":{"type":"choice","options":["shape"],"default":"shape"}});
    manifest["selectionShape"] = json!({"parameter":"effect","values":["shape"]});
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.fill_rect(Rect::new(28, 28, 32, 32), &[1.0]);
    let plugin = Plugin::load(&module(&manifest, WHITE, &overlap(8)), Limits::default()).unwrap();
    let out = plugin.apply(&input(), Rect::new(0, 0, 64, 64), Some(&selection), &json!({})).unwrap();
    assert_eq!(out.pixel(20, 20), [1.0; 4]);
    assert_eq!(out.pixel(39, 39), [1.0; 4]);
    assert_eq!(out.pixel(19, 20), input().pixel(19, 20));
    assert_eq!(out.pixel(40, 39), input().pixel(40, 39));

    manifest["area"] = json!("content");
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.fill_rect(Rect::new(28, 28, 32, 32), &[0.3, 0.5, 0.7, 1.0]);
    let plugin = Plugin::load(&module(&manifest, WHITE, &overlap(8)), Limits::default()).unwrap();
    let out = plugin.apply(&source, Rect::new(0, 0, 64, 64), None, &json!({})).unwrap();
    assert_eq!(out.content_bounds(), source.content_bounds().inflate(8));
    assert_eq!(out.pixel(20, 20), [1.0; 4]);
}

#[test]
fn malformed_overlap_export_and_out_of_range_requests_fail_explicitly() {
    for extra in ["(func (export \"pc_overlap\") (result i32) (i32.const 0))", "(global (export \"pc_overlap\") i32 (i32.const 0))"] {
        assert!(matches!(Plugin::load(&module(&manifest(), "(i32.const 0)", extra), Limits::default()), Err(Error::Abi(_))));
    }
    for requested in [-1, i32::MAX, photocraft_plugins::manifest::MAX_OVERLAP as i32 + 1] {
        let plugin = Plugin::load(&module(&manifest(), "(i32.const 0)", &overlap(requested)), Limits::default()).unwrap();
        let Err(error) = plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})) else {
            panic!("out-of-range overlap {requested} must fail");
        };
        assert!(matches!(error, Error::Limit(_)), "{requested}: {error}");
        assert!(error.to_string().contains("overlap"));
    }
}

#[test]
fn overlap_negotiation_is_fuel_bounded_and_memory_failures_are_transactional() {
    let extra = "(func (export \"pc_overlap\") (param i32 i32) (result i32) (loop $forever (br $forever)) (i32.const 0))";
    let plugin = Plugin::load(&module(&manifest(), "(i32.const 0)", extra), Limits { fuel_base: 50_000, ..Limits::default() }).unwrap();
    let Err(error) = plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})) else {
        panic!("unbounded overlap negotiation must fail");
    };
    assert_eq!(error, Error::Limit("instruction budget".into()));
    let source = input();
    let before = source.clone();
    let plugin = Plugin::load(&module(&manifest(), WHITE, &overlap(256)), Limits { max_memory_bytes: 1 << 20, ..Limits::default() }).unwrap();
    assert!(matches!(plugin.apply(&source, Rect::new(0, 0, 64, 64), None, &json!({})), Err(Error::Limit(_))));
    assert_eq!(source, before);
}

#[test]
fn platform_overlap_limit_is_explicit_and_static_manifests_obey_it() {
    let expected = if cfg!(target_arch = "wasm32") { 256 } else { 1536 };
    assert_eq!(photocraft_plugins::manifest::MAX_OVERLAP, expected);
    let mut manifest = manifest();
    manifest["overlap"] = json!(expected);
    assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_ok());
    manifest["overlap"] = json!(expected + 1);
    assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_err());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn desktop_accepts_1500_pixel_requests_under_default_memory_limits() {
    // Small output still requires the complete 1500-pixel source context on each side.
    let filter = "(if (result i32) (i32.eq (local.get $w) (i32.const 3001)) (then (i32.const 0)) (else (i32.const 7)))";
    let plugin = Plugin::load(&module(&manifest(), filter, &overlap(1500)), Limits::default()).unwrap();
    let mut source = Surface::new(PixelFormat::GRAY8);
    source.fill_rect(Rect::new(0, 0, 1, 1), &[0.5]);
    assert_eq!(plugin.apply(&source, Rect::new(0, 0, 1, 1), None, &json!({})).unwrap(), source);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn clipping_is_opt_in_and_reports_actual_output_offsets() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    for (overlap_pixels, dimensions, fragment) in
        [(300, 136, "\"outputX\":36"), (300, 136, "\"outputY\":36"), (300, 136, "\"outputWidth\":64"), (300, 136, "\"outputHeight\":64")]
    {
        let extra = format!("{} {}", overlap(overlap_pixels), json_contains(fragment));
        let filter = format!(
            "(if (result i32) (i32.and (call $contains (local.get $pp) (local.get $pl)) \
                (i32.and (i32.eq (local.get $w) (i32.const {dimensions})) (i32.eq (local.get $h) (i32.const {dimensions})))) \
                (then (i32.const 0)) (else (i32.const 9)))"
        );
        let plugin = Plugin::load(&module(&manifest, &filter, &extra), Limits::default()).unwrap();
        assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})).unwrap(), input());
    }
    // Write the first actual output sample. A symmetric crop at overlap=300 would miss it.
    let filter = r#"(f32.store (i32.add (local.get $buf) (i32.mul (i32.add (i32.mul (i32.const 36) (local.get $w)) (i32.const 36)) (i32.const 16))) (f32.const 0.875)) (i32.const 0)"#;
    let plugin = Plugin::load(&module(&manifest, filter, &overlap(300)), Limits::default()).unwrap();
    let out = plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})).unwrap();
    assert_eq!(out.pixel(0, 0)[0], 0.875);
    assert_eq!(out.pixel(1, 0), input().pixel(1, 0));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn clipped_selection_context_keeps_zero_coverage_outside_the_canvas() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    manifest["params"] = json!({"effect":{"type":"choice","options":["shape"],"default":"shape"}});
    manifest["selectionShape"] = json!({"parameter":"effect","values":["shape"]});
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.fill_rect(Rect::new(0, 0, 64, 64), &[1.0]);
    // The boundary guard repeats original edge colour but must not repeat selection alpha.
    let filter = r#"(if (result i32) (f32.eq (f32.load offset=12 (local.get $buf)) (f32.const 0))
        (then (i32.const 0)) (else (i32.const 9)))"#;
    let plugin = Plugin::load(&module(&manifest, filter, &overlap(300)), Limits::default()).unwrap();
    assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), Some(&selection), &json!({})).unwrap(), input());
}

#[test]
fn ordinary_overlap_keeps_symmetric_buffers_even_after_clipping_opt_in() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    let extra = format!("{} {}", overlap(64), json_contains("\"outputX\""));
    let filter = r#"(if (result i32) (i32.and
        (i32.eqz (call $contains (local.get $pp) (local.get $pl)))
        (i32.and (i32.eq (local.get $w) (i32.const 192)) (i32.eq (local.get $h) (i32.const 192))))
        (then (i32.const 0)) (else (i32.const 9)))"#;
    let plugin = Plugin::load(&module(&manifest, filter, &extra), Limits::default()).unwrap();
    assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})).unwrap(), input());
}

#[test]
fn platform_memory_defaults_and_explicit_lower_limits_are_preserved() {
    assert_eq!(Limits::default().max_memory_bytes, if cfg!(target_arch = "wasm32") { 512 << 20 } else { 1024 << 20 });
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    let limits = Limits { max_memory_bytes: 8 << 20, ..Limits::default() };
    let plugin = Plugin::load(&module(&manifest, "(i32.const 0)", &overlap(300)), limits).unwrap();
    assert_eq!(plugin.limits().max_memory_bytes, 8 << 20);
    assert!(matches!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &json!({})), Err(Error::Limit(_))));
}

#[test]
fn clipping_conditions_reject_malformed_or_conditional_choice_scopes() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    manifest["params"] = json!({"effect":{"type":"choice","options":["clipped","full"],"default":"clipped"}});
    for condition in [
        json!({"parameter":"missing","values":["clipped"]}),
        json!({"parameter":"effect","values":[]}),
        json!({"parameter":"effect","values":["unknown"]}),
        json!({"parameter":"effect","values":["clipped","clipped"]}),
        json!({"parameter":"effect","values":"clipped"}),
    ] {
        manifest["clampContextWhen"] = condition;
        assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_err(), "{manifest}");
    }
    manifest["clampContextWhen"] = json!({"parameter":"effect","values":["clipped"]});
    manifest["params"]["gate"] = json!({"type":"bool","default":true});
    manifest["params"]["effect"]["visibleWhen"] = json!({"gate":true});
    assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_err());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn excluded_choices_keep_symmetric_context_while_included_defaults_clip() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    manifest["params"] = json!({"effect":{"type":"choice","options":["clipped","full"],"default":"clipped"}});
    manifest["clampContextWhen"] = json!({"parameter":"effect","values":["clipped"]});
    for (enabled, params, size, has_output) in [(true, json!({}), 136, 1), (true, json!({"effect":"full"}), 664, 0), (false, json!({}), 664, 0)] {
        manifest["clampContext"] = json!(enabled);
        let extra = format!("{} {}", overlap(300), json_contains("\"outputX\""));
        let filter = format!(
            "(if (result i32) (i32.and (i32.eq (call $contains (local.get $pp) (local.get $pl)) (i32.const {has_output})) \
                (i32.and (i32.eq (local.get $w) (i32.const {size})) (i32.eq (local.get $h) (i32.const {size})))) \
                (then (i32.const 0)) (else (i32.const 9)))"
        );
        let plugin = Plugin::load(&module(&manifest, &filter, &extra), Limits::default()).unwrap();
        assert_eq!(plugin.apply(&input(), Rect::new(0, 0, 64, 64), None, &params).unwrap(), input());
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn clipped_selection_crops_actual_channel_strides_without_changing_identity() {
    let mut manifest = manifest();
    manifest["clampContext"] = json!(true);
    manifest["params"] = json!({"effect":{"type":"choice","options":["shape"],"default":"shape"}});
    manifest["selectionShape"] = json!({"parameter":"effect","values":["shape"]});
    let plugin = Plugin::load(&module(&manifest, "(i32.const 0)", &overlap(300)), Limits::default()).unwrap();
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.fill_rect(Rect::new(0, 0, 64, 64), &[0.6]);
    for fmt in [PixelFormat::RGBA32F, PixelFormat { alpha: false, ..PixelFormat::RGBA32F }, PixelFormat::GRAY8, PixelFormat::GRAYA8] {
        let mut source = Surface::new(fmt);
        for y in 0..64 {
            for x in 0..64 {
                let value = (x + y * 3) as f32 / 256.0;
                source.write_pixel(x, y, &vec![value; fmt.channels()]);
            }
        }
        assert_eq!(plugin.apply(&source, Rect::new(0, 0, 64, 64), Some(&selection), &json!({})).unwrap(), source);
    }
}
