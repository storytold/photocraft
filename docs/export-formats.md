# Save and export format catalog

Research date: 2026-10-10. This is the union of documented image/document saving formats and
registered export plug-ins in Photoshop, Krita and GIMP, including specialist asset formats.
It is a work list, not a claim that PhotoCraft implements every format. Optional dependencies,
image mode/depth, platform, application version and installed plug-ins change the visible menu.

## Menu behavior

- **Photoshop:** Save As / Save a Copy expose document and raster containers; Export As and
  Quick Export are narrower delivery workflows. Save for Web covers PNG, JPEG, GIF and WBMP.
  The supported-format reference includes import-only entries: PICT and PICT Resource must not
  be counted as writable. Cloud documents and video exports are separate workflows.
- **Krita:** native working documents use KRA; Save As and Export expose installed export
  filters. Animation rendering is separate. The file-format manual describes PDF, SVG and
  JPEG 2000, but the document registry has import filters only for those three. SVG vector layers have
  a separate Save Vector Layer as SVG command.
- **GIMP:** Save / Save As preserve XCF; Export As selects other formats. Export / Overwrite
  repeats the last export or imported-file format. The registry includes optional and specialist
  plug-ins beyond the formats covered individually in the manual.
- **PhotoCraft:** Save As changes the working path; Save a Copy leaves the original path and
  saved revision alone. Flat formats flatten the output and existing fidelity warnings report
  conversions/losses. macOS now chooses the format in the editor before its native destination
  sheet; Windows/Linux use their native file-type list. Export As uses the same writable document/raster catalog, including PhotoCraft, PSD and PSB.

## Full image/document and asset union

`Yes` means documented saving/export support or a registered export procedure; `Optional` means
build/plug-in dependent; `Import` is deliberately excluded from export coverage; `—` means not
found in the inspected sources. Krita/GIMP source snapshots are development registries, not proof
that every released binary ships each plug-in. The PhotoCraft column describes this first change.

| Format / common extensions | Photoshop | Krita | GIMP | PhotoCraft |
|---|---|---|---|---|
| PhotoCraft `.pcraft` | — | — | — | Layered native save |
| Photoshop `.psd` | Yes | Yes | Yes | Layered save |
| Large Photoshop document `.psb` | Yes | — | Yes | Separate layered choice |
| Photoshop cloud document `.psdc` | Cloud | — | — | Future cloud workflow |
| Krita document `.kra` | — | Yes | — | Future |
| Krita archive `.krz` | — | Yes | — | Future |
| GIMP document `.xcf` (also compressed) | — | — | Native save | Future |
| OpenRaster `.ora` | — | Yes | Yes | Future layered interchange |
| PNG `.png` | Yes | Yes | Yes | Flat export |
| APNG `.apng` | — | — | Import in inspected PNG plug-in | Recognized alias; single frame only |
| JPEG `.jpg .jpeg .jpe .jfif` | Yes | Yes | Yes | Flat export |
| TIFF `.tif .tiff` | Yes | Yes | Yes | Flat or layered save |
| BMP / DIB `.bmp .dib` | Yes | Yes | Yes | Newly discoverable |
| GIF `.gif` | Yes | Yes | Yes | Newly discoverable; single frame |
| WebP `.webp` | Yes | Optional | Yes | Flat export; single frame |
| Targa `.tga .icb .vda .vst` | Yes | Yes | Yes | Flat export |
| OpenEXR `.exr` | Yes | Optional | Optional | Flat HDR export |
| Radiance RGBE `.hdr` | Yes | Yes | Optional | Newly discoverable |
| Netpbm `.pnm .pbm .pgm .ppm .pam .pfm` | PBM family | PBM/PGM/PPM | Yes | Newly discoverable; subtype chosen from depth/channels |
| Icon `.ico` | — | Yes | Yes | Newly discoverable; max 256 × 256 |
| QOI `.qoi` | — | — | Yes | Newly discoverable |
| AVIF `.avif` | Yes | Optional | Optional | Encoder-feature dependent; not in default build |
| HEIF / HEIC `.heif .heic` | Platform dependent | Optional | Optional | Import only with `heif`; future export |
| JPEG XL `.jxl` | — | Optional | Optional | Future |
| JPEG 2000 `.jp2 .j2k .j2c .jpc` | Yes | Import | Optional | Future |
| JPEG 2000 in HEIF `.hej2` | — | — | Optional | Future |
| PDF `.pdf` | Yes | Import | Yes | Future |
| PostScript `.ps` | — | — | Yes | Future |
| Encapsulated PostScript `.eps` | Yes | — | Yes | Future |
| Photoshop DCS 1.0 / 2.0 `.eps` and companion files | Yes | — | — | Future print separation |
| SVG `.svg` | Separate paths workflow | Vector-layer export only | Yes (development registry) | Import only; future document export |
| PCX `.pcx .pcc` | Yes | — | Yes | Future |
| IFF / ILBM `.iff .lbm` | Yes | — | Import in inspected plug-in | Future |
| Pixar `.pxr` | Yes | — | — | Future |
| Scitex CT `.sct` | Yes | — | — | Future |
| Cineon `.cin` | Yes | — | Export procedure declared; version dependent | Future |
| DPX `.dpx` | Video / image sequence | — | Export procedure declared; version dependent | Future |
| Photoshop Raw `.raw` | Yes | — | Raw data `.data .raw` | Future; requires explicit layout parameters |
| DICOM `.dcm .dicom` | Listed by Adobe; saving is document dependent | — | Yes | Future |
| Wireless bitmap `.wbmp` | Save for Web | — | Import in inspected plug-in | Existing Save for Web output; separate from general encoder registry |
| X bitmap `.xbm` | — | Yes | Yes | Future |
| X pixmap `.xpm` | — | Yes | Yes | Future |
| X window dump `.xwd` | — | — | Yes | Future |
| Xcursor (X11 cursor files) | — | — | Yes | Future |
| Windows cursor `.cur` | — | — | Yes | Future |
| Windows animated cursor `.ani` | — | — | Yes | Future |
| macOS icon `.icns` | — | — | Yes | Future |
| DirectDraw Surface `.dds` | Optional third-party plug-in | — | Yes | Future |
| FITS `.fit .fits` | — | — | Yes | Future |
| Autodesk animation `.fli .flc` | — | — | Yes | Future animation workflow |
| Multiple-image network graphics `.mng` | — | — | Yes | Future animation workflow |
| SGI `.sgi .rgb .rgba .bw` | — | — | Yes | Future |
| Sun raster `.ras .sun .im1 .im8 .im24 .im32 .rs` | — | — | Yes | Future |
| Alias PIX `.pix .matte .mask .alpha .als` | — | — | Yes | Future |
| Farbfeld `.ff` | — | — | Yes | Future |
| KiSS CEL `.cel` | — | — | Yes | Future |
| PlayStation TIM `.tim` | — | — | Yes | Future |
| PaintShop Pro `.psp .tub` | — | — | Yes | Future |
| GIMP brush `.gbr` | — | Yes | Yes | Future asset export |
| GIMP animated brush `.gih` | — | Yes | Yes | Future asset export |
| GIMP pattern `.pat` | — | — | Yes | Future asset export; distinct from Adobe PAT |
| Krita brush preset `.kpp` | — | Yes (PNG filter) | — | Future asset export |
| Animation CSV `.csv` + images | — | Yes | — | Future animation interchange |
| Height map `.r8 .r16 .r32` | — | Yes | — | Future |
| Spriter `.scml` + images | — | Yes | — | Future animation asset export |
| Qt Quick `.qml` + images | — | Yes | — | Future specialized asset export |
| C source `.c`, C header `.h` | — | — | Yes | Future code asset export |
| HTML table `.html .htm` | — | — | Yes | Future; PhotoCraft's slice HTML is a different workflow |
| ASCII / ANSI art `.txt .text .ansi` | — | — | Optional | Future specialized export |

Palette libraries (Krita KPL and GIMP GPL), Photoshop paths `.ai`, swatches `.aco/.ase`, presets,
lookup tables, scripts and measurements are auxiliary exports, not document image containers.
Camera RAW formats (DNG, CR2/CR3, NEF, ARW, etc.) are import/development inputs, not normal image
exports. Photoshop's video reference lists MOV, MP4, DPX, EXR and JPEG2000 QuickTime; Krita's
animation rendering and GIMP's frame exports require separate timing/codec work. Audio and retired
3D formats should not be added to an image Save As chooser.

## First implementation and next steps

1. Make existing encoders discoverable everywhere: one capability-derived catalog, macOS format
   chooser, BMP and other missing native filters, separate PSB, and the full raster Export As list.
2. Add OpenRaster first for layered exchange with both Krita and GIMP; require real multilayer
   round trips, explicit unsupported-feature warnings and bounded ZIP/XML reads.
3. Add PDF and SVG document export; distinguish raster pages/embedded images from vector-preserving
   output and test ICC, transparency, dimensions and font handling.
4. Evaluate pure-Rust JXL, JPEG2000 and HEIF encoders, then legacy/asset formats by concrete workflow.
5. Add explicit Netpbm subtype selection and animation/frame export. Current Netpbm chooses P5/P6/P7
   or PFM from pixels, not the typed suffix; GIF/WebP/APNG aliases do not imply animation support.

## Sources and reproducibility

- [Adobe supported file formats](https://helpx.adobe.com/photoshop/using/file-formats.html)
  (reference updated 2026-06-09), [image format details](https://helpx.adobe.com/photoshop/desktop/save-and-export/export-files-to-different-formats/image-file-formats-supported-in-photoshop.html),
  [Export As](https://helpx.adobe.com/photoshop/desktop/save-and-export/export-files-to-different-formats/fine-tune-your-export-settings-using-the-export-as-option.html).
- [Krita file-format manual](https://docs.krita.org/en/general_concepts/file_formats.html),
  [export filter registry at 3ee2270](https://github.com/KDE/krita/tree/3ee2270d8ead36966a055ab72265b8d767d162b8/plugins/impex).
  Checked `_export.json` MIME/extension metadata and the build registry; PDF/JP2/SVG have no export
  metadata in this snapshot; [SVG vector-layer export](https://docs.krita.org/en/general_concepts/file_formats/file_svg.html)
  is a separate menu command. Qt image writers and optional library availability affect the menu.
- [GIMP Export As](https://docs.gimp.org/3.0/en/gimp-file-export-as.html),
  [export format manual](https://docs.gimp.org/3.0/en/export-file-formats.html),
  [plug-in registry at f562a3e](https://github.com/GNOME/gimp/tree/f562a3e60244a713a57a1aac83330746a8bdbd2c/plug-ins).
  Checked export procedure declarations and extension registration, including OpenRaster's Python
  registration and GEGL's HDR/EXR entries. Source metadata establishes available procedures, not
  cross-platform release/runtime verification. No implementation code was copied.
- PhotoCraft: `crates/codecs/src/format.rs`, `crates/io/src/lib.rs`, `crates/codecs/src/codecs/pnm.rs`,
  `crates/engine/src/web_cmds.rs`, `apps/photocraft/src/services.rs`, `crates/ui-egui/src/save_formats.rs`.
- rfd 0.17.2 macOS `panel_ffi.rs::add_filters` joins filter extensions into one AppKit allowed-type
  array; it does not install a format dropdown. The editor chooser uses safe Rust/egui and retains
  the asynchronous, parented native destination sheet.
