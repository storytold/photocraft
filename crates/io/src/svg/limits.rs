//! Hard caps on the memory resvg needs to draw a subtree, checked before it draws.
//!
//! [`super::MAX_RASTER_PIXELS`] bounds the target pixmap only. While it draws, resvg also
//! allocates, and a failed allocation aborts the process rather than returning an error:
//! - a layer for every isolated group (opacity, blend mode, clip, mask, filter), clamped to 5 × 5
//!   times the target, and one more per link of a clip-path chain, per clip child that has its own
//!   clip-path, and per link of a mask-on-mask chain;
//! - for a filter, a result per primitive (all kept until the filter ends) and a clone of the layer.
//!   The filter region is NOT clamped: `x`, `y`, `width` and `height` in the file set the size;
//! - a pattern tile of `width × height × scale`;
//! - for an embedded SVG image or an `feImage`, a pixmap of the current layer or filter region with
//!   a nested drawing inside.
//!
//! The checks are worst-case products, not a model of resvg's phases. A node is checked against
//! the layers of its ancestors (which are alive while it is drawn) and nothing else, and each
//! shared definition is walked once per size class, so the walk is linear in the file.

use std::collections::{HashMap, HashSet};

use resvg::tiny_skia;
use resvg::usvg;

use super::MAX_PEAK_PIXEL_BYTES;

/// Bytes of RGBA8 buffers alive at once that a drawing may need: the existing 1.5 GiB peak budget.
/// A 24 MP target (96 MB) leaves room for about 14 more full-canvas layers.
const BUDGET: f64 = MAX_PEAK_PIXEL_BYTES as f64;
/// Filter primitives per filter. A result of every primitive is kept; real filters have 1 to 12
/// (Inkscape's drop shadow has 5), and 64 leaves headroom for generated ones.
const MAX_PRIMITIVES: usize = 64;
/// Region-sized buffers charged for a filter with a drop shadow on top of its results: it holds the
/// input, its blur and the shadow at once (`apply_drop_shadow`).
const DROP_SHADOW_EXTRA: f64 = 2.0;
/// Links of one clip-path chain, mask-on-mask chain or nested clip children with their own
/// clip-path. Each link keeps a layer-sized buffer alive while the next is applied; real files use
/// one or two, and 16 layers of a 24 MP canvas (1.5 GiB) is the whole budget anyway.
const MAX_LINKS: usize = 16;
/// Patterns drawn inside patterns. Each tile is alive while the inner one is drawn.
const MAX_PATTERN_DEPTH: usize = 8;
/// Embedded SVG images inside embedded SVG images: the parser's own limit. Each draws on a pixmap
/// of the enclosing layer, which the live-bytes check charges, so deep chains of small images are
/// harmless and chains of big ones stop at the budget long before this.
const MAX_IMAGE_DEPTH: usize = super::MAX_EMBEDDED_SVG_DEPTH;
/// Nested isolated groups, patterns, embedded images and filter images followed. It keeps this
/// walk, and resvg's own recursion, far from a stack overflow; real drawings stay under a few dozen.
const MAX_DEPTH: usize = 200;
/// Nodes the walk visits before it gives up as too complex to check. A node costs a few
/// nanoseconds, so this bounds the walk to well under a second.
const MAX_NODES: usize = 2_000_000;

/// Why a drawing was not drawn.
#[derive(Debug)]
pub(super) struct Refusal {
    /// Too many elements to check, rather than too much memory.
    pub(super) complex: bool,
    pub(super) message: String,
}

fn mib(bytes: f64) -> String {
    if bytes < 1e15 { format!("about {} MiB", (bytes / 1_048_576.0).ceil()) } else { "an enormous amount of memory".to_string() }
}

fn memory(what: &str, bytes: f64) -> Refusal {
    let limit = MAX_PEAK_PIXEL_BYTES >> 20;
    Refusal { complex: false, message: format!("{what} needs {}, over the {limit} MiB limit", mib(bytes)) }
}

fn limit(what: &str, value: usize) -> Refusal {
    Refusal { complex: false, message: format!("{what} (limit {value})") }
}

/// A pixel count that is never NaN: unknown sizes count as unbounded.
pub(super) fn px(v: f64) -> f64 {
    if v.is_nan() { f64::INFINITY } else { v }
}

/// What is alive where a node is drawn.
#[derive(Clone, Copy)]
struct Cx {
    /// Bytes held by the target and the layers of the ancestors.
    live: f64,
    /// Pixels of the pixmap being drawn on.
    layer: f64,
    /// Pixels of the largest group layer resvg makes in it (5 × 5 times its first target).
    cap: f64,
    depth: usize,
    patterns: usize,
    images: usize,
}

struct Walk {
    nodes: usize,
    /// Definitions already walked at a size: pointer and a size class (about 19 % wide).
    seen: HashSet<(usize, u32)>,
    clips: HashMap<usize, usize>,
    masks: HashMap<usize, usize>,
}

/// Check that drawing `nodes` at `ts` onto a `w × h` pixmap stays inside the budget, given
/// `held` bytes already in use (the target itself and anything the caller keeps).
pub(super) fn check(nodes: &[usvg::Node], ts: tiny_skia::Transform, w: u32, h: u32, held: u64) -> Result<(), Refusal> {
    let target = f64::from(w) * f64::from(h);
    let cx = Cx { live: held as f64 + target * 4.0, layer: target, cap: 25.0 * target, depth: 0, patterns: 0, images: 0 };
    let mut walk = Walk { nodes: 0, seen: HashSet::new(), clips: HashMap::new(), masks: HashMap::new() };
    walk.nodes(nodes, ts, cx)
}

fn class(live: f64) -> u32 {
    (live.max(1.0).log2() * 4.0) as u32
}

impl Walk {
    fn nodes(&mut self, nodes: &[usvg::Node], ts: tiny_skia::Transform, cx: Cx) -> Result<(), Refusal> {
        nodes.iter().try_for_each(|n| self.node(n, ts, cx))
    }

    fn node(&mut self, node: &usvg::Node, ts: tiny_skia::Transform, cx: Cx) -> Result<(), Refusal> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(Refusal { complex: true, message: format!("the drawing has more than {MAX_NODES} elements to check") });
        }
        match node {
            usvg::Node::Group(g) => self.group(g, ts, cx),
            usvg::Node::Text(t) => self.group(t.flattened(), ts, cx),
            usvg::Node::Path(p) if p.is_visible() => {
                for paint in p.fill().map(|f| f.paint()).into_iter().chain(p.stroke().map(|s| s.paint())) {
                    if let usvg::Paint::Pattern(pattern) = paint {
                        self.pattern(pattern, ts, cx)?;
                    }
                }
                Ok(())
            }
            usvg::Node::Path(_) => Ok(()),
            // resvg draws an embedded SVG on a pixmap the size of the current one, with its own 5 × 5 clamp.
            usvg::Node::Image(image) => {
                let usvg::ImageKind::SVG(tree) = image.kind() else { return Ok(()) };
                if cx.images >= MAX_IMAGE_DEPTH {
                    return Err(limit("embedded SVG images are nested too deeply", MAX_IMAGE_DEPTH));
                }
                let live = cx.live + cx.layer * 4.0;
                if live > BUDGET {
                    return Err(memory("an embedded SVG image", live));
                }
                let nested = Cx { live, cap: 25.0 * cx.layer, depth: cx.depth + 1, images: cx.images + 1, ..cx };
                self.deeper(nested)?;
                self.nodes(tree.root().children(), ts, nested)
            }
        }
    }

    fn deeper(&self, cx: Cx) -> Result<(), Refusal> {
        if cx.depth > MAX_DEPTH {
            return Err(limit("layers are nested too deeply", MAX_DEPTH));
        }
        Ok(())
    }

    fn group(&mut self, g: &usvg::Group, ts: tiny_skia::Transform, cx: Cx) -> Result<(), Refusal> {
        let ts = ts.pre_concat(g.transform());
        // A plain group is no layer, and nests at most as deep as the XML.
        if !g.should_isolate() {
            return self.nodes(g.children(), ts, cx);
        }
        let Some(bb) = g.layer_bounding_box().transform(ts) else { return Ok(()) };
        let own = px((f64::from(bb.width()).ceil() + 4.0) * (f64::from(bb.height()).ceil() + 4.0)).min(cx.cap);
        let inner = Cx { live: cx.live + own * 4.0, layer: own, depth: cx.depth + 1, ..cx };
        self.deeper(inner)?;
        // Filter, clip and mask are applied one after another; each needs its own extra buffers
        // beside the group's layer.
        let clip = g.clip_path().map_or(Ok(0), |c| self.clip_links(c, 0))?;
        let mask = g.mask().map_or(Ok(0), |m| self.mask_links(m, 0))?;
        let layers = 1 + clip.max(mask);
        if cx.live + own * 4.0 * layers as f64 > BUDGET {
            return Err(memory(&format!("a group with {layers} layers of {} px", own as u64), cx.live + own * 4.0 * layers as f64));
        }
        for filter in g.filters() {
            self.filter(filter, ts, cx, own)?;
        }
        let mut mask_ref = g.mask();
        while let Some(mask) = mask_ref {
            let key = (std::ptr::from_ref(mask) as usize, class(inner.live));
            if self.seen.insert(key) {
                self.nodes(mask.root().children(), ts, inner)?;
            }
            mask_ref = mask.mask();
        }
        self.nodes(g.children(), ts, inner)
    }

    /// Layers a clip-path keeps alive: its own, the deepest clip child with a clip-path of its own,
    /// or the next link of the chain, whichever is deepest. Nothing is walked twice.
    fn clip_links(&mut self, clip: &usvg::ClipPath, depth: usize) -> Result<usize, Refusal> {
        let key = std::ptr::from_ref(clip) as usize;
        if let Some(n) = self.clips.get(&key) {
            return Ok(*n);
        }
        let too_long = || limit("clip-paths are chained or nested too deeply", MAX_LINKS);
        if depth >= MAX_LINKS {
            return Err(too_long());
        }
        let inside = self.clip_children(clip.root(), depth + 1)?;
        let next = clip.clip_path().map_or(Ok(0), |c| self.clip_links(c, depth + 1))?;
        let n = 1 + inside.max(next);
        if n > MAX_LINKS {
            return Err(too_long());
        }
        self.clips.insert(key, n);
        Ok(n)
    }

    /// resvg's `clip_group`: a clip child with its own clip-path is drawn on a layer of its own,
    /// kept while its descendants are drawn and its clip is applied.
    fn clip_children(&mut self, group: &usvg::Group, depth: usize) -> Result<usize, Refusal> {
        let mut deepest = 0;
        for child in group.children() {
            let inner = match child {
                usvg::Node::Group(g) => {
                    let inside = self.clip_children(g, depth + usize::from(g.clip_path().is_some()))?;
                    match g.clip_path() {
                        Some(c) => 1 + inside.max(self.clip_links(c, depth + 1)?),
                        None => inside,
                    }
                }
                usvg::Node::Text(t) => self.clip_children(t.flattened(), depth)?,
                _ => 0,
            };
            if inner + depth > MAX_LINKS {
                return Err(limit("clip-paths are chained or nested too deeply", MAX_LINKS));
            }
            deepest = deepest.max(inner);
        }
        Ok(deepest)
    }

    /// Layers a mask chain keeps alive: a mask pixmap per link while the next is applied.
    fn mask_links(&mut self, mask: &usvg::Mask, depth: usize) -> Result<usize, Refusal> {
        let key = std::ptr::from_ref(mask) as usize;
        if let Some(n) = self.masks.get(&key) {
            return Ok(*n);
        }
        if depth >= MAX_LINKS {
            return Err(limit("masks are chained too deeply", MAX_LINKS));
        }
        let n = 1 + mask.mask().map_or(Ok(0), |m| self.mask_links(m, depth + 1))?;
        self.masks.insert(key, n);
        Ok(n)
    }

    /// A filter keeps a result per primitive, each as large as the filter region (or smaller), plus
    /// transients. `own` is the size of the layer it filters.
    fn filter(&mut self, f: &usvg::filter::Filter, ts: tiny_skia::Transform, cx: Cx, own: f64) -> Result<(), Refusal> {
        let prims = f.primitives().len();
        if prims > MAX_PRIMITIVES {
            return Err(limit("a filter has too many primitives", MAX_PRIMITIVES));
        }
        let Some(region) = f.rect().transform(ts) else { return Ok(()) };
        let region = px(f64::from(region.width()).ceil() * f64::from(region.height()).ceil());
        let shadow = f.primitives().iter().any(|p| matches!(p.kind(), usvg::filter::Kind::DropShadow(_)));
        let extra = if shadow { DROP_SHADOW_EXTRA } else { 0.0 };
        // The group's layer and the clone of it that `SourceGraphic` makes (`get_input`) are layer
        // sized; every primitive's result stays alive until the filter ends. A colour space
        // conversion only copies a result a later primitive still shares, which is counted already.
        let bytes = cx.live + own * 4.0 * 2.0 + region * 4.0 * (prims as f64 + extra);
        if bytes > BUDGET {
            return Err(memory(&format!("a filter with {prims} primitives over a region of {} px", region.min(1e18) as u64), bytes));
        }
        for primitive in f.primitives() {
            if let usvg::filter::Kind::Image(fe) = primitive.kind() {
                let live = cx.live + own * 4.0 + region * 4.0;
                let nested = Cx { live, layer: region, cap: 25.0 * region, depth: cx.depth + 1, ..cx };
                self.deeper(nested)?;
                if self.seen.insert((std::ptr::from_ref(fe) as usize, class(live))) {
                    self.nodes(fe.root().children(), ts, nested)?;
                }
            }
        }
        Ok(())
    }

    /// A pattern tile is `width × height × scale` pixels, whatever the target.
    fn pattern(&mut self, pattern: &usvg::Pattern, ts: tiny_skia::Transform, cx: Cx) -> Result<(), Refusal> {
        let (sx, sy) = ts.pre_concat(pattern.transform()).get_scale();
        let rect = pattern.rect();
        let (tw, th) = (f64::from(rect.width() * sx).round(), f64::from(rect.height() * sy).round());
        // resvg draws no tile smaller than a pixel (or not a number).
        if !(tw >= 1.0 && th >= 1.0) {
            return Ok(());
        }
        let tile = tw * th;
        let live = cx.live + tile * 4.0;
        if live > BUDGET {
            return Err(memory(&format!("a pattern tile of {tw} × {th} px"), live));
        }
        if cx.patterns >= MAX_PATTERN_DEPTH {
            return Err(limit("patterns are nested too deeply", MAX_PATTERN_DEPTH));
        }
        let nested = Cx { live, layer: tile, depth: cx.depth + 1, patterns: cx.patterns + 1, ..cx };
        self.deeper(nested)?;
        if self.seen.insert((std::ptr::from_ref(pattern) as usize, class(live))) {
            self.nodes(pattern.root().children(), tiny_skia::Transform::from_scale(sx, sy), nested)?;
        }
        Ok(())
    }
}
