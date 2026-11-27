//! The Type tool's session state: the open text edit, the pending insertion style and the temporary transform.

use serde::{Deserialize, Serialize};

/// A font choice for subsequent typing, invalidated by navigation or a document edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeInsertionStyle {
    /// The document the choice was made in.
    pub document: u64,
    /// The edited layer.
    pub layer: u64,
    /// The editing session's history key.
    pub session: String,
    /// The document revision the choice holds for.
    pub revision: u64,
    /// The caret the choice holds at, in characters.
    pub caret: usize,
    /// The style the next typed characters take.
    pub style: photocraft_doc::text::CharStyle,
}

/// How the commit of a Type Mask combines with the selection that was already there when the mask
/// started (#12508). The names are the `operation` values `type.createSelection` takes for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
#[serde(rename_all = "lowercase")]
pub enum MaskSelectionMode {
    /// ⇧ at the click that started the mask: its glyphs join the selection.
    Add,
    /// ⌥ at the start of the drag that started the mask: its glyphs are cut out of the selection.
    Subtract,
}

/// In-progress inline type editing (Type tool). Offsets are character indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
pub struct TextEdit {
    /// The edited layer.
    pub layer: u64,
    /// Id of the document being edited (layer ids can repeat across documents, e.g. one .pcraft
    /// opened twice). `None`: the active document.
    #[serde(default)]
    pub document: Option<u64>,
    /// The caret, the end of the selection that moves.
    pub caret: usize,
    /// The end of the selection that stays.
    pub anchor: usize,
    /// History coalescing key: the whole editing session is one "Edit Type Layer" step.
    pub session: String,
    /// The layer was created by this session (its name follows the text; empty on commit = delete).
    pub created: bool,
    /// A Type Mask tool session: the layer is temporary and commits as a selection.
    #[serde(default)]
    pub mask: bool,
    /// How this mask's commit combines with the selection that was already there when the mask
    /// started (#12508): ⇧ at the click adds its glyphs, ⌥ at the start of a drag subtracts them.
    /// Read once, from the press, when the release opens the session. `None` replaces the selection,
    /// the rule #12073 measured and what every other start keeps.
    #[serde(default)]
    pub mask_start_mode: Option<MaskSelectionMode>,
}

/// Temporary Type-tool transform. The document is unchanged until it commits; this is view state.
#[derive(Clone, Debug)]
pub struct TypeTransform {
    /// The document the transform was started in.
    pub document: u64,
    /// The document revision it was started at.
    pub revision: u64,
    /// The layer's transform when the session began.
    pub original: photocraft_geom::Affine,
    /// The framework's transform session over the layer's text frame.
    pub session: craft::draw::ui::transform::TransformSession<()>,
    /// The capture began on the move reach of the hidden box, which keeps it to the release.
    pub from_reach: bool,
}

// The framework session has no `PartialEq`: the fields that decide what is shown are compared.
impl PartialEq for TypeTransform {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
            && self.revision == other.revision
            && self.original == other.original
            && self.session.rect() == other.session.rect()
            && self.session.state() == other.session.state()
    }
}
