# Actions editor glyphs

Thirty original SVGs for the identifier grid measured in issue #10946. Geometry was
drawn for PhotoCraft from simple paths, circles and rectangles. Only the identifier
names and picker order come from the reference; no reference artwork was copied.
These assets are MIT OR Apache-2.0, as recorded in `ATTRIBUTION.md`.

Each source uses a 24 px viewBox, currentColor and the existing 2 px round stroke
convention. The existing SvgIconSet pipeline recolours and overrides stroke width
at display time. No new renderer or dependencies are needed.

The mapping in `crates/ui-egui/src/panels/actions/editor_glyphs.rs` is registered
through `icon_data.rs` and used by the editor picker and modern Actions rows.
Preserve each raw metadata identifier on save;
unknown identifiers display the existing neutral `square` icon, whose Lucide and
Feather attribution remains in the parent attribution table.

Source tests prepare coverage of order, exact lookup, fallback and SvgIconSet's
byte transformation. Actual picker rendering, small-size legibility, light/dark
contrast, selection/focus and DPI qualification remain integration obligations.
