//! CPU reference compositor.
//!
//! Flattens a [`Document`] layer tree into straight-alpha RGBA (f32) for any rectangle. It encodes
//! Photoshop layer semantics in one place:
//! - blend modes, opacity × fill opacity, visibility
//! - layer masks with density
//! - clipping groups (clipped layers composite *atop* their base)
//! - pass-through vs isolated groups
//! - adjustment layers (applied to the composite below)
//! - fill layers, Dissolve
//!
//! This is the reference the GPU backend (milestone M5) must match within 1/255. Compositing
//! currently happens in display RGB. CMYK layers are read through the document's CMYK profile
//! (the built-in coated CMYK when untagged) and composited in sRGB; mode-native (CMYK/Lab)
//! compositing arrives with ICC in M8.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod adjust;
pub mod advanced;
pub mod bounds;
pub mod cmyk;
pub mod effects;
pub mod fill_layout;
pub mod filter_mask;
pub mod gradient_fill;
pub mod lut_rows;
pub mod masks;
pub mod multichannel;
pub mod native;
pub mod native_lab;
pub mod pattern;
pub mod proxy;
pub mod psblend;
pub mod shape_split;
pub mod smart_mix;
mod uniform;
pub mod viewport;

use photocraft_color::blend::BlendMode;
use photocraft_doc::{Document, Fill, Layer, LayerContent, Pattern};
use photocraft_geom::Rect;
use photocraft_raster::{Rgba8Image, Surface};
use psblend as blend;

pub use advanced::advanced_active;

mod composite;
mod content;
mod ctx;
mod extents;
mod fx_cache;
mod reduced;
mod render;

pub use composite::{adjustment_quantum, blend_if_active, occludes_below, text_gamma};
#[cfg(test)]
use composite::{blend_into_fill, blend_into_g, branchless_floor, composite_layer_plain, composite_pixel_layer, pixel_surface, quantize};
use composite::{composite_stack_at, dissolve_noise};
use content::empty_in;
#[cfg(test)]
use content::render_content;
#[cfg(test)]
use content::render_fill;
pub use content::{RENDER_TILE, channel_weights, fill_frame, layer_shape, render_fill_content, render_fill_content_in, stroke_frame, surface_to_buffer};
use ctx::{Ctx, prepare_effects};
#[cfg(test)]
use ctx::{LayerKeys, PREPARE_DEPTH, effect_layers, prepare_tones};
use extents::mask_vals;
pub use extents::{
    Ancestors, Placement, adjustment_reach, change_bounds, change_bounds_in, composite_bounds, effect_outline, effect_source_bounds, effects_frame,
    hard_shape_bounds, layer_bounds, layer_change_bounds, paint_bounds, relayout_damage, shape_bounds, siblings_of, soft_shape_frame, transparent_outside,
};
#[cfg(test)]
use extents::{mask_k, shapeless_stroke_bounds, trim_to_painted};
pub use fx_cache::{
    DEFAULT_FX_CACHE_BUDGET, discard_unreadable_pixel_caches, effect_cache_budget, effect_cache_bytes, purge_effect_cache, set_effect_cache_budget,
};
#[cfg(test)]
use fx_cache::{effect_maps, effect_region, fx_region_bytes};
pub use reduced::{
    PROXY_THUMBNAIL_PIXELS, render_reduced, render_reduced_by, render_reduced_damage, render_reduced_rect, thumbnail, thumbnail_buffer, thumbnail_proxy,
    thumbnail_size,
};
#[cfg(test)]
use reduced::{ReducedStats, render_reduced_in_bands, render_reduced_with};
use render::in_blend_space;
#[cfg(test)]
use render::render_tiled_with;
pub(crate) use render::visible_layers;
pub use render::{
    BAND_PIXELS, MAX_RENDER_PIXELS, RenderBandsError, band_rows_for, cmyk_space, composite_to_surface, composite_to_surface_with_interrupt, flatten,
    flatten_to_surface, render, render_bands, render_layer, render_layer_in, render_tiled, try_render_bands,
};

/// Straight-alpha RGBA float buffer covering a rectangle.
///
/// The pixel storage comes from, and goes back to, the scratch pool.
#[derive(Debug, PartialEq)]
pub struct Buffer {
    pub rect: Rect,
    pub px: Vec<[f32; 4]>,
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        let mut px = photocraft_raster::scratch::take(self.px.len(), [0.0; 4]);
        px.copy_from_slice(&self.px);
        Self { rect: self.rect, px }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        photocraft_raster::scratch::give(std::mem::take(&mut self.px));
    }
}

impl Buffer {
    pub fn transparent(rect: Rect) -> Self {
        Self::filled(rect, [0.0; 4])
    }
    pub fn filled(rect: Rect, c: [f32; 4]) -> Self {
        Self { rect, px: photocraft_raster::scratch::take(rect.width() as usize * rect.height() as usize, c) }
    }
    /// Fallibly allocate a transparent RGBA buffer, preserving `filled`'s scratch-pool slack.
    pub fn try_transparent(rect: Rect) -> Result<Self, photocraft_raster::AllocationError> {
        Self::try_filled(rect, [0.0; 4])
    }
    /// Fallibly allocate and fill an RGBA buffer.
    pub fn try_filled(rect: Rect, c: [f32; 4]) -> Result<Self, photocraft_raster::AllocationError> {
        let width = usize::try_from(rect.width()).map_err(|_| photocraft_raster::AllocationError::SizeOverflow)?;
        let height = usize::try_from(rect.height()).map_err(|_| photocraft_raster::AllocationError::SizeOverflow)?;
        let len = width.checked_mul(height).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let bytes = len.checked_mul(std::mem::size_of::<[f32; 4]>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let px = photocraft_raster::scratch::try_take(len, c).ok_or(photocraft_raster::AllocationError::NotEnoughMemory { bytes })?;
        Ok(Self { rect, px })
    }
    /// Fallibly clone the pixel storage while keeping the same rectangle.
    pub fn try_clone(&self) -> Result<Self, photocraft_raster::AllocationError> {
        let bytes = self.px.len().checked_mul(std::mem::size_of::<[f32; 4]>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let mut px = photocraft_raster::scratch::try_take(self.px.len(), [0.0; 4]).ok_or(photocraft_raster::AllocationError::NotEnoughMemory { bytes })?;
        px.copy_from_slice(&self.px);
        Ok(Self { rect: self.rect, px })
    }
    /// The pixels, taken out of the buffer (it can't be destructured: it returns them to the
    /// pool on drop).
    pub fn into_px(mut self) -> Vec<[f32; 4]> {
        std::mem::take(&mut self.px)
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [f32; 4] {
        self.px[((y - self.rect.y0) as usize) * self.rect.width() as usize + (x - self.rect.x0) as usize]
    }
    pub fn to_rgba8(&self) -> Rgba8Image {
        let mut img = Rgba8Image::new(self.rect.width(), self.rect.height());
        let quantize = |out: &mut [[u8; 4]], src: &[[f32; 4]]| {
            for (o, p) in out.iter_mut().zip(src) {
                for (dst, v) in o.iter_mut().zip(p) {
                    *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        };
        let out = img.pixels.as_chunks_mut::<4>().0;
        // In parallel off wasm: a 24 MP composite took 120 ms on one core (the CPU canvas,
        // previews, thumbnails and 8-bit exports all come through here).
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            out.par_chunks_mut(16 * 1024).zip(self.px.par_chunks(16 * 1024)).for_each(|(o, p)| quantize(o, p));
        }
        #[cfg(target_arch = "wasm32")]
        quantize(out, &self.px);
        img
    }
    /// Flatten over an opaque background colour.
    pub fn over_background(&self, bg: [f32; 3]) -> Buffer {
        let mut out = self.clone();
        for p in &mut out.px {
            let a = p[3];
            for i in 0..3 {
                p[i] = p[i] * a + bg[i] * (1.0 - a);
            }
            p[3] = 1.0;
        }
        out
    }
}

/// Selects infallible compatibility behavior or propagating buffer allocation errors in the
/// shared compositor core.
pub(crate) trait BufferAlloc {
    type Error: Send;
    fn filled(&self, rect: Rect, color: [f32; 4]) -> Result<Buffer, Self::Error>;
    fn clone(&self, buffer: &Buffer) -> Result<Buffer, Self::Error>;
    fn scalar_filled(&self, rect: Rect, value: f32) -> Result<Vec<f32>, Self::Error>;
}

pub(crate) struct InfallibleBufferAlloc;
impl BufferAlloc for InfallibleBufferAlloc {
    type Error = std::convert::Infallible;
    fn filled(&self, rect: Rect, color: [f32; 4]) -> Result<Buffer, Self::Error> {
        Ok(Buffer::filled(rect, color))
    }
    fn clone(&self, buffer: &Buffer) -> Result<Buffer, Self::Error> {
        Ok(buffer.clone())
    }
    fn scalar_filled(&self, rect: Rect, value: f32) -> Result<Vec<f32>, Self::Error> {
        Ok(photocraft_raster::scratch::take(rect.width() as usize * rect.height() as usize, value))
    }
}

pub(crate) struct FallibleBufferAlloc;
impl BufferAlloc for FallibleBufferAlloc {
    type Error = photocraft_raster::AllocationError;
    fn filled(&self, rect: Rect, color: [f32; 4]) -> Result<Buffer, Self::Error> {
        Buffer::try_filled(rect, color)
    }
    fn clone(&self, buffer: &Buffer) -> Result<Buffer, Self::Error> {
        buffer.try_clone()
    }
    fn scalar_filled(&self, rect: Rect, value: f32) -> Result<Vec<f32>, Self::Error> {
        let width = usize::try_from(rect.width()).map_err(|_| photocraft_raster::AllocationError::SizeOverflow)?;
        let height = usize::try_from(rect.height()).map_err(|_| photocraft_raster::AllocationError::SizeOverflow)?;
        let len = width.checked_mul(height).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        let bytes = len.checked_mul(std::mem::size_of::<f32>()).ok_or(photocraft_raster::AllocationError::SizeOverflow)?;
        photocraft_raster::scratch::try_take(len, value).ok_or(photocraft_raster::AllocationError::NotEnoughMemory { bytes })
    }
}

#[cfg(test)]
mod soft_shape_tests;
#[cfg(test)]
mod stroke_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod advanced_tests;
