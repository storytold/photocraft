//! # photocraft-io
//!
//! Import and export between [`photocraft_doc::Document`] and files:
//!
//! * PSD / PSB (detected by the `8BPS` signature) via `photocraft-psd`, with
//!   fidelity levels 1–3 (raster layers, masks, blend/opacity/fill,
//!   visibility, names, groups, clipping, adjustment and fill layers).
//!   Every layer keeps its unmodelled tagged blocks in `Layer::psd_blocks`
//!   (vector masks, blending options, original adjustment/fill blocks, text
//!   and smart-object data) and re-export writes them back; text, shape and
//!   smart-object layers also keep their pixels as the cached raster, and fill
//!   layers keep Photoshop's rendering in `Layer::fill_cache`.
//! * Camera raws (DNG, CR2, uncompressed / lossless TIFF-EP raws) via
//!   `photocraft-raw`, developed into a 16-bit ProPhoto RGB "Background"
//!   layer; unsupported raw variants fall back to the embedded JPEG preview.
//! * Layered TIFFs (Photoshop layer data in tags 37724 and 34377) open with their
//!   layers through the PSD path and are written back the same way; see `tiff_layers`.
//! * Paint.NET PDN3 documents open as editable bitmap layers (import only).
//! * OpenRaster (`.ora`) documents open and save with their layers, groups, blend modes,
//!   opacity and visibility; see `ora`.
//! * Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`) open natively
//!   with no source save path, what isn't imported listed in the warnings; a file
//!   whose native data can't be read opens as its embedded preview; see `affinity`.
//! * Every other format goes through `photocraft-codecs` as a single layer
//!   (depth and Gray/RGB/CMYK model preserved): a locked "Background" when the
//!   image is opaque, a normal "Layer 0" when it has transparency.
//!
//! Exports to PSD render the merged composite with
//! `photocraft_compose::flatten`; flat exports report what is lost.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod abr_map;
mod adjust_map;
pub mod affinity;
pub mod annotations_map;
pub mod blocks;
mod channel_map;
pub mod comps_map;
pub mod effects_map;
mod flat;
mod gradient_bake;
pub mod linked;
mod multichannel_map;
mod ora;
pub mod pattern_map;
mod pdn;
pub mod pixels;
mod psd_export;
mod psd_import;
pub mod raw;
pub mod slices_map;
pub mod smart_map;
pub mod svg;
mod text_import;
pub mod text_styles_map;
pub mod tiff_layers;
pub mod vector_map;

use photocraft_codecs::{CodecError, EncodeOptions};
use photocraft_doc::Document;
use photocraft_psd::{PsdError, PsdFile};

pub use adjust_map::ADJUSTMENT_KEYS;
pub use flat::{document_to_image, import_tiff_page};
pub use psd_export::{PsdExportOptions, document_to_psd, document_to_psd_with};
pub use psd_import::{psd_to_document, psd_to_document_with};

/// Errors from import/export.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// Paint.NET document decode failure.
    #[error("PDN: {0}")]
    Pdn(String),
    /// OpenRaster read failure.
    #[error("OpenRaster: {0}")]
    Ora(String),
    /// PSD parse/write failure.
    #[error("PSD: {0}")]
    Psd(#[from] PsdError),
    /// Codec failure.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    /// The file type could not be determined.
    #[error("unknown file format: {0}")]
    UnknownFormat(String),
    /// The requested conversion is not possible.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Native `.pcraft` bundle failure.
    #[error("pcraft: {0}")]
    Pcraft(#[from] photocraft_format::FormatError),
    /// Camera raw decode failure.
    #[error("{0}")]
    Raw(#[from] photocraft_raw::RawError),
    /// An SVG that does not parse (or is too large to rasterise).
    #[error("SVG: {0}")]
    Svg(String),
    /// A background import was cancelled ([`import_with`]).
    #[error("cancelled")]
    Cancelled,
}

/// Result of [`import`].
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// The imported document.
    pub document: Document,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
    /// Save must not write back to an import-only source, such as Affinity or Paint.NET.
    pub source_read_only: bool,
    /// Only a stand-in picture of the file (an Affinity document whose native data couldn't be
    /// read): Open shows it with its warning; Place and other auxiliary imports refuse it.
    pub preview_only: bool,
}

/// The import note for a file whose horizontal and vertical resolutions (`x`, `y`, in pixels per
/// inch) differ: a document has one resolution, so it keeps the horizontal one and saves it on
/// both axes (#1018). Differences under 0.01 ppi are below what the warning can show.
pub(crate) fn unequal_resolution_warning(x: f64, y: f64) -> Option<String> {
    ((x - y).abs() >= 0.01)
        .then(|| format!("the vertical resolution ({y:.2} ppi) differs from the horizontal ({x:.2} ppi); the document keeps {x:.2} ppi for both"))
}

/// Result of [`export`].
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Encoded file.
    pub bytes: Vec<u8>,
    /// Human-readable notes about anything approximated or dropped.
    pub warnings: Vec<String>,
}

/// Which part of the document's XMP packet and free-form text a flat export embeds.
/// PSD/PSB always keep XMP; `.pcraft` keeps both. Save As and conversions keep the whole packet, as
/// Photoshop's Save As does; Export As starts at `None`, because the packet lists the text of
/// every type layer and one id per placed document (#647).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XmpEmbed {
    /// Embed the document's whole XMP packet and free-form text.
    #[default]
    All,
    /// Embed no XMP or free-form text.
    None,
}

/// Export options.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Codec options for flat formats.
    pub encode: EncodeOptions,
    /// Write PSB even for `.psd` names when the document is small.
    pub force_psb: bool,
    /// TIFF: keep the layers (Photoshop layer data in tag 37724). Off by default, so scripted
    /// and agent saves (CLI, batch, MCP) write a flat TIFF unless they ask for layers; the app's
    /// Save As sets it from its Layers option, which keeps them as Photoshop does. `false` is
    /// Photoshop's "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Which part of the document's XMP packet and free-form text a flat export embeds.
    /// PSD/PSB always keep XMP; `.pcraft` keeps both. Everything by default, as Save As does;
    /// Export As offers None.
    pub xmp: XmpEmbed,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { encode: EncodeOptions::default(), force_psb: false, tiff_layers: false, xmp: XmpEmbed::All }
    }
}

/// `true` if `bytes` start with the PSD/PSB signature.
pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS")
}

/// Imports a file. PSD/PSB, Affinity containers and camera raws are detected by magic; everything
/// else is decoded with `photocraft-codecs`.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth(name, bytes, photocraft_doc::MAX_GROUP_DEPTH)
}

/// [`import`] for a background open: checks `ctl` between stages (and per layer for PSD/PSB) and
/// reports progress. A cancelled import fails with [`IoError::Cancelled`].
pub fn import_with(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, photocraft_doc::MAX_GROUP_DEPTH, ctl)
}

/// Imports a file with a configurable SVG editability threshold. SVG groups deeper than this are
/// rasterised during conversion; the XML parser safety cap and document nesting limit stay fixed.
pub fn import_with_svg_group_depth(name: &str, bytes: &[u8], max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_with_svg_group_depth_and_interrupt(name, bytes, max_group_depth, &photocraft_raster::Interrupt::NONE)
}

/// [`import_with`] with the SVG editable-group threshold selected by the user.
pub fn import_with_svg_group_depth_and_interrupt(
    name: &str,
    bytes: &[u8],
    max_group_depth: usize,
    ctl: &photocraft_raster::Interrupt,
) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let r = import_stages(name, bytes, ctl, max_group_depth)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok(r)
}

fn import_stages(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt, max_group_depth: usize) -> Result<ImportResult, IoError> {
    // A declared native extension must reach its loader so malformed bundles retain format errors.
    if has_extension(name, photocraft_format::EXTENSION) || photocraft_format::is_pcraft(bytes) {
        return Ok(ImportResult { document: photocraft_format::load_from_bytes(bytes)?, warnings: Vec::new(), source_read_only: false, preview_only: false });
    }
    if is_psd(bytes) {
        let file = PsdFile::from_bytes(bytes)?;
        // Nesting past the document model's cap could never be saved (.pcraft refuses it) and
        // would overflow the importer's recursion; reject the file with the actionable limit.
        if psd_import::group_depth(&file) > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(IoError::Unsupported(format!("layer groups nested deeper than {}", photocraft_doc::MAX_GROUP_DEPTH)));
        }
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(0.05);
        let (mut document, warnings) = psd_import::psd_to_document_with(&file, ctl).ok_or(IoError::Cancelled)?;
        document.name = name.to_string();
        text_import::prepare(&mut document);
        return Ok(ImportResult { document, warnings, source_read_only: false, preview_only: false });
    }
    if affinity::is_affinity(bytes) || affinity::has_extension(name) {
        return affinity::import(name, bytes);
    }
    if ora::is_ora(bytes) || has_extension(name, "ora") {
        return ora::import(name, bytes, ctl);
    }
    if has_extension(name, "pdn") || bytes.starts_with(b"PDN3") {
        return pdn::import(name, bytes, ctl);
    }
    if raw::is_raw(bytes) {
        return raw::import_raw(name, bytes);
    }
    if is_svg_input(name, bytes) {
        return svg::import_svg_with_group_depth(name, bytes, max_group_depth);
    }
    flat::import_flat(name, bytes)
}

/// Extension names can be wrong (for example, an image downloaded as WebP and renamed `.svg`).
/// Valid plain SVG is recognized by its XML content; an otherwise unknown `.svg` is passed to the
/// SVG parser for a useful malformed-file error, and `.svgz` disambiguates gzip streams.
fn is_svg_input(name: &str, bytes: &[u8]) -> bool {
    // Keep the SVG extension as a fallback for malformed files so the SVG parser can return an
    // actionable SVG error. A recognized raster signature still wins over a misleading `.svg`
    // suffix (for example, a WebP downloaded as Firefox_logo.svg).
    svg::is_svg(bytes)
        || (has_extension(name, "svg") && photocraft_codecs::detect(bytes).is_none())
        || (has_extension(name, "svgz") && bytes.starts_with(&[0x1f, 0x8b]))
}

fn extension(name_or_ext: &str) -> String {
    name_or_ext.rsplit(['.', '/', '\\']).next().unwrap_or(name_or_ext).to_ascii_lowercase()
}

fn has_extension(name: &str, expected: &str) -> bool {
    name.rsplit(['/', '\\']).next().and_then(|name| name.rsplit_once('.')).is_some_and(|(base, ext)| !base.is_empty() && ext.eq_ignore_ascii_case(expected))
}

/// Exports `doc` to the format named by `name_or_ext` (a file name, path or
/// bare extension).
pub fn export(doc: &Document, name_or_ext: &str, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let ext = extension(name_or_ext);
    if ext == "pdn" {
        return Err(IoError::Unsupported("PDN is import-only; save as .pcraft to preserve all layers and blend modes".into()));
    }
    if affinity::EXTENSIONS.contains(&ext.as_str()) {
        return Err(IoError::Unsupported("Affinity export is not implemented; save a new PSD, PNG or .pcraft copy".into()));
    }
    if ext == "ora" {
        return ora::export(doc);
    }
    if ext == photocraft_format::EXTENSION {
        let previews = photocraft_format::SaveOptions {
            thumbnail: Some(photocraft_compose::thumbnail(doc, 256)),
            composite: Some(photocraft_compose::thumbnail(doc, 1024)),
        };
        return Ok(ExportResult { bytes: photocraft_format::save_to_bytes(doc, &previews)?, warnings: Vec::new() });
    }
    if ext == "psd" || ext == "psb" {
        let o = PsdExportOptions { force_psb: opts.force_psb || ext == "psb", ..Default::default() };
        let (mut file, mut warnings) = document_to_psd_with(doc, &o);
        if !doc.metadata.text.is_empty() {
            warnings.push("free-form text metadata is not supported by PSD/PSB; it will be dropped".into());
        }
        warnings.extend(tiff_layers::strip_foreign_order_blocks(&mut file));
        // Never write a header the reader would refuse (e.g. a zero-sized canvas).
        file.header.validate()?;
        let bytes = file.to_bytes()?;
        return Ok(ExportResult { bytes, warnings });
    }
    let format = photocraft_codecs::from_extension(&ext).ok_or_else(|| IoError::UnknownFormat(name_or_ext.to_string()))?;
    if format == photocraft_codecs::Format::Tiff && opts.tiff_layers && tiff_layers::would_write_layers(doc) {
        return tiff_layers::export_layered(doc, opts);
    }
    flat::export_flat(doc, format, opts)
}

/// The PSD's merged composite as straight RGBA floats (row-major), converted
/// with the document pixel model and un-matted from white (Photoshop mattes
/// the merged image of transparent documents). Used as the compositing oracle.
pub fn merged_composite(file: &PsdFile) -> Result<Vec<[f32; 4]>, IoError> {
    let h = &file.header;
    // The RGBA8 preview clips signed/HDR values. Decode float merged images at their
    // original depth through the model path below; they are the rendering oracle.
    let img = if h.depth == 32 { None } else { file.composite_rgba8().ok() };
    // CMYK goes through the colour-managed model conversion (the PSD crate's RGBA preview is a
    // naive, profile-free conversion).
    if let (Some(img), false) = (img, matches!(h.color_mode, photocraft_psd::ColorMode::Lab | photocraft_psd::ColorMode::Cmyk)) {
        let unmatte = file.merged_has_alpha();
        return Ok(img
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let mut v: [f32; 4] = std::array::from_fn(|i| f32::from(p[i]) / 255.0);
                if unmatte {
                    for c in 0..3 {
                        v[c] = pixels::unmatte(v[c], v[3], 1.0).clamp(0.0, 1.0);
                    }
                }
                v
            })
            .collect());
    }
    // Generic path (Lab, CMYK and others) via the raster model conversion.
    // Keep the merged-alpha marker while discarding layers: otherwise an extra
    // spot/alpha channel in an opaque composite is mistaken for transparency.
    let layer_info = Some(photocraft_psd::LayerInfo { merged_alpha: file.merged_has_alpha(), layers: Vec::new(), padding: None });
    let (doc, _) = psd_to_document(&PsdFile { layer_info, ..file.clone() });
    // Multichannel documents keep their channels apart (no layer): composite them.
    if doc.layers.is_empty() && doc.mode == photocraft_color::ColorMode::Multichannel {
        return Ok(photocraft_compose::flatten(&doc).px);
    }
    let l = doc.layers.first().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    let s = l.surface().ok_or_else(|| IoError::Unsupported("no merged image".into()))?;
    Ok(photocraft_compose::surface_to_buffer(s, doc.bounds()).px)
}

#[cfg(test)]
mod tests {
    use super::is_svg_input;

    #[test]
    fn svg_extension_does_not_override_another_format_signature() {
        let webp_header = b"RIFF\x04\0\0\0WEBP";
        assert!(!is_svg_input("Firefox_logo,_2019.svg", webp_header));
        assert!(is_svg_input("drawing.svg", b"<?xml version=\"1.0\"?><svg/>"));
        assert!(is_svg_input("drawing.svgz", &[0x1f, 0x8b, 0, 0]));
        assert!(!is_svg_input("drawing.svgz", webp_header));
    }
}
