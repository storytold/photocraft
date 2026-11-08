# Feature naming

PhotoCraft names its own raw-processing workspace **RAW Develop**, the adjustment **RAW Develop Filter**, and its raw preferences **RAW Defaults**. All 34 shipped non-English catalogs use the new keys and localized labels.

## Compatibility boundary

The `filter.cameraRaw` command, raw preference IDs, Rust type names and serialized settings remain stable. Existing documents retain their own saved filter names. New filters get the neutral display name.

Adobe Camera Raw PSD/action descriptor class names, XMP namespaces and the settings-file compatibility descriptions are unchanged. Import/export mapping code and attribution are unchanged.

## Validation before readiness

Review menu and dialog layout in the supported interfaces, run the engine/UI tests and the existing PSD/action/XMP round-trip tests, and complete the normal platform gates. This is a feature naming change, not a legal determination.

The dormant composer/PDF/Photoshop preference translations identified in the inventory are outside this active-feature change. They need a separate call-site and localization catalog cleanup, without altering compatibility labels.
