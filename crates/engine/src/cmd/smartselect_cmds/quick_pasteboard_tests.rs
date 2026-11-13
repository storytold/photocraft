//! #12931: a Quick Selection click on the pasteboard, outside the canvas, leaves no selection and
//! adds one "Quick Selection" History step, as in the reference application. A drag from the
//! pasteboard into an object still selects it, Subtract (not measured) and Select Subject keep
//! today's "found nothing, change nothing".

use serde_json::{Value, json};

use crate::Session;

/// 800 × 600, white Background, the active layer "Shapes" with a red rectangle 100,100 to 300,250.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s
}

fn bounds(s: &Session) -> Option<[i32; 4]> {
    s.active().unwrap().doc.selection.as_ref().map(|m| {
        let r = m.content_bounds();
        [r.x0, r.y0, r.x1, r.y1]
    })
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn quick(s: &mut Session, points: Value, mode: &str) -> Value {
    s.execute("select.quick", json!({"points": points, "size": 30, "mode": mode})).unwrap()
}

#[test]
fn a_pasteboard_click_clears_the_selection_as_one_step() {
    let mut s = session();
    let red = bounds(&s);
    assert_eq!(red, Some([100, 100, 300, 250]));
    let steps = history(&s).len();
    quick(&mut s, json!([[-50, -50]]), "add");
    assert_eq!(bounds(&s), None, "no selection afterwards");
    let h = history(&s);
    assert_eq!((h.len(), h.last().map(String::as_str)), (steps + 1, Some("Quick Selection")));
    assert!(s.undo());
    assert_eq!(bounds(&s), red, "Undo brings the selection back");
}

#[test]
fn a_pasteboard_click_without_a_selection_still_adds_the_step() {
    let mut s = session();
    s.execute("select.deselect", json!({})).unwrap();
    let steps = history(&s).len();
    quick(&mut s, json!([[-50, -50]]), "add");
    assert_eq!(bounds(&s), None);
    let h = history(&s);
    assert_eq!((h.len(), h.last().map(String::as_str)), (steps + 1, Some("Quick Selection")));
}

#[test]
fn a_drag_from_the_pasteboard_into_the_rectangle_selects_it() {
    let mut s = session();
    s.execute("select.deselect", json!({})).unwrap();
    quick(&mut s, json!([[-50, 150], [0, 150], [50, 150], [150, 150], [200, 175]]), "add");
    let b = bounds(&s).expect("the drag selects");
    assert!(b[0] <= 100 && b[1] <= 100 && b[2] >= 300 && b[3] >= 250, "{b:?}");
}

#[test]
fn subtract_and_select_subject_finding_nothing_still_change_nothing() {
    let mut s = session();
    let (red, steps) = (bounds(&s), history(&s).len());
    quick(&mut s, json!([[-50, -50]]), "subtract");
    assert_eq!((bounds(&s), history(&s).len()), (red, steps), "Subtract: not measured, as today");

    // A blank canvas: Select Subject finds nothing and leaves the selection and History alone.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let (sel, steps) = (bounds(&s), history(&s).len());
    let r = s.execute("select.subject", json!({})).unwrap();
    if r["changed"] == false {
        assert_eq!((bounds(&s), history(&s).len()), (sel, steps), "{r}");
    }
}
