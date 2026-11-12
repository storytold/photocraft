//! Krita (`.kra`, `.krz`) import. Layout observed from files Krita writes (no Krita code used):
//! a ZIP whose first, stored entry is `mimetype` = `application/x-krita`, a `maindoc.xml` node
//! tree, one tiled pixel file per paint layer and mask under `<image name>/layers/`, the
//! image's ICC profile in `<image name>/annotations/icc`, and Krita's rendering in
//! `mergedimage.png`.
//!
//! Tiled pixel files are a short text header (`VERSION 2`, `TILEWIDTH`, `TILEHEIGHT`,
//! `PIXELSIZE`, `DATA <tiles>`) followed by, per tile, a `x,y,LZF,<bytes>` line and that many
//! bytes: a flag (1 = LZF-compressed, 0 = raw) and the tile's pixels stored as byte planes
//! (every pixel's first byte, then every pixel's second byte, ...). RGB pixels are in
//! Krita's BGRA order, 16-bit samples little-endian.
//!
//! Imported: paint layers (8 and 16-bit RGBA and gray), groups (isolated and pass-through),
//! names, visibility, opacity, blend modes, locks, layer offsets and transparency masks.
//! Every other node kind (vector, filter, fill, file and clone layers, filter, transform and
//! colorize masks) is left out with a warning; Krita's merged image still shows their look.
//! Documents in a colour space we don't read open as Krita's merged image, flagged as a preview.

#[cfg(test)]
mod tests;
mod tiles;

use craft_kra::tiles::{Space, TileFile};
use craft_kra::{Archive, Node};
use photocraft_color::{BlendMode, PixelFormat};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, Surface};

use crate::ora::{MAX_PIXEL_BYTES, Warnings, krita_blend};
use crate::{ImportResult, IoError};
use tiles::SpaceExt;

pub(crate) use craft_kra::{EXTENSIONS, is_kra};

type Result<T> = std::result::Result<T, IoError>;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Kra(message.into())
}

/// The format crate's error as this crate's.
fn kra_err(e: craft_kra::Error) -> IoError {
    match e {
        craft_kra::Error::Invalid(m) => invalid(m),
        craft_kra::Error::Unsupported(m) => IoError::Unsupported(m),
    }
}

struct Reader<'a, 'c> {
    archive: Archive<'a>,
    ctl: &'c Interrupt<'c>,
    /// The document's pixel format (layers in another supported space are converted to it).
    fmt: PixelFormat,
    nodes: usize,
    budget: usize,
    warnings: Warnings,
}

impl Reader<'_, '_> {
    /// A tiled pixel file in `space` as a surface in `fmt`, offset by (`dx`, `dy`).
    fn surface(&mut self, name: &str, space: Space, fmt: PixelFormat, dx: i32, dy: i32) -> Result<Option<Surface>> {
        let Some(bytes) = self.archive.layer_file(name).map_err(kra_err)? else { return Ok(None) };
        let file = TileFile::parse(&bytes).map_err(kra_err)?;
        if file.pixel_size != space.pixel_size() {
            return Err(invalid(format!("{name}: pixel size {} does not match its colour space", file.pixel_size)));
        }
        let cost = tiles::decoded_bytes(&file, fmt.bytes_per_pixel(), dx, dy)?;
        self.budget = self.budget.checked_sub(cost).ok_or_else(|| invalid("decoded layers exceed the 1 GiB limit"))?;
        let mut surface = match self.archive.default_pixel(name, space).map_err(kra_err)? {
            Some(px) if px.iter().any(|v| *v != 0.0) => Surface::with_default(fmt, &space.to_format(&px, fmt)),
            _ => Surface::new(fmt),
        };
        tiles::decode_into(&file, &mut surface, space, dx, dy, self.ctl)?;
        surface.prune();
        Ok(Some(surface))
    }

    fn props(&mut self, node: &Node, layer: &mut Layer) {
        layer.name = node.name.as_deref().filter(|n| !n.is_empty()).map_or_else(|| format!("Layer {}", self.nodes), str::to_string);
        layer.visible = node.visible;
        layer.opacity = (node.opacity / 255.0).clamp(0.0, 1.0) as f32;
        if node.locked {
            layer.locks.all = true;
        }
        let op = node.composite_op.as_deref().unwrap_or("normal");
        layer.blend = match krita_blend(op) {
            Some((mode, approx)) => {
                if approx {
                    self.warnings.note(&format!("Krita's {op} blend mode is approximated by {}", mode.label()));
                }
                mode
            }
            None => {
                self.warnings.layer(&layer.name, &format!("Krita's {op} blend mode is not supported; it opens as Normal"));
                BlendMode::Normal
            }
        };
        if node.channel_flags.as_deref().is_some_and(|f| f.contains('0')) {
            self.warnings.layer(&layer.name, "channel locks (Krita's per-channel blending) are ignored");
        }
        if node.inherit_alpha {
            self.warnings.layer(&layer.name, "Inherit Alpha is not supported; these layers blend normally");
        }
        if node.keyframes.as_deref().is_some_and(|k| !k.is_empty()) {
            self.warnings.layer(&layer.name, "animated layers open with the frame Krita saved in the layer");
        }
    }

    /// The node's transparency masks, combined into one layer mask.
    fn masks(&mut self, node: &Node, layer: &mut Layer, dx: i32, dy: i32) -> Result<()> {
        for mask in &node.masks {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            match mask.node_type.as_str() {
                "transparencymask" => {}
                // A layer's local selection: not part of its look.
                "selectionmask" => continue,
                other => {
                    let what = match other {
                        "filtermask" => "filter masks",
                        "transformmask" => "transform masks",
                        "colorizemask" => "colorize masks",
                        _ => "unknown masks",
                    };
                    self.warnings.layer(&layer.name, &format!("{what} are not supported and were left out"));
                    continue;
                }
            }
            if layer.mask.is_some() {
                self.warnings.layer(&layer.name, "only the first transparency mask of a layer is kept");
                continue;
            }
            let Some(file) = mask.filename.as_deref() else { continue };
            let (mx, my) = (dx.saturating_add(mask.x), dy.saturating_add(mask.y));
            match self.surface(&format!("{file}.pixelselection"), Space::Alpha8, PixelFormat::GRAY8, mx, my)? {
                Some(surface) => {
                    layer.mask = Some(LayerMask { surface, enabled: mask.visible, ..LayerMask::reveal_all() });
                }
                None => self.warnings.layer(&layer.name, "a transparency mask's pixels are missing from the file"),
            }
        }
        Ok(())
    }

    /// `nodes` (topmost first, as maindoc.xml lists them) as layers, bottom-to-top.
    fn layers(&mut self, nodes: &[Node], depth: usize, dx: i32, dy: i32) -> Result<Vec<Layer>> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(IoError::Unsupported(format!("layer groups nested deeper than {}", photocraft_doc::MAX_GROUP_DEPTH)));
        }
        let mut out = Vec::new();
        for child in nodes {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            self.nodes += 1;
            let (x, y) = (dx.saturating_add(child.x), dy.saturating_add(child.y));
            let mut layer = match child.node_type.as_str() {
                "paintlayer" => {
                    let mut layer = Layer::raster("", self.fmt);
                    self.props(child, &mut layer);
                    let space = child.colorspace_name.as_deref().and_then(Space::from_name);
                    match (space, child.filename.as_deref()) {
                        (Some(space), Some(file)) => match self.surface(file, space, self.fmt, x, y)? {
                            Some(s) => layer.content = LayerContent::Raster(s),
                            None => self.warnings.layer(&layer.name, "the layer's pixels are missing from the file; it opens empty"),
                        },
                        (None, _) => {
                            let cs = child.colorspace_name.as_deref().unwrap_or("?");
                            self.warnings.layer(&layer.name, &format!("layers in Krita's {cs} colour space are not supported and were left out"));
                            continue;
                        }
                        (_, None) => self.warnings.layer(&layer.name, "the layer names no pixel file; it opens empty"),
                    }
                    layer
                }
                "grouplayer" => {
                    let mut layer = Layer::group("", Vec::new());
                    self.props(child, &mut layer);
                    if child.passthrough {
                        if layer.blend != BlendMode::Normal {
                            self.warnings.layer(&layer.name, "a pass-through group's blend mode is ignored, as in Krita");
                        }
                        layer.blend = BlendMode::PassThrough;
                    }
                    // A group's own x/y is not applied to its children: Krita moves a group by
                    // moving their pixels, and its merged image ignores the group offset.
                    let kids = self.layers(&child.layers, depth + 1, dx, dy)?;
                    if let LayerContent::Group(g) = &mut layer.content {
                        g.children = kids;
                        g.expanded = !child.collapsed;
                    }
                    layer
                }
                other => {
                    let name = child.name.as_deref().unwrap_or(other).to_string();
                    let what = match other {
                        "shapelayer" => "vector layers",
                        "adjustmentlayer" => "filter layers",
                        "generatorlayer" => "fill layers",
                        "filelayer" => "file layers",
                        "clonelayer" => "clone layers",
                        _ => "unknown layer kinds",
                    };
                    self.warnings.layer(&name, &format!("Krita {what} are not supported yet and were left out"));
                    continue;
                }
            };
            self.masks(child, &mut layer, x, y)?;
            out.push(layer);
        }
        out.reverse();
        Ok(out)
    }
}

/// Krita's merged image as a one-layer stand-in, for documents we can't read layer by layer.
fn preview(name: &str, archive: &mut Archive<'_>, why: String) -> Result<ImportResult> {
    let png = match archive.merged_image() {
        Ok(Some(png)) => png,
        Ok(None) => return Err(invalid(format!("{why}, and the file has no merged image"))),
        Err(e) => return Err(invalid(format!("{why}, and the merged image can't be read: {e}"))),
    };
    let mut r = crate::flat::import_flat(name, &png)?;
    r.document.name = name.to_string();
    r.warnings.insert(0, format!("{why}; the file opens as Krita's merged image, without its layers"));
    r.source_read_only = true;
    r.preview_only = true;
    Ok(r)
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    let mut archive = Archive::open(bytes).map_err(kra_err)?;
    let image = archive.image().clone();
    let Some(space) = Space::from_name(&image.colorspace_name) else {
        return preview(name, &mut archive, format!("Krita's {} colour space is not supported yet", image.colorspace_name));
    };
    let (mode, depth) = space.model();
    let mut reader = Reader { archive, ctl, fmt: PixelFormat::new(mode, depth, true), nodes: 0, budget: MAX_PIXEL_BYTES, warnings: Warnings::default() };
    let layers = reader.layers(&image.layers, 1, 0, 0)?;
    ctl.progress(0.9);
    let mut document = Document::new(name, Size::new(image.width, image.height), mode, depth);
    document.layers = layers;
    if let Some(profile) = reader.archive.icc_profile().map_err(kra_err)? {
        document.icc_profile = Some(std::sync::Arc::new(profile));
    }
    let (xres, yres) = (image.x_res, image.y_res);
    if xres > 0.0 && xres < 1.0e6 {
        document.resolution_dpi = xres as f32;
        if let Some(w) = crate::unequal_resolution_warning(xres, yres) {
            reader.warnings.note(&w);
        }
    }
    if document.layers.is_empty() {
        reader.warnings.note("no layers of the Krita document could be imported");
    }
    if image.assistants || image.reference_images {
        reader.warnings.note("drawing assistants and reference images were left out");
    }
    Ok(ImportResult { document, warnings: reader.warnings.finish(), source_read_only: true, preview_only: false })
}
