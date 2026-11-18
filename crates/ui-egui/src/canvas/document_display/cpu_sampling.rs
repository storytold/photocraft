//! Sampling and occlusion of the CPU display's real base/detail uploads.

use std::cell::RefCell;

use craft::behavior::PixelSize;
use craft::ui::canvas::{DetailPlan, PixelRect, RasterSource};
use craft::ui::primitives::Revision;
use egui::{ColorImage, Context, Id};
use photocraft_geom::Rect;

use crate::gpu_canvas::OpaqueTexels;

#[derive(Clone)]
pub(super) struct DetailAlpha {
    plan: DetailPlan,
    opaque: OpaqueTexels,
}

/// An observer of texture reads, not another compositor. The mask changes only when RasterDisplay uploads pixels.
pub(super) struct CpuSource<'a> {
    source: &'a dyn RasterSource,
    ctx: &'a Context,
    key: Id,
    bounds: PixelRect,
    factor: u32,
    scale: f32,
    detail: RefCell<Option<DetailAlpha>>,
}

impl<'a> CpuSource<'a> {
    /// None means the bounded opacity mask cannot be retained; the caller draws only the base texture instead.
    pub(super) fn new(
        source: &'a dyn RasterSource,
        ctx: &'a Context,
        key: Id,
        bounds: PixelRect,
        factor: u32,
        scale: f32,
        plan: Option<DetailPlan>,
    ) -> Option<Self> {
        let previous = ctx.data(|data| data.get_temp::<DetailAlpha>(key));
        let detail = if let Some(plan) = plan {
            if let Some(previous) = previous.filter(|previous| previous.plan == plan) {
                Some(previous)
            } else {
                let width = plan.region.width().unsigned_abs().div_ceil(plan.factor.max(1));
                let height = plan.region.height().unsigned_abs().div_ceil(plan.factor.max(1));
                Some(DetailAlpha { plan, opaque: OpaqueTexels::new(width, height, std::iter::empty())? })
            }
        } else {
            None
        };
        Some(Self { source, ctx, key, bounds, factor, scale, detail: RefCell::new(detail) })
    }

    pub(super) fn finish(self, shown: Option<DetailPlan>) -> CpuSampling {
        let detail = self.detail.into_inner().filter(|detail| Some(detail.plan) == shown);
        self.ctx.data_mut(|data| {
            if let Some(detail) = &detail {
                data.insert_temp(self.key, detail.clone());
            } else {
                data.remove::<DetailAlpha>(self.key);
            }
        });
        CpuSampling { bounds: self.bounds, factor: self.factor, scale: self.scale, detail }
    }
}

impl RasterSource for CpuSource<'_> {
    fn size(&self) -> PixelSize {
        self.source.size()
    }

    fn revision(&self) -> Revision {
        self.source.revision()
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        self.source.damage(since)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        self.source.read(region, factor, out);
        let mut detail = self.detail.borrow_mut();
        let Some(detail) = detail.as_mut().filter(|detail| detail.plan.factor == factor) else { return };
        let plan = detail.plan;
        if factor == 0 || region.intersect(&plan.region) != region {
            return;
        }
        let x = i64::from(region.x0) - i64::from(plan.region.x0);
        let y = i64::from(region.y0) - i64::from(plan.region.y0);
        let (Ok(x), Ok(y), Ok(width)) = (u32::try_from(x), u32::try_from(y), u32::try_from(out.size[0])) else { return };
        detail.opaque.update([x / factor, y / factor], width, out.pixels.iter().map(|pixel| pixel.a() == 255));
    }
}

/// The last painted CPU grids and their exact detail occlusion. Clones share masks until an upload changes them.
#[derive(Clone)]
pub(crate) struct CpuSampling {
    bounds: PixelRect,
    factor: u32,
    scale: f32,
    detail: Option<DetailAlpha>,
}

impl CpuSampling {
    pub(super) fn base(bounds: PixelRect, factor: u32, scale: f32) -> Self {
        Self { bounds, factor, scale, detail: None }
    }

    pub(super) fn any_region(&self, x: f64, y: f64, mut test: impl FnMut(Rect) -> bool) -> bool {
        let base = super::cpu_sampled_rect(x, y, self.bounds, self.factor, self.factor > 1 || self.scale < 1.0);
        if let Some(detail) = self.detail.as_ref().filter(|detail| {
            x >= f64::from(detail.plan.region.x0)
                && x < f64::from(detail.plan.region.x1)
                && y >= f64::from(detail.plan.region.y0)
                && y < f64::from(detail.plan.region.y1)
        }) {
            let plan = detail.plan;
            let area = super::cpu_sampled_rect(x, y, plan.region, plan.factor, plan.factor > 1 || self.scale < 1.0);
            if test(area) {
                return true;
            }
            let texel = |v: i32, origin: i32| ((i64::from(v) - i64::from(origin)).max(0) as u64 / u64::from(plan.factor.max(1))) as u32;
            let opaque = detail.opaque.all(
                texel(area.x0, plan.region.x0),
                texel(area.y0, plan.region.y0),
                texel(area.x1.saturating_sub(1), plan.region.x0).saturating_add(1),
                texel(area.y1.saturating_sub(1), plan.region.y0).saturating_add(1),
            );
            if opaque || area == base {
                return false;
            }
        }
        !base.is_empty() && test(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct AlphaSource(Cell<u8>);

    impl RasterSource for AlphaSource {
        fn size(&self) -> PixelSize {
            PixelSize::new(512, 512)
        }
        fn revision(&self) -> Revision {
            Revision::of(&0)
        }
        fn damage(&self, _: Revision) -> Option<PixelRect> {
            None
        }
        fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
            out.pixels.fill(egui::Color32::from_rgba_premultiplied(0, 0, 0, self.0.get()));
        }
    }

    #[test]
    fn partial_detail_upload_changes_occlusion_without_mutating_the_painted_snapshot() {
        let ctx = Context::default();
        let key = Id::unique("detail-occlusion");
        let bounds = PixelRect::new(259, 261, 267, 269);
        let plan = DetailPlan::of(bounds, 4, 1.0, PixelRect::new(261, 263, 263, 265)).unwrap();
        assert_eq!(plan.region, PixelRect::new(261, 263, 263, 265));
        let pixels = AlphaSource(Cell::new(255));
        let source = CpuSource::new(&pixels, &ctx, key, bounds, 2, 2.0, Some(plan)).unwrap();
        let mut image = ColorImage::filled([2, 2], egui::Color32::TRANSPARENT);
        source.read(plan.region, 1, &mut image);
        let painted = source.finish(Some(plan));
        let coarse_only = |area: Rect| area.contains(260, 262);
        assert!(!painted.any_region(261.5, 263.5, coarse_only), "opaque detail hides coarse-only ink");

        pixels.0.set(0);
        let source = CpuSource::new(&pixels, &ctx, key, bounds, 2, 2.0, Some(plan)).unwrap();
        let mut patch = ColorImage::filled([1, 1], egui::Color32::TRANSPARENT);
        source.read(PixelRect::new(261, 263, 262, 264), 1, &mut patch);
        let next = source.finish(Some(plan));
        assert!(next.any_region(261.5, 263.5, coarse_only), "transparent updated texel reveals the base");
        assert!(!painted.any_region(261.5, 263.5, coarse_only), "the previous picture stays immutable");
        assert!(!next.any_region(262.5, 264.5, |area| area.contains(264, 265)), "unmodified opaque texels still occlude");
    }
}
