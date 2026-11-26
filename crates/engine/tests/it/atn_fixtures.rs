//! Descriptor builders the moved atn tests use: test-local copies of the one-line builders in the
//! engine-core atn module.

use craft_psd::descriptor::{Class, Id, ReferenceItem, UnicodeString, Value as D};

pub fn code(s: &str) -> Id {
    if s.len() < 4 { Id::new(&format!("{s:<4}")) } else { Id::new(s) }
}

pub fn class(c: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: code(c) }
}

pub fn px(v: f64) -> D {
    D::UnitFloat { unit: *b"#Pxl", value: v }
}

pub fn pct(v: f64) -> D {
    D::UnitFloat { unit: *b"#Prc", value: v }
}

pub fn long(v: f64) -> D {
    D::Integer(v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

pub fn text(s: &str) -> D {
    D::Text(UnicodeString::new_nul(s))
}

pub fn enm(ty: &str, v: &str) -> D {
    D::Enumerated { type_id: code(ty), value: code(v) }
}

pub fn target_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new("Ordn"), value: Id::new("Trgt") }])
}

pub fn name_ref(cls: &str, name: &str) -> D {
    D::Reference(vec![ReferenceItem::Name { class: class(cls), name: UnicodeString::new_nul(name) }])
}

pub fn class_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Class(class(cls))])
}

pub fn desc(cls: &str) -> craft_psd::descriptor::Descriptor {
    craft_psd::descriptor::Descriptor::new(cls)
}
