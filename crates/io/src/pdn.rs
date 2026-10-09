//! Paint.NET documents (`.pdn`): layers, blend modes, opacities, and BGRA/RGBA pixels.

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

use crate::{ImportResult, IoError};

pub use photocraft_pdn::is_pdn;

/// Extensions recognized as Paint.NET documents.
pub const EXTENSIONS: &[&str] = &["pdn"];

pub fn has_extension(name: &str) -> bool {
    name.rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .is_some_and(|(_, ext)| EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    let pdn_doc = photocraft_pdn::read(bytes, photocraft_pdn::Limits::default())
        .map_err(|e| IoError::Unsupported(format!("PDN: {e}")))?;

    let size = Size::new(pdn_doc.width, pdn_doc.height);
    let mut doc = Document::new(name, size, ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 96.0;

    let mut warnings = pdn_doc.warnings.clone();

    for (index, pdn_layer) in pdn_doc.layers.into_iter().enumerate() {
        let (blend_mode, warning) = map_blend_mode(pdn_layer.blend_mode);
        if let Some(w) = warning
            && !warnings.contains(&w)
        {
            warnings.push(w);
        }

        let rect = Rect::new(0, 0, pdn_layer.width as i32, pdn_layer.height as i32);
        let mut surface = Surface::from_interleaved(PixelFormat::RGBA8, rect, &pdn_layer.rgba_pixels);
        surface.prune();

        let mut layer = Layer::new(pdn_layer.name, LayerContent::Raster(surface));
        layer.visible = pdn_layer.visible;
        layer.opacity = (f32::from(pdn_layer.opacity) / 255.0).clamp(0.0, 1.0);
        layer.blend = blend_mode;

        if index == 0 && pdn_layer.is_background {
            layer.locks.transparency = true;
        }

        doc.layers.push(layer);
    }

    Ok(ImportResult {
        document: doc,
        warnings,
        source_read_only: true,
        preview_only: false,
    })
}

fn map_blend_mode(mode: photocraft_pdn::BlendMode) -> (BlendMode, Option<String>) {
    match mode {
        photocraft_pdn::BlendMode::Normal => (BlendMode::Normal, None),
        photocraft_pdn::BlendMode::Multiply => (BlendMode::Multiply, None),
        photocraft_pdn::BlendMode::Additive => (BlendMode::LinearDodge, None),
        photocraft_pdn::BlendMode::ColorBurn => (BlendMode::ColorBurn, None),
        photocraft_pdn::BlendMode::ColorDodge => (BlendMode::ColorDodge, None),
        photocraft_pdn::BlendMode::Reflect => (
            BlendMode::ColorDodge,
            Some("PDN: Reflect blend mode approximated as Color Dodge".into()),
        ),
        photocraft_pdn::BlendMode::Glow => (
            BlendMode::ColorDodge,
            Some("PDN: Glow blend mode approximated as Color Dodge".into()),
        ),
        photocraft_pdn::BlendMode::Overlay => (BlendMode::Overlay, None),
        photocraft_pdn::BlendMode::Difference => (BlendMode::Difference, None),
        photocraft_pdn::BlendMode::Negation => (
            BlendMode::Exclusion,
            Some("PDN: Negation blend mode approximated as Exclusion".into()),
        ),
        photocraft_pdn::BlendMode::Lighten => (BlendMode::Lighten, None),
        photocraft_pdn::BlendMode::Darken => (BlendMode::Darken, None),
        photocraft_pdn::BlendMode::Screen => (BlendMode::Screen, None),
        photocraft_pdn::BlendMode::Xor => (
            BlendMode::Difference,
            Some("PDN: XOR blend mode approximated as Difference".into()),
        ),
    }
}
