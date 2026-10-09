# PDF import and export

PhotoCraft supports PDF in File > Open, drag-and-drop, Save As, Export As,
and the CLI's convert and batch operations.

File > Export > Export All Tabs to PDF combines the current tabs into one binder,
in left-to-right tab order, including their current edits. Each canvas or artboard
becomes a page at its own size and resolution. Optionally select "Close exported
tabs after successful export". Closing happens only after the complete PDF has
been atomically saved; cancellation or failure leaves the tabs intact. Tabs opened
or changed after export began remain open. Export does not change source files or
the saved state of tabs that remain open.

Opening or dropping a PDF shows a page picker before any editing tabs are created.
Preview pages, click their checkboxes, or Shift-click to select an inclusive range.
Open selected imports only those pages. Open all pages explicitly imports the binder.
Each selected page is rasterized at 144 ppi and opens in its own named tab, in binder
order. Cancel leaves existing documents untouched. The CLI retains multipage
artboard import for batch conversion.
Crop boxes and page rotation are applied by Hayro, the pure Rust PDF renderer
also used by PdfCraft/PrintCraft. Imported PDFs have no automatic overwrite path.

Save As > PDF and Export As > PDF write one page per artboard, top to bottom in
the Layers panel. An ordinary image document becomes a single-page PDF. Exports
use lossless compression, 8-bit sRGB pixels, embedded ICC colour information,
and soft masks for transparency. Physical page size comes from pixel dimensions
and document resolution. Export As > Metadata > None omits XMP metadata.

PDF text, vector objects, links, forms, annotations, and signatures are not
preserved as editable PDF objects. These are raster image editing workflows.
Use .pcraft for an editable layered master and PDF for the exported result.
Password-protected PDFs must first be unlocked in a PDF editor.

Limits: 100 pages; 16,384 pixels per side; 64 megapixels per page;
512 megapixels total; 256 MB input file. Large PDFs should first be split.
Imports check cancellation between pages. A single page render may take time.

Build with `cargo build --release -p photocraft -p photocraft-cli`.
The focused regression tests are `cargo test -p photocraft-io --test pdf`.
`file.export.allTabsPdf` also accepts `{ "path": "binder.pdf", "closeAfter": false }` from the local CLI command runner.
