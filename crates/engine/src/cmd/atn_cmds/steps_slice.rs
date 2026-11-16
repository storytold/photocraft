//! Recorded slice creation, single-slice selection and count-based division.

use super::descriptors::{PX, class_ref, desc, get_ref, px, target_ref};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Id, ReferenceItem, Value, json};

const EDGES: [&str; 4] = ["Top ", "Left", "Btom", "Rght"];

/// Require every known key exactly once so unknown or duplicate settings cannot be discarded.
fn exact_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.len() == keys.len() && keys.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() == 1)
}

fn whole(v: f64) -> Option<i32> {
    (v.is_finite() && v.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v)).then_some(v as i32)
}

fn rect_from(d: &Descriptor) -> Option<[i32; 4]> {
    if !d.class_id.is("Rctn") || !exact_keys(d, &EDGES) {
        return None;
    }
    let edge = |key: &str| match d.get(key)? {
        D::UnitFloat { unit: PX, value } => whole(*value),
        _ => None,
    };
    let (top, left, bottom, right) = (edge("Top ")?, edge("Left")?, edge("Btom")?, edge("Rght")?);
    let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
    (width > 0 && height > 0).then_some([left, top, width, height])
}

fn recorded(d: &Descriptor) -> Option<[i32; 4]> {
    if !exact_keys(d, &["null", "Usng"])
        || !matches!(d.get("null"), Some(D::Reference(r)) if matches!(r.as_slice(), [ReferenceItem::Class(c)] if c.class_id.is("slice")))
    {
        return None;
    }
    let D::Descriptor(u) = d.get("Usng")? else { return None };
    // `user` is a four-byte string ID; the same bytes as a char ID name another identifier.
    if !u.class_id.is("slice")
        || !exact_keys(u, &["Type", "At  "])
        || !matches!(u.get("Type"), Some(D::Enumerated { type_id, value: Id::Str(value) }) if type_id.is("sliceType") && value.as_slice() == b"user")
    {
        return None;
    }
    let D::Descriptor(r) = u.get("At  ")? else { return None };
    rect_from(r)
}

pub(super) fn make_from(d: &Descriptor) -> Mapped {
    match recorded(d) {
        Some(rect) => step("slice.new", json!({"rect": rect})),
        None => Mapped::Unsupported { harmless: false },
    }
}

/// Only fixed, integral pixel rectangles: the measured descriptor has no Slice Options fields.
pub(super) fn make_to(p: &Value) -> Option<Descriptor> {
    const POSITION: [&str; 4] = ["x", "y", "width", "height"];
    if p.as_object()?.iter().any(|(k, v)| !v.is_null() && k != "rect" && !POSITION.contains(&k.as_str())) {
        return None;
    }
    let value = |v: &Value| whole(v.as_f64()?);
    let rect: [i32; 4] = match p.get("rect") {
        Some(Value::Array(r)) if r.len() == 4 && POSITION.iter().all(|key| p.get(*key).is_none_or(Value::is_null)) => {
            r.iter().map(value).collect::<Option<Vec<_>>>()?.try_into().ok()?
        }
        None | Some(Value::Null) => POSITION.map(|key| p.get(key).and_then(value)).into_iter().collect::<Option<Vec<_>>>()?.try_into().ok()?,
        _ => return None,
    };
    let [x, y, width, height] = rect;
    if width <= 0 || height <= 0 {
        return None;
    }
    let edges = [y, x, y.checked_add(height)?, x.checked_add(width)?];
    let r = EDGES.into_iter().zip(edges).fold(desc("Rctn"), |r, (key, v)| r.with(key, px(f64::from(v))));
    let user = D::Enumerated { type_id: Id::new("sliceType"), value: Id::Str(b"user".to_vec()) };
    Some(desc("null").with("null", class_ref("slice")).with("Usng", D::Descriptor(desc("slice").with("Type", user).with("At  ", D::Descriptor(r)))))
}

/// Only single-slice selection by a pixel point is represented; additive selection stays kept.
pub(super) fn select_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let keys: &[&str] = if d.get("addToSelection").is_some() { &["null", "At  ", "addToSelection"] } else { &["null", "At  "] };
    if !exact_keys(d, keys)
        || !matches!(d.get("addToSelection"), None | Some(D::Boolean(false)))
        || !matches!(get_ref(d, "null"), Some([ReferenceItem::Class(class)]) if class.class_id.is("slice"))
    {
        return unsupported;
    }
    let Some(D::Descriptor(point)) = d.get("At  ") else { return unsupported };
    if !point.class_id.is("Pnt ") || !exact_keys(point, &["Hrzn", "Vrtc"]) {
        return unsupported;
    }
    let coordinate = |key| match point.get(key) {
        Some(D::UnitFloat { unit, value }) if *unit == PX && value.is_finite() => Some(*value),
        _ => None,
    };
    let (Some(x), Some(y)) = (coordinate("Hrzn"), coordinate("Vrtc")) else { return unsupported };
    step("slice.select", json!({"point": [x, y]}))
}

pub(super) fn select_to(p: &Value) -> Option<Descriptor> {
    if p.as_object()?.len() != 1 {
        return None;
    }
    let point = p.get("point")?.as_array().filter(|p| p.len() == 2)?;
    let x = point.first()?.as_f64().filter(|n| n.is_finite())?;
    let y = point.get(1)?.as_f64().filter(|n| n.is_finite())?;
    Some(
        desc("null")
            .with("null", class_ref("slice"))
            .with("At  ", D::Descriptor(desc("Pnt ").with("Hrzn", px(x)).with("Vrtc", px(y))))
            .with("addToSelection", D::Boolean(false)),
    )
}

fn division_counts_fit(down: i32, across: i32) -> bool {
    // Match the native division bounds instead of silently clamping a recorded count.
    (1..=1000).contains(&down) && (1..=1000).contains(&across) && (down > 1 || across > 1) && down * across <= crate::slice_cmds::MAX_DIVIDE_PARTS
}

pub(super) fn divide_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let keys: Vec<_> = ["null", "slicesDown", "slicesAcross"].into_iter().filter(|key| d.get(key).is_some()).collect();
    if !exact_keys(d, &keys)
        || !matches!(get_ref(d, "null"), Some([ReferenceItem::Enumerated { class, type_id, value }])
            if class.class_id.is("slice") && type_id.is("Ordn") && value.is("Trgt"))
    {
        return unsupported;
    }
    let count = |key| match d.get(key) {
        None => Some(1),
        Some(D::Integer(n)) => Some(*n),
        _ => None,
    };
    let (Some(down), Some(across)) = (count("slicesDown"), count("slicesAcross")) else { return unsupported };
    if !division_counts_fit(down, across) {
        return unsupported;
    }
    let mut params = json!({});
    for (source, target, n) in [("slicesDown", "horizontal", down), ("slicesAcross", "vertical", across)] {
        if d.get(source).is_some() {
            params[target] = json!(n);
        }
    }
    step("slice.divide", params)
}

pub(super) fn divide_to(p: &Value) -> Option<Descriptor> {
    // Stored IDs/display numbers have no portable equivalent to Photoshop's current target.
    if p.as_object()?.keys().any(|key| !matches!(key.as_str(), "horizontal" | "vertical")) {
        return None;
    }
    let count = |key| p.get(key).map_or(Some(1), |value| i32::try_from(value.as_i64()?).ok());
    let (down, across) = (count("horizontal")?, count("vertical")?);
    if !division_counts_fit(down, across) {
        return None;
    }
    let mut d = desc("null").with("null", target_ref("slice"));
    for (source, target, n) in [("horizontal", "slicesDown", down), ("vertical", "slicesAcross", across)] {
        if p.get(source).is_some() {
            d = d.with(target, D::Integer(n));
        }
    }
    Some(d)
}
