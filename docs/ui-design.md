# UI design system

## Themes

| Theme | Intent |
|---|---|
| **Pro** (default) | Photoshop-style Spectrum dark: flat charcoal panels (#323232), dark tab strips, Spectrum blue accent (#378ef0), pill buttons, checkboxes, compact 12 px type |
| Studio | Dark studio style: near-black, rounded cards, pill tabs, violet accent, toggles |
| Studio Light | Studio on light surfaces |
| Classic | Windows-2000 bevels, square corners, navy selection |

Switch themes with the sun icon, Window → Theme, or `ui.set {"theme":"classic"}` over the control channel.

## Rules

- **Colours and radii come from tokens.** Read them with `Tokens::get(ctx)`; never hard-code a colour in a widget.
- **Shared widgets live in `widgets.rs`:** `card` (panel group; Pro renders a Photoshop tab strip), `value_field`, `slider`/`slider_row`, `toggle`/`checkbox`, `primary_button`/`secondary_button`, `dropdown` (with a chevron icon), `hairline`/`vline`.
- **Icons:** Lucide SVGs in `assets/icons` (ISC licence), embedded via `icon_data.rs`. Regenerate that file when you add icons.
- **Fonts:** Inter (UI) and JetBrains Mono (numbers), both OFL. Named families `medium` and `semibold` are available via `theme::medium()` and `theme::semibold()`.
- **Photoshop layout grammar (Pro):**
  - Essentials dock order: Color | Swatches, then Properties | Adjustments, then Layers | Channels | Paths (Layers fills the remaining height).
  - Options-bar labels end with a colon ("Size:").
  - Document tabs read "name @ 12.5% (RGB/8)".
  - Toolbar tool groups carry a corner triangle.
  - The toolbar header's double arrow switches one/two columns (`panels.toolbar_double`: `true`,
    `false`, or `null` for automatic, two columns only when one doesn't fit). A chosen single
    column too tall for the window moves the remaining tools into a "More tools" flyout.
  - In two columns the tools fill the rows in order, two per row, in every theme, so no tool sits
    alone with a gap beside it. Studio and Studio Light keep their group dividers only in one
    column. One column has 4 px of extra room on each side.
  - The selected tool (Pro themes) sits in a darker square: the toolbar colour about a third
    darker, with the icon in its normal colour and no blue outline. Other themes keep the accent.
  - Colour chips (every theme): square foreground and background chips with a dark edge and a
    white ring, centred in the toolbar; the swap arrow in the top-right corner and the small
    default-colours chips in the bottom-left. Quick Mask and Screen Mode (Pro) sit side by side
    in two columns.
  - Color Picker: OK and Cancel stacked at the top right; the new/current swatch beside them;
    values in two columns (H S B, R G B and # on the left; L a b and C M Y K on the right); "Only
    Web Colors" under the colour field. Every one of H, S, B, R, G, B, L, a and b has a radio
    that picks what the field and slider show.
  - Color Picker CMYK values go through a printing profile: the active CMYK document's own
    profile, else the working CMYK from Edit › Color Settings (`ColorState::rgb_to_ink` /
    `ink_to_rgb`). Load the same profile as Photoshop and the numbers match it.
  - Add to Swatches (Color Picker) asks for a name and saves the colour (`swatches.add`); saved
    swatches live in the preferences (`swatches`) and follow the built-in ones in the Swatches
    panel, which scrolls. Alt-click deletes a saved swatch (`swatches.delete`); the panel's +
    button saves the foreground colour.
- **Motion:** menus, popups and tooltips fade in over `theme::ANIMATION_TIME` (1/12 s).
  Preferences › Interface › "Animate menus and panels" turns that off (`animation_time` 0), so
  they appear at once.
- **Main window:** the desktop app opens as it was last left (`mainWindow` in the preferences:
  the un-maximized size and whether it was maximized), maximized at 1440×900 on first launch.
  The size is saved once it has held still for half a second; full screen isn't saved.
- **What PhotoCraft remembers across restarts** (all in the preferences file):
  - the right dock's width (with the panel layout, `dockWidth`, when Remember Workspace Changes
    is on; saved workspaces carry it too);
  - the folder the last file was opened from (`lastOpenFolder`): File › Open, Place, Browse and
    preset import start there;
  - the last Export As settings (`exportAs`: format, quality, transparency, scale);
  - the last 8 entries run from the command palette (`recentCommands`), listed under "Recent"
    when its search is empty.
- **UI scale:** Preferences › Interface › UI Scale offers Auto, 100, 115, 125, 150 and 200 %.
- **Document tabs:** right-click a tab for Close, Close Others, Close All and, for a saved
  document, Reveal in Finder / Show in Explorer / Show in Folder (`file.reveal`). The reveal
  wording follows the platform everywhere (`layer_menu_cmds::REVEAL_LABEL`).
- **Sliders:** double-click a slider to reset it: to its default where the caller gives one
  (`widgets::slider_row_default`, e.g. the adjustment editors), else to the value it had when it
  appeared. Hue/Saturation's Saturation and Lightness tracks show the hue they act on.
- **Zoom:** ⌥/Alt + mouse wheel zooms around the pointer, about 5 % per wheel notch.
- **Rulers:** Preferences › Units & Rulers › Show Rulers in New Documents turns rulers on when a
  document is created or opened.
- **Verify every visual change** with `ui.screenshot`, at several window sizes and in every theme.

## Adding icons

```sh
curl -sfL -o assets/icons/<name>.svg https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/<name>.svg
# regenerate the embedded table
{ printf '%s\n' '//! Lucide icons (ISC licence), embedded and tinted at runtime.' '' 'pub static ICONS: &[(&str, &[u8])] = &['; \
  for f in assets/icons/*.svg; do n=$(basename $f .svg); echo "    (\"$n\", include_bytes!(\"../../../assets/icons/$n.svg\")),"; done; echo '];'; } \
  > crates/ui-egui/src/icon_data.rs
```

## Interaction models (match Photoshop CC)

| Feature | Module | Behaviour |
|---|---|---|
| Type tool | `type_tool.rs` | Click: point text with the placeholder "Lorem Ipsum" selected. Drag: paragraph box. Inline caret and selection drawn from the text engine layout. ⌥/⌘ word and line navigation, ↩ newline, ⌘↩ or Esc commits, a click outside commits. One history step per session (`coalesce`). A new layer is named after its text; an empty one is deleted. |
| Free Transform | `transform_tool.rs` | ⌘T. Corners scale proportionally (⇧ frees them); edges scale one axis; ⌥ scales about the reference point; ⌘-corner distorts; dragging outside rotates (⇧ snaps to 15°); dragging inside moves. Preview = document without the moving pixels + a textured 24×24 mesh. ↩ or a double-click commits via `edit.transform {rect, quad}`. |
| Layer masks | `panels.rs` | Clicking the mask thumbnail targets the mask (corner-bracket frame; the tab reads "Layer, Layer Mask/8"). Brush, eraser (paints background colour), gradient and bucket then send `"target": "mask"`. Adjustment and fill layers target their mask automatically. |
| Levels / Curves | `tone.rs` | Histogram of the image *below* the adjustment. Curves: click to add a point, drag out to delete. Every change is a coalesced `layer.setAdjustment`, so one drag = one undo step and the canvas updates at full resolution on the GPU. |

Where the font lacks a symbol (e.g. ∠ ↦ ▔), draw it with the painter or use a Lucide icon; never ship
missing-glyph boxes. Check every new panel with the offscreen snapshot tool (`docs/development.md`).

## Menus

`menu_catalog.rs` holds Photoshop's menu tree (standard command names, order, separators, default shortcuts). Items whose id matches an engine or UI command are live; others render disabled until implemented. Give new commands the catalogue's id (for example `image.imageSize`) and they light up in the right place automatically.

## Automation for visual checks

Use `ui.click {x,y}`, `ui.move`, `ui.key` and `ui.type` (synthetic input in screen points) to open menus, popups and context menus, then `ui.screenshot`.

## High DPI and 4K displays

Edit → Preferences → Interface → UI Scale applies immediately. Auto follows the operating
system's display scale (including fractional scales). For a 4K or larger monitor,
Auto uses at least 200% so text and controls remain readable. Detection uses the current monitor,
including portrait displays, and updates when the window moves between monitors. If monitor
size is unavailable, Auto follows system DPI. The 100% and 200% choices set an absolute UI
scale, allowing large 4K displays to use smaller controls when desired. Canvas zoom shortcuts
continue to control the document independently of UI scaling.
