//! Per-actor inverse history.
//!
//! `photocraft-ops` keeps whole-document snapshots so one person can step back through everything
//! they did. A live session needs the other order: the step that undoes actor A must not undo
//! actor B (TASK §6, ADR-0010). This log keeps a stack per actor and the before/after image of
//! every tile a step touched; undo builds the inverse delta, checks that the tiles are still the
//! ones the step left, and hands the delta back to the session as a new operation.

use std::collections::BTreeMap;

use photocraft_doc::Document;

use crate::delta::{ApplyError, ChangeSet, DiffError, TileDelta, TileImage, apply, changed_indices, image_patch, images, tile_image};
use crate::ids::TileAddr;

/// One participant, as the server names it (`sub` from the JWT, ADR-0008 §7). The adapter does
/// not invent identity: the caller passes the actor the session was authenticated as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Actor(pub u64);

/// One tile of one step: what it held before, what it held after, and the pixels between the two.
#[derive(Clone, Debug)]
pub struct TileStep {
    pub addr: TileAddr,
    pub before: TileImage,
    pub after: TileImage,
    /// Pixel indices this step changed inside the tile; undo touches only these.
    pub changed: Vec<u32>,
}

/// One undoable step: the operation it came from and every tile it touched.
#[derive(Clone, Debug, Default)]
pub struct UndoStep {
    /// The session operation number the step was recorded from, for the journal and for audit.
    pub op: u64,
    pub tiles: Vec<TileStep>,
}

#[derive(Clone, Debug, Default)]
pub struct ActorHistory {
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
}

impl ActorHistory {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

/// Why an undo or redo did not happen. A refusal never changes the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoRefusal {
    /// This actor has nothing to undo or redo.
    Empty,
    /// Another actor changed the tile after the step; ADR-0010 §3 asks for a partial refusal, not
    /// a silent overwrite.
    Conflict { addr: TileAddr },
    /// A tile's pixels could not be read (scratch disk failure), so the check cannot be made.
    Unreadable(String),
    /// The document no longer has the layer or surface the step touched.
    Apply(ApplyError),
}

impl std::fmt::Display for UndoRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UndoRefusal::Empty => write!(f, "nothing to undo"),
            UndoRefusal::Conflict { addr } => write!(f, "tile {}x{} of layer {} changed after the step", addr.coord.tx, addr.coord.ty, addr.layer.0),
            UndoRefusal::Unreadable(why) => write!(f, "tile pixels unreadable: {why}"),
            UndoRefusal::Apply(why) => write!(f, "cannot apply the inverse: {why}"),
        }
    }
}

impl std::error::Error for UndoRefusal {}

impl From<DiffError> for UndoRefusal {
    fn from(error: DiffError) -> Self {
        UndoRefusal::Unreadable(error.0)
    }
}

impl From<ApplyError> for UndoRefusal {
    fn from(error: ApplyError) -> Self {
        UndoRefusal::Apply(error)
    }
}

/// A per-actor undo and redo log.
#[derive(Clone, Debug)]
pub struct UndoLog {
    actors: BTreeMap<Actor, ActorHistory>,
    /// Steps kept per actor (ADR-0010 §4 names 200 as the default).
    pub max_steps: usize,
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new(200)
    }
}

impl UndoLog {
    pub fn new(max_steps: usize) -> Self {
        Self { actors: BTreeMap::new(), max_steps: max_steps.max(1) }
    }

    pub fn history(&self, actor: Actor) -> Option<&ActorHistory> {
        self.actors.get(&actor)
    }

    /// Record the step an operation left: the tiles in `change`, read from the documents before
    /// and after it. An empty change records nothing; recording clears the actor's redo stack, as
    /// a new edit does in any editor.
    pub fn record(&mut self, actor: Actor, op: u64, before: &Document, change: &ChangeSet, after: &Document) -> Result<(), DiffError> {
        let mut addrs: Vec<TileAddr> = change.tiles.iter().map(TileDelta::addr).collect();
        addrs.sort_unstable();
        addrs.dedup();
        if addrs.is_empty() {
            return Ok(());
        }
        let old = images(before, &addrs)?;
        let new = images(after, &addrs)?;
        let tiles = old
            .into_iter()
            .zip(new)
            .map(|((addr, before), (after_addr, after))| {
                debug_assert_eq!(addr, after_addr);
                let changed = changed_indices(&before, &after);
                TileStep { addr, before, after, changed }
            })
            .collect();
        let history = self.actors.entry(actor).or_default();
        history.undo.push(UndoStep { op, tiles });
        history.redo.clear();
        let over = history.undo.len().saturating_sub(self.max_steps);
        if over > 0 {
            history.undo.drain(..over);
        }
        Ok(())
    }

    /// Build and apply the inverse of the actor's last step and return it, so the caller can put
    /// it on the wire as a new operation (ADR-0010 §2). The step moves to the actor's redo stack
    /// only when the document accepted it.
    pub fn undo(&mut self, actor: Actor, doc: &mut Document) -> Result<ChangeSet, UndoRefusal> {
        let step = {
            let history = self.actors.get(&actor).ok_or(UndoRefusal::Empty)?;
            history.undo.last().cloned().ok_or(UndoRefusal::Empty)?
        };
        verify(doc, &step, Side::After)?;
        let set = inverse(&step, Side::Before);
        apply(doc, &set)?;
        let history = self.actors.entry(actor).or_default();
        history.undo.pop();
        history.redo.push(step);
        Ok(set)
    }

    /// The mirror of [`Self::undo`]: put the actor's last undone step back.
    pub fn redo(&mut self, actor: Actor, doc: &mut Document) -> Result<ChangeSet, UndoRefusal> {
        let step = {
            let history = self.actors.get(&actor).ok_or(UndoRefusal::Empty)?;
            history.redo.last().cloned().ok_or(UndoRefusal::Empty)?
        };
        verify(doc, &step, Side::Before)?;
        let set = inverse(&step, Side::After);
        apply(doc, &set)?;
        let history = self.actors.entry(actor).or_default();
        history.redo.pop();
        history.undo.push(step);
        Ok(set)
    }

    /// Drop one actor's history (a participant left, or the session ended).
    pub fn forget(&mut self, actor: Actor) -> Option<ActorHistory> {
        self.actors.remove(&actor)
    }
}

#[derive(Clone, Copy)]
enum Side {
    Before,
    After,
}

/// Every pixel the step changed must still hold the value its side names, or another actor has
/// touched those pixels since (ADR-0010 §3). Pixels the step did not touch do not matter: another
/// actor may paint the rest of the same tile and this step still undoes.
fn verify(doc: &Document, step: &UndoStep, side: Side) -> Result<(), UndoRefusal> {
    for tile in &step.tiles {
        if tile.changed.is_empty() {
            continue;
        }
        let current = tile_image_surface(doc, tile.addr)?;
        let expected = match side {
            Side::Before => &tile.before,
            Side::After => &tile.after,
        };
        let bpp = current.format.bytes_per_pixel();
        for &index in &tile.changed {
            let at = index as usize * bpp;
            let same = at + bpp <= current.bytes.len() && at + bpp <= expected.bytes.len() && current.bytes[at..at + bpp] == expected.bytes[at..at + bpp];
            if !same {
                return Err(UndoRefusal::Conflict { addr: tile.addr });
            }
        }
    }
    Ok(())
}

fn tile_image_surface(doc: &Document, addr: TileAddr) -> Result<TileImage, UndoRefusal> {
    let surface = match addr.part {
        crate::ids::Part::Pixels => doc.layer(addr.layer).and_then(|l| match &l.content {
            photocraft_doc::LayerContent::Raster(s) => Some(s),
            _ => None,
        }),
        crate::ids::Part::Mask => doc.layer(addr.layer).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
    };
    let surface = surface.ok_or(UndoRefusal::Apply(ApplyError::NoPart(crate::delta::SurfaceAddr { layer: addr.layer, part: addr.part })))?;
    Ok(tile_image(surface, addr.coord)?)
}

/// A delta restoring every tile of the step to one side, touching only the pixels the step
/// changed: another actor's pixels in the same tile stay.
fn inverse(step: &UndoStep, side: Side) -> ChangeSet {
    ChangeSet {
        tiles: step
            .tiles
            .iter()
            .filter(|tile| !tile.changed.is_empty())
            .map(|tile| {
                let image = match side {
                    Side::Before => &tile.before,
                    Side::After => &tile.after,
                };
                TileDelta::Patch { addr: tile.addr, patch: image_patch(image, &tile.changed) }
            })
            .collect(),
        ..Default::default()
    }
}
