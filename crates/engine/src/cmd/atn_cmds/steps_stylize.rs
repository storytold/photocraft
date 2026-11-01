//! Descriptor adapters for the measured Stylize and Pixelate action events.

use super::descriptors::desc;
use super::fields::{Mapped, step};
use super::{Descriptor, Value, json};

pub(super) fn from(id: &str, d: &Descriptor) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

pub(super) fn to(_id: &str, p: &Value) -> Option<Descriptor> {
    let params = p.as_object()?;
    // These events have no fields for a layer reference or any native filter settings.
    params.keys().all(|key| key == "target").then(|| desc("null"))
}
