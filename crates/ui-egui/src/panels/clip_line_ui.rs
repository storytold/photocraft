//! Alt-click on the line between two layers in the Layers panel clips the upper one to the one
//! below, or releases it when it is clipped already (Photoshop, #967): the commands it runs.

use photocraft_doc::{Document, LayerId};
use serde_json::{Value, json};

/// A line between two layers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipLine {
    /// The layer above the line: the one clipped or released.
    pub upper: LayerId,
    /// It is clipped already, so the click releases it.
    pub release: bool,
}

impl ClipLine {
    /// The commands a click runs: Create Clipping Mask on the upper layer, or Release Clipping
    /// Mask on it and the clipped layers stacked above it (`key` makes that one history step)
    /// before it becomes the active layer.
    pub fn commands(&self, doc: &Document, key: &str) -> Vec<(String, Value)> {
        if !self.release {
            return vec![("layer.createClippingMask".into(), json!({"layer": self.upper.0}))];
        }
        let mut out: Vec<(String, Value)> =
            release_chain(doc, self.upper).into_iter().map(|id| ("layer.releaseClippingMask".into(), json!({"layer": id.0, "coalesce": key}))).collect();
        out.push(("layer.select".into(), json!({"layer": self.upper.0, "mode": "replace"})));
        out
    }
}

/// The layers a release on `upper` frees: it and the clipped layers stacked directly above it.
fn release_chain(doc: &Document, upper: LayerId) -> Vec<LayerId> {
    let mut out = vec![upper];
    let Some(mut path) = doc.path_of(upper) else { return out };
    while let Some(last) = path.pop() {
        let Some(next) = last.checked_add(1) else { break };
        path.push(next);
        match doc.layer_at(&path) {
            Some(l) if l.clipped => out.push(l.id),
            _ => break,
        }
    }
    out
}

#[cfg(test)]
mod tests;
