//! Layer Style › Scale Effects rounding and ranges, measured against the reference (#14989).
//!
//! Scaled sizes and distances are stored rounded half up to whole pixels and held to their
//! fields' ranges (Size 0..250, Stroke Size 1..250, shadow Distance 0..30000); percents are not
//! scaled; an action may pass more than 1000 %.

use photocraft_doc::{Effect, Effects, LayerId};

use super::*;
use crate::image_cmds::{scale_effects, scale_effects_rounded};
use crate::layer_style::effect_from_params;

const CMD: &str = "layer.layerStyle.scaleEffects";

fn fx(kind: &str, p: Value) -> Effect {
    effect_from_params(kind, &p).unwrap()
}

fn one(e: Effect, pct: f64) -> Effect {
    let mut f = Effects { items: vec![e], ..Effects::default() };
    scale_effects_rounded(&mut f, pct);
    f.items.into_iter().next().unwrap()
}

fn shadow(e: &Effect) -> (f32, f32) {
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => (s.distance, s.size),
        e => panic!("not a shadow: {e:?}"),
    }
}

/// A document whose active layer carries `e`.
fn styled(kind: &str, p: Value) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shape"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute(&format!("layer.layerStyle.{kind}"), p).unwrap();
    (s, id)
}

fn first(s: &Session, id: LayerId) -> Effect {
    s.active().unwrap().doc.layer(id).unwrap().effects.items.first().cloned().unwrap()
}

#[test]
fn inner_shadow_3_7_at_50_becomes_2_4() {
    let e = one(fx("innerShadow", json!({"distance": 3, "size": 7})), 50.0);
    assert_eq!(shadow(&e), (2.0, 4.0));
}

#[test]
fn half_pixels_round_up_and_small_values_round_to_zero() {
    let e = one(fx("dropShadow", json!({"distance": 1, "size": 1})), 50.0);
    assert_eq!(shadow(&e), (1.0, 1.0));
    let e = one(fx("dropShadow", json!({"distance": 1, "size": 1})), 1.0);
    assert_eq!(shadow(&e), (0.0, 0.0));
    // 10 × 35 % is exactly 3.5 and rounds up.
    let e = one(fx("dropShadow", json!({"distance": 10, "size": 10})), 35.0);
    assert_eq!(shadow(&e), (4.0, 4.0));
}

#[test]
fn stroke_size_stays_within_1_to_250() {
    let stroke = |size: f32, pct: f64| match one(fx("stroke", json!({"size": size})), pct) {
        Effect::Stroke(s) => s.size,
        e => panic!("{e:?}"),
    };
    assert_eq!(stroke(100.0, 1.0), 1.0);
    assert_eq!(stroke(4.0, 5000.0), 200.0);
    assert_eq!(stroke(100.0, 1000.0), 250.0);
}

#[test]
fn sizes_cap_at_250_and_shadow_distance_at_30000() {
    let e = one(fx("dropShadow", json!({"distance": 9000, "size": 100})), 1000.0);
    assert_eq!(shadow(&e), (30000.0, 250.0));
    let e = one(fx("dropShadow", json!({"distance": 10, "size": 4})), 5000.0);
    assert_eq!(shadow(&e), (500.0, 200.0));
    match one(fx("satin", json!({"distance": 100, "size": 100})), 1000.0) {
        Effect::Satin(s) => assert_eq!((s.distance, s.size), (250.0, 250.0)),
        e => panic!("{e:?}"),
    }
    match one(fx("outerGlow", json!({"size": 7})), 50.0) {
        Effect::OuterGlow(g) => assert_eq!(g.size, 4.0),
        e => panic!("{e:?}"),
    }
}

#[test]
fn bevel_soften_rounds_without_the_size_cap() {
    match one(fx("bevelEmboss", json!({"size": 7, "soften": 16})), 1000.0) {
        Effect::BevelEmboss(b) => assert_eq!((b.size, b.soften), (70.0, 160.0)),
        e => panic!("{e:?}"),
    }
    match one(fx("bevelEmboss", json!({"size": 7, "soften": 3})), 50.0) {
        Effect::BevelEmboss(b) => assert_eq!((b.size, b.soften), (4.0, 2.0)),
        e => panic!("{e:?}"),
    }
}

#[test]
fn pattern_scale_keeps_its_1_to_1000_range() {
    // Stored as a fraction: 3 % at 3 % holds at the 1 % floor, 900 % at 1000 % at the 1000 % cap.
    let (min, max) = photocraft_doc::effects::PATTERN_SCALE_RANGE;
    for (from, pct, want) in [(3.0, 3.0, min), (900.0, 1000.0, max), (50.0, 50.0, 0.25)] {
        match one(fx("patternOverlay", json!({"scale": from})), pct) {
            Effect::PatternOverlay { scale, .. } => assert!((scale - want).abs() < 1e-6, "{from} at {pct}: {scale}"),
            e => panic!("{e:?}"),
        }
    }
}

#[test]
fn percents_are_not_scaled() {
    let e = one(fx("dropShadow", json!({"distance": 3, "size": 7, "spread": 40, "opacity": 60, "noise": 10})), 50.0);
    let before = fx("dropShadow", json!({"distance": 3, "size": 7, "spread": 40, "opacity": 60, "noise": 10}));
    match (e, before) {
        (Effect::DropShadow(a), Effect::DropShadow(b)) => {
            assert_eq!((a.spread, a.noise, a.common.opacity), (b.spread, b.noise, b.common.opacity));
        }
        _ => panic!("not shadows"),
    }
}

#[test]
fn image_size_scaling_stays_unrounded() {
    let mut f = Effects { items: vec![fx("innerShadow", json!({"distance": 3, "size": 7}))], ..Effects::default() };
    scale_effects(&mut f, 0.5);
    assert_eq!(shadow(&f.items[0]), (1.5, 3.5));
}

#[test]
fn the_command_rounds_and_accepts_more_than_1000() {
    let (mut s, id) = styled("innerShadow", json!({"distance": 3, "size": 7}));
    s.execute(CMD, json!({"scale": 50})).unwrap();
    assert_eq!(shadow(&first(&s, id)), (2.0, 4.0));

    let (mut s, id) = styled("dropShadow", json!({"distance": 10, "size": 4}));
    s.execute(CMD, json!({"scale": 5000, "layer": id})).unwrap();
    assert_eq!(shadow(&first(&s, id)), (500.0, 200.0));
}

#[test]
fn a_huge_scale_never_panics_and_stays_finite() {
    for pct in [1e300, f64::MAX, 1e-300, 0.0, -5.0] {
        let (mut s, id) = styled("dropShadow", json!({"distance": 0, "size": 4}));
        s.execute(CMD, json!({"scale": pct})).unwrap();
        let (d, z) = shadow(&first(&s, id));
        assert!(d.is_finite() && z.is_finite(), "{pct}: {d} {z}");
        assert!((0.0..=30000.0).contains(&d) && (0.0..=250.0).contains(&z), "{pct}: {d} {z}");
    }
    let mut f = Effects { items: vec![fx("dropShadow", json!({"distance": 0, "size": 4}))], ..Effects::default() };
    scale_effects_rounded(&mut f, f64::INFINITY);
    assert_eq!(shadow(&f.items[0]), (0.0, 250.0));
}

#[test]
fn a_hundred_percent_changes_nothing_and_adds_no_history_step() {
    let (mut s, id) = styled("dropShadow", json!({"distance": 10, "size": 10}));
    s.execute("layer.layerStyle.stroke", json!({"size": 4})).unwrap();
    // Values the dialog would round or hold to a range, as Image Size can leave them.
    s.edit("Odd style", |doc, _| {
        let l = doc.layer_mut(id).unwrap();
        for e in &mut l.effects.items {
            match e {
                Effect::DropShadow(d) => (d.size, d.distance) = (3.5, 300.0),
                Effect::Stroke(st) => st.size = 0.5,
                _ => {}
            }
        }
        Ok(())
    })
    .unwrap();
    let items = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().effects.items.clone();
    let before = items(&s);
    assert_eq!(before.len(), 2, "a shadow and a stroke");
    let steps = s.active().unwrap().history.past_len();
    s.execute(CMD, json!({"scale": 100})).unwrap();
    assert_eq!(items(&s), before, "the style is untouched at 100 %, ranges included");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no History step");

    let mut f = Effects { items: before.clone(), ..Effects::default() };
    scale_effects_rounded(&mut f, 100.0);
    assert_eq!(f.items, before, "the helper keeps 3.5, 300 and a 0.5 stroke at 100 %");
}

#[test]
fn a_missing_layer_is_an_error_at_every_percent() {
    let (mut s, _) = styled("dropShadow", json!({"distance": 3, "size": 7}));
    for pct in [99, 100, 101] {
        assert!(s.execute(CMD, json!({"scale": pct, "layer": 99999})).is_err(), "{pct} %");
    }
}

#[test]
fn psd_raw_survives_a_hundred_percent_and_goes_at_fifty() {
    let (mut s, id) = styled("dropShadow", json!({"distance": 3, "size": 7}));
    s.edit("Imported", |doc, _| {
        doc.layer_mut(id).unwrap().effects.psd_raw = Some(std::sync::Arc::new(vec![1, 2, 3]));
        Ok(())
    })
    .unwrap();
    let raw = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().effects.psd_raw.is_some();
    s.execute(CMD, json!({"scale": 100})).unwrap();
    assert!(raw(&s), "kept at 100 %");
    s.execute(CMD, json!({"scale": 50})).unwrap();
    assert!(!raw(&s), "dropped once the style changes");
}
