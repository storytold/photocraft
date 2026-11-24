use super::{Deserialize, Serialize};

use craft::behavior::geometry::TransformMode;
use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{Point, Rect};
use craft::draw::ui::transform::{TransformSession, TransformWarp};
use craft::imaging::ui::warp::WarpMesh;
use photocraft_algo::transform::Homography;
use serde_json::{Value, json};

/// The source frame as `[x0, y0, x1, y1]`, the form the engine commands and the record take.
fn rect_array(r: Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// A box's corners as `[[x, y]; 4]`, the form the engine commands and the record take.
fn quad_array(q: [Point; 4]) -> [[f64; 2]; 4] {
    q.map(|p| [p.x, p.y])
}

/// The camelCase name the record gives a mode.
fn mode_name(mode: TransformMode) -> &'static str {
    match mode {
        TransformMode::Rotate => "rotate",
        TransformMode::Skew => "skew",
        TransformMode::Distort => "distort",
        TransformMode::Perspective => "perspective",
        _ => "free",
    }
}

/// The app's warp as the framework session holds it in the Warp phase: PhotoCraft's [`Warp`](photocraft_geom::warp::Warp)
/// with the control mesh the imaging warp grid edits, copied once when the warp is made.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionWarp {
    /// The warp the engine commands take.
    pub warp: photocraft_geom::warp::Warp,
    /// The same warp's control mesh in the imaging library's form.
    pub mesh: WarpMesh,
    /// The warp is Warp Text on an editable type layer: committing it changes the style even when it is the
    /// identity warp (a Warp Text set back to None).
    pub on_type: bool,
    /// The warp is a smart object's stored warp, edited over its unwarped source: the session's frame is the
    /// warp's bounds, not the render's.
    pub stored: bool,
}

impl SessionWarp {
    /// The session warp of `warp`; `on_type` for Warp Text.
    pub fn new(warp: photocraft_geom::warp::Warp, on_type: bool) -> Self {
        let source = warp.to_mesh(1, 1);
        let points = source.points.iter().map(|p| Point::new(p[0], p[1])).collect();
        let bounds = Rect::new(warp.bounds[0], warp.bounds[1], warp.bounds[2], warp.bounds[3]);
        let mesh = WarpMesh::new(source.us, source.vs, points).unwrap_or_else(|| WarpMesh::identity(bounds, 1, 1));
        Self { warp, mesh, on_type, stored: false }
    }
}

impl SessionWarp {
    /// This warp as a smart object's stored warp ([`stored`](Self::stored)).
    pub fn stored(mut self) -> Self {
        self.stored = true;
        self
    }
}

impl TransformWarp for SessionWarp {
    fn frame(&self) -> Option<Rect> {
        self.stored.then(|| Rect::new(self.warp.bounds[0], self.warp.bounds[1], self.warp.bounds[2], self.warp.bounds[3]))
    }

    fn is_identity(&self) -> bool {
        !self.on_type && self.warp.is_identity()
    }
}

/// The box a smart object opens on: its corners where its placement puts the source's own rect,
/// with the reference point at their centre (the image of the rect's centre). The Free Transform
/// session and the Move tool's controls both start from it.
pub fn placed_state(quad: [[f64; 2]; 4]) -> BoxState {
    let points = quad.map(|[x, y]| Point::new(x, y));
    let centre = Point::new(points.iter().map(|p| p.x).sum::<f64>() / 4.0, points.iter().map(|p| p.y).sum::<f64>() / 4.0);
    BoxState::new(points, centre)
}

/// A smart object's box that keeps referencing the object's own bounds (its source rect, as the
/// session's frame), so it reopens aligned to a rotated object and its fields are relative to the
/// original placement. The session's quad is then the placement of the source rect, in document
/// space, while the preview texture still covers the document-space rect [`frame`](Self::frame).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    /// Document to source: the placement the box opened with, inverted.
    pub inverse: Homography,
    /// The box as it opened. The session is untouched while its quad is this.
    pub quad: [Point; 4],
    /// The axis-aligned rect the preview texture covers, in document space.
    pub frame: Rect,
}

/// What the document-space map of a placed box does to points: the box's changes since it opened.
#[derive(Clone, Copy, Debug)]
pub struct DocMap {
    h: Homography,
    frame: [f64; 4],
}

impl DocMap {
    /// `p` carried through the map.
    pub fn apply(&self, p: Point) -> Point {
        let (x, y) = self.h.apply(p.x, p.y);
        Point::new(x, y)
    }

    /// [`apply`](Self::apply) for a point on the front side of the map's horizon (the side the
    /// frame lies on); `None` behind it.
    pub fn apply_in_front(&self, p: Point) -> Option<Point> {
        (self.h.w(p.x, p.y) * self.h.front_side(self.frame) > 0.0).then(|| self.apply(p))
    }
}

/// Free Transform in progress: the framework session (box, mode, warp, undo steps) and what PhotoCraft
/// adds to say what the box moves.
#[derive(Clone, Debug)]
pub struct FreeTransform {
    /// The box, its mode, the Warp phase and the session's own undo steps.
    pub session: TransformSession<SessionWarp>,
    /// The session's id (the canvas key of its preview document derives from it).
    pub id: u64,
    /// The active layer.
    pub layer: u64,
    /// Free Transform of several selected layers (#6501): every layer the box moves, in
    /// `edit.transform`'s `layers` order. Empty when it moves `layer` alone. `layer` stays the
    /// active layer, which need not be one of them (a selected group carries its selected
    /// children).
    pub layers: Vec<u64>,
    /// Select › Transform Selection: the box transforms the selection outline, not pixels.
    pub selection: bool,
    /// `edit.transform`'s `"target"` when the box moves an unlinked layer mask, an alpha channel or
    /// the Quick Mask by itself (`None`: the layer, with its linked masks).
    pub target: Option<Value>,
    /// Free Transform Path: `path.transform`'s `name` (and `layer`) when the box moves a
    /// path's anchors and handles instead of pixels.
    pub path: Option<Value>,
    /// The layer was made for this session (⌥⌘T's copy, #352; a file dropped on the canvas), so
    /// Cancel takes it back and OK folds it into one history step with the transform.
    pub made: Option<MadeLayer>,
    /// Edit › Content-Aware Scale (#3096): the box only scales and moves, and OK runs
    /// `edit.contentAwareScale` with these options instead of `edit.transform`.
    pub content_aware: Option<ContentAwareOptions>,
    /// A smart object's box over its own bounds ([`Placed`]); `None` for the document-space box every
    /// other layer gets.
    pub placed: Option<Placed>,
}

impl FreeTransform {
    /// The box has not been changed: closing it leaves the document as it was. A placed box starts
    /// away from its frame's corners, on the object, so it is compared with where it opened.
    pub fn untouched(&self) -> bool {
        match (&self.placed, self.session.warp()) {
            (Some(p), None) => self.session.quad() == p.quad,
            _ => self.session.is_untouched(),
        }
    }

    /// For a placed box, the document-space map of what the box did since it opened: the object's
    /// source goes through the new placement, having come from the old one. `None` for any other box
    /// and for a degenerate one.
    pub fn doc_map(&self) -> Option<DocMap> {
        let p = self.placed.as_ref()?;
        let s = self.session.homography()?;
        let [a, b, c] = [s.row(0), s.row(1), s.row(2)];
        let now = Homography([a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]);
        let h = now.mul(&p.inverse);
        h.0.iter().all(|v| v.is_finite()).then_some(DocMap { h, frame: [p.frame.x0, p.frame.y0, p.frame.x1, p.frame.y1] })
    }

    /// The source frame as `[x0, y0, x1, y1]`.
    pub fn rect_array(&self) -> [f64; 4] {
        rect_array(self.session.rect())
    }

    /// The box's corners, clockwise from the top left.
    pub fn quad_array(&self) -> [[f64; 2]; 4] {
        quad_array(self.session.quad())
    }

    /// The session as the control channel and the `ui` dump show it.
    pub fn record(&self) -> Value {
        let s = &self.session;
        let pivot = s.pivot();
        json!({
            "session": self.id,
            "layer": self.layer,
            "rect": rect_array(s.rect()),
            "quad": quad_array(s.quad()),
            "pivot": [pivot.x, pivot.y],
            "interpolation": s.interpolation().id(),
            "warp": s.warp().map(|w| &w.warp),
            "selection": self.selection,
            "target": self.target,
            "path": self.path,
            "made": self.made,
            "mode": mode_name(s.current_mode()),
            "contentAware": self.content_aware,
            "skewV": s.skew_v(),
            "layers": self.layers,
        })
    }

    /// Sets the box through the session as a typed field would: one undo step.
    #[cfg(test)]
    pub(crate) fn edit_box(&mut self, f: impl FnOnce(&mut craft::behavior::geometry::BoxState)) {
        use craft::behavior::{EditEvent, Mixed};
        use craft::draw::ui::transform::TransformEdit;
        let before = *self.session.state();
        let mut after = before;
        f(&mut after);
        let label: std::sync::Arc<str> = std::sync::Arc::from("Edit box");
        self.session.apply(TransformEdit::Box(EditEvent::set(Mixed::Same(before), after, label, None)), 0.0);
    }
}

impl Serialize for FreeTransform {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.record().serialize(serializer)
    }
}

impl PartialEq for FreeTransform {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.record() == other.record()
    }
}

/// Content-Aware Scale's options bar: Amount, Protect and Protect Skin Tones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAwareOptions {
    /// How much is content-aware, 0..100 % (the rest is a plain resample).
    pub amount: f32,
    /// The alpha channel whose white areas keep their shape (`None`: no protection).
    #[serde(default)]
    pub protect: Option<String>,
    #[serde(default)]
    pub protect_skin_tones: bool,
}

impl Default for ContentAwareOptions {
    fn default() -> Self {
        Self { amount: 100.0, protect: None, protect_skin_tones: false }
    }
}

/// Why a Free Transform session's layer was made for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MadeLayer {
    /// Free Transform on a copy (⌥⌘T): OK makes the copy and the transform one Free Transform step.
    Copy,
    /// A file dropped on the canvas: OK makes the place and the transform one Place Embedded step.
    Place,
}

/// Content-Aware Move with Transform On Drop: the box on the dropped patch. The document is
/// unchanged until it is committed; this is view state.
#[derive(Clone, Debug)]
pub struct CamTransform {
    pub document: u64,
    pub revision: u64,
    /// How far the selection was dragged; the session's frame is the selection's bounds there.
    pub offset: [i32; 2],
    /// The box over the dropped patch.
    pub session: TransformSession<()>,
}

impl CamTransform {
    /// The box as the control channel and the `ui` dump show it.
    pub fn record(&self) -> Value {
        let pivot = self.session.pivot();
        json!({
            "document": self.document,
            "revision": self.revision,
            "offset": self.offset,
            "rect": rect_array(self.session.rect()),
            "quad": quad_array(self.session.quad()),
            "pivot": [pivot.x, pivot.y],
        })
    }
}

impl Serialize for CamTransform {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.record().serialize(serializer)
    }
}

impl PartialEq for CamTransform {
    fn eq(&self, other: &Self) -> bool {
        self.record() == other.record()
    }
}

/// View-menu overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
#[serde(default)]
pub struct Extras {
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub snap: bool,
    pub lock_guides: bool,
}

impl Default for Extras {
    fn default() -> Self {
        Self { rulers: false, grid: false, guides: true, snap: true, lock_guides: false }
    }
}
