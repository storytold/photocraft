# Feature naming

PhotoCraft names its own raw-processing workspace **RAW Develop**, the adjustment **RAW Develop Filter**, and its raw preferences **RAW Defaults**. All 34 shipped non-English catalogs use the new keys and localized labels.

## Compatibility boundary

The `filter.cameraRaw` command, raw preference IDs, Rust type names and serialized settings remain stable. Existing documents retain their own saved filter names. New filters get the neutral display name.

Adobe Camera Raw PSD/action descriptor class names, XMP namespaces and the settings-file compatibility descriptions are unchanged. Import/export mapping code and attribution are unchanged.

A Photoshop action's step name is stored in the action file, so a saved Camera Raw step keeps Photoshop's "Camera Raw Filter" wording and a loaded action round-trips unchanged. Only PhotoCraft's own step labels, menus and windows say RAW Develop Filter.

## Validation before readiness

Review menu and dialog layout in the supported interfaces, run the engine/UI tests and the existing PSD/action/XMP round-trip tests, and complete the normal platform gates. This is a feature naming change, not a legal determination.

The catalog cleanup removes 136 dormant rows for branded composer labels, PDF presets, old Photoshop defaults and the unused raw-opening label. No active non-localization call sites use those keys. Current PhotoCraft defaults and required compatibility descriptions remain intact. Regression guards cover production Rust display literals and catalog keys; explicit exceptions document interchange descriptions and identifiers.

## TIFF Byte Order and Pixel Order (2026-11-20)

Checked for the TIFF Options byte order and pixel order (#15908), on 2026-11-20.

- **Not used: "IBM PC" and "Macintosh".** They are the names of other companies' computers, which the naming standard keeps out of our labels. Scripts, the control channel, MCP and the CLI say `little` and `big` (`tiffByteOrder`, `--tiff-byte-order`), and help text says little-endian (`II`) and big-endian (`MM`), the TIFF 6.0 specification's own terms.
- **Byte Order, Pixel Order, Interleaved, Per Channel, little-endian, big-endian: kept as ordinary descriptive words.** The TIFF 6.0 specification names the byte order of the header and the PlanarConfiguration field; unrelated products use the same words for the same choices in their own TIFF writers: FME's TIFF writer (Byte Order, Interleaving Type), Aspose.Imaging's TiffOptions (ByteOrder), Accusoft ImagXpress (ByteOrder), Oracle Outside In (TIFF byte order) and PTC IsoDraw's TIFF export (Byte Order). No trademark register search (USPTO, EUIPO, TMview, WIPO) was run for them; they are generic technical terms.
- **Not shipped as dialog labels yet.** The TIFF Options dialog has no rows for these options: its rows come from the framework's TIFF Options form. When rows are added, use "Byte Order: Little-endian / Big-endian" and "Pixel Order: Interleaved / Per Channel", not Photoshop's computer names.
