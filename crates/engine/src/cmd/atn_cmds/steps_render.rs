//! Recorded descriptors for the Render filters.

use super::descriptors::desc;
use super::fields::{Mapped, step};
use super::{Descriptor, Value, json};

/// Clouds events have no fields; tool colors are read when the step plays.
pub(super) fn clouds_from(id: &str, d: &Descriptor) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Explicit seeds, colors and layer references have no fields in these ATN events.
pub(super) fn clouds_to(_: &str, p: &Value) -> Option<Descriptor> {
    let params = p.as_object()?;
    (super::events::pixel_target(p) && params.keys().all(|key| key == "target")).then(|| desc("null"))
}
