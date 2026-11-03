use super::{Deserialize, Serialize};

/// Edit › Transform's mode: what a handle drag does with no modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformMode {
    /// Free Transform and Scale: handles scale, drag outside rotates.
    #[default]
    Free,
    /// Edit › Transform › Rotate: handle drags rotate about the reference point (#3941).
    Rotate,
    /// Edge and corner handles skew: each moves along one axis.
    Skew,
    /// Corner handles move freely, one at a time; nothing else moves them (no rotating, no
    /// edges) and nothing snaps.
    Distort,
    /// Corner handles move in pairs, mirrored (one-point perspective).
    Perspective,
}

impl TransformMode {
    /// The mode an `edit.transform.*` / `edit.freeTransform` menu id starts.
    pub fn for_command(id: &str) -> Self {
        match id {
            "edit.transform.rotate" => Self::Rotate,
            "edit.transform.skew" => Self::Skew,
            "edit.transform.distort" => Self::Distort,
            "edit.transform.perspective" => Self::Perspective,
            _ => Self::Free,
        }
    }
}

/// Free Transform in progress: the source frame `rect` and where its corners currently are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformSession {
    pub session: u64,
    pub layer: u64,
    pub rect: [f64; 4],
    /// Corners clockwise from top-left (document px).
    pub quad: [[f64; 2]; 4],
    /// Reference point (rotation / ⌥-scale centre).
    pub pivot: [f64; 2],
    pub interpolation: String,
    /// Warp mode (Edit › Transform › Warp): the warp being edited over `rect`, in document px.
    #[serde(default)]
    pub warp: Option<photocraft_geom::warp::Warp>,
    /// Select › Transform Selection: the box transforms the selection outline, not pixels.
    #[serde(default)]
    pub selection: bool,
    /// `edit.transform`'s `"target"` when the box moves an unlinked layer mask, an alpha channel or
    /// the Quick Mask by itself (`None`: the layer, with its linked masks).
    #[serde(default)]
    pub target: Option<serde_json::Value>,
    /// Free Transform Path: `path.transform`'s `name` (and `layer`) when the box moves a
    /// path's anchors and handles instead of pixels.
    #[serde(default)]
    pub path: Option<serde_json::Value>,
    /// The layer was made for this session (⌥⌘T's copy, #352; a file dropped on the canvas), so
    /// Cancel takes it back and OK folds it into one history step with the transform.
    #[serde(default)]
    pub made: Option<MadeLayer>,
    /// Edit › Transform › Rotate / Skew / Distort / Perspective (`Free` for Free Transform and Scale).
    #[serde(default)]
    pub mode: TransformMode,
    /// Edit › Content-Aware Scale (#3096): the box only scales and moves, and OK runs
    /// `edit.contentAwareScale` with these options instead of `edit.transform`.
    #[serde(default)]
    pub content_aware: Option<ContentAwareOptions>,
    /// The options bar's V (vertical skew, degrees) last typed. Any box can be read as scale,
    /// skew and rotation with any vertical skew, so the bar keeps showing the typed one.
    #[serde(default)]
    pub skew_v: f64,
    /// Free Transform of several selected layers (#6501): every layer the box moves, in
    /// `edit.transform`'s `layers` order. Empty when it moves `layer` alone. `layer` stays the
    /// active layer, which need not be one of them (the Background is left out, a selected group
    /// carries its selected children).
    #[serde(default)]
    pub layers: Vec<u64>,
    /// Where the box opened when that isn't `rect`'s corners: a smart object's placement, rotated,
    /// skewed or distorted, as Photoshop keeps it (#3649). `rect` is then a frame of the same
    /// size, so W and H start at 100 % and the angle and skew read the placement's; the commit
    /// transforms from here to `quad`.
    #[serde(default)]
    pub start: Option<[[f64; 2]; 4]>,
    /// Warp mode on a smart object: `warp` is in the contents' own space, and this (row-major 3×3,
    /// contents → document) is where the placement and the box's move put it on screen, as
    /// Photoshop shows a smart object's warp through its Distort (#8927). `None`: `warp` is in
    /// document space.
    #[serde(default)]
    pub warp_place: Option<[f64; 9]>,
    /// A warp made in Warp mode and kept when the box went back to another mode before Enter (one
    /// transform session, as in Photoshop): the box then transforms the warped result. In the same
    /// space as `warp` was (a smart object's contents, with `held_place` its placement; else
    /// document space). Enter applies both in one step.
    #[serde(default)]
    pub held_warp: Option<photocraft_geom::warp::Warp>,
    #[serde(default)]
    pub held_place: Option<[f64; 9]>,
    /// Warp mode: the warp it opened with (leaving with another one holds it, `held_warp`) and
    /// the box as it was before (an untouched leave brings it back). In the session, so the
    /// box's own Undo restores them with the rest.
    #[serde(default)]
    pub warp_entry: Option<photocraft_geom::warp::Warp>,
    #[serde(default)]
    pub warp_box: Option<WarpBox>,
    /// Warp mode on a smart object: its stored warp and placement when Warp mode opened, which
    /// the commit compares with and the selection outline is moved from.
    #[serde(default)]
    pub warp_origin: Option<(photocraft_geom::warp::Warp, [f64; 9])>,
}

/// How many patches per side [`TransformSession::held_doc_warp`] cuts a held warp into under a
/// projective box.
pub const HELD_SPLITS: usize = 16;

/// The box as it was when Warp mode took over ([`TransformSession::warp_box`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpBox {
    pub rect: [f64; 4],
    pub quad: [[f64; 2]; 4],
    pub start: Option<[[f64; 2]; 4]>,
    pub pivot: [f64; 2],
}

impl TransformSession {
    /// A point of the warp being edited, where it shows in the document ([`Self::warp_place`]).
    pub fn warp_to_doc(&self, p: [f64; 2]) -> [f64; 2] {
        match self.warp_place.map(photocraft_algo::transform::Homography) {
            Some(h) => {
                let (x, y) = h.apply(p[0], p[1]);
                [x, y]
            }
            None => p,
        }
    }

    /// A document point in the warp's own space (the inverse of [`Self::warp_to_doc`]).
    pub fn doc_to_warp(&self, p: [f64; 2]) -> [f64; 2] {
        match self.warp_place.and_then(|m| photocraft_algo::transform::Homography(m).inverse()) {
            Some(h) => {
                let (x, y) = h.apply(p[0], p[1]);
                [x, y]
            }
            None => p,
        }
    }

    /// The box's move since it opened (identity when it can't be inverted).
    pub fn box_move(&self) -> photocraft_algo::transform::Homography {
        use photocraft_algo::transform::Homography;
        Homography::quad_to_quad(self.start_quad(), self.quad).unwrap_or(Homography::IDENTITY)
    }

    /// Where a smart object's held warp ([`Self::held_warp`], in its contents' space) shows a
    /// point now: through its placement, then the box's move since it opened.
    pub fn held_to_doc(&self) -> impl Fn([f64; 2]) -> [f64; 2] + use<> {
        use photocraft_algo::transform::Homography;
        let h = self.box_move().mul(&Homography(self.held_place.unwrap_or(Homography::IDENTITY.0)));
        move |p| {
            let (x, y) = h.apply(p[0], p[1]);
            [x, y]
        }
    }

    /// A layer's warp held under the box ([`Self::held_warp`] with no placement), as the box
    /// leaves it, in document space: what Enter applies, and what the preview and the selection
    /// outline show. An affine box maps the mesh's control points exactly; a Distort or
    /// Perspective bends each patch, so the mesh is cut into [`HELD_SPLITS`]² finer patches first
    /// and those are mapped, close enough to the curve that it shows as one.
    pub fn held_doc_warp(&self) -> Option<photocraft_geom::warp::Warp> {
        let w = self.held_warp.as_ref().filter(|_| self.held_place.is_none())?;
        let d = self.box_move();
        let mut mesh = w.to_mesh(1, 1);
        let m = d.0;
        if (m[6].abs() > 1e-12 || m[7].abs() > 1e-12) && mesh.us.len() <= HELD_SPLITS + 1 {
            for k in 1..HELD_SPLITS {
                let at = k as f64 / HELD_SPLITS as f64;
                let _ = mesh.split_u(at);
                let _ = mesh.split_v(at);
            }
        }
        let mesh = mesh.map_points(|q| {
            let (x, y) = d.apply(q[0], q[1]);
            [x, y]
        });
        Some(photocraft_geom::warp::Warp::custom(mesh, w.bounds))
    }

    /// Document pixels per warp-space pixel near document point `p`: hit tolerances are given
    /// in document pixels.
    pub fn warp_scale_at(&self, p: [f64; 2]) -> f64 {
        1.0 / self.warp_unit(self.doc_to_warp(p))
    }

    /// How long one document pixel is in the warp's space at warp-space point `q`: 1, or for a
    /// smart object's warp the inverse of its placement's scale there, so hit tolerances and
    /// snapping distances stay the same on screen whatever the contents' size.
    pub fn warp_unit(&self, q: [f64; 2]) -> f64 {
        let Some(h) = self.warp_place.map(photocraft_algo::transform::Homography) else { return 1.0 };
        let s = h.local_scale(q[0], q[1]);
        if s.is_finite() && s > 1e-9 { 1.0 / s } else { 1.0 }
    }

    /// The corners the box opened on: [`Self::start`], else `rect`'s. A box still there is
    /// untouched.
    pub fn start_quad(&self) -> [[f64; 2]; 4] {
        let r = self.rect;
        self.start.unwrap_or([[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]])
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

/// A paragraph-box handle drag: the handle, and the box (text-space) and layer transform as
/// they were at the press. Every move recomputes from this snapshot, so ⌥ pressed or released
/// mid-drag mirrors about the centre the box started with — as if it had been held from the
/// start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxResize {
    pub handle: u8,
    pub shape: [f32; 4],
    pub aff: photocraft_geom::Affine,
}

/// A font choice for subsequent typing, invalidated by navigation or a document edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeInsertionStyle {
    pub document: u64,
    pub layer: u64,
    pub session: String,
    pub revision: u64,
    pub caret: usize,
    pub style: photocraft_doc::text::CharStyle,
}

/// In-progress inline type editing (Type tool). Offsets are character indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextEdit {
    pub layer: u64,
    /// Id of the document being edited (layer ids can repeat across documents, e.g. one .pcraft
    /// opened twice). `None`: the active document.
    #[serde(default)]
    pub document: Option<u64>,
    pub caret: usize,
    pub anchor: usize,
    /// History coalescing key: the whole editing session is one "Edit Type Layer" step.
    pub session: String,
    /// The layer was created by this session (its name follows the text; empty on commit = delete).
    pub created: bool,
    /// A Type Mask tool session: the layer is temporary and commits as a selection.
    #[serde(default)]
    pub mask: bool,
    #[serde(skip)]
    pub dragging: bool,
    /// Paragraph-box handle being dragged (0-3 corners from top-left clockwise, 4-7 top/right/bottom/left edges).
    #[serde(skip)]
    pub resize: Option<BoxResize>,
    /// IME composition in progress: (start, length) in characters. The preedit text lives in the
    /// layer so it lays out like typed text; each IME update replaces it.
    #[serde(skip)]
    pub preedit: Option<(usize, usize)>,
}

/// Temporary Type-tool drag. The document is unchanged until release; this frame is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeTransform {
    pub document: u64,
    pub revision: u64,
    pub original: photocraft_geom::Affine,
    pub frame: TransformSession,
    pub(crate) gesture: crate::transform_tool::Gesture,
}

/// Content-Aware Move with Transform On Drop: the box on the dropped patch. The document is
/// unchanged until it is committed; this is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CamTransform {
    pub document: u64,
    pub revision: u64,
    /// How far the selection was dragged; the box's `rect` is the selection's bounds there.
    pub offset: [i32; 2],
    pub frame: TransformSession,
    #[serde(skip)]
    pub(crate) gesture: Option<crate::transform_tool::Gesture>,
}

/// View-menu overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
