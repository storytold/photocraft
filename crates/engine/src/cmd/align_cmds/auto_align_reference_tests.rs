//! #14199: with the Background selected, Auto-Align keeps the Background as the reference even when another selected
//! layer is position-locked; that layer is aligned onto the Background. Without a selected Background a locked layer
//! stays the reference (#7080), and an explicit `reference` still wins.

use super::*;

/// Deterministic texture written into layer `id` at offset (dx, dy).
fn paint_texture(s: &mut Session, id: LayerId, dx: i32, dy: i32) {
    s.edit("tex", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let mut seed = 99u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        surf.fill_rect(Rect::new(dx, dy, dx + 240, dy + 180), &[0.4, 0.4, 0.4, 1.0]);
        for _ in 0..120 {
            let (x, y) = ((rnd() * 220.0) as i32, (rnd() * 160.0) as i32);
            let (sw, sh) = (4 + (rnd() * 14.0) as i32, 4 + (rnd() * 10.0) as i32);
            let v = rnd() as f32;
            surf.fill_rect(Rect::new(dx + x, dy + y, dx + x + sw, dy + y + sh), &[v, 1.0 - v, v * 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
}

fn texture_layer(s: &mut Session, name: &str, dx: i32, dy: i32) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    paint_texture(s, id, dx, dy);
    id
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
}

fn lock(s: &mut Session, id: LayerId, which: &str) {
    s.edit("lock", |doc, _| {
        doc.layer_mut(id).unwrap().locks.set(which, true);
        Ok(())
    })
    .unwrap();
}

fn origin(s: &Session, id: LayerId) -> (i32, i32) {
    let b = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
    (b.x0, b.y0)
}

fn near(a: (i32, i32), b: (i32, i32)) -> bool {
    (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1
}

/// The issue's document, scaled: a textured Background at 20,20, "Shift" the same texture moved by +24,+16 and "Rot"
/// moved by -10,+8 with its position locked; all three selected.
fn setup() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 240, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    paint_texture(&mut s, bg, 20, 20);
    let shift = texture_layer(&mut s, "Shift", 44, 36);
    let rot = texture_layer(&mut s, "Rot", 10, 28);
    lock(&mut s, rot, "position");
    select(&mut s, &[bg, shift, rot]);
    (s, bg, shift, rot)
}

#[test]
fn a_selected_background_stays_the_reference_over_a_position_locked_layer() {
    let (mut s, bg, shift, rot) = setup();
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(r["reference"], bg.0, "{r}");
    assert!(s.active().unwrap().doc.is_background(bg), "the Background keeps its place and stays the Background");
    assert!(near(origin(&s, shift), (20, 20)), "Shift moved onto the Background: {:?} {r}", origin(&s, shift));
    assert!(near(origin(&s, rot), (20, 20)), "the position-locked layer moved onto it too: {:?} {r}", origin(&s, rot));
}

#[test]
fn without_a_selected_background_the_locked_layer_stays_the_reference() {
    let (mut s, _, shift, rot) = setup();
    select(&mut s, &[shift, rot]);
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(r["reference"], rot.0, "{r}");
    assert_eq!(origin(&s, rot), (10, 28), "the locked layer stays put");
    assert!(near(origin(&s, shift), (10, 28)), "{:?} {r}", origin(&s, shift));
}

#[test]
fn an_explicit_reference_still_wins() {
    let (mut s, _, shift, rot) = setup();
    // Shift as the reference: the position-locked layer would have to move, which is refused as before.
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": shift.0})).is_err());
    assert_eq!(origin(&s, rot), (10, 28));
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": rot.0})).unwrap();
    assert_eq!(r["reference"], rot.0, "{r}");
}

/// An explicit `reference` is the caller's choice: naming the Background does not let a position-locked layer move.
#[test]
fn an_explicit_background_reference_still_refuses_to_move_a_position_locked_layer() {
    let (mut s, bg, _, rot) = setup();
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": bg.0})).is_err());
    assert_eq!(origin(&s, rot), (10, 28), "the locked layer stays put");
}

/// A Background locked all the way is picked as the reference by the locked-layer rule, not by the Background rule: the
/// position-locked layer still must not move onto it (CodeRabbit on #16073).
#[test]
fn a_background_locked_all_the_way_does_not_let_a_position_locked_layer_move() {
    let (mut s, bg, _, rot) = setup();
    lock(&mut s, bg, "all");
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err());
    assert_eq!(origin(&s, rot), (10, 28), "the locked layer stays put");
}

/// A layer locked all the way (not measured beside a selected Background) keeps today's rule: it is the reference.
#[test]
fn a_fully_locked_layer_beside_the_background_keeps_todays_rule() {
    let (mut s, _, _, rot) = setup();
    lock(&mut s, rot, "all");
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(r["reference"], rot.0, "{r}");
    assert_eq!(origin(&s, rot), (10, 28));
}
