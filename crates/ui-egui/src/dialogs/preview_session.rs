//! What a dialog's live preview on the GPU canvas shares (#5772): a session (one document
//! revision and target), preview keys tagged with it, the area its previews can change, and which
//! part of that the canvas has composited so far. Switching between the session's previews, or
//! back to the committed document, recomposites only the area; of a preview, only what the view
//! shows, catching up as the view moves ([`PreviewSession::uncovered`]).
//!
//! Used by the adjustment dialog's and the Properties panel's previews (`adjust_preview`) and
//! by the filter dialog's GPU preview (`filter_gpu_preview`).

use photocraft_doc::DocId;
use photocraft_geom::Rect;

/// Bits of a preview key below the session: the settings hash.
const SESSION_SHIFT: u32 = 40;

/// FNV-1a over `parts`, never 0.
pub fn hash(parts: &[&[u8]]) -> u64 {
    parts.iter().flat_map(|p| p.iter()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)) | 1
}

/// One preview session of a document.
#[derive(Clone, Debug)]
pub struct PreviewSession {
    /// The key bit naming the kind of preview (never shared with another kind's).
    tag: u64,
    /// Mask of the session bits kept in a key, between the tag and the settings hash.
    mask: u64,
    pub doc: DocId,
    pub revision: u64,
    /// The session's hash (document, revision, target, …).
    pub session: u64,
    /// Where the session's previews can differ from the committed document.
    pub region: Rect,
    /// The canvas texture shows preview `key` only within this rect; elsewhere in the region it
    /// is older (see [`Self::uncovered`]).
    covered: Option<(u64, Rect)>,
}

impl PreviewSession {
    /// A session of `doc` at `revision`, keyed `tag` with `session_bits` of `session` in its keys
    /// (above bit 40, below the tag), whose previews change `region`.
    pub fn new(tag: u64, session_bits: u32, doc: DocId, revision: u64, session: u64, region: Rect) -> Self {
        let mask = (1u64 << session_bits.min(23)) - 1;
        PreviewSession { tag, mask, doc, revision, session, region, covered: None }
    }

    /// The preview key for settings hash `h`.
    pub fn key(&self, h: u64) -> u64 {
        self.tag | ((self.session & self.mask) << SESSION_SHIFT) | (h & ((1 << SESSION_SHIFT) - 1))
    }

    /// Whether `key` is one of this session's previews.
    pub fn owns(&self, key: u64) -> bool {
        key >> SESSION_SHIFT == self.key(0) >> SESSION_SHIFT
    }

    /// The canvas area to recomposite when `doc` (at `revision`) switches from preview key `from`
    /// to `to` (raw keys, display transform removed): both must be this session's previews or
    /// the committed document (0). `None` means "everything".
    pub fn switch_region(&self, doc: DocId, revision: u64, from: u64, to: u64) -> Option<Rect> {
        if self.doc != doc || self.revision != revision {
            return None;
        }
        let ours = |k: u64| k == 0 || self.owns(k);
        (ours(from) && ours(to) && (from != 0 || to != 0)).then_some(self.region)
    }

    /// The part of a switch's region (from [`Self::switch_region`]) to composite now: to a preview,
    /// only what the view shows (`visible`, document pixels), remembered so [`Self::uncovered`]
    /// catches up as the view moves; back to the committed document (key 0), the whole region.
    pub fn switch_damage(&mut self, to: u64, region: Rect, visible: Rect) -> Rect {
        if to == 0 {
            self.covered = None;
            return region;
        }
        let shown = region.intersect(&visible);
        self.covered = Some((to, shown));
        shown
    }

    /// Where the canvas texture (showing preview `key`) is out of date within `visible`: the
    /// region's visible part when the view moved past what was composited. Marks it covered.
    pub fn uncovered(&mut self, doc: DocId, key: u64, visible: Rect) -> Option<Rect> {
        if self.doc != doc {
            return None;
        }
        let (k, cov) = self.covered?;
        if k != key {
            return None;
        }
        let want = self.region.intersect(&visible);
        if want.is_empty() || cov.contains_rect(&want) {
            return None;
        }
        self.covered = Some((k, want));
        Some(want)
    }

    /// Forget what the canvas composited (the canvas texture was redrawn in full).
    pub fn reset_covered(&mut self) {
        self.covered = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_switches_and_catch_up() {
        let doc = DocId(7);
        let region = Rect::new(0, 0, 1000, 800);
        let mut s = PreviewSession::new(1 << 57, 17, doc, 3, hash(&[b"session"]), region);
        let (a, b) = (s.key(hash(&[b"a"])), s.key(hash(&[b"b"])));
        assert!(a != b && s.owns(a) && s.owns(b) && a & (1 << 57) != 0);
        let other = PreviewSession::new(1 << 57, 17, doc, 3, hash(&[b"other"]), region);
        assert!(!s.owns(other.key(1)), "another session's preview");
        assert_eq!(s.switch_region(doc, 3, 0, a), Some(region));
        assert_eq!(s.switch_region(doc, 3, a, b), Some(region));
        assert_eq!(s.switch_region(doc, 3, 0, 0), None, "no preview involved");
        assert_eq!(s.switch_region(doc, 4, a, b), None, "another revision");
        assert_eq!(s.switch_region(doc, 3, a, other.key(1)), None, "another session");
        // Only the view now; the rest as the view moves there.
        let view = Rect::new(0, 0, 300, 200);
        assert_eq!(s.switch_damage(b, region, view), view);
        assert_eq!(s.uncovered(doc, b, view), None);
        assert_eq!(s.uncovered(doc, b, Rect::new(200, 100, 500, 300)), Some(Rect::new(200, 100, 500, 300)));
        assert_eq!(s.uncovered(doc, a, Rect::new(600, 600, 900, 700)), None, "not the shown preview");
        assert_eq!(s.switch_damage(0, region, view), region, "back to the document: everything");
        assert_eq!(s.uncovered(doc, b, Rect::new(900, 700, 1000, 800)), None);
        assert_eq!(hash(&[]), 0xcbf2_9ce4_8422_2325 | 1);
    }
}
