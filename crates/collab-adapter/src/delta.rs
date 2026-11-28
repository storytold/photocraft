//! Tile deltas: what changed between two documents, and how to put it back.

use std::collections::BTreeMap;

use photocraft_color::PixelFormat;
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::TileCoord;
use photocraft_raster::Surface;

use crate::ids::{Part, TileAddr};

/// A layer surface, without a tile coordinate yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceAddr {
    pub layer: LayerId,
    pub part: Part,
}

/// A whole tile as the document held it, with the format and default pixel needed to read it
/// outside that document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileImage {
    pub format: PixelFormat,
    /// The surface's default pixel for untouched tiles, `format.bytes_per_pixel()` bytes.
    pub default: Vec<u8>,
    /// `TILE_SIZE² × format.bytes_per_pixel()` bytes, row-major.
    pub bytes: Vec<u8>,
}

/// The pixels of one tile that changed, in document order: `indices[i]` is a pixel index inside
/// the tile and `pixels[i * bpp .. (i + 1) * bpp]` its value after the change.
///
/// Sparse on purpose: TASK §5 asks that two brushes in one tile lose no pixels, and a server can
/// only merge them if the delta says which pixels the author touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TilePatch {
    pub format: PixelFormat,
    /// The surface's default pixel (the value an absent tile reads as), `bpp` bytes.
    pub default: Vec<u8>,
    pub indices: Vec<u32>,
    pub pixels: Vec<u8>,
}

impl TilePatch {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// One changed tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TileDelta {
    /// Only the listed pixels changed; everything else in the tile stays as the replica has it.
    Patch { addr: TileAddr, patch: TilePatch },
    /// The whole tile is replaced: a new tile, or a tile whose surface default changed.
    Replace { addr: TileAddr, image: TileImage },
}

impl TileDelta {
    pub fn addr(&self) -> TileAddr {
        match self {
            TileDelta::Patch { addr, .. } | TileDelta::Replace { addr, .. } => *addr,
        }
    }
}

/// What one operation changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Incremental pixel and mask edits, tile by tile.
    pub tiles: Vec<TileDelta>,
    /// Surfaces whose format or default pixel changed, or that appeared/disappeared: a structural
    /// operation replaces them whole; tile deltas cannot express it.
    pub resets: Vec<SurfaceAddr>,
    /// Layers the operation added or removed. Their pixels travel with the structural operation,
    /// not as tile deltas (first increment).
    pub layers_added: Vec<LayerId>,
    pub layers_removed: Vec<LayerId>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty() && self.resets.is_empty() && self.layers_added.is_empty() && self.layers_removed.is_empty()
    }
}

/// A tile delta could not be produced: a tile's pixels were unreadable (scratch disk failure).
#[derive(Debug)]
pub struct DiffError(pub String);

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot read tile pixels: {}", self.0)
    }
}

impl std::error::Error for DiffError {}

/// The delta does not fit the document it is applied to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// The layer is gone (deleted, or moved to a document this replica does not have).
    NoLayer(LayerId),
    /// The layer has no such part (a raster layer has pixels, a layer without a mask has none).
    NoPart(SurfaceAddr),
    /// The surface's pixel format differs: a structural operation must replace it first.
    Format { addr: SurfaceAddr },
    /// The surface's default pixel differs for the same reason.
    Default { addr: SurfaceAddr },
    /// A tile image or patch is the wrong size for the surface.
    Shape { addr: TileAddr },
    /// A patch index lies outside the tile.
    Pixel { addr: TileAddr },
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::NoLayer(id) => write!(f, "no layer {}", id.0),
            ApplyError::NoPart(addr) => write!(f, "layer {} has no {:?}", addr.layer.0, addr.part),
            ApplyError::Format { addr } => write!(f, "layer {} {:?}: pixel format differs", addr.layer.0, addr.part),
            ApplyError::Default { addr } => write!(f, "layer {} {:?}: default pixel differs", addr.layer.0, addr.part),
            ApplyError::Shape { addr } => write!(f, "tile {}x{} of layer {}: wrong byte length", addr.coord.tx, addr.coord.ty, addr.layer.0),
            ApplyError::Pixel { addr } => write!(f, "tile {}x{} of layer {}: pixel index outside the tile", addr.coord.tx, addr.coord.ty, addr.layer.0),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Every pixel and mask surface of the document, by layer id.
fn surfaces(doc: &Document) -> BTreeMap<SurfaceAddr, &Surface> {
    let mut out = BTreeMap::new();
    for (_, _, layer) in doc.walk() {
        if let LayerContent::Raster(surface) = &layer.content {
            out.insert(SurfaceAddr { layer: layer.id, part: Part::Pixels }, surface);
        }
        if let Some(mask) = &layer.mask {
            out.insert(SurfaceAddr { layer: layer.id, part: Part::Mask }, &mask.surface);
        }
    }
    out
}

/// FNV-1a over the bytes. A fingerprint is a conflict check, not a security hash: it answers "is
/// this tile still the one the step left" (ADR-0010 §3), and a collision only costs a refused
/// undo, never a silent overwrite of different pixels in the same document version.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

/// The fingerprint of a tile image: its format, its default pixel and its bytes.
pub fn image_fingerprint(image: &TileImage) -> u64 {
    let mut h = fingerprint(&(image.format.bytes_per_pixel() as u64).to_le_bytes());
    h ^= fingerprint(&image.default).rotate_left(17);
    h ^ fingerprint(&image.bytes)
}

fn tile_len(bpp: usize) -> usize {
    // `photocraft_geom::TILE_SIZE` is the side of every tile; the pixel count follows from it.
    (photocraft_geom::TILE_SIZE as usize) * (photocraft_geom::TILE_SIZE as usize) * bpp
}

fn default_repeated(default: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        out.extend_from_slice(default);
    }
    out
}

/// The tile as it reads now; an absent tile is the default pixel repeated, so it fingerprints
/// like an all-default tile.
pub fn tile_image(surface: &Surface, coord: TileCoord) -> Result<TileImage, DiffError> {
    match read_image(surface, coord)? {
        Some(image) => Ok(image),
        None => Ok(TileImage {
            format: surface.format(),
            default: surface.default_bytes().to_vec(),
            bytes: default_repeated(surface.default_bytes(), tile_len(surface.format().bytes_per_pixel())),
        }),
    }
}

/// The fingerprint of a tile as the surface holds it now (see [`image_fingerprint`]).
pub fn tile_fingerprint(surface: &Surface, coord: TileCoord) -> Result<u64, DiffError> {
    Ok(image_fingerprint(&tile_image(surface, coord)?))
}

/// The pixel indices whose bytes differ between two images of one tile. Both read the same tile
/// format and default by construction.
pub(crate) fn changed_indices(a: &TileImage, b: &TileImage) -> Vec<u32> {
    let bpp = a.format.bytes_per_pixel();
    let pixels = a.bytes.len().min(b.bytes.len()) / bpp;
    (0..pixels).filter(|&i| a.bytes[i * bpp..(i + 1) * bpp] != b.bytes[i * bpp..(i + 1) * bpp]).map(|i| i as u32).collect()
}

/// A patch taking the pixel values at `indices` from `image`.
pub(crate) fn image_patch(image: &TileImage, indices: &[u32]) -> TilePatch {
    let bpp = image.format.bytes_per_pixel();
    let mut pixels = Vec::with_capacity(indices.len() * bpp);
    for &index in indices {
        let at = index as usize * bpp;
        pixels.extend_from_slice(&image.bytes[at..at + bpp]);
    }
    TilePatch { format: image.format, default: image.default.clone(), indices: indices.to_vec(), pixels }
}

/// A whole tile image read from a surface, or `None` when the tile is absent (all default).
fn read_image(surface: &Surface, coord: TileCoord) -> Result<Option<TileImage>, DiffError> {
    let Some(tile) = surface.tile(coord) else { return Ok(None) };
    let bytes = tile.try_bytes().map_err(DiffError)?;
    Ok(Some(TileImage { format: surface.format(), default: surface.default_bytes().to_vec(), bytes: bytes.to_vec() }))
}

/// The delta between two documents: incremental tiles of surfaces that exist in both, resets for
/// surfaces that appeared, disappeared or changed format, and the layer ids added or removed.
pub fn diff(before: &Document, after: &Document) -> Result<ChangeSet, DiffError> {
    let old = surfaces(before);
    let new = surfaces(after);
    let mut set = ChangeSet::default();
    let mut seen_layers = std::collections::BTreeSet::new();
    for (_, _, layer) in before.walk() {
        seen_layers.insert(layer.id);
    }
    for (_, _, layer) in after.walk() {
        if seen_layers.remove(&layer.id) {
            continue;
        }
        set.layers_added.push(layer.id);
    }
    set.layers_removed.extend(seen_layers);

    let mut addrs: Vec<SurfaceAddr> = old.keys().chain(new.keys()).copied().collect();
    addrs.sort_unstable();
    addrs.dedup();
    for addr in addrs {
        match (old.get(&addr), new.get(&addr)) {
            (Some(a), Some(b)) => {
                if a.format() != b.format() || a.default_bytes() != b.default_bytes() {
                    set.resets.push(addr);
                    continue;
                }
                diff_surface(a, b, addr, &mut set)?;
            }
            // A part that appeared or disappeared is a structural change; its content travels
            // with the operation, not as tile deltas.
            _ => set.resets.push(addr),
        }
    }
    Ok(set)
}

fn diff_surface(a: &Surface, b: &Surface, addr: SurfaceAddr, set: &mut ChangeSet) -> Result<(), DiffError> {
    let (mut xs, mut ys) = (a.tiles().peekable(), b.tiles().peekable());
    loop {
        let coord: TileCoord = match (xs.peek(), ys.peek()) {
            (None, None) => return Ok(()),
            (Some((ca, _)), Some((cb, _))) => (**ca).min(**cb),
            (Some((ca, _)), None) => **ca,
            (None, Some((cb, _))) => **cb,
        };
        // Two replicas sharing the tile pointer have the same pixels: no read, no delta.
        let unchanged = match (xs.peek(), ys.peek()) {
            (Some((ca, ta)), Some((cb, tb))) => **ca == coord && **cb == coord && std::sync::Arc::ptr_eq(ta, tb),
            _ => false,
        };
        if !unchanged && let Some(patch) = tile_patch(a, b, coord)? {
            set.tiles.push(TileDelta::Patch { addr: TileAddr { layer: addr.layer, part: addr.part, coord }, patch });
        }
        if xs.peek().is_some_and(|(c, _)| **c == coord) {
            xs.next();
        }
        if ys.peek().is_some_and(|(c, _)| **c == coord) {
            ys.next();
        }
    }
}

/// The pixels that differ between the two surfaces at `coord`; the absent side reads as default.
fn tile_patch(a: &Surface, b: &Surface, coord: TileCoord) -> Result<Option<TilePatch>, DiffError> {
    debug_assert_eq!(a.format(), b.format());
    let bpp = b.format().bytes_per_pixel();
    let default = b.default_bytes();
    let before = match a.tile(coord) {
        Some(t) => Some(t.try_bytes().map_err(DiffError)?),
        None => None,
    };
    let after = match b.tile(coord) {
        Some(t) => Some(t.try_bytes().map_err(DiffError)?),
        None => None,
    };
    let len = tile_len(bpp);
    let before_len = before.as_ref().map_or(len, |x| x.len());
    let after_len = after.as_ref().map_or(len, |x| x.len());
    let pixels = before_len.min(after_len) / bpp;
    let mut indices = Vec::new();
    let mut values = Vec::new();
    for i in 0..pixels {
        let old = before.as_ref().map_or(default, |x| &x[i * bpp..(i + 1) * bpp]);
        let new = after.as_ref().map_or(default, |x| &x[i * bpp..(i + 1) * bpp]);
        if old != new {
            indices.push(i as u32);
            values.extend_from_slice(new);
        }
    }
    if indices.is_empty() {
        return Ok(None);
    }
    Ok(Some(TilePatch { format: b.format(), default: default.to_vec(), indices, pixels: values }))
}

/// Apply a delta to a document: every patch touches only its own pixels, every replace its own
/// tile, so an unrelated layer or tile is untouched.
pub fn apply(doc: &mut Document, set: &ChangeSet) -> Result<(), ApplyError> {
    for delta in &set.tiles {
        match delta {
            TileDelta::Patch { addr, patch } => apply_patch(doc, *addr, patch)?,
            TileDelta::Replace { addr, image } => apply_image(doc, *addr, image)?,
        }
    }
    Ok(())
}

fn part_mut(doc: &mut Document, addr: SurfaceAddr) -> Result<&mut Surface, ApplyError> {
    let layer = doc.layer_mut(addr.layer).ok_or(ApplyError::NoLayer(addr.layer))?;
    match addr.part {
        Part::Pixels => layer.surface_mut().ok_or(ApplyError::NoPart(addr)),
        Part::Mask => layer.mask.as_mut().map(|m| &mut m.surface).ok_or(ApplyError::NoPart(addr)),
    }
}

fn apply_patch(doc: &mut Document, addr: TileAddr, patch: &TilePatch) -> Result<(), ApplyError> {
    let bpp = patch.format.bytes_per_pixel();
    let surface_addr = SurfaceAddr { layer: addr.layer, part: addr.part };
    let surface = part_mut(doc, surface_addr)?;
    if surface.format() != patch.format {
        return Err(ApplyError::Format { addr: surface_addr });
    }
    if surface.default_bytes() != patch.default.as_slice() {
        return Err(ApplyError::Default { addr: surface_addr });
    }
    let len = tile_len(bpp);
    if patch.pixels.len() != patch.indices.len() * bpp {
        return Err(ApplyError::Shape { addr });
    }
    let tile = surface.tile_mut(addr.coord);
    let mut bytes = tile.bytes_mut();
    if bytes.len() != len {
        return Err(ApplyError::Shape { addr });
    }
    for (i, &index) in patch.indices.iter().enumerate() {
        let at = index as usize * bpp;
        if at + bpp > len {
            return Err(ApplyError::Pixel { addr });
        }
        bytes[at..at + bpp].copy_from_slice(&patch.pixels[i * bpp..(i + 1) * bpp]);
    }
    Ok(())
}

fn apply_image(doc: &mut Document, addr: TileAddr, image: &TileImage) -> Result<(), ApplyError> {
    let surface_addr = SurfaceAddr { layer: addr.layer, part: addr.part };
    let surface = part_mut(doc, surface_addr)?;
    if surface.format() != image.format {
        return Err(ApplyError::Format { addr: surface_addr });
    }
    if surface.default_bytes() != image.default.as_slice() {
        return Err(ApplyError::Default { addr: surface_addr });
    }
    if image.bytes.len() != tile_len(image.format.bytes_per_pixel()) {
        return Err(ApplyError::Shape { addr });
    }
    let tile = surface.tile_mut(addr.coord);
    let mut bytes = tile.bytes_mut();
    if bytes.len() != image.bytes.len() {
        return Err(ApplyError::Shape { addr });
    }
    bytes.copy_from_slice(&image.bytes);
    Ok(())
}

/// Read every tile an operation touched, keyed by address; the undo log keeps both sides.
pub(crate) fn images(doc: &Document, addrs: &[TileAddr]) -> Result<Vec<(TileAddr, TileImage)>, DiffError> {
    let mut out = Vec::with_capacity(addrs.len());
    for &addr in addrs {
        let surface = match addr.part {
            Part::Pixels => doc.layer(addr.layer).and_then(|l| match &l.content {
                LayerContent::Raster(s) => Some(s),
                _ => None,
            }),
            Part::Mask => doc.layer(addr.layer).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
        };
        let surface = surface.ok_or_else(|| DiffError(format!("layer {} lost its {:?}", addr.layer.0, addr.part)))?;
        out.push((addr, tile_image(surface, addr.coord)?));
    }
    Ok(out)
}
