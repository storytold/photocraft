//! The tool rail's commands: one `toolSelect.<tool>` command per [`Tool`], each also answering to the production id
//! `tools.select.<tool>` (saved shortcuts, `.kys` files, protocol calls), and the rail's slots built from them.

use craft::ui::contract::CommandSet;
use craft::ui::rails::{RailTool, SlotId, ToolSlot};
use photocraft_engine::prefs::ToolbarCustomization;

use super::toolbar::layout_sections;
use crate::state::Tool;

/// Selecting a tool, as a command: the rail's buttons and the Edit Toolbar drawer's rows run it, and the application
/// routes a fired one to `tool_memory::select`. A tool's letter is the command's default key and its group, so the rail's tooltips and the menus show it; the
/// tools of one letter cycle through `shortcut_dispatch::run_fired`, and the Shift+letter step is `tool_shortcuts`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "toolSelect")]
pub(crate) enum ToolCommand {
    #[command(label = "Move Tool", icon = "move", kind = "tool", group = "V", default = "V", alias = "tools.select.move")]
    Move,
    #[command(label = "Artboard Tool", icon = "artboard", kind = "tool", group = "V", default = "V", alias = "tools.select.artboard")]
    Artboard,
    #[command(label = "Rectangular Marquee Tool", icon = "square-dashed", kind = "tool", group = "M", default = "M", alias = "tools.select.rectMarquee")]
    RectMarquee,
    #[command(label = "Elliptical Marquee Tool", icon = "ellipse_dashed", kind = "tool", group = "M", default = "M", alias = "tools.select.ellipseMarquee")]
    EllipseMarquee,
    #[command(label = "Single Row Marquee Tool", icon = "marquee_row", kind = "tool", group = "tools", alias = "tools.select.singleRowMarquee")]
    SingleRowMarquee,
    #[command(label = "Single Column Marquee Tool", icon = "marquee_column", kind = "tool", group = "tools", alias = "tools.select.singleColumnMarquee")]
    SingleColumnMarquee,
    #[command(label = "Lasso Tool", icon = "lasso", kind = "tool", group = "L", default = "L", alias = "tools.select.lasso")]
    Lasso,
    #[command(label = "Polygonal Lasso Tool", icon = "pentagon", kind = "tool", group = "L", default = "L", alias = "tools.select.polygonLasso")]
    PolygonLasso,
    #[command(label = "Magnetic Lasso Tool", icon = "lasso_magnetic", kind = "tool", group = "L", default = "L", alias = "tools.select.magneticLasso")]
    MagneticLasso,
    #[command(label = "Magic Wand Tool", icon = "wand_sparkles", kind = "tool", group = "W", default = "W", alias = "tools.select.magicWand")]
    MagicWand,
    #[command(label = "Crop Tool", icon = "crop", kind = "tool", group = "C", default = "C", alias = "tools.select.crop")]
    Crop,
    #[command(label = "Perspective Crop Tool", icon = "perspective_crop", kind = "tool", group = "C", default = "C", alias = "tools.select.perspectiveCrop")]
    PerspectiveCrop,
    #[command(label = "Eyedropper Tool", icon = "pipette", kind = "tool", group = "I", default = "I", alias = "tools.select.eyedropper")]
    Eyedropper,
    #[command(label = "Color Sampler Tool", icon = "sampler", kind = "tool", group = "I", default = "I", alias = "tools.select.colorSampler")]
    ColorSampler,
    #[command(label = "Ruler Tool", icon = "ruler", kind = "tool", group = "I", default = "I", alias = "tools.select.ruler")]
    Ruler,
    #[command(label = "Note Tool", icon = "message-square", kind = "tool", group = "I", default = "I", alias = "tools.select.note")]
    Note,
    #[command(label = "Count Tool", icon = "target", kind = "tool", group = "I", default = "I", alias = "tools.select.count")]
    Count,
    #[command(label = "Brush Tool", icon = "brush", kind = "tool", group = "B", default = "B", alias = "tools.select.brush")]
    Brush,
    #[command(label = "Pencil Tool", icon = "pencil", kind = "tool", group = "B", default = "B", alias = "tools.select.pencil")]
    Pencil,
    #[command(label = "Mixer Brush Tool", icon = "palette", kind = "tool", group = "B", default = "B", alias = "tools.select.mixerBrush")]
    MixerBrush,
    #[command(label = "Color Replacement Tool", icon = "brush_replace", kind = "tool", group = "B", default = "B", alias = "tools.select.colorReplacement")]
    ColorReplacement,
    #[command(label = "Eraser Tool", icon = "eraser", kind = "tool", group = "E", default = "E", alias = "tools.select.eraser")]
    Eraser,
    #[command(label = "Background Eraser Tool", icon = "eraser_background", kind = "tool", group = "E", default = "E", alias = "tools.select.backgroundEraser")]
    BackgroundEraser,
    #[command(label = "Magic Eraser Tool", icon = "eraser_magic", kind = "tool", group = "E", default = "E", alias = "tools.select.magicEraser")]
    MagicEraser,
    #[command(label = "Gradient Tool", icon = "gradient", kind = "tool", group = "G", default = "G", alias = "tools.select.gradient")]
    Gradient,
    #[command(label = "Paint Bucket Tool", icon = "paint_bucket", kind = "tool", group = "G", default = "G", alias = "tools.select.paintBucket")]
    PaintBucket,
    #[command(label = "Horizontal Type Tool", icon = "type", kind = "tool", group = "T", default = "T", alias = "tools.select.type")]
    Type,
    #[command(label = "Vertical Type Tool", icon = "type", kind = "tool", group = "T", default = "T", alias = "tools.select.verticalType")]
    VerticalType,
    #[command(label = "Horizontal Type Mask Tool", icon = "type_mask", kind = "tool", group = "T", default = "T", alias = "tools.select.horizontalTypeMask")]
    HorizontalTypeMask,
    #[command(
        label = "Vertical Type Mask Tool",
        icon = "type_mask_vertical",
        kind = "tool",
        group = "T",
        default = "T",
        alias = "tools.select.verticalTypeMask"
    )]
    VerticalTypeMask,
    #[command(label = "Hand Tool", icon = "hand", kind = "tool", group = "H", default = "H", alias = "tools.select.hand")]
    Hand,
    #[command(label = "Rotate View Tool", icon = "compass", kind = "tool", group = "R", default = "R", alias = "tools.select.rotateView")]
    RotateView,
    #[command(label = "Zoom Tool", icon = "zoom_in", kind = "tool", group = "Z", default = "Z", alias = "tools.select.zoom")]
    Zoom,
    #[command(label = "Remove Tool", icon = "bandage_sparkle", kind = "tool", group = "J", default = "J", alias = "tools.select.remove")]
    Remove,
    #[command(label = "Spot Healing Brush Tool", icon = "bandage", kind = "tool", group = "J", default = "J", alias = "tools.select.spotHealing")]
    SpotHealing,
    #[command(label = "Healing Brush Tool", icon = "bandage", kind = "tool", group = "J", default = "J", alias = "tools.select.healing")]
    Healing,
    #[command(label = "Patch Tool", icon = "lasso_select", kind = "tool", group = "J", default = "J", alias = "tools.select.patch")]
    Patch,
    #[command(label = "Content-Aware Move Tool", icon = "arrow-left-right", kind = "tool", group = "J", default = "J", alias = "tools.select.contentAwareMove")]
    ContentAwareMove,
    #[command(label = "Red Eye Tool", icon = "eye", kind = "tool", group = "J", default = "J", alias = "tools.select.redEye")]
    RedEye,
    #[command(label = "Clone Stamp Tool", icon = "stamp", kind = "tool", group = "S", default = "S", alias = "tools.select.cloneStamp")]
    CloneStamp,
    #[command(label = "Pattern Stamp Tool", icon = "grid", kind = "tool", group = "S", default = "S", alias = "tools.select.patternStamp")]
    PatternStamp,
    #[command(label = "History Brush Tool", icon = "history-brush", kind = "tool", group = "Y", default = "Y", alias = "tools.select.historyBrush")]
    HistoryBrush,
    #[command(label = "Art History Brush Tool", icon = "sparkles", kind = "tool", group = "Y", default = "Y", alias = "tools.select.artHistoryBrush")]
    ArtHistoryBrush,
    #[command(label = "Blur Tool", icon = "droplet", kind = "tool", group = "tools", alias = "tools.select.blur")]
    Blur,
    #[command(label = "Sharpen Tool", icon = "triangle", kind = "tool", group = "tools", alias = "tools.select.sharpen")]
    Sharpen,
    #[command(label = "Smudge Tool", icon = "pointer", kind = "tool", group = "tools", alias = "tools.select.smudge")]
    Smudge,
    #[command(label = "Dodge Tool", icon = "lollipop", kind = "tool", group = "O", default = "O", alias = "tools.select.dodge")]
    Dodge,
    #[command(label = "Burn Tool", icon = "flame", kind = "tool", group = "O", default = "O", alias = "tools.select.burn")]
    Burn,
    #[command(label = "Sponge Tool", icon = "cloud", kind = "tool", group = "O", default = "O", alias = "tools.select.sponge")]
    Sponge,
    #[command(label = "Quick Selection Tool", icon = "quick_selection", kind = "tool", group = "W", default = "W", alias = "tools.select.quickSelection")]
    QuickSelection,
    #[command(
        label = "Object Selection Tool",
        icon = "square-dashed-mouse-pointer",
        kind = "tool",
        group = "W",
        default = "W",
        alias = "tools.select.objectSelection"
    )]
    ObjectSelection,
    #[command(label = "Pen Tool", icon = "pen_tool", kind = "tool", group = "P", default = "P", alias = "tools.select.pen")]
    Pen,
    #[command(label = "Freeform Pen Tool", icon = "pen_freeform", kind = "tool", group = "P", default = "P", alias = "tools.select.freeformPen")]
    FreeformPen,
    #[command(label = "Curvature Pen Tool", icon = "pen_curvature", kind = "tool", group = "P", default = "P", alias = "tools.select.curvaturePen")]
    CurvaturePen,
    #[command(label = "Add Anchor Point Tool", icon = "pen_add_anchor", kind = "tool", group = "tools", alias = "tools.select.addAnchorPoint")]
    AddAnchorPoint,
    #[command(label = "Delete Anchor Point Tool", icon = "pen_delete_anchor", kind = "tool", group = "tools", alias = "tools.select.deleteAnchorPoint")]
    DeleteAnchorPoint,
    #[command(label = "Convert Point Tool", icon = "convert_point", kind = "tool", group = "tools", alias = "tools.select.convertPoint")]
    ConvertPoint,
    #[command(label = "Path Selection Tool", icon = "mouse_pointer_2", kind = "tool", group = "A", default = "A", alias = "tools.select.pathSelection")]
    PathSelection,
    #[command(label = "Direct Selection Tool", icon = "direct_select", kind = "tool", group = "A", default = "A", alias = "tools.select.directSelection")]
    DirectSelection,
    #[command(label = "Rectangle Tool", icon = "rectangle-horizontal", kind = "tool", group = "U", default = "U", alias = "tools.select.rectangle")]
    Rectangle,
    #[command(label = "Ellipse Tool", icon = "circle", kind = "tool", group = "U", default = "U", alias = "tools.select.ellipseShape")]
    EllipseShape,
    #[command(label = "Triangle Tool", icon = "triangle", kind = "tool", group = "U", default = "U", alias = "tools.select.triangle")]
    Triangle,
    #[command(label = "Polygon Tool", icon = "pentagon", kind = "tool", group = "U", default = "U", alias = "tools.select.polygon")]
    Polygon,
    #[command(label = "Star Tool", icon = "star", kind = "tool", group = "U", default = "U", alias = "tools.select.star")]
    Star,
    #[command(label = "Line Tool", icon = "slash", kind = "tool", group = "U", default = "U", alias = "tools.select.line")]
    Line,
    #[command(label = "Custom Shape Tool", icon = "cloud", kind = "tool", group = "U", default = "U", alias = "tools.select.customShape")]
    CustomShape,
    #[command(label = "Slice Tool", icon = "slice_knife", kind = "tool", group = "C", default = "C", alias = "tools.select.slice")]
    Slice,
    #[command(label = "Slice Select Tool", icon = "slice_select", kind = "tool", group = "C", default = "C", alias = "tools.select.sliceSelect")]
    SliceSelect,
    #[command(label = "Frame Tool", icon = "frame_placeholder", kind = "tool", group = "K", default = "K", alias = "tools.select.frame")]
    Frame,
}

/// The command that selects `tool`.
pub(crate) fn command_of(tool: Tool) -> ToolCommand {
    match tool {
        Tool::Move => ToolCommand::Move,
        Tool::Artboard => ToolCommand::Artboard,
        Tool::RectMarquee => ToolCommand::RectMarquee,
        Tool::EllipseMarquee => ToolCommand::EllipseMarquee,
        Tool::SingleRowMarquee => ToolCommand::SingleRowMarquee,
        Tool::SingleColumnMarquee => ToolCommand::SingleColumnMarquee,
        Tool::Lasso => ToolCommand::Lasso,
        Tool::PolygonLasso => ToolCommand::PolygonLasso,
        Tool::MagneticLasso => ToolCommand::MagneticLasso,
        Tool::MagicWand => ToolCommand::MagicWand,
        Tool::Crop => ToolCommand::Crop,
        Tool::PerspectiveCrop => ToolCommand::PerspectiveCrop,
        Tool::Eyedropper => ToolCommand::Eyedropper,
        Tool::ColorSampler => ToolCommand::ColorSampler,
        Tool::Ruler => ToolCommand::Ruler,
        Tool::Note => ToolCommand::Note,
        Tool::Count => ToolCommand::Count,
        Tool::Brush => ToolCommand::Brush,
        Tool::Pencil => ToolCommand::Pencil,
        Tool::MixerBrush => ToolCommand::MixerBrush,
        Tool::ColorReplacement => ToolCommand::ColorReplacement,
        Tool::Eraser => ToolCommand::Eraser,
        Tool::BackgroundEraser => ToolCommand::BackgroundEraser,
        Tool::MagicEraser => ToolCommand::MagicEraser,
        Tool::Gradient => ToolCommand::Gradient,
        Tool::PaintBucket => ToolCommand::PaintBucket,
        Tool::Type => ToolCommand::Type,
        Tool::VerticalType => ToolCommand::VerticalType,
        Tool::HorizontalTypeMask => ToolCommand::HorizontalTypeMask,
        Tool::VerticalTypeMask => ToolCommand::VerticalTypeMask,
        Tool::Hand => ToolCommand::Hand,
        Tool::RotateView => ToolCommand::RotateView,
        Tool::Zoom => ToolCommand::Zoom,
        Tool::Remove => ToolCommand::Remove,
        Tool::SpotHealing => ToolCommand::SpotHealing,
        Tool::Healing => ToolCommand::Healing,
        Tool::Patch => ToolCommand::Patch,
        Tool::ContentAwareMove => ToolCommand::ContentAwareMove,
        Tool::RedEye => ToolCommand::RedEye,
        Tool::CloneStamp => ToolCommand::CloneStamp,
        Tool::PatternStamp => ToolCommand::PatternStamp,
        Tool::HistoryBrush => ToolCommand::HistoryBrush,
        Tool::ArtHistoryBrush => ToolCommand::ArtHistoryBrush,
        Tool::Blur => ToolCommand::Blur,
        Tool::Sharpen => ToolCommand::Sharpen,
        Tool::Smudge => ToolCommand::Smudge,
        Tool::Dodge => ToolCommand::Dodge,
        Tool::Burn => ToolCommand::Burn,
        Tool::Sponge => ToolCommand::Sponge,
        Tool::QuickSelection => ToolCommand::QuickSelection,
        Tool::ObjectSelection => ToolCommand::ObjectSelection,
        Tool::Pen => ToolCommand::Pen,
        Tool::FreeformPen => ToolCommand::FreeformPen,
        Tool::CurvaturePen => ToolCommand::CurvaturePen,
        Tool::AddAnchorPoint => ToolCommand::AddAnchorPoint,
        Tool::DeleteAnchorPoint => ToolCommand::DeleteAnchorPoint,
        Tool::ConvertPoint => ToolCommand::ConvertPoint,
        Tool::PathSelection => ToolCommand::PathSelection,
        Tool::DirectSelection => ToolCommand::DirectSelection,
        Tool::Rectangle => ToolCommand::Rectangle,
        Tool::EllipseShape => ToolCommand::EllipseShape,
        Tool::Triangle => ToolCommand::Triangle,
        Tool::Polygon => ToolCommand::Polygon,
        Tool::Star => ToolCommand::Star,
        Tool::Line => ToolCommand::Line,
        Tool::CustomShape => ToolCommand::CustomShape,
        Tool::Slice => ToolCommand::Slice,
        Tool::SliceSelect => ToolCommand::SliceSelect,
        Tool::Frame => ToolCommand::Frame,
    }
}

/// The tool a [`ToolCommand`] selects.
pub(crate) fn tool_of(command: ToolCommand) -> Tool {
    match command {
        ToolCommand::Move => Tool::Move,
        ToolCommand::Artboard => Tool::Artboard,
        ToolCommand::RectMarquee => Tool::RectMarquee,
        ToolCommand::EllipseMarquee => Tool::EllipseMarquee,
        ToolCommand::SingleRowMarquee => Tool::SingleRowMarquee,
        ToolCommand::SingleColumnMarquee => Tool::SingleColumnMarquee,
        ToolCommand::Lasso => Tool::Lasso,
        ToolCommand::PolygonLasso => Tool::PolygonLasso,
        ToolCommand::MagneticLasso => Tool::MagneticLasso,
        ToolCommand::MagicWand => Tool::MagicWand,
        ToolCommand::Crop => Tool::Crop,
        ToolCommand::PerspectiveCrop => Tool::PerspectiveCrop,
        ToolCommand::Eyedropper => Tool::Eyedropper,
        ToolCommand::ColorSampler => Tool::ColorSampler,
        ToolCommand::Ruler => Tool::Ruler,
        ToolCommand::Note => Tool::Note,
        ToolCommand::Count => Tool::Count,
        ToolCommand::Brush => Tool::Brush,
        ToolCommand::Pencil => Tool::Pencil,
        ToolCommand::MixerBrush => Tool::MixerBrush,
        ToolCommand::ColorReplacement => Tool::ColorReplacement,
        ToolCommand::Eraser => Tool::Eraser,
        ToolCommand::BackgroundEraser => Tool::BackgroundEraser,
        ToolCommand::MagicEraser => Tool::MagicEraser,
        ToolCommand::Gradient => Tool::Gradient,
        ToolCommand::PaintBucket => Tool::PaintBucket,
        ToolCommand::Type => Tool::Type,
        ToolCommand::VerticalType => Tool::VerticalType,
        ToolCommand::HorizontalTypeMask => Tool::HorizontalTypeMask,
        ToolCommand::VerticalTypeMask => Tool::VerticalTypeMask,
        ToolCommand::Hand => Tool::Hand,
        ToolCommand::RotateView => Tool::RotateView,
        ToolCommand::Zoom => Tool::Zoom,
        ToolCommand::Remove => Tool::Remove,
        ToolCommand::SpotHealing => Tool::SpotHealing,
        ToolCommand::Healing => Tool::Healing,
        ToolCommand::Patch => Tool::Patch,
        ToolCommand::ContentAwareMove => Tool::ContentAwareMove,
        ToolCommand::RedEye => Tool::RedEye,
        ToolCommand::CloneStamp => Tool::CloneStamp,
        ToolCommand::PatternStamp => Tool::PatternStamp,
        ToolCommand::HistoryBrush => Tool::HistoryBrush,
        ToolCommand::ArtHistoryBrush => Tool::ArtHistoryBrush,
        ToolCommand::Blur => Tool::Blur,
        ToolCommand::Sharpen => Tool::Sharpen,
        ToolCommand::Smudge => Tool::Smudge,
        ToolCommand::Dodge => Tool::Dodge,
        ToolCommand::Burn => Tool::Burn,
        ToolCommand::Sponge => Tool::Sponge,
        ToolCommand::QuickSelection => Tool::QuickSelection,
        ToolCommand::ObjectSelection => Tool::ObjectSelection,
        ToolCommand::Pen => Tool::Pen,
        ToolCommand::FreeformPen => Tool::FreeformPen,
        ToolCommand::CurvaturePen => Tool::CurvaturePen,
        ToolCommand::AddAnchorPoint => Tool::AddAnchorPoint,
        ToolCommand::DeleteAnchorPoint => Tool::DeleteAnchorPoint,
        ToolCommand::ConvertPoint => Tool::ConvertPoint,
        ToolCommand::PathSelection => Tool::PathSelection,
        ToolCommand::DirectSelection => Tool::DirectSelection,
        ToolCommand::Rectangle => Tool::Rectangle,
        ToolCommand::EllipseShape => Tool::EllipseShape,
        ToolCommand::Triangle => Tool::Triangle,
        ToolCommand::Polygon => Tool::Polygon,
        ToolCommand::Star => Tool::Star,
        ToolCommand::Line => Tool::Line,
        ToolCommand::CustomShape => Tool::CustomShape,
        ToolCommand::Slice => Tool::Slice,
        ToolCommand::SliceSelect => Tool::SliceSelect,
        ToolCommand::Frame => Tool::Frame,
    }
}

/// The rail's entry for `tool`: the tool itself, running its command.
pub(crate) fn rail_tool(tool: Tool) -> RailTool<Tool> {
    RailTool::new(tool, command_of(tool))
}

/// The rail's slots: one per toolbar group of the stored layout ([`layout_sections`]), the last of each section ending a
/// group. A slot's id is its first tool's default slot index, so a slot keeps its id when others move.
pub(crate) fn slots(toolbar: &ToolbarCustomization) -> Vec<ToolSlot<Tool>> {
    let mut slots = Vec::new();
    for section in layout_sections(toolbar) {
        let last = section.len().saturating_sub(1);
        for (at, (index, tools)) in section.into_iter().enumerate() {
            let id = SlotId::try_from(index).unwrap_or(SlotId::MAX);
            let tools: Vec<RailTool<Tool>> = tools.into_iter().map(rail_tool).collect();
            slots.push(if at == last { ToolSlot::ending_group(id, tools) } else { ToolSlot::new(id, tools) });
        }
    }
    slots
}

/// The rail's [`slots`] for the stored layout, built once per preferences revision ([`PrefsStore::rev`]): the toolbar, its
/// measure and the dock's grid read them several times a frame, and the layout changes only with the preferences.
///
/// [`PrefsStore::rev`]: photocraft_engine::prefs::PrefsStore::rev
#[derive(Default)]
pub(crate) struct ToolbarSlots(std::cell::RefCell<Option<(u64, RailSlots)>>);

/// The rail's slots, shared by the readers of one preferences revision.
type RailSlots = std::sync::Arc<[ToolSlot<Tool>]>;

impl ToolbarSlots {
    /// The slots of `prefs`' toolbar layout.
    pub(crate) fn get(&self, prefs: &photocraft_engine::prefs::PrefsStore) -> RailSlots {
        let mut held = self.0.borrow_mut();
        match &*held {
            Some((rev, slots)) if *rev == prefs.rev() => slots.clone(),
            _ => {
                let built: RailSlots = slots(&prefs.get().toolbar).into();
                *held = Some((prefs.rev(), built.clone()));
                built
            }
        }
    }
}

/// `slots` as the group layout `edit.toolbar {"toolbar": ..}` stores: one list of `Tool` names per slot.
pub(crate) fn saved_groups(slots: &[ToolSlot<Tool>]) -> serde_json::Value {
    slots.iter().map(|slot| slot.tools.iter().map(|tool| format!("{:?}", tool.id)).collect::<Vec<String>>()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_maps_to_its_command_and_back() {
        for tool in Tool::ALL {
            assert_eq!(tool_of(command_of(tool)), tool);
        }
    }
}
