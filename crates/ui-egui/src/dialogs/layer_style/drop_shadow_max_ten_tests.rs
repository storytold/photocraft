//! #14891: the Layer Style dialog adds at most ten Drop Shadows to a layer, as in Photoshop. The
//! "+" is greyed at ten and adds nothing, and neither does the fx menu. A layer that already has
//! more keeps them all.
use super::*;

fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// The dialog's fields for the active layer after `n` drop shadows were added through the engine.
fn fields_with_shadows(s: &mut photocraft_engine::Session, n: usize) -> Map<String, Value> {
    for i in 0..n {
        s.execute("layer.layerStyle.dropShadow", json!({"distance": i + 1, "add": i > 0})).unwrap();
    }
    let st = s.active().unwrap();
    initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), None, st.doc.global_light.angle)
}

fn shadows(f: &Map<String, Value>) -> usize {
    ids_of(f, "dropShadow").len()
}

fn layer_shadows(s: &photocraft_engine::Session) -> usize {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().effects.items.iter().filter(|e| matches!(e, Effect::DropShadow(_))).count()
}

#[test]
fn plus_stops_at_ten_drop_shadows() {
    let mut s = session();
    let mut f = fields_with_shadows(&mut s, 1);
    // Eleven clicks on "+" from one Drop Shadow: ten rows, the last clicks add nothing.
    for _ in 0..11 {
        add_instance(&mut f, "dropShadow");
    }
    assert_eq!(shadows(&f), MAX_DROP_SHADOWS);
    assert!(!can_add(&f, "dropShadow"), "the + is greyed at ten");
    let selected = f["selected"].clone();
    add_instance(&mut f, "dropShadow");
    assert_eq!(shadows(&f), 10);
    assert_eq!(f["selected"], selected, "a click at the limit leaves the selection alone");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(layer_shadows(&s), 10, "OK keeps all ten");
}

#[test]
fn fx_menu_adds_no_eleventh_drop_shadow() {
    let mut s = session();
    let mut f = fields_with_shadows(&mut s, 10);
    pick_from_fx_menu(&mut f, "dropShadow");
    assert_eq!(shadows(&f), 10);
}

#[test]
fn removing_one_of_ten_lets_plus_add_again() {
    let mut s = session();
    let mut f = fields_with_shadows(&mut s, 10);
    assert!(!can_add(&f, "dropShadow"));
    let first = ids_of(&f, "dropShadow")[0].clone();
    remove_instance(&mut f, &first);
    assert!(can_add(&f, "dropShadow"), "the + is enabled again at nine");
    add_instance(&mut f, "dropShadow");
    assert_eq!(shadows(&f), 10);
    add_instance(&mut f, "dropShadow");
    assert_eq!(shadows(&f), 10);
}

/// A file with more than ten keeps them all through opening the dialog and OK; only adding stops.
#[test]
fn more_than_ten_drop_shadows_are_kept() {
    let mut s = session();
    let mut f = fields_with_shadows(&mut s, 12);
    assert_eq!(shadows(&f), 12, "all twelve load into the dialog");
    add_instance(&mut f, "dropShadow");
    pick_from_fx_menu(&mut f, "dropShadow");
    assert_eq!(shadows(&f), 12);
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(layer_shadows(&s), 12, "no instance is dropped on OK");
}

#[test]
fn other_kinds_are_not_limited() {
    let mut s = session();
    let mut f = fields_with_shadows(&mut s, 10);
    for kind in ["stroke", "innerShadow", "colorOverlay", "gradientOverlay"] {
        for _ in 0..12 {
            add_instance(&mut f, kind);
        }
        assert_eq!(ids_of(&f, kind).len(), 12, "{kind} is not limited by the Drop Shadow cap");
        assert!(can_add(&f, kind));
    }
}
