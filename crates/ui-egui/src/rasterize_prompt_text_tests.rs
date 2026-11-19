//! #11899: the rasterize prompt for a smart object uses the reference application's wording;
//! the type, shape and fill prompts are unchanged.

use super::*;

#[test]
fn the_four_prompts_read_as_measured() {
    assert_eq!(
        Kind::SmartObject.message(),
        "This smart object must be rasterized before proceeding. Edit Contents will no longer be available. Rasterize the smart object?"
    );
    assert_eq!(Kind::Type.message(), "This type layer must be rasterized before proceeding. Its text will no longer be editable. Rasterize the type?");
    assert_eq!(Kind::Shape.message(), "This shape layer must be rasterized before proceeding. It will no longer be editable as a shape. Rasterize the shape?");
    assert_eq!(
        Kind::Fill.message(),
        "The fill for this layer must be rasterized before proceeding. It will no longer be editable as a fill. Rasterize the fill?"
    );
    assert!(!Kind::SmartObject.message().contains("  "), "single spaces between sentences");
}
