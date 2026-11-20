use super::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    Move,
    Artboard,
    RectMarquee,
    EllipseMarquee,
    /// Selects one pixel row across the whole canvas at the clicked y (Photoshop's flyout only:
    /// no key, not in the ⇧M cycle).
    SingleRowMarquee,
    /// Selects one pixel column down the whole canvas at the clicked x.
    SingleColumnMarquee,
    Lasso,
    PolygonLasso,
    MagneticLasso,
    MagicWand,
    Crop,
    PerspectiveCrop,
    Eyedropper,
    ColorSampler,
    Ruler,
    Note,
    Count,
    Brush,
    Pencil,
    MixerBrush,
    ColorReplacement,
    Eraser,
    BackgroundEraser,
    MagicEraser,
    Gradient,
    PaintBucket,
    Type,
    VerticalType,
    /// Horizontal Type Mask: types a selection in the shape of the glyphs, no type layer.
    HorizontalTypeMask,
    VerticalTypeMask,
    Hand,
    RotateView,
    Zoom,
    Remove,
    SpotHealing,
    Healing,
    Patch,
    ContentAwareMove,
    RedEye,
    CloneStamp,
    PatternStamp,
    HistoryBrush,
    ArtHistoryBrush,
    Blur,
    Sharpen,
    Smudge,
    Dodge,
    Burn,
    Sponge,
    QuickSelection,
    ObjectSelection,
    Pen,
    FreeformPen,
    CurvaturePen,
    AddAnchorPoint,
    DeleteAnchorPoint,
    /// Converts an anchor between smooth and corner, and pulls handles out of a corner
    /// (Photoshop's Convert Point tool; also the Pen's ⌥ mode, `direct_select.rs`).
    ConvertPoint,
    PathSelection,
    DirectSelection,
    Rectangle,
    EllipseShape,
    Triangle,
    Polygon,
    /// The shape flyout's Star Tool (#11377): a live polygon shape with a star ratio.
    Star,
    Line,
    CustomShape,
    Slice,
    SliceSelect,
    Frame,
}

impl Tool {
    /// Stable identifier used by tool-selection commands.
    pub fn id(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Artboard => "artboard",
            Self::RectMarquee => "rectMarquee",
            Self::EllipseMarquee => "ellipseMarquee",
            Self::SingleRowMarquee => "singleRowMarquee",
            Self::SingleColumnMarquee => "singleColumnMarquee",
            Self::Lasso => "lasso",
            Self::PolygonLasso => "polygonLasso",
            Self::MagneticLasso => "magneticLasso",
            Self::MagicWand => "magicWand",
            Self::Crop => "crop",
            Self::PerspectiveCrop => "perspectiveCrop",
            Self::Eyedropper => "eyedropper",
            Self::ColorSampler => "colorSampler",
            Self::Ruler => "ruler",
            Self::Note => "note",
            Self::Count => "count",
            Self::Brush => "brush",
            Self::Pencil => "pencil",
            Self::MixerBrush => "mixerBrush",
            Self::ColorReplacement => "colorReplacement",
            Self::Eraser => "eraser",
            Self::BackgroundEraser => "backgroundEraser",
            Self::MagicEraser => "magicEraser",
            Self::Gradient => "gradient",
            Self::PaintBucket => "paintBucket",
            Self::Type => "type",
            Self::VerticalType => "verticalType",
            Self::HorizontalTypeMask => "horizontalTypeMask",
            Self::VerticalTypeMask => "verticalTypeMask",
            Self::Hand => "hand",
            Self::RotateView => "rotateView",
            Self::Zoom => "zoom",
            Self::Remove => "remove",
            Self::SpotHealing => "spotHealing",
            Self::Healing => "healing",
            Self::Patch => "patch",
            Self::ContentAwareMove => "contentAwareMove",
            Self::RedEye => "redEye",
            Self::CloneStamp => "cloneStamp",
            Self::PatternStamp => "patternStamp",
            Self::HistoryBrush => "historyBrush",
            Self::ArtHistoryBrush => "artHistoryBrush",
            Self::Blur => "blur",
            Self::Sharpen => "sharpen",
            Self::Smudge => "smudge",
            Self::Dodge => "dodge",
            Self::Burn => "burn",
            Self::Sponge => "sponge",
            Self::QuickSelection => "quickSelection",
            Self::ObjectSelection => "objectSelection",
            Self::Pen => "pen",
            Self::FreeformPen => "freeformPen",
            Self::CurvaturePen => "curvaturePen",
            Self::AddAnchorPoint => "addAnchorPoint",
            Self::DeleteAnchorPoint => "deleteAnchorPoint",
            Self::ConvertPoint => "convertPoint",
            Self::PathSelection => "pathSelection",
            Self::DirectSelection => "directSelection",
            Self::Rectangle => "rectangle",
            Self::EllipseShape => "ellipseShape",
            Self::Triangle => "triangle",
            Self::Polygon => "polygon",
            Self::Star => "star",
            Self::Line => "line",
            Self::CustomShape => "customShape",
            Self::Slice => "slice",
            Self::SliceSelect => "sliceSelect",
            Self::Frame => "frame",
        }
    }

    /// Resolve an exact tool identifier.
    pub fn from_id(id: &str) -> Option<Self> {
        static IDS: std::sync::OnceLock<std::collections::HashMap<&'static str, Tool>> = std::sync::OnceLock::new();
        IDS.get_or_init(|| Self::ALL.into_iter().filter(|tool| Self::from_name(tool.id()) == Some(*tool)).map(|tool| (tool.id(), tool)).collect())
            .get(id)
            .copied()
    }

    pub const ALL: [Tool; 69] = [
        Tool::Move,
        Tool::Artboard,
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::SingleRowMarquee,
        Tool::SingleColumnMarquee,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagneticLasso,
        Tool::MagicWand,
        Tool::Crop,
        Tool::PerspectiveCrop,
        Tool::Eyedropper,
        Tool::ColorSampler,
        Tool::Ruler,
        Tool::Note,
        Tool::Count,
        Tool::Brush,
        Tool::Pencil,
        Tool::ColorReplacement,
        Tool::MixerBrush,
        Tool::Eraser,
        Tool::BackgroundEraser,
        Tool::MagicEraser,
        Tool::Gradient,
        Tool::PaintBucket,
        Tool::Type,
        Tool::VerticalType,
        Tool::HorizontalTypeMask,
        Tool::VerticalTypeMask,
        Tool::Hand,
        Tool::RotateView,
        Tool::Zoom,
        Tool::SpotHealing,
        Tool::Remove,
        Tool::Healing,
        Tool::Patch,
        Tool::ContentAwareMove,
        Tool::RedEye,
        Tool::CloneStamp,
        Tool::PatternStamp,
        Tool::HistoryBrush,
        Tool::ArtHistoryBrush,
        Tool::Blur,
        Tool::Sharpen,
        Tool::Smudge,
        Tool::Dodge,
        Tool::Burn,
        Tool::Sponge,
        Tool::QuickSelection,
        Tool::ObjectSelection,
        Tool::Pen,
        Tool::FreeformPen,
        Tool::CurvaturePen,
        Tool::AddAnchorPoint,
        Tool::DeleteAnchorPoint,
        Tool::ConvertPoint,
        Tool::PathSelection,
        Tool::DirectSelection,
        Tool::Rectangle,
        Tool::EllipseShape,
        Tool::Triangle,
        Tool::Polygon,
        Tool::Star,
        Tool::Line,
        Tool::CustomShape,
        Tool::Slice,
        Tool::SliceSelect,
        Tool::Frame,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Move => "Move Tool",
            Tool::Artboard => "Artboard Tool",
            Tool::RectMarquee => "Rectangular Marquee Tool",
            Tool::EllipseMarquee => "Elliptical Marquee Tool",
            Tool::SingleRowMarquee => "Single Row Marquee Tool",
            Tool::SingleColumnMarquee => "Single Column Marquee Tool",
            Tool::Brush => "Brush Tool",
            Tool::Pencil => "Pencil Tool",
            Tool::MixerBrush => "Mixer Brush Tool",
            Tool::ColorReplacement => "Color Replacement Tool",
            Tool::Eraser => "Eraser Tool",
            Tool::BackgroundEraser => "Background Eraser Tool",
            Tool::MagicEraser => "Magic Eraser Tool",
            Tool::Eyedropper => "Eyedropper Tool",
            Tool::ColorSampler => "Color Sampler Tool",
            Tool::Ruler => "Ruler Tool",
            Tool::Note => "Note Tool",
            Tool::Count => "Count Tool",
            Tool::Lasso => "Lasso Tool",
            Tool::PolygonLasso => "Polygonal Lasso Tool",
            Tool::MagneticLasso => "Magnetic Lasso Tool",
            Tool::MagicWand => "Magic Wand Tool",
            Tool::Crop => "Crop Tool",
            Tool::PerspectiveCrop => "Perspective Crop Tool",
            Tool::Slice => "Slice Tool",
            Tool::SliceSelect => "Slice Select Tool",
            Tool::Frame => "Frame Tool",
            Tool::Gradient => "Gradient Tool",
            Tool::PaintBucket => "Paint Bucket Tool",
            Tool::Type => "Horizontal Type Tool",
            Tool::VerticalType => "Vertical Type Tool",
            Tool::HorizontalTypeMask => "Horizontal Type Mask Tool",
            Tool::VerticalTypeMask => "Vertical Type Mask Tool",
            Tool::Hand => "Hand Tool",
            Tool::RotateView => "Rotate View Tool",
            Tool::Zoom => "Zoom Tool",
            Tool::Remove => "Remove Tool",
            Tool::SpotHealing => "Spot Healing Brush Tool",
            Tool::Healing => "Healing Brush Tool",
            Tool::Patch => "Patch Tool",
            Tool::ContentAwareMove => "Content-Aware Move Tool",
            Tool::RedEye => "Red Eye Tool",
            Tool::CloneStamp => "Clone Stamp Tool",
            Tool::PatternStamp => "Pattern Stamp Tool",
            Tool::HistoryBrush => "History Brush Tool",
            Tool::ArtHistoryBrush => "Art History Brush Tool",
            Tool::Blur => "Blur Tool",
            Tool::Sharpen => "Sharpen Tool",
            Tool::Smudge => "Smudge Tool",
            Tool::Dodge => "Dodge Tool",
            Tool::Burn => "Burn Tool",
            Tool::Sponge => "Sponge Tool",
            Tool::QuickSelection => "Quick Selection Tool",
            Tool::ObjectSelection => "Object Selection Tool",
            Tool::Pen => "Pen Tool",
            Tool::FreeformPen => "Freeform Pen Tool",
            Tool::CurvaturePen => "Curvature Pen Tool",
            Tool::AddAnchorPoint => "Add Anchor Point Tool",
            Tool::DeleteAnchorPoint => "Delete Anchor Point Tool",
            Tool::ConvertPoint => "Convert Point Tool",
            Tool::PathSelection => "Path Selection Tool",
            Tool::DirectSelection => "Direct Selection Tool",
            Tool::Rectangle => "Rectangle Tool",
            Tool::EllipseShape => "Ellipse Tool",
            Tool::Triangle => "Triangle Tool",
            Tool::Polygon => "Polygon Tool",
            Tool::Star => "Star Tool",
            Tool::Line => "Line Tool",
            Tool::CustomShape => "Custom Shape Tool",
        }
    }
    pub fn is_type(self) -> bool {
        matches!(self, Self::Type | Self::VerticalType | Self::HorizontalTypeMask | Self::VerticalTypeMask)
    }
    /// The Type Mask tools: their text becomes a selection on commit.
    pub fn is_type_mask(self) -> bool {
        matches!(self, Self::HorizontalTypeMask | Self::VerticalTypeMask)
    }
    /// Type tools that lay text out vertically.
    pub fn is_vertical_type(self) -> bool {
        matches!(self, Self::VerticalType | Self::VerticalTypeMask)
    }

    /// Retouching and painting tools that stroke with the brush (share the brush cursor and chip).
    pub fn is_brushlike(self) -> bool {
        matches!(
            self,
            Tool::Brush
                | Tool::Pencil
                | Tool::MixerBrush
                | Tool::ColorReplacement
                | Tool::Eraser
                | Tool::BackgroundEraser
                | Tool::Remove
                | Tool::SpotHealing
                | Tool::Healing
                | Tool::CloneStamp
                | Tool::PatternStamp
                | Tool::HistoryBrush
                | Tool::ArtHistoryBrush
                | Tool::Blur
                | Tool::Sharpen
                | Tool::Smudge
                | Tool::Dodge
                | Tool::Burn
                | Tool::Sponge
        )
    }
    /// Photoshop default single-key shortcut (`'\0'` = none, e.g. the Blur group).
    pub fn key(self) -> char {
        match self {
            Tool::Move | Tool::Artboard => 'V',
            Tool::RectMarquee | Tool::EllipseMarquee => 'M',
            Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::ColorReplacement => 'B',
            Tool::Eraser | Tool::BackgroundEraser | Tool::MagicEraser => 'E',
            Tool::Eyedropper | Tool::ColorSampler | Tool::Ruler | Tool::Note | Tool::Count => 'I',
            Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso => 'L',
            Tool::MagicWand => 'W',
            Tool::Crop | Tool::PerspectiveCrop | Tool::Slice | Tool::SliceSelect => 'C',
            Tool::Frame => 'K',
            Tool::Gradient | Tool::PaintBucket => 'G',
            Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => 'T',
            Tool::Hand => 'H',
            Tool::RotateView => 'R',
            Tool::Zoom => 'Z',
            Tool::Remove | Tool::SpotHealing | Tool::Healing | Tool::Patch | Tool::ContentAwareMove | Tool::RedEye => 'J',
            Tool::CloneStamp | Tool::PatternStamp => 'S',
            Tool::HistoryBrush | Tool::ArtHistoryBrush => 'Y',
            Tool::Blur | Tool::Sharpen | Tool::Smudge => '\0',
            // In the Marquee flyout, but M cycles only the Rectangular and Elliptical Marquee.
            Tool::SingleRowMarquee | Tool::SingleColumnMarquee => '\0',
            Tool::Dodge | Tool::Burn | Tool::Sponge => 'O',
            Tool::QuickSelection | Tool::ObjectSelection => 'W',
            Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen => 'P',
            // In the Pen flyout, but P never reaches them.
            Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint => ' ',
            Tool::PathSelection | Tool::DirectSelection => 'A',
            Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Star | Tool::Line | Tool::CustomShape => 'U',
        }
    }
    /// Glyph drawn in the toolbar (vector icons come later).
    pub fn glyph(self) -> &'static str {
        match self {
            Tool::Move => "✥",
            Tool::RectMarquee => "⬚",
            Tool::EllipseMarquee => "◌",
            Tool::Brush => "🖌",
            Tool::MixerBrush => "🖌",
            Tool::Eraser => "⌫",
            Tool::Eyedropper => "💧",
            Tool::ColorSampler => "◎",
            Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso => "L",
            Tool::MagicWand => "W",
            Tool::Crop => "C",
            Tool::Gradient | Tool::PaintBucket => "G",
            Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => "T",
            Tool::Hand => "✋",
            Tool::RotateView => "🧭",
            Tool::Zoom => "🔍",
            _ => "•",
        }
    }
    /// The tool `s` names: its variant name, that name without "marquee", or its label, in any case and with spaces,
    /// `_` and `-` ignored. The first tool of [`Tool::ALL`] a spelling names wins.
    pub fn from_name(s: &str) -> Option<Tool> {
        // Panels resolve tool names per row as they draw, and the command gate per tool command: one table instead of
        // formatting every tool's names per lookup.
        static NAMES: std::sync::OnceLock<std::collections::HashMap<String, Tool>> = std::sync::OnceLock::new();
        let names = NAMES.get_or_init(|| {
            let mut names = std::collections::HashMap::new();
            for t in Tool::ALL {
                let a = format!("{t:?}").to_ascii_lowercase();
                for spelling in [a.replace("marquee", ""), t.label().to_ascii_lowercase().replace(' ', ""), a] {
                    names.entry(spelling).or_insert(t);
                }
            }
            names
        });
        names.get(&s.to_ascii_lowercase().replace([' ', '_', '-'], "")).copied()
    }
}
