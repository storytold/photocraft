//! The one picture holder a workspace's preview job writes into and its display reads from.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft::behavior::PixelSize;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::primitives::Revision;
use egui::ColorImage;

/// How many writes the picture can still tell the damage of: a display's textures may lag a few passes behind.
const DAMAGE_RING: usize = 8;

/// A picture a background job writes and a display reads. The picture's revision moves on every write, so the
/// display re-reads without the app replacing its source.
pub(crate) struct SharedPicture {
    size: PixelSize,
    revision: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    image: Option<ColorImage>,
    /// The document pixel of the image's first texel.
    origin: [i32; 2],
    /// Document pixels per texel of the image.
    k: u32,
    /// The writes still remembered: the revision each made and the region it changed (`None`: everything).
    damage: VecDeque<(u64, Option<PixelRect>)>,
}

impl SharedPicture {
    /// An empty picture over a document of `size`; it reads as transparent until the first write.
    pub(crate) fn new(size: PixelSize) -> Arc<Self> {
        Arc::new(SharedPicture { size, revision: AtomicU64::new(0), state: Mutex::new(State::default()) })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, state: &mut State, rect: Option<PixelRect>) {
        let revision = self.revision.load(Ordering::Acquire) + 1;
        if state.damage.len() == DAMAGE_RING {
            state.damage.pop_front();
        }
        state.damage.push_back((revision, rect));
        // The data is in place before the revision shows it.
        self.revision.store(revision, Ordering::Release);
    }

    /// Replaces the whole picture: `image` is the document reduced `k` times, starting at the document's origin.
    pub(crate) fn replace(&self, image: ColorImage, k: u32) {
        self.replace_at([0, 0], image, k);
    }

    /// Replaces the whole picture with `image`, `k` document pixels per texel, whose first texel is the document
    /// pixel `origin` (a preview of the fill's rectangle only).
    pub(crate) fn replace_at(&self, origin: [i32; 2], image: ColorImage, k: u32) {
        let mut state = self.lock();
        state.image = Some(image);
        state.origin = origin;
        state.k = k.max(1);
        self.record(&mut state, None);
    }

    /// Whether the picture holds a result.
    pub(crate) fn is_ready(&self) -> bool {
        self.revision.load(Ordering::Acquire) != 0
    }
}

impl RasterSource for SharedPicture {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        Revision::new(self.revision.load(Ordering::Acquire))
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        let now = self.revision.load(Ordering::Acquire);
        let since = since.get();
        if since == now {
            return Some(PixelRect::default());
        }
        let state = self.lock();
        // Every write after `since` must still be remembered.
        if state.damage.front()?.0 > since + 1 {
            return None;
        }
        let mut region: Option<PixelRect> = None;
        for (_, rect) in state.damage.iter().filter(|(revision, _)| *revision > since) {
            let rect = (*rect)?;
            region = Some(region.map_or(rect, |so_far| so_far.union(&rect)));
        }
        region
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let state = self.lock();
        let Some(image) = state.image.as_ref() else { return };
        let (k, step) = (i64::from(state.k), i64::from(factor.max(1)));
        let [width, height] = out.size;
        let [image_w, image_h] = image.size;
        for ty in 0..height {
            let y = i64::from(region.y0) + ty as i64 * step - i64::from(state.origin[1]);
            let iy = y.div_euclid(k);
            if iy < 0 || iy >= image_h as i64 {
                continue;
            }
            for tx in 0..width {
                let x = i64::from(region.x0) + tx as i64 * step - i64::from(state.origin[0]);
                let ix = x.div_euclid(k);
                if ix >= 0 && ix < image_w as i64 {
                    out.pixels[ty * width + tx] = image.pixels[iy as usize * image_w + ix as usize];
                }
            }
        }
    }
}
