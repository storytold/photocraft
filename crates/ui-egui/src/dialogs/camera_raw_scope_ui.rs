//! What the RAW Develop source answers about the picture's scopes: histogram counts, the
//! vectorscope's counts, the clipping state and the pixel under a point. The workspace draws the
//! scope strip, the floating scope and the clipping overlay; nothing here draws.
//!
//! One [`ScopeData`] is built per develop and kept with its result, so a pass only reads it.
use craft::cms::Transform;
use craft::imaging::scopes::{ChannelMask, ClippingMask};
use craft::imaging::ui::histogram::ChannelColor;
use craft::imaging::ui::scopes::{ClippingFrame, HistogramSet, ScopeChannel, ScopeFrame, ScopeReadout, VectorscopeFrame};
use craft::ui::primitives::Revision;
use photocraft_algo::histogram::RgbHistogram;
use photocraft_algo::vectorscope::{HUES, SATURATIONS, Vectorscope, hue_saturation};
use std::sync::Arc;

/// Vectorscope analysis samples at most this many proxy pixels.
const SCOPE_SAMPLES: usize = 8192;

/// What one develop hands [`ScopeData::new`].
pub(crate) struct ScopeInputs<'a> {
    /// The develop's own revision: the clipping mask carries it.
    pub(crate) revision: u64,
    /// The proxy's width and height.
    pub(crate) size: [usize; 2],
    /// The proxy before the settings (Before), straight RGBA.
    pub(crate) proxy: &'a [[f32; 4]],
    /// The proxy as developed (After), straight RGBA.
    pub(crate) developed: &'a [[f32; 4]],
    /// The selection's coverage per proxy pixel, when the document has one.
    pub(crate) selection: Option<&'a [f32]>,
    /// The counts of `proxy`.
    pub(crate) before_histogram: &'a RgbHistogram,
    /// The counts of `developed`.
    pub(crate) after_histogram: &'a RgbHistogram,
    /// The proxy's colours into the vectorscope's (sRGB) domain.
    pub(crate) scope_transform: &'a Transform,
}

/// The counts of one develop: both pictures' histograms, the vectorscope of each picture over the
/// whole frame and over the selected region, and the After picture's clipping mask.
pub(crate) struct ScopeData {
    /// Indexed by [`side`]; the tests read the counts through [`ScopeData::histogram`].
    #[cfg(test)]
    histograms: [RgbHistogram; 2],
    /// Per [`side`]: the composite (sum of the three channels), then red, green, blue.
    bins: [[Vec<u32>; 4]; 2],
    /// Per [`side`]: the channels clipped at each end anywhere.
    clipped: [ClippingFrame; 2],
    /// Per [`side`], then per region (0 whole frame, 1 selected region).
    vectorscopes: [[Vectorscope; 2]; 2],
    mask: Arc<ClippingMask>,
    revision: Revision,
}

/// The index of a picture in the per-picture arrays: 0 After, 1 Before.
fn side(before: bool) -> usize {
    usize::from(before)
}

/// The index of a region in the per-region arrays: 0 whole frame, 1 selected region.
fn region_index(region: bool) -> usize {
    usize::from(region)
}

fn bin(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The channels with a clipped pixel, as the bits the warning buttons fill with.
fn clipped_channels(counts: &[u64; 3]) -> ChannelMask {
    counts.iter().enumerate().fold(ChannelMask::NONE, |mask, (channel, n)| if *n > 0 { mask.union(ChannelMask(1 << channel)) } else { mask })
}

impl ScopeData {
    /// Counts both pictures. The vectorscopes sample a fixed 2-D grid of at most [`SCOPE_SAMPLES`]
    /// pixels, so the colour management and the analysis cost the same at any proxy size. Rows and
    /// columns step together, so a stride cannot alias with the row width into stripes.
    pub(crate) fn new(inputs: ScopeInputs<'_>) -> Self {
        let ScopeInputs { revision, size, proxy, developed, selection, before_histogram, after_histogram, scope_transform } = inputs;
        let histograms = [after_histogram.clone(), before_histogram.clone()];
        let bins = histograms.each_ref().map(|h| {
            let [red, green, blue] = h.channels.each_ref().map(|c| c.iter().map(|n| bin(*n)).collect::<Vec<u32>>());
            let composite = red.iter().zip(&green).zip(&blue).map(|((r, g), b)| r.saturating_add(*g).saturating_add(*b)).collect();
            [composite, red, green, blue]
        });
        let clipped = histograms.each_ref().map(|h| ClippingFrame::new(clipped_channels(&h.shadows), clipped_channels(&h.highlights)));
        let vectorscopes = [developed, proxy].map(|pixels| {
            [false, true].map(|region| {
                let step = ((pixels.len() as f64 / SCOPE_SAMPLES as f64).sqrt().ceil() as usize).max(1);
                let counted: Vec<[f32; 4]> = (0..size[1])
                    .step_by(step)
                    .flat_map(|y| (0..size[0]).step_by(step).map(move |x| y * size[0] + x))
                    .filter_map(|i| pixels.get(i).map(|p| (i, p)))
                    .filter(|(i, p)| p[3] > 0.0 && p.iter().all(|v| v.is_finite()) && (!region || selection.and_then(|m| m.get(*i)).is_some_and(|v| *v > 0.0)))
                    .filter_map(|(_, p)| {
                        let mut rgb = [0.0; 3];
                        scope_transform.eval(&p[..3], &mut rgb).ok()?;
                        Some([rgb[0], rgb[1], rgb[2], p[3]])
                    })
                    .collect();
                Vectorscope::from_rgba(&counted)
            })
        });
        let mask = ClippingMask::from_rgba(size, developed).unwrap_or_else(|| ClippingMask::new(size));
        Self {
            #[cfg(test)]
            histograms,
            bins,
            clipped,
            vectorscopes,
            mask: Arc::new(mask),
            revision: Revision::new(revision),
        }
    }

    /// Hands `show` the frame of the picture before or after the settings, counted over the whole
    /// frame or the selected region, with the vectorscope's hover target at `hover` (hue in turns,
    /// saturation in 0..=1). The frame borrows these counts for the call.
    pub(crate) fn show(&self, before: bool, region: bool, hover: Option<[f32; 2]>, show: &mut dyn FnMut(Option<ScopeFrame<'_>>)) {
        let picture = side(before);
        let [composite, red, green, blue] = &self.bins[picture];
        let channels = [
            ScopeChannel::new("rgb", tl!("RGB"), ChannelColor::Neutral, composite, false),
            ScopeChannel::new("red", tl!("Red"), ChannelColor::Red, red, true),
            ScopeChannel::new("green", tl!("Green"), ChannelColor::Green, green, true),
            ScopeChannel::new("blue", tl!("Blue"), ChannelColor::Blue, blue, true),
        ];
        let vectorscope = &self.vectorscopes[picture][region_index(region)];
        let frame = ScopeFrame::histogram(HistogramSet::new(&channels))
            .vectorscope(VectorscopeFrame::new(&vectorscope.counts, HUES, SATURATIONS, hover))
            .clipping(self.clipped[picture]);
        show(Some(frame));
    }

    // PLAN: `RawDevelopSource::clipping` has no Before flag, so Before mode overlays the After picture's mask.
    /// The After picture's clipping mask and the revision of the develop it was made from.
    pub(crate) fn clipping(&self) -> (Arc<ClippingMask>, Revision) {
        (self.mask.clone(), self.revision)
    }

    #[cfg(test)]
    /// The counts of the picture before or after the settings, for the control channel's reply.
    pub(crate) fn histogram(&self, before: bool) -> &RgbHistogram {
        &self.histograms[side(before)]
    }

    /// The vectorscope of the picture before or after the settings over the whole frame or the
    /// selected region, for the control channel's reply.
    pub(crate) fn vectorscope(&self, before: bool, region: bool) -> &Vectorscope {
        &self.vectorscopes[side(before)][region_index(region)]
    }
}

/// The vectorscope of the picture before or after the settings, over the whole frame or the selected region, as the control
/// channel's reply.
pub(crate) fn vectorscope_reply(data: &ScopeData, before: bool, region: bool) -> serde_json::Value {
    let vectorscope = data.vectorscope(before, region);
    serde_json::json!({"before": before, "selectedRegion": region, "hues": HUES, "saturations": SATURATIONS, "samples": vectorscope.samples, "counts": vectorscope.counts})
}

/// The proxy pixel under `at` (0..=1 each way over the picture), straight RGBA. `refined` is the
/// full-resolution develop's pixel there, which wins when it has one. `None` outside the picture,
/// over a transparent pixel or one that is not finite.
pub(crate) fn pixel_at(at: [f64; 2], size: [usize; 2], pixels: &[[f32; 4]], refined: impl FnOnce([f32; 2]) -> Option<[f32; 4]>) -> Option<[f32; 4]> {
    if !at.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) || size[0] == 0 || size[1] == 0 {
        return None;
    }
    let at = at.map(|v| v as f32);
    let x = ((at[0] * size[0] as f32) as usize).min(size[0] - 1);
    let y = ((at[1] * size[1] as f32) as usize).min(size[1] - 1);
    let q = refined(at).or_else(|| pixels.get(y.checked_mul(size[0])?.checked_add(x)?).copied())?;
    (q[3] > 0.0 && q.iter().all(|v| v.is_finite())).then_some(q)
}

/// `pixel` as the readout form `lab` asks for: Lab through `lab_transform`, else RGB.
pub(crate) fn readout(pixel: [f32; 4], lab: bool, lab_transform: &Transform) -> ScopeReadout {
    let rgb = [pixel[0], pixel[1], pixel[2]];
    let mut l = [0.0; 3];
    if lab && lab_transform.eval(&rgb, &mut l).is_ok() {
        ScopeReadout::Lab([l[0] * 100.0, l[1] * 255.0 - 128.0, l[2] * 255.0 - 128.0])
    } else {
        ScopeReadout::Rgb(rgb)
    }
}

/// The hue (turns) and saturation (0..=1) of `pixel` in the vectorscope's domain: the hover
/// target. `None` when the colour management gives a value that is not finite.
pub(crate) fn hover_target(pixel: [f32; 4], scope_transform: &Transform) -> Option<[f32; 2]> {
    let mut rgb = [0.0; 3];
    scope_transform.eval(&pixel[..3], &mut rgb).ok()?;
    if !rgb.iter().all(|x| x.is_finite()) {
        return None;
    }
    let (hue, saturation) = hue_saturation(rgb.map(|x| x.clamp(0.0, 1.0)));
    Some([hue, saturation])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srgb() -> Arc<Transform> {
        let profile = craft::cms::Builtin::Srgb.profile();
        craft::cms::cached(profile, profile, Default::default()).unwrap()
    }

    /// Before is all red, After all blue; the selection covers the left half of a 4 x 2 frame.
    fn data() -> ScopeData {
        let proxy = vec![[1.0, 0.0, 0.0, 1.0]; 8];
        let developed = vec![[0.0, 0.0, 1.0, 1.0]; 8];
        let selection: Vec<f32> = (0..8).map(|i| if i % 4 < 2 { 1.0 } else { 0.0 }).collect();
        ScopeData::new(ScopeInputs {
            revision: 7,
            size: [4, 2],
            proxy: &proxy,
            developed: &developed,
            selection: Some(&selection),
            before_histogram: &RgbHistogram::from_rgba(&proxy),
            after_histogram: &RgbHistogram::from_rgba(&developed),
            scope_transform: &srgb(),
        })
    }

    fn counted(data: &ScopeData, before: bool, region: bool) -> u32 {
        let mut shown = false;
        data.show(before, region, None, &mut |frame| shown = frame.is_some());
        assert!(shown, "a develop's counts always make a frame");
        data.vectorscope(before, region).counts.iter().sum()
    }

    #[test]
    fn each_picture_and_region_reads_its_own_counts() {
        let data = data();
        assert_eq!(counted(&data, false, false), 8, "After, whole frame");
        assert_eq!(counted(&data, false, true), 4, "After, selected region");
        assert_eq!(counted(&data, true, false), 8, "Before, whole frame");
        assert_ne!(data.vectorscope(true, true).counts, data.vectorscope(false, true).counts, "red and blue land in different cells");
        assert_eq!(data.histogram(true).channels[0][255], 8, "Before is the red picture");
        assert_eq!(data.histogram(false).channels[2][255], 8, "After is the blue picture");
        let (mask, revision) = data.clipping();
        assert_eq!((revision, mask.size), (Revision::new(7), [4, 2]));
        assert_eq!(mask.at(0, 0).highlights, ChannelMask::B);
        assert_eq!(mask.at(0, 0).shadows, ChannelMask(3), "red and green are 0 in the blue picture");
    }

    #[test]
    fn pixel_at_prefers_the_refined_pixel_and_rejects_bad_points() {
        let pixels = vec![[0.1, 0.2, 0.3, 1.0], [0.4, 0.5, 0.6, 1.0]];
        assert_eq!(pixel_at([0.9, 0.0], [2, 1], &pixels, |_| None), Some([0.4, 0.5, 0.6, 1.0]));
        assert_eq!(pixel_at([0.9, 0.0], [2, 1], &pixels, |_| Some([0.7, 0.7, 0.7, 1.0])), Some([0.7, 0.7, 0.7, 1.0]));
        assert!(pixel_at([f64::NAN, 0.0], [2, 1], &pixels, |_| None).is_none());
        assert!(pixel_at([1.5, 0.0], [2, 1], &pixels, |_| None).is_none());
        assert!(pixel_at([0.0, 0.0], [2, 1], &[[0.1, 0.2, 0.3, 0.0]], |_| None).is_none(), "a transparent pixel has no sample");
    }
}
