//! #12062: the Vertical Type tool has its own glyph (a T with a downward arrow), so the rail, the
//! T group's flyout and the options bar's tool preset picker tell it from the horizontal Type tool.

use craft::ui::primitives::has_icon;

use crate::icon_data::ICONS;
use crate::icons::{register, tool_icon};
use crate::state::Tool;

/// The two Type tools are different tools, so they show different glyphs: the vertical one draws the
/// T with a downward arrow, not the horizontal one's plain T.
#[test]
fn the_type_tools_show_different_glyphs() {
    register();
    assert_eq!(tool_icon(Tool::Type).name(), "type", "the horizontal Type tool keeps its glyph");
    assert_eq!(tool_icon(Tool::VerticalType).name(), "type-vertical");
    assert_ne!(tool_icon(Tool::VerticalType), tool_icon(Tool::Type));
}

/// The vertical glyph is PhotoCraft's own asset, not a catalog name, so the app set has to carry it:
/// an unregistered name paints nothing and only logs.
#[test]
fn the_vertical_glyph_is_an_app_icon() {
    register();
    assert!(has_icon(&tool_icon(Tool::VerticalType)), "the vertical glyph resolves");
    assert!(ICONS.iter().any(|&(name, _)| name == "type-vertical"), "it comes from the app's icon set");
}
