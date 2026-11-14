//! GIMP XCF (`.xcf`, `.xcf.gz`) read. Layout from the public specification
//! (https://developer.gimp.org/core/standards/xcf/, versions 0 to 26): a header, image
//! properties, then pointers to layer, channel and path structures; each drawable's pixels sit in
//! 64×64 tiles, uncompressed, RLE-coded or zlib-coded.
//!
//! Read: pixel layers at any position and size, groups (nested, pass-through), names, opacity,
//! visibility, offsets, layer masks (applied or disabled), locks, colour tags, blend modes (the
//! legacy and the 2.10 sets; those without a Photoshop equivalent open as Normal with a warning),
//! 8/16/32-bit integer and 16/32/64-bit float precisions (linear precisions get a linear-light
//! profile), the embedded ICC profile, resolution, guides, the comment, extra channels and the
//! saved selection; indexed images open as RGB. Text, vector and link layers open as their pixels;
//! non-destructive layer effects, paths and floating selections are reported, not applied.
//! Write: not supported (GIMP's own format; `.pcraft` keeps everything PhotoCraft holds).
//!
//! The reader is written from the specification and from files GIMP 3.2 writes; no GIMP code.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use craft_xcf::{BaseType, Orientation, Pixels, Precision, TreeNode, Xcf};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask, Locks};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

#[cfg(test)]
use craft_xcf::{MAX_PIXEL_BYTES, MAX_SIDE};
pub(crate) use craft_xcf::{is_gzipped_xcf, is_xcf};

type Result<T> = std::result::Result<T, IoError>;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Xcf(message.into())
}

/// The format crate's error as this crate's.
fn xcf_err(e: craft_xcf::Error) -> IoError {
    match e {
        craft_xcf::Error::Invalid(m) => invalid(m),
        craft_xcf::Error::Cancelled => IoError::Cancelled,
    }
}

/// Inflates a gzipped XCF, bounded.
pub(crate) fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    craft_xcf::gunzip(bytes).map_err(xcf_err)
}

/// The document depth the samples open at.
fn depth(p: Precision) -> SampleType {
    match (p.float, p.bytes) {
        (false, 1) => SampleType::U8,
        (false, _) => SampleType::U16,
        (true, _) => SampleType::F32,
    }
}

/// A layer as read, before the tree is built.
struct Node {
    layer: Layer,
    group: bool,
}

/// GIMP's layer mode as Photoshop's, with a note when the two differ.
fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("GIMP's legacy Soft light")),
        45 => (B::SoftLight, Some("GIMP's Soft light")),
        23 => (B::Overlay, None),
        6 | 32 => (B::Difference, None),
        7 | 33 => (B::LinearDodge, None),
        8 | 34 => (B::Subtract, None),
        9 | 35 => (B::Darken, None),
        10 | 36 => (B::Lighten, None),
        11 | 37 => (B::Hue, Some("HSV Hue")),
        12 | 38 => (B::Saturation, Some("HSV Saturation")),
        13 | 39 => (B::Color, Some("HSL Color")),
        14 | 40 => (B::Luminosity, Some("HSV Value")),
        15 | 41 => (B::Divide, None),
        16 | 42 => (B::ColorDodge, None),
        17 | 43 => (B::ColorBurn, None),
        18 | 44 => (B::HardLight, None),
        20 | 46 => (B::Normal, Some("Grain extract")),
        21 | 47 => (B::Normal, Some("Grain merge")),
        22 | 57 => (B::Normal, Some("Color erase")),
        24 => (B::Hue, Some("LCH Hue")),
        25 => (B::Saturation, Some("LCH Chroma")),
        26 => (B::Color, Some("LCH Color")),
        27 => (B::Luminosity, Some("LCH Lightness")),
        48 => (B::VividLight, None),
        49 => (B::PinLight, None),
        50 => (B::LinearLight, None),
        51 => (B::HardMix, None),
        52 => (B::Exclusion, None),
        53 => (B::LinearBurn, None),
        54 => (B::DarkerColor, Some("Luma darken only")),
        55 => (B::LighterColor, Some("Luma lighten only")),
        56 => (B::Luminosity, Some("Luminance")),
        58 => (B::Normal, Some("Erase")),
        59 => (B::Normal, Some("Merge")),
        60 => (B::Normal, Some("Split")),
        61 => (B::PassThrough, None),
        _ => (B::Normal, Some("an unknown mode")),
    }
}

fn label(tag: u32) -> LabelColor {
    match tag {
        1 => LabelColor::Blue,
        2 => LabelColor::Green,
        3 => LabelColor::Yellow,
        4 => LabelColor::Orange,
        6 => LabelColor::Red,
        7 => LabelColor::Violet,
        8 => LabelColor::Gray,
        _ => LabelColor::None,
    }
}

/// Writes `px` (its channels matching `surface`'s) at `x, y` of the document, charging the
/// tiles that creates to `budget`: the drawable was admitted by its logical samples, and a
/// narrow drawable (or one referenced by many layer pointers) stores far more in whole tiles.
fn write_pixels(surface: &mut Surface, px: &Pixels, x: i32, y: i32, budget: &mut usize) -> Result<()> {
    let (w, h) = (px.width as i32, px.height as i32);
    let rect = Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?);
    if surface.channels() != px.channels || px.data.len() != (px.width as usize) * (px.height as usize) * px.channels {
        return Err(invalid("pixel data does not match the drawable"));
    }
    // Checked before the write (it allocates the whole rect's tiles); only the tiles kept after
    // `prune()` are charged.
    let tiles = usize::try_from(photocraft_raster::tile_storage_bytes(rect, &surface.format())).unwrap_or(usize::MAX);
    if tiles > *budget {
        return Err(invalid("the layers exceed the 1 GiB pixel limit"));
    }
    let before = surface.tile_count();
    surface.write_region(rect, &px.data);
    surface.prune();
    let added = surface.tile_count().saturating_sub(before) as u64;
    *budget = budget.saturating_sub(usize::try_from(photocraft_raster::stored_tiles_bytes(added, &surface.format())).unwrap_or(usize::MAX));
    Ok(())
}

/// Imports an XCF file (already inflated when it was gzipped).
pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    let xcf = Xcf::parse(bytes).map_err(xcf_err)?;
    let info = xcf.image();
    let version = info.version;
    let mut warnings = Warnings::default();
    if version > craft_xcf::KNOWN_VERSION {
        warnings.note(&format!("XCF version {version} is newer than this reader (version {}); unknown parts were skipped", craft_xcf::KNOWN_VERSION));
    }
    let mode = match info.base_type {
        BaseType::Gray => ColorMode::Grayscale,
        BaseType::Rgb | BaseType::Indexed => ColorMode::Rgb,
    };
    let precision = info.precision;
    let depth = depth(precision);
    if info.base_type == BaseType::Indexed {
        warnings.note("the indexed image opens as RGB");
    }
    if !precision.float && precision.bytes == 4 {
        warnings.note("32-bit integer samples open as 16-bit");
    }

    let mut reader = Reader { xcf: &xcf, ctl, budget: craft_xcf::MAX_PIXEL_BYTES, warnings, clipped_to_backdrop: 0 };
    let fmt = PixelFormat::new(mode, depth, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let total = info.layers.len().max(1) as f32;

    // Layers, topmost first in the file.
    let mut nodes: Vec<Option<Node>> = Vec::with_capacity(info.layers.len());
    let mut linear_compositing = false;
    for i in 0..info.layers.len() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(0.05 + 0.85 * i as f32 / total);
        match reader.layer(i, fmt, mask_fmt)? {
            Some((node, linear)) => {
                linear_compositing |= linear;
                nodes.push(Some(node));
            }
            None => nodes.push(None),
        }
    }
    let tree = info.tree();
    for &i in &tree.too_deep {
        if let Some(l) = info.layers.get(i) {
            reader.warnings.layer(&l.name, "is nested deeper than the reader allows and was moved to the top level");
        }
    }
    for &i in &tree.misplaced {
        if let Some(l) = info.layers.get(i) {
            reader.warnings.layer(&l.name, "has an item path that leads to no group; opened at the top level");
        }
    }
    let layers = finish(&tree.roots, &mut nodes);

    // Channels: the selection and the extras.
    let mut channels = Vec::new();
    let mut selection = None;
    let keep_going = || ctl.check().is_ok();
    for i in 0..info.channel_count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (channel, px) = xcf.channel(i, &mut reader.budget, &keep_going).map_err(xcf_err)?;
        let mut surface = Surface::new(mask_fmt);
        write_pixels(&mut surface, &px, 0, 0, &mut reader.budget)?;
        if channel.selection {
            selection = Some(surface);
        } else {
            channels.push(AlphaChannel::new(channel.name, surface));
        }
    }

    let mut document = Document::new(name, Size::new(info.width, info.height), mode, depth);
    document.layers = layers;
    document.channels = channels;
    document.selection = selection;
    if let Some((x, y)) = info.resolution.filter(|(x, _)| *x > 0.0 && *x < 1.0e6) {
        document.resolution_dpi = x;
        if let Some(w) = crate::unequal_resolution_warning(f64::from(x), f64::from(y)) {
            reader.warnings.note(&w);
        }
    }
    for guide in &info.guides {
        match guide.orientation {
            Orientation::Horizontal => document.guides.horizontal.push(f64::from(guide.position)),
            Orientation::Vertical => document.guides.vertical.push(f64::from(guide.position)),
        }
    }
    document.icc_profile = match (precision.linear, info.icc_profile.clone(), mode) {
        (true, icc, ColorMode::Grayscale) => {
            if icc.is_some() {
                reader.warnings.note("the linear-light image's own profile was replaced by linear gray");
            }
            Some(craft_cms::builtin::linear_gray().to_bytes())
        }
        (true, icc, _) => {
            if icc.is_some() {
                reader.warnings.note("the linear-light image's own profile was replaced by linear sRGB");
            }
            Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes())
        }
        (false, Some(icc), _) => Some(Arc::new(icc)),
        (false, None, _) => None,
    };
    if let Some(comment) = info.comment.clone().filter(|c| !c.is_empty()) {
        document.metadata.text.push(("Comment".into(), comment));
    }
    if info.path_count > 0 {
        reader.warnings.note(&format!("{} path(s) were not imported", info.path_count));
    }
    add_document_warnings(&mut reader, &document, precision, linear_compositing);
    ctl.progress(1.0);
    Ok(ImportResult { document, warnings: reader.warnings.finish(), source_read_only: true, preview_only: false })
}

/// What the layers need while they are read.
struct Reader<'x, 'a, 'c> {
    xcf: &'x Xcf<'a>,
    ctl: &'c Interrupt<'c>,
    budget: usize,
    warnings: Warnings,
    /// Layers GIMP composites with "Clip to backdrop" (its default for every mode but Normal).
    clipped_to_backdrop: usize,
}

fn add_document_warnings(reader: &mut Reader<'_, '_, '_>, document: &Document, precision: Precision, linear_compositing: bool) {
    // GIMP clips a layer in any mode but Normal to the layers beneath it, so it shows nothing where
    // they are transparent; PhotoCraft (like Photoshop) shows it there. Only tell when there is
    // transparency to show through: a bottom layer that is not opaque.
    if reader.clipped_to_backdrop > 0 && document.layers.first().is_some_and(|l| !l.locks.transparency || l.is_group()) {
        reader.warnings.note(&format!(
            "{} layer(s) use a mode GIMP clips to the layers beneath; where those are transparent GIMP shows nothing and PhotoCraft shows the layer",
            reader.clipped_to_backdrop
        ));
    }
    // GIMP blends and composites in linear light by default (since 2.10); PhotoCraft, like
    // Photoshop, in the document's own encoding, so semi-transparent edges and most blend modes
    // render a little differently. Layers set to a perceptual space in GIMP match.
    if !precision.linear && linear_compositing {
        reader
            .warnings
            .note("GIMP composites this image in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
    }
    if document.layers.is_empty() {
        reader.warnings.note("the XCF file has no layers");
    }
}

/// The tree's nodes (top-first) into layers bottom-first, groups recursively. Each layer is
/// taken out of `nodes` once.
fn finish(tree: &[TreeNode], nodes: &mut [Option<Node>]) -> Vec<Layer> {
    let mut out = Vec::with_capacity(tree.len());
    for t in tree.iter().rev() {
        let Some(mut node) = nodes.get_mut(t.layer).and_then(Option::take) else { continue };
        if node.group {
            let children = finish(&t.children, nodes);
            if let Some(c) = node.layer.children_mut() {
                *c = children;
            }
        }
        out.push(node.layer);
    }
    out
}

impl Reader<'_, '_, '_> {
    /// Layer `index`, and whether GIMP composites it in linear light. `None` for a floating
    /// selection, which is reported instead.
    fn layer(&mut self, index: usize, fmt: PixelFormat, mask_fmt: PixelFormat) -> Result<Option<(Node, bool)>> {
        let Some(l) = self.xcf.image().layers.get(index) else { return Ok(None) };
        let name = l.name.clone();
        if l.floating_selection {
            self.warnings.layer(&name, "is a floating selection and was not imported (anchor it in GIMP first)");
            return Ok(None);
        }
        let (blend, note) = blend_mode(l.mode);
        if let Some(note) = note {
            self.warnings.layer(
                &name,
                &format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".to_string() } else { format!("Photoshop's {blend:?}") }
                ),
            );
        }
        match l.composite_mode.unsigned_abs() {
            0 | 1 => {}
            2 => self.clipped_to_backdrop += 1,
            3 => self.warnings.layer(&name, "composites with Clip to layer, which is not reproduced"),
            4 => self.warnings.layer(&name, "composites with Intersection, which is not reproduced"),
            _ => {}
        }
        if l.text {
            self.warnings.layer(&name, "is a text layer and opens as pixels");
        }
        if l.vector {
            self.warnings.layer(&name, "is a vector layer and opens as pixels");
        }
        if l.link {
            self.warnings.layer(&name, "is a link layer and opens as pixels");
        }
        if l.effects > 0 {
            self.warnings.layer(&name, &format!("has {} layer effect(s) (non-destructive filters) that are not applied", l.effects));
        }

        let mut layer = if l.group {
            let mut g = Layer::group(name.clone(), Vec::new());
            if let LayerContent::Group(gr) = &mut g.content {
                gr.expanded = l.expanded;
            }
            g.blend = blend;
            g
        } else {
            let mut layer = Layer::raster(name.clone(), fmt);
            layer.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            layer
        };
        layer.opacity = l.opacity;
        layer.visible = l.visible;
        layer.locks = Locks { transparency: l.lock_alpha, pixels: l.lock_content, position: l.lock_position, ..Locks::default() };
        layer.label = label(l.color_tag);
        layer.clipped = l.clipped;

        let ctl = self.ctl;
        let keep_going = || ctl.check().is_ok();
        if !l.group
            && let Some(mut px) = self.xcf.layer_pixels(index, &mut self.budget, &keep_going).map_err(xcf_err)?
        {
            let alpha = l.has_alpha();
            if !alpha {
                // An opaque layer: add the alpha channel the document's pixel format carries.
                let n = px.channels;
                let mut data = Vec::with_capacity(px.data.len() / n * (n + 1));
                for p in px.data.chunks_exact(n) {
                    data.extend_from_slice(p);
                    data.push(1.0);
                }
                px = Pixels { width: px.width, height: px.height, channels: n + 1, data };
            }
            let mut surface = Surface::new(fmt);
            write_pixels(&mut surface, &px, l.x, l.y, &mut self.budget)?;
            layer.content = LayerContent::Raster(surface);
            if !alpha {
                layer.locks.transparency = true;
            }
        }
        if let Some((_, px)) = self.xcf.layer_mask(index, &mut self.budget, &keep_going).map_err(xcf_err)? {
            let mut surface = Surface::new(mask_fmt);
            write_pixels(&mut surface, &px, l.x, l.y, &mut self.budget)?;
            layer.mask = Some(LayerMask { surface, enabled: l.apply_mask, linked: true, density: 1.0, feather: 0.0 });
        }
        let linear = l.composite_space.unsigned_abs() == 1 || l.blend_space.unsigned_abs() == 1;
        Ok(Some((Node { layer, group: l.group }, linear)))
    }
}
