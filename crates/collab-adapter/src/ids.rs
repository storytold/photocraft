//! Addresses a delta can name.
//!
//! `LayerId` is stable inside a document and survives PSD round trips (`psd_id`), but it is
//! issued by a process counter. ADR-0002 puts the network identity in the server's hands; the
//! adapter keeps the document's own id and its part, and the map from a server key to it belongs
//! to the structural increment (see `docs/collab/adapters.md`).

use photocraft_doc::LayerId;
use photocraft_geom::TileCoord;

/// Which surface of a layer a tile belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// The layer's own pixels (`LayerContent::Raster`).
    Pixels,
    /// The layer mask (`Layer::mask`), edited separately from the pixels (TASK §5).
    Mask,
}

/// One tile of one layer part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileAddr {
    pub layer: LayerId,
    pub part: Part,
    pub coord: TileCoord,
}

impl TileAddr {
    pub const fn new(layer: LayerId, part: Part, coord: TileCoord) -> Self {
        Self { layer, part, coord }
    }
}
