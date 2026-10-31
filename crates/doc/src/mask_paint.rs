//! Editable effects composited into a layer mask. The flattened surface remains
//! available to existing compositors and exports.
use crate::LayerMask;
use photocraft_color::BlendMode;
use photocraft_raster::Surface;

/// Compact blend-mode choices exposed by mask Paint and Fill effects.
/// These modes have simple, useful scalar grayscale behavior; color-component modes are excluded.
pub const MASK_BLEND_MODES: [BlendMode; 8] = [
    BlendMode::Normal,
    BlendMode::Darken,
    BlendMode::Multiply,
    BlendMode::Lighten,
    BlendMode::Screen,
    BlendMode::LinearDodge,
    BlendMode::Subtract,
    BlendMode::Difference,
];

pub fn is_mask_blend_mode(mode: BlendMode) -> bool {
    MASK_BLEND_MODES.contains(&mode)
}

/// Bound total accumulated Blur halo so tile previews and cache refreshes remain predictable.
pub const MAX_BLUR_RADIUS_SUM: u16 = 512;

/// Ordered editable effect in a mask stack. Fill and Levels keep their parameters
/// separate from the cached flattened mask, so they remain non-destructive.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    /// Scalar grayscale compositing mode. Commands expose [`MASK_BLEND_MODES`]; the native format
    /// also preserves older scalar blend values for compatibility.
    pub blend: BlendMode,
    /// Paint pixels. Effects keep an empty placeholder here for format/API compatibility.
    pub surface: Surface,
    /// Optional grayscale Fill input (a cropped selection or a library image).
    pub fill_map: Option<Surface>,
    /// Pixel size of `fill_map`; retained even when the map is entirely black.
    pub fill_map_size: [u32; 2],
    /// Destination rectangle in document pixels. `None` means the full canvas.
    pub fill_bounds: Option<[i32; 4]>,
    pub kind: EffectKind,
}

/// The operations supported by the mask stack.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectKind {
    /// Transparent grayscale paint pixels stored as RGBA for independent coverage.
    Paint,
    /// A constant grayscale value.
    Fill { value: f32, rotation: f32, repeat_u: bool, repeat_v: bool, mirror_u: bool, mirror_v: bool },
    /// Tonal remapping applied to the accumulated mask at this stack position.
    Levels { input_black: f32, gamma: f32, input_white: f32, output_black: f32, output_white: f32 },
    /// Gaussian blur applied to the accumulated grayscale mask at this stack position.
    Blur { radius: u8 },
}

impl Paint {
    /// Return an editable surface only for Paint layers.
    pub fn paint_surface(&self) -> Option<&Surface> {
        match &self.kind {
            EffectKind::Paint => Some(&self.surface),
            EffectKind::Fill { .. } | EffectKind::Levels { .. } | EffectKind::Blur { .. } => None,
        }
    }

    /// Return an editable surface only for Paint layers.
    pub fn paint_surface_mut(&mut self) -> Option<&mut Surface> {
        match &mut self.kind {
            EffectKind::Paint => Some(&mut self.surface),
            EffectKind::Fill { .. } | EffectKind::Levels { .. } | EffectKind::Blur { .. } => None,
        }
    }
}

/// Original mask plus ordered effects (bottom to top).
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub base: Surface,
    pub layers: Vec<Paint>,
    pub active: usize,
}

impl LayerMask {
    /// The selected brush layer, or the ordinary mask when no stack exists.
    pub fn paint_surface(&self) -> &Surface {
        self.paint.as_ref().and_then(|s| s.layers.get(s.active)).and_then(Paint::paint_surface).unwrap_or(&self.surface)
    }

    /// Mutable brush target. The caller refreshes the flattened mask after painting.
    pub fn paint_surface_mut(&mut self) -> &mut Surface {
        match self.paint.as_mut().and_then(|s| s.layers.get_mut(s.active)).and_then(Paint::paint_surface_mut) {
            Some(surface) => surface,
            None => &mut self.surface,
        }
    }

    /// Explicit brush target, which is absent when the selected item is Fill or Levels.
    pub fn active_paint_surface(&self) -> Option<&Surface> {
        let stack = self.paint.as_ref()?;
        stack.layers.get(stack.active)?.paint_surface()
    }

    /// Explicit mutable brush target, which is absent when selected item is an effect.
    pub fn active_paint_surface_mut(&mut self) -> Option<&mut Surface> {
        let stack = self.paint.as_mut()?;
        stack.layers.get_mut(stack.active)?.paint_surface_mut()
    }
}
