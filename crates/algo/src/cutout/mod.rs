//! Cutout (Filter Gallery › Artistic): the image as flat sheets of cut paper.
//!
//! The image is traced into shapes, and the shapes are filled back in:
//!
//! 1. **Smooth** within tones, edges kept: a bilateral filter (C. Tomasi and R. Manduchi,
//!    "Bilateral Filtering for Gray and Color Images", 1998) in Oklab, so a photo's noise doesn't
//!    decide pixel by pixel which of two colours a pixel gets.
//! 2. **Palette**: median cut (Heckbert 1982) then k-means over the smoothed colours, Number of
//!    Levels tones of the picture's own; a tone made of two clearly different hues splits in two,
//!    and colours only noise apart are merged.
//! 3. **Regions**: 4-connected areas of one palette colour. Regions smaller than the Edge
//!    Simplicity area, or thin slivers (the blends along an edge between two other colours), are
//!    merged smallest first into the neighbour they share most border with. Each region is filled
//!    with the mean colour of its own pixels.
//! 4. **Edges**: the cracks between regions are linked into chains from junction to junction
//!    (where three regions meet, or the canvas edge). Each crack is moved along its normal to where
//!    the pixels on either side say the edge is (an anti-aliased pixel is part one colour, part the
//!    other), so outlines are right to a fraction of a pixel. A chain is fitted once, so two
//!    neighbouring shapes share one and the same edge.
//! 5. **Curves**: turns sharper than Edge Fidelity allows are kept as corners (squared back where
//!    the pixels rounded them); the runs between are simplified to the Edge Simplicity tolerance
//!    (Douglas–Peucker) and drawn as Catmull-Rom curves.
//! 6. **Fill**: each region is scan-converted with exact area coverage, so edges are anti-aliased
//!    and the coverages of shapes that share an edge sum to one: no seams.
//!
//! Tracing reads the whole image and runs once; filling runs per tile and gives the same pixels
//! whatever the tiling.
//!
//! Each stage is its own module: [`colour`] (steps 1–2) so far.

// The stages land one at a time; the Filter Gallery uses them once the last one is in.
#![allow(dead_code)]

mod colour;
mod util;

type Px = [f32; 3];
type Pt = [f32; 2];

/// Region id of the area outside the image.
const OUT: u32 = u32::MAX;

/// The filter's three sliders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    /// Number of Levels (2–8): the most colours the palette has.
    pub levels: usize,
    /// Edge Simplicity (0–10): smaller shapes merge and outlines simplify more.
    pub simplicity: f32,
    /// Edge Fidelity (1–3): more corners are kept.
    pub fidelity: f32,
}

impl Settings {
    /// Regions with fewer pixels merge into a neighbour.
    fn min_area(&self) -> u32 {
        (1.0 + self.simplicity.clamp(0.0, 10.0)).powi(2).round() as u32
    }
    /// Regions thinner than this (twice the area over the perimeter, in pixels) merge too.
    fn sliver(&self) -> f32 {
        0.6 + 0.1 * self.simplicity.clamp(0.0, 10.0)
    }
    /// Douglas–Peucker tolerance of the outlines, in pixels: looser with less Edge Fidelity
    /// (twice as loose at 1, half at 3).
    fn tolerance(&self) -> f32 {
        (0.5 + 0.12 * self.simplicity.clamp(0.0, 10.0)) * 2f32.powf(2.0 - self.fidelity.clamp(1.0, 3.0))
    }
    /// Below Edge Fidelity 2 the outline is rounded off before it is fitted: points averaged with
    /// this many neighbours either side (3 at fidelity 1).
    fn rounding(&self) -> usize {
        ((2.0 - self.fidelity.clamp(1.0, 3.0)) * 3.0).round().max(0.0) as usize
    }
    /// Turns sharper than this stay corners.
    fn corner_turn(&self) -> f32 {
        (100.0 - 22.5 * self.fidelity.clamp(1.0, 3.0)).to_radians()
    }
    /// Rounds of edge-preserving smoothing.
    fn smooth_rounds(&self) -> usize {
        2 + (self.simplicity.clamp(0.0, 10.0) / 4.0) as usize
    }
}
