//! Revision markers: red boxes drawn over a document (typically a manual / 说明书) that mark
//! content which still needs to be changed. A marker saves a small snapshot of the framed pixels;
//! when the document is reopened (or a detection command runs) the framed area is re-rendered and
//! compared with that snapshot. Unchanged markers stay red; markers whose content changed turn
//! green, so nothing that was supposed to be revised is missed.
//!
//! A marker can be bound to a layer (`layer_id`): moving that layer with the Move tool shifts the
//! marker box along with the content, so the box keeps framing the same piece of content.
//!
//! Markers are document data like notes or guides: they round-trip through `.pcraft` and are
//! stored in PSD as a private image resource (see `photocraft-io`'s `revision_map`).

use serde::{Deserialize, Serialize};

use crate::LayerId;

/// One revision marker: a box over content that must be changed, with the snapshot its detection
/// compares against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RevisionMarker {
    /// Marker name, shown in the panel and on the canvas.
    pub name: String,
    /// Box top-left corner in document pixels (f64 like the rest of the view geometry, so a box
    /// placed in ruler units keeps its exact place).
    pub pos: [f64; 2],
    /// Box size in document pixels (width, height).
    pub size: [f64; 2],
    /// Layer the box is bound to. When that layer is moved whole (Move tool / `layer.translate`)
    /// the box moves with it. `None` keeps the box fixed in document coordinates.
    pub layer_id: Option<LayerId>,
    /// Snapshot of the framed pixels at save time: RGBA8, downsampled to the fixed grid below,
    /// row-major. Detection re-renders the framed area to the same grid and compares.
    pub snapshot: Vec<u8>,
    /// Snapshot grid size (width, height); `snapshot` holds `w * h * 4` bytes.
    pub snapshot_size: [u32; 2],
    /// Detection result: `false` = content unchanged (red box), `true` = content changed since the
    /// snapshot (green box).
    pub done: bool,
    /// Free-form note the author leaves on the marker.
    pub note: String,
}

impl RevisionMarker {
    /// The grid every marker snapshot is sampled to. Small enough to keep files light, large
    /// enough that a real content change (a word swapped, a number edited) shows up clearly.
    pub const SNAPSHOT_GRID: [u32; 2] = [48, 48];

    /// Box centre in document pixels.
    pub fn center(&self) -> [f64; 2] {
        [self.pos[0] + self.size[0] / 2.0, self.pos[1] + self.size[1] / 2.0]
    }

    /// Whether the box overlaps a document rectangle (used to keep markers on the canvas).
    pub fn overlaps(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        self.pos[0] < x1 && self.pos[0] + self.size[0] > x0 && self.pos[1] < y1 && self.pos[1] + self.size[1] > y0
    }
}

impl Default for RevisionMarker {
    fn default() -> Self {
        Self {
            name: String::new(),
            pos: [0.0, 0.0],
            size: [100.0, 60.0],
            layer_id: None,
            snapshot: Vec::new(),
            snapshot_size: Self::SNAPSHOT_GRID,
            done: false,
            note: String::new(),
        }
    }
}
