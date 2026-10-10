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
  sheet; Windows/Linux use their native file-type list. Export As includes every writable Save As choice, including PhotoCraft, PSD, PSB and OpenRaster, plus export-only raster PDF. The default build has 28 Save As groups and 29 Export As groups.

## Full image/document and asset union

`Yes` means documented saving/export support or a registered export procedure; `Optional` means
build/plug-in dependent; `Import` is deliberately excluded from export coverage; `—` means not
found in the inspected sources. Krita/GIMP source snapshots are development registries, not proof
that every released binary ships each plug-in. The PhotoCraft column includes the additional-export follow-up.

| Format / common extensions | Photoshop | Krita | GIMP | PhotoCraft |
|---|---|---|---|---|
| PhotoCraft `.pcraft` | — | — | — | Layered native save |
| Photoshop `.psd` | Yes | Yes | Yes | Layered save |
| Large Photoshop document `.psb` | Yes | — | Yes | Separate layered choice |
| Photoshop cloud document `.psdc` | Cloud | — | — | Future cloud workflow |
| Krita document `.kra` | — | Yes | — | Future |
| Krita archive `.krz` | — | Yes | — | Future |
| GIMP document `.xcf` (also compressed) | — | — | Native save | Future |
| OpenRaster `.ora` | — | Yes | Yes | Baseline PNG layers/groups; 8/16-bit, with baking/fallback warnings |
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
| PDF `.pdf` | Yes | Import | Yes | Single raster page in Export As; transparency, ICC and 8/16-bit; import pending |
| PostScript `.ps` | — | — | Yes | Future |
| Encapsulated PostScript `.eps` | Yes | — | Yes | Future |
| Photoshop DCS 1.0 / 2.0 `.eps` and companion files | Yes | — | — | Future print separation |
| SVG `.svg` | Separate paths workflow | Vector-layer export only | Yes (development registry) | Raster document export (embedded PNG); SVG/SVGZ import and export |
| PCX `.pcx .pcc` | Yes | — | Yes | RGB8 export; RGB/palette PCX import |
| IFF / ILBM `.iff .lbm` | Yes | — | Import in inspected plug-in | Future |
| Pixar `.pxr` | Yes | — | — | Future |
| Scitex CT `.sct` | Yes | — | — | Future |
| Cineon `.cin` | Yes | — | Export procedure declared; version dependent | Future |
| DPX `.dpx` | Video / image sequence | — | Export procedure declared; version dependent | Future |
| Photoshop Raw `.raw` | Yes | — | Raw data `.data .raw` | Future; requires explicit layout parameters |
| DICOM `.dcm .dicom` | Listed by Adobe; saving is document dependent | — | Yes | Future |
| Wireless bitmap `.wbmp` | Save for Web | — | Import in inspected plug-in | Type-0 monochrome read/write; threshold at 50% |
| X bitmap `.xbm` | — | Yes | Yes | Byte-packed monochrome read/write; threshold at 50% |
| X pixmap `.xpm` | — | Yes | Yes | RGB palette + one-bit transparency read/write |
| X window dump `.xwd` | — | — | Yes | Future |
| Xcursor (X11 cursor files) | — | — | Yes | Future |
| Windows cursor `.cur` | — | — | Yes | Single 32-bit bitmap cursor, ≤256 × 256, hotspot (0,0) |
| Windows animated cursor `.ani` | — | — | Yes | Future |
| macOS icon `.icns` | — | — | Yes | Single PNG representation; square 16/32/64/128/256/512/1024 |
| DirectDraw Surface `.dds` | Optional third-party plug-in | — | Yes | Future |
| FITS `.fit .fits` | — | — | Yes | Future |
| Autodesk animation `.fli .flc` | — | — | Yes | Future animation workflow |
| Multiple-image network graphics `.mng` | — | — | Yes | Future animation workflow |
| SGI `.sgi .rgb .rgba .bw` | — | — | Yes | 8/16-bit planar export; verbatim and RLE import |
| Sun raster `.ras .sun .im1 .im8 .im24 .im32 .rs` | — | — | Yes | RGB24 uncompressed export; uncompressed 8/24-bit import |
| Alias PIX `.pix .matte .mask .alpha .als` | — | — | Yes | Future |
| Farbfeld `.ff` | — | — | Yes | RGBA16 read/write, big-endian storage |
| KiSS CEL `.cel` | — | — | Yes | Future |
| PlayStation TIM `.tim` | — | — | Yes | Future |
| PaintShop Pro `.psp .tub` | — | — | Yes | Future |
| GIMP brush `.gbr` | — | Yes | Yes | RGBA8 version-2 brush read/write; default spacing 25% |
| GIMP animated brush `.gih` | — | Yes | Yes | Future asset export |
| GIMP pattern `.pat` | — | — | Yes | Gray/GrayA/RGB/RGBA8 pattern read/write; distinct from Adobe PAT |
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
2. Baseline OpenRaster now round-trips PNG layers/groups, order, names, opacity, visibility and
   8/16-bit pixels. The upstream PR uses main's OpenRaster implementation; grayscale is saved as RGB, and ICC/EXIF/XMP/text are discarded with warnings. The older fork retains the standalone ORA port. ZIP/XML reads, decoded layers and nesting are bounded.
   Masks, text, shapes and effects are baked; advanced blending, clipping, adjustments and artboards
   trigger a warned composite fallback. Imported mixed colour models/profiles are rejected rather
   than silently misinterpreted. Zip64 input and non-PNG layer sources remain unsupported.
3. Raster PDF and SVG/SVGZ delivery exports now work. PDF preserves image ICC, grayscale/RGB,
   8/16-bit samples, soft-mask transparency and physical dimensions from document DPI. It is
   Export As-only because PDF import is still absent. SVG wraps an embedded PNG; no editable vector
   or font preservation is claimed. Multi-page/vector PDF, SVG path export and PDF import remain open.
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

## Additional-export implementation and limits

The eleven new standalone codecs are PCX, SGI, Sun Raster, Farbfeld, WBMP, XBM, XPM, CUR, ICNS,
GBR and GIMP PAT. Each declares supported layouts/depths and participates in the existing depth,
alpha, ICC and metadata warning pipeline. No native codec library or new unsafe code is required.
Decode support covers the emitted baseline plus the variants listed above, not every historical
variant. XPM supports RGB hex colours and a small basic named-colour set, with a one-million-entry palette cap; X10 short-packed XBM,
compressed Sun Raster, palettized low-bit PCX and non-PNG ICNS representations remain unsupported.
ICNS rejects unsuitable dimensions instead of silently resizing; CUR stores one image and uses
an explicit default hotspot. XPM and monochrome formats lose transparency/colour precision as warned.

Tests cover all 11 × 6 layouts × 4 sample types (264 conversions), odd-width rows, SGI RLE,
handwritten XBM/XPM, truncations, random input, allocation budgets and icon size errors. The existing
malformed-input and limits suites exercise all registered codecs. OpenRaster tests cover nesting,
negative offsets, metadata/order/visibility, depth, fallback composites, XML entities and ZIP errors.
SVG/SVGZ reopen tests and PDF xref/page/depth/ICC/soft-mask checks cover the delivery containers.
Independent local readers: Pillow verified PCX/SGI/Sun/XBM/CUR/ICNS/GBR/PAT pixels; Poppler rendered
RGB and grayscale PDF outputs at 8/16/32-bit source depths with correct dimensions and colours.
This is not native Photoshop/Krita/GIMP acceptance testing.

Public format specifications used for clean-room implementation:

- [ZSoft PCX technical reference](https://files.mpoli.fi/unpacked/software/programm/c/pcx.zip/doc.txt).
- [SGI/Paul Haeberli specification](https://ftp.zx.net.nz/pub/archive/ftp.sgi.com/graphics/grafica/sgiimage.html).
- [SunOS rasterfile manual](https://www.cs.cmu.edu/~maxwell/misc/vascHelpPages/sunRasterFormat.html).
- [Farbfeld specification](https://git.suckless.org/farbfeld/file/FORMAT.html).
- [WAP WBMP Type 0 specification](https://wapforum.org/what/technical/SPEC-WAESpec-19990524.pdf).
- [X.Org XPM specification](https://www.x.org/docs/XPM/xpm.pdf).
- [GIMP brush](https://developer.gimp.org/core/standards/gbr/) and
  [pattern](https://developer.gimp.org/core/standards/pat/) specifications.
- [OpenRaster file layout](https://www.openraster.org/baseline/file-layout-spec.html) and
  [layer stack](https://www.openraster.org/baseline/layer-stack-spec.html).
- [SVG image element](https://www.w3.org/TR/SVG11/struct.html) and
  [Adobe PDF reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).

### Local release export cost

Apple M1 Max, one release run of `cargo run --release -p photocraft-codecs --example export_bench`,
24 MP opaque RGBA8 with a repeating 256-colour pattern. Times measure the standalone encoder,
including format conversion, excluding composition, colour management and disk writes. This is
an initial measurement, not a before/after speedup or a representative photographic XPM benchmark.

| Format | Encode ms | Output bytes |
|---|---:|---:|
| PCX | 288.69 | 90,000,128 |
| SGI | 64.48 | 96,000,512 |
| Sun Raster | 153.17 | 72,000,032 |
| Farbfeld | 91.98 | 192,000,016 |
| WBMP | 91.04 | 3,000,006 |
| XBM | 143.53 | 15,004,107 |
| XPM | 1,145.66 | 192,021,700 |
| GBR | 2.29 | 96,000,039 |
| GIMP PAT | 2.27 | 96,000,035 |
| CUR (256 × 256) | 0.12 | 270,398 |
| ICNS (1024 × 1024) | 5.23 | 6,539 |

PhotoCraft's explicitly marked raster SVG wrappers reopen through the embedded PNG, retaining
pixel depth, ICC and DPI; other SVG drawings continue through the existing vector importer.
The fork port supports reopening these raster wrappers; its older tree has no general SVG importer.
