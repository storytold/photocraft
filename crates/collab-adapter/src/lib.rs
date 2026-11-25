#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The PhotoCraft adapter between [`photocraft_doc::Document`] and a live session.
//!
//! RFC-0020 keeps the live-session contract in the shared layer and leaves each application an
//! adapter of its own. PhotoCraft's half is pixels: the document is a tree of layers whose pixels
//! live in 256×256 copy-on-write tiles, and an operation must cost kilobytes on the wire, not a
//! document. This crate turns a pair of documents — the one the session held before an edit and
//! the one the engine produced — into a [`delta::ChangeSet`], and keeps the per-actor inverse
//! history RFC-0020 and ADR-0010 ask for.
//!
//! What travels between two processes stays the shared contract's business: nothing here names a
//! wire envelope, a message kind or an encoding. The tiles are the ones the document already
//! holds; the undo log names the actor and the operation, not a transport.
//!
//! ## The tile delta
//!
//! A [`delta::TileDelta`] addresses one tile of one layer part ([`ids::Part::Pixels`] or
//! [`ids::Part::Mask`]) and carries either the whole tile ([`delta::TileDelta::Replace`]) or the
//! pixels that changed ([`delta::TileDelta::Patch`]). The patch is the load-bearing half: TASK §5
//! asks that two brushes in one tile lose no pixels, and a whole-tile replacement cannot promise
//! that. `diff` computes the patch pixel by pixel, so a server applies only the author's pixels
//! over its current tile and every replica converges on the same bytes.
//!
//! ## The undo log
//!
//! [`undo::UndoLog`] holds a stack per actor. A step keeps the before and after images of every
//! tile it touched plus the after image's fingerprint; undoing verifies the fingerprint first, so
//! a tile another actor changed after the step is refused with [`undo::UndoRefusal::Changed`]
//! instead of being silently overwritten (ADR-0010 §3). Undo and redo produce a delta that goes
//! back through the session as a new operation; the document's own snapshot history
//! (`photocraft-ops`) is not used for this and is not changed.
//!
//! The first increment covers pixels and masks. Layer properties, structure, presence, transport
//! and commit are listed in `docs/collab/adapters.md`.

pub mod delta;
pub mod ids;
pub mod undo;

pub use delta::{
    ApplyError, ChangeSet, DiffError, SurfaceAddr, TileDelta, TileImage, TilePatch, apply, diff, fingerprint, image_fingerprint, tile_fingerprint, tile_image,
};
pub use ids::{Part, TileAddr};
pub use undo::{Actor, ActorHistory, UndoLog, UndoRefusal, UndoStep};
