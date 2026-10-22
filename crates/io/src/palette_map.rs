//! Indexed Color and Bitmap documents -> PSD colour modes 2 and 0.
//!
//! Neither mode has layers in Photoshop, so the file holds only the merged image (the document
//! flattened): for Indexed Color one plane of palette indices, with the Color Table as Color Mode
//! Data (768 bytes: 256 reds, 256 greens, 256 blues), its length in resource 1046 and the
//! transparent entry in resource 1047; for Bitmap one plane of 1 bit per pixel, 1 = black.

use photocraft_color::ColorMode;
use photocraft_doc::{ColorTable, Document};
use photocraft_psd::resources::{ImageResource, ResolutionInfo, ids};
use photocraft_psd::{ColorMode as PsdMode, Compression, Header, ImageData, PsdFile};

/// Indexed Color Table Count: the number of colours in the table (u16).
pub(crate) const COLOR_TABLE_COUNT: u16 = 1046;
/// Transparency Index: the colour table entry shown as transparent (u16).
pub(crate) const TRANSPARENCY_INDEX: u16 = 1047;
/// Channel resources (names, display info, alpha identifiers): these modes have no extra channels.
const CHANNEL_RESOURCES: [u16; 4] = [1006, 1045, 1053, crate::channel_map::DISPLAY_INFO];

/// The PSD for an Indexed Color document with a colour table of at most 256 entries, one of them
/// opaque, or a Bitmap document; None for any other document (it is written like the others). A
/// document with alpha channels or a Quick Mask is written like the others too: the importer does
/// not read the extra channels of these modes back, so they would be lost.
pub(crate) fn document_to_psd(doc: &Document, force_psb: bool) -> Option<(PsdFile, Vec<String>)> {
    if !doc.channels.is_empty() || doc.quick_mask.is_some() {
        return None;
    }
    let table = doc.color_table.as_ref().filter(|t| t.colors.len() <= 256 && (0..t.colors.len()).any(|i| Some(i) != t.transparent.map(usize::from)));
    let (mode, plane, color_mode_data) = match doc.mode {
        ColorMode::Indexed => {
            let table = table?;
            (PsdMode::Indexed, indices(doc, table), palette(table))
        }
        ColorMode::Bitmap => (PsdMode::Bitmap, bits(doc), Vec::new()),
        _ => return None,
    };
    let mut warnings = Vec::new();
    if !flattens_losslessly(doc) {
        warnings.push(format!("{:?} documents have no layers in Photoshop: written flattened", doc.mode));
    }
    let big = doc.size.width > 30_000 || doc.size.height > 30_000;
    let size_exceeds_limit = !force_psb && !big && crate::psd_export::psd_size_exceeds_limit(doc);
    let version = crate::psd_export::psd_version(doc, force_psb, size_exceeds_limit);
    let depth = if mode == PsdMode::Bitmap { 1 } else { 8 };
    let header = Header::new(version, doc.size.width, doc.size.height, 1, depth, mode);
    let image_data = ImageData::encode(Compression::Rle, &plane, &header)
        .or_else(|_| ImageData::encode(Compression::Raw, &plane, &header))
        .unwrap_or(ImageData { compression: Compression::Raw, data: plane });
    let mut resources = vec![
        ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(f64::from(doc.resolution_dpi)).to_bytes()),
        ImageResource::new(ids::GLOBAL_ANGLE, (doc.global_light.angle.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(ids::GLOBAL_ALTITUDE, (doc.global_light.altitude.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(1032, crate::psd_export::guides_resource(doc)),
    ];
    if let (PsdMode::Indexed, Some(t)) = (mode, table) {
        resources.push(ImageResource::new(COLOR_TABLE_COUNT, (t.colors.len() as u16).to_be_bytes().to_vec()));
        if let Some(i) = t.transparent {
            resources.push(ImageResource::new(TRANSPARENCY_INDEX, u16::from(i).to_be_bytes().to_vec()));
        }
        // Photoshop keeps the RGB profile of an Indexed document (Bitmap has none). A profile of
        // another colour space (kept from a Grayscale or CMYK source) does not describe the table.
        if let Some(icc) = doc.icc_profile.as_ref().filter(|icc| icc.get(16..20) == Some(b"RGB ".as_slice())) {
            resources.push(ImageResource::new(ids::ICC_PROFILE, icc.to_vec()));
        }
    }
    // An imported transparency index that no longer applies is not copied either.
    let keep = |id: u16, _: &[u8]| !CHANNEL_RESOURCES.contains(&id) && id != TRANSPARENCY_INDEX && id != COLOR_TABLE_COUNT;
    let file = crate::multichannel_map::layerless_file(doc, header, image_data, color_mode_data, resources, keep, &mut warnings);
    Some((file, warnings))
}

/// Whether the merged image holds everything the layers do: no layer, or a single visible pixel
/// layer at full opacity, Normal blend, unclipped, without masks or layer effects. Anything else
/// (more layers, a type, shape or smart object layer, a mask, a hidden layer) is lost when the
/// document is written flattened.
fn flattens_losslessly(doc: &Document) -> bool {
    match doc.layers.as_slice() {
        [] => true,
        [l] => {
            l.visible
                && matches!(l.content, photocraft_doc::LayerContent::Raster(_))
                && l.mask.is_none()
                && l.vector_mask.is_none()
                && !l.clipped
                && l.opacity >= 1.0
                && l.fill_opacity >= 1.0
                && l.blend == photocraft_color::BlendMode::Normal
                && !(l.effects.enabled && l.effects.items.iter().any(photocraft_doc::Effect::enabled))
        }
        _ => false,
    }
}

/// The Color Table as Color Mode Data: 256 reds, 256 greens, 256 blues (unused entries black).
fn palette(t: &ColorTable) -> Vec<u8> {
    let mut m = vec![0u8; 768];
    for (i, c) in t.colors.iter().take(256).enumerate() {
        m[i] = c[0];
        m[256 + i] = c[1];
        m[512 + i] = c[2];
    }
    m
}

/// The flattened document as palette indices: the transparent entry where the document is less
/// than half covered, else the nearest other entry (the transparent one may duplicate an opaque
/// colour, as in many PNG-8 files). Without a transparent entry the document is flattened over
/// white, as Photoshop does.
fn indices(doc: &Document, t: &ColorTable) -> Vec<u8> {
    let (opaque_indices, colors): (Vec<u8>, Vec<_>) =
        t.colors.iter().copied().enumerate().filter(|(i, _)| Some(*i) != t.transparent.map(usize::from)).map(|(i, c)| (i as u8, c)).unzip();
    let opaque = ColorTable { colors, transparent: None };
    let mut idx = Vec::with_capacity(doc.size.area() as usize);
    let _ = photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
        idx.extend(band.px.iter().map(|p| match t.transparent {
            Some(i) if p[3] < 0.5 => i,
            Some(_) => opaque_indices.get(opaque.nearest([p[0], p[1], p[2]])).copied().unwrap_or(0),
            None => {
                let over_white = |c: f32| c * p[3] + (1.0 - p[3]);
                t.nearest([over_white(p[0]), over_white(p[1]), over_white(p[2])]) as u8
            }
        }));
        Ok(())
    });
    idx
}

/// The flattened document over white as 1-bit rows (padded to a byte), 1 = black: pixels darker
/// than middle grey.
fn bits(doc: &Document) -> Vec<u8> {
    let w = doc.size.width as usize;
    let row = w.div_ceil(8);
    let mut out = Vec::with_capacity(row * doc.size.height as usize);
    let _ = photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
        for line in band.px.chunks(w.max(1)) {
            let mut bytes = vec![0u8; row];
            for (x, p) in line.iter().enumerate() {
                let grey = (p[0] + p[1] + p[2]) / 3.0 * p[3] + (1.0 - p[3]);
                if grey < 0.5 {
                    bytes[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.extend(bytes);
        }
        Ok(())
    });
    out
}
