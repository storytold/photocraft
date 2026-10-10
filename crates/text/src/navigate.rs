//! Caret movement, caret geometry and selection shapes for laid-out type (stage 1b).
//!
//! **Approach C** (spec §3): parley decides the *order* of the caret stops on a line, through its
//! bidi-aware visual cursor, and PhotoCraft's [`crate::ClusterInfo`] decides their *geometry*:
//! the layout moves glyphs after parley (alignment, indents, kerning, "Justify all") without
//! changing their visual order. Lines are crossed here rather than by parley, whose cursor leaves
//! a line left to right and top to bottom whatever the paragraph direction (backwards in RTL).
//!
//! Positions are byte offsets in the layer text; geometry is in line space (see
//! [`crate::TextLayout::to_text`]). Pure functions of a layout and its text, shared by the Type
//! tool and the engine's `type.*` caret commands. Nothing here panics: indices go through `get`,
//! and every loop is bounded.

use parley::Layout;

use crate::layout::RunBrush;

/// A paragraph's parley layout, kept for caret movement (hidden lines of an overflowing box
/// included). parley's text equals the layer's `start..end` after a `prefix`-byte LRM/RLM (0 or 3
/// bytes, from the paragraph direction), so `parley offset = layer offset − start + prefix`.
#[derive(Clone, Debug)]
pub(crate) struct ParagraphNav {
    pub(crate) layout: Layout<RunBrush>,
    /// Layer byte range of the paragraph's text, without its break.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Length of the direction mark parley's text starts with.
    pub(crate) prefix: usize,
}
