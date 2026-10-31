# Publication figures

Tools for preparing figures for journals (for example CT and MRI panels at 300 dpi): building a
multi-panel figure, labelling panels, marking findings with arrows and markers, and exporting
print-ready files. None of these has a Photoshop counterpart; each is an engine command, so the
same work can be scripted through the control channel or MCP.

## Make Figure (File › Automate › Make Figure…, `figure.make`)

Lays several images out as one new document at a journal width and resolution:

- **Width:** single column (85 mm), 1.5 column (120 mm), double column (175 mm) or a custom width
  in mm, at 300 dpi by default.
- **Layout:** *Rows* scales every image in a row to the same height and fills the width exactly,
  so images of different sizes line up without cropping. *Grid* fits each image into equal cells.
- **Gutters:** the lines between panels, with a width in mm and a colour (white or black is usual).
- **Labels:** a, b, c / A, B, C / (a) / (A) / a) / A), in any corner, sized in points, bold, with an
  optional contrasting outline. Labels are editable type layers; each image is its own pixel
  layer, so a panel can still be nudged, cropped or annotated.

The dialog takes a folder and places its images in name order; `figure.make` also takes an
explicit list of files.

## Prepare Collage for Publication (File › Automate, `figure.fromCollage`)

For a collage saved as one image, such as an OsiriX or PACS grid capture. It works on the open
document and leaves it unchanged:

1. It finds the grid from the edges running all the way across the image (or use Rows/Columns;
   0 means detect).
2. It splits the collage into its panels, in reading order.
3. It lays them out again as Make Figure does: journal width and dpi, separators in mm and
   colour, labels A, B, C (or another style) with an optional outline, in a new document.

The dialogs for this, Make Figure and Export for Publication open with the choices of their last
run.

### OsiriX workflow

1. Open the OsiriX collage in PhotoCraft.
2. Optional: remove unwanted columns or rows with the Cut Out tool (scissors, C) to go from, say,
   5 × 2 to 4 × 2 or 4 × 1. Drag across the column or row: Auto direction follows the drag, and
   with Snap to panels on (the default) the band's edges snap to the panel edges, so exactly that
   column or row goes. Repeat for each one.
3. File › Automate › Prepare Collage for Publication…: the dialog asks every time for the labels
   (A, B, C capital or a, b, c small, with or without brackets), Bold or regular, the corner (top
   or bottom, left or right), size and colour, plus the width (double column = 175 mm), 300 dpi and
   the separator width and colour. A new figure document opens with the Arrow tool selected.
4. Add arrows (Arrow tool) and stars or other markers (Marker tool). Labels are type layers: drag
   one with the Move tool if it covers something.
5. File › Export › Export for Publication…: TIFF and PNG at 300 dpi.

## Arrow tool (U)

See [Shape stroke options](shape-strokes.md) for the arrowhead model. For print work the width is
shown in **points** at the document's resolution (2 pt is 8 px at 300 dpi; documents under 144 dpi
use 2 px per point so screenshot arrows stay visible). Quick styles include solid, double-headed,
curved, dashed and dotted arrows and two outlined styles. **Outline** adds a thin contrasting
Stroke layer style (black on light colours, white on dark ones) so an arrow reads on dark and
bright tissue alike; **Curved** makes new arrows start with a gentle bend (drag the blue middle
handle to change it).

## Marker tool (U)

Click to stamp a star, asterisk, circle or triangle centred on the click, sized in points, in any
colour, with the same optional outline. Each marker is a shape layer.

## Export for Publication (File › Export › Export for Publication…, `file.export.publication`)

Writes the flattened image as TIFF (LZW, lossless) and/or PNG in one step, with the resolution
stored in the files. It can also resample to a journal column width at 300 or 600 dpi, and warns
when that enlarges the image more than 1.5×. The document itself is not changed. Files go next to
the document by default, named `<document>_<dpi>dpi.tif` / `.png`.

## Screenshots

- **File › New Screenshot** (⌘⇧9) inside PhotoCraft: hides PhotoCraft, shows the macOS
  crosshair (drag a region, Space for a window, Esc cancels) and opens the capture.
- **PhotoCraft Capture** (`packaging/macos/capture`, built into /Applications by `make-app.sh`)
  does the same without PhotoCraft open; bind it to a key with the Shortcuts app (a shortcut with
  one *Open App › PhotoCraft Capture* action and a keyboard shortcut).
- Captures go to the library folder (Pictures › PhotoCraft Screenshots unless changed in File ›
  Screenshot Library…) as `YYYY-MM-DD HH-MM-SS.png`. Any PNG in that folder, however it is
  opened, opens as an editable `.pcraft` beside it (made the first time); every save of the
  `.pcraft` rewrites the PNG. File › Screenshot Library… lists them newest first.
