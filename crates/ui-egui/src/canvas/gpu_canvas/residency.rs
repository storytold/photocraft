//! One display-texture budget ([`super::canvas_budget`]) for every open document, rather than
//! the whole budget for each (N documents could hold N times it, #5332 review).
//!
//! The app publishes the documents its windows show and what each one's whole-document texture
//! set needs ([`super::GpuCanvas::show_documents`]). Those split the budget max-min fairly: a
//! document that fits its share keeps its texture set, a larger one is drawn from viewport pages
//! within it, and a document alone on screen still has the whole budget. The split changes only
//! when the shown documents (or their sizes) do, never with focus, so switching between windows
//! moves nothing. Documents no window shows have no share: they keep their textures while there
//! is room (switching back to a tab is free) and are the first freed, least recently shown
//! first, when a shown document needs the room.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use super::resources::{DocTextures, Resources};
use super::texels::texel_bytes;

/// Keys tagged above this bit are previews (filter preview, adjustment proxies, gradients,
/// thumbnails): never evicted for another document.
const TAG_BITS: u64 = !((1u64 << 58) - 1);
/// The tag of a document's viewport pages (`viewport_pages::stream_key`).
const PAGES_BIT: u64 = 1u64 << 57;
/// A background texture set more than this many times the bytes asked for is kept while the
/// total stays within the allowance over the budget ([`Residency::victims`]).
const LARGE_FACTOR: u64 = 4;

impl DocTextures {
    /// GPU bytes of every texture (all mip levels) in this set or page set.
    pub(super) fn bytes(&self) -> u64 {
        let bpp = texel_bytes(self.format);
        let level = |w: u32, h: u32, l: u32| u64::from((w >> l).max(1)) * u64::from((h >> l).max(1)) * bpp;
        self.tiles.iter().map(|t| (0..t.texture.mip_level_count()).map(|l| level(t.texture.width(), t.texture.height(), l)).sum::<u64>()).sum()
    }
}

impl Resources {
    /// Free the textures of documents no window shows ([`Residency::victims`]) so that `key`
    /// can hold `bytes` within the canvas budget.
    pub(super) fn make_room(&mut self, key: u64, bytes: u64) {
        let budget = self.canvas_budget.load(Ordering::Relaxed);
        // A quarter over the budget only on a card whose own memory is known: the budget is 3/8
        // of it (`vram_share`), so that stays under half. Otherwise (integrated or software
        // adapters, or a card that can't be read) the budget comes from system memory, which
        // the textures share, and it is kept to.
        let allowance = if self.card_memory_known { budget / 4 } else { 0 };
        let held: Vec<(u64, u64)> = self.docs.iter().map(|(k, d)| (*k, d.bytes())).collect();
        for k in self.residency.victims(&held, key, bytes, budget, allowance) {
            log::debug!("freeing the display textures of background key {k:#x} for {key:#x}");
            self.docs.remove(&k);
        }
    }
}

/// Document `doc`'s allocation when documents needing `needs` bytes split `budget` max-min
/// fairly (`doc` indexes `needs`): those needing less than an equal share take what they need,
/// the rest split what is left; the leftover, if any, is `doc`'s too.
pub(super) fn fair_share(budget: u64, needs: &[u64], doc: usize) -> u64 {
    let mut order: Vec<usize> = (0..needs.len()).collect();
    order.sort_by_key(|&i| needs[i]);
    let mut left = budget;
    let mut others = 0u64;
    for (n, &i) in order.iter().enumerate() {
        let take = needs[i].min(left / (order.len() - n) as u64);
        left -= take;
        if i != doc {
            others += take;
        }
    }
    budget - others
}

/// The documents on screen and when each was last shown.
#[derive(Default)]
pub(super) struct Residency {
    /// Shown documents: (document id, bytes of its whole texture set).
    shown: Vec<(u64, u64)>,
    /// When each document was last among the shown (a counter of changes).
    seen: HashMap<u64, u64>,
    tick: u64,
}

impl Residency {
    /// Record the shown documents. False when nothing changed.
    pub(super) fn publish(&mut self, shown: &[(u64, u64)]) -> bool {
        if self.shown == shown {
            return false;
        }
        self.tick += 1;
        for (id, _) in shown {
            self.seen.insert(*id, self.tick);
        }
        self.shown = shown.to_vec();
        true
    }

    /// The share of `budget` document `id` (its texture set `need` bytes) may hold: the whole
    /// budget when nothing else is shown.
    pub(super) fn share(&self, budget: u64, id: u64, need: u64) -> u64 {
        let mut needs: Vec<u64> = self.shown.iter().filter(|(d, _)| *d != id).map(|(_, n)| *n).collect();
        needs.push(need);
        fair_share(budget, &needs, needs.len() - 1)
    }

    /// Which held keys (`held`: key and bytes) to free, in order, so that `key` can hold
    /// `bytes` within `budget`: the texture sets and pages of documents no window shows, least
    /// recently shown first. Never a shown document's, nor `key`'s own document's, nor a preview
    /// or a key of a document never shown (an adjustment proxy). It may not free enough: what
    /// a shown document takes is bounded by its share instead. Sets many times `bytes`
    /// ([`LARGE_FACTOR`]) go last, and not at all while the total is within `allowance` over
    /// the budget.
    pub(super) fn victims(&self, held: &[(u64, u64)], key: u64, bytes: u64, budget: u64, allowance: u64) -> Vec<u64> {
        let owner = |k: u64| (k & TAG_BITS == 0).then_some(k & !PAGES_BIT);
        let mut total = held.iter().filter(|(k, _)| *k != key).map(|(_, b)| *b).fold(bytes, u64::saturating_add);
        if total <= budget {
            return Vec::new();
        }
        let mine = owner(key);
        let mut cold: Vec<(u64, u64, u64)> = held
            .iter()
            .filter_map(|&(k, b)| {
                let d = owner(k)?;
                let at = *self.seen.get(&d)?;
                (Some(d) != mine && !self.shown.iter().any(|(s, _)| *s == d)).then_some((at, k, b))
            })
            .collect();
        cold.sort_unstable();
        // A background texture set many times the size of what is asked for is the expensive
        // one to lose: a gigapixel storyboard filled the budget alone, so showing any other tab
        // freed it and switching back redrew it from scratch (about 6 s). Smaller ones go first;
        // the large ones stay while the total is within `allowance` over the budget.
        let large = |b: u64| b > bytes.saturating_mul(LARGE_FACTOR);
        let mut out = Vec::new();
        for &(_, k, b) in cold.iter().filter(|(_, _, b)| !large(*b)) {
            if total <= budget {
                return out;
            }
            total -= b;
            out.push(k);
        }
        if total <= budget.saturating_add(allowance) {
            return out;
        }
        for &(_, k, b) in cold.iter().filter(|(_, _, b)| large(*b)) {
            if total <= budget {
                break;
            }
            total -= b;
            out.push(k);
        }
        out
    }

    /// Whether `shown` is what was published last.
    pub(super) fn shows(&self, shown: &[(u64, u64)]) -> bool {
        self.shown == shown
    }

    /// Forget closed documents.
    pub(super) fn retain(&mut self, live: impl Fn(u64) -> bool) {
        self.seen.retain(|d, _| live(*d));
        self.shown.retain(|(d, _)| live(*d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn one_document_has_the_whole_budget() {
        let r = Residency::default();
        assert_eq!(r.share(4 * GB, 1, 9 * GB), 4 * GB);
        let mut r = Residency::default();
        r.publish(&[(1, 9 * GB)]);
        assert_eq!(r.share(4 * GB, 1, 9 * GB), 4 * GB, "shown alone");
        assert_eq!(r.share(4 * GB, 1, GB), 4 * GB);
    }

    #[test]
    fn shown_documents_split_the_budget_max_min_fairly() {
        let mut r = Residency::default();
        // A small document takes what it needs; the large one the rest.
        r.publish(&[(1, GB), (2, 9 * GB)]);
        assert_eq!(r.share(4 * GB, 1, GB), GB);
        assert_eq!(r.share(4 * GB, 2, 9 * GB), 3 * GB);
        // What the others leave is the asking document's too.
        r.publish(&[(1, GB), (2, 2 * GB)]);
        assert_eq!(r.share(4 * GB, 2, 2 * GB), 3 * GB, "the leftover is its own too");
        // Two large ones split it evenly, whichever has focus: no churn switching windows.
        r.publish(&[(1, 3 * GB), (2, 3 * GB)]);
        assert_eq!(r.share(4 * GB, 1, 3 * GB), 2 * GB);
        assert_eq!(r.share(4 * GB, 2, 3 * GB), 2 * GB);
        // Three, one of them small.
        r.publish(&[(1, 3 * GB), (2, 3 * GB), (3, GB / 2)]);
        assert_eq!(r.share(4 * GB, 1, 3 * GB), 7 * GB / 4);
        assert_eq!(r.share(4 * GB, 3, GB / 2), GB / 2, "needs only half a GB");
        // Everything fits: each share covers its need, together within the budget.
        r.publish(&[(1, GB), (2, GB)]);
        assert!(r.share(4 * GB, 1, GB) >= GB && r.share(4 * GB, 2, GB) >= GB);
        // A document no window shows (a refresh of a background tab) is priced as one more.
        assert_eq!(r.share(4 * GB, 7, 4 * GB), 2 * GB);
    }

    #[test]
    fn republishing_the_same_documents_changes_nothing() {
        let mut r = Residency::default();
        assert!(r.publish(&[(1, GB)]));
        assert!(!r.publish(&[(1, GB)]));
        assert!(r.publish(&[(1, 2 * GB)]), "the document was resized");
    }

    #[test]
    fn background_documents_are_freed_least_recently_shown_first() {
        let mut r = Residency::default();
        // Tabs 1, 2, 3 shown in turn, then 4.
        for d in 1..=4 {
            r.publish(&[(d, GB)]);
        }
        let pages = |d: u64| d ^ PAGES_BIT;
        let held = [(1, GB), (pages(2), GB / 2), (2, GB / 2), (3, GB), (1 << 61 | 3, GB), (99, GB)];
        // Room enough: nothing goes.
        assert!(r.victims(&held, 4, GB, 16 * GB, 0).is_empty());
        // 4 needs 2 GB within 6 GB (5 held): tab 1 goes first, then tab 2's pages and set.
        assert_eq!(r.victims(&held, 4, 2 * GB, 6 * GB, 0), vec![1]);
        assert_eq!(r.victims(&held, 4, 3 * GB, 6 * GB, 0), vec![1, 2, pages(2)]);
        // Never a shown document's, a preview's or an unknown key's, even when that isn't enough.
        assert_eq!(r.victims(&held, 4, 10 * GB, 6 * GB, 0), vec![1, 2, pages(2), 3]);
        // Nor the document's own: its pages make room by evicting the other tabs.
        assert_eq!(r.victims(&held, pages(3), GB, 4 * GB, 0), vec![1, 2, pages(2)]);
    }

    /// A background canvas many times the size asked for stays while the total is within the
    /// allowance over the budget, smaller background sets going first. Past the allowance, or with
    /// none (the card's own memory unknown), large sets go too, least recently shown first.
    #[test]
    fn a_large_background_canvas_survives_a_small_tab() {
        let mut r = Residency::default();
        // Tabs shown in turn: storyboards 1 and 2 (6 GB each), small tab 3 (1 GB), then tab 4,
        // which asks for half a GB: a set over 2 GB counts as large.
        for d in 1..=4 {
            r.publish(&[(d, GB)]);
        }
        let ask = |held: &[(u64, u64)], budget: u64, allowance: u64| r.victims(held, 4, GB / 2, budget, allowance);
        // One storyboard fills the 6 GB budget: kept within a quarter over it.
        assert!(ask(&[(1, 6 * GB)], 6 * GB, 6 * GB / 4).is_empty(), "kept: within the allowance");
        // Without an allowance (the card's memory unknown), the budget is kept to.
        assert_eq!(ask(&[(1, 6 * GB)], 6 * GB, 0), vec![1], "no allowance: it goes");
        // A small set goes first, even though it was shown more recently; the storyboard stays.
        assert_eq!(ask(&[(1, 6 * GB), (3, GB)], 6 * GB, 6 * GB / 4), vec![3], "the small set goes first");
        // Past the allowance even without the small set: the least recently shown large set
        // goes (1, not 2), and only as many as needed.
        assert_eq!(ask(&[(1, 6 * GB), (2, 6 * GB), (3, GB)], 8 * GB, 2 * GB), vec![3, 1], "least recently shown large set next");
    }
}
