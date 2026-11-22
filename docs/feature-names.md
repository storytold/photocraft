# Feature naming

PhotoCraft names its own raw-processing workspace **RAW Develop**, the adjustment **RAW Develop Filter**, and its raw preferences **RAW Defaults**. All 34 shipped non-English catalogs use the new keys and localized labels.

## Compatibility boundary

The `filter.cameraRaw` command, raw preference IDs, Rust type names and serialized settings remain stable. Existing documents retain their own saved filter names. New filters get the neutral display name.

Adobe Camera Raw PSD/action descriptor class names, XMP namespaces and the settings-file compatibility descriptions are unchanged. Import/export mapping code and attribution are unchanged.

A Photoshop action's step name is stored in the action file, so a saved Camera Raw step keeps Photoshop's "Camera Raw Filter" wording and a loaded action round-trips unchanged. Only PhotoCraft's own step labels, menus and windows say RAW Develop Filter.

## Validation before readiness

Review menu and dialog layout in the supported interfaces, run the engine/UI tests and the existing PSD/action/XMP round-trip tests, and complete the normal platform gates. This is a feature naming change, not a legal determination.

The catalog cleanup removes 136 dormant rows for branded composer labels, PDF presets, old Photoshop defaults and the unused raw-opening label. No active non-localization call sites use those keys. Current PhotoCraft defaults and required compatibility descriptions remain intact. Regression guards cover production Rust display literals and catalog keys; explicit exceptions document interchange descriptions and identifiers.

## Brush stroke options

Options in Brush Settings > Shape Dynamics that PhotoCraft names in plain descriptive words.

| Label | What it does | Check (2026-11-22) |
| --- | --- | --- |
| Smooth Curves | The stroke follows a curve through the pen's points instead of straight segments between them | Ordinary descriptive words; no product or feature of that name found among drawing apps' brush and smoothing options (Blender, TVPaint, Toon Boom Harmony, Sketchbook name theirs Smooth Stroke, Line Smoothing, Centerline Smoothing, Steady Stroke). No register search was run (WIPO, USPTO, EUIPO open). |
| Taper Start, Taper End | The stroke's pressure grows from nothing over the first pixels and shrinks to nothing over the last, for pointed ends | Descriptive word, used for the same thing by unrelated drawing apps (Procreate, Adobe Fresco's Begin taper and End taper brushes). Not Procreate's Pressure Taper or Touch Taper. No register search was run (WIPO, USPTO, EUIPO open). |
