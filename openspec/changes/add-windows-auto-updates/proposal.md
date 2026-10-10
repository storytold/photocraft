## Why

PhotoCraft users with a Windows MSI installation currently need to visit the release page, identify their architecture, and install updates manually. Showing the available version in Help > About and applying the matching official MSI there makes staying current easier while keeping installation tied to the user's explicit action.

## What Changes

- Check the latest stable PhotoCraft release in the official GitHub repository and show its version beside the installed version in Help > About.
- Add an Update action beside the About dialog's confirmation button, with clear checking, available, current, and error states.
- Download the MSI matching the current Windows architecture, verify its published SHA256 checksum, and run the installer after protecting open work.
- Enable direct updating only for MSI-managed installations; link other installation types to the official release page.
- Provide a link to the official release page when checking or installing cannot complete.

## Capabilities

### New Capabilities
- `windows-auto-update`: Check official stable releases and safely update Windows MSI installations.

### Modified Capabilities

## Impact

- Affected UI: `crates/ui-egui/src/dialogs.rs` and the About dialog state.
- Affected desktop services: `apps/photocraft/src/services.rs` and Windows MSI install detection, download, and installer lifecycle.
- Affected dependencies and packaging: HTTPS release metadata/download access, SHA256 verification, and the Windows MSI release assets; no update behavior is added to non-Windows builds or the Web build.
- Updates use the official GitHub Releases API and assets over HTTPS. The PhotoCraft website remains an official fallback link and currently directs users to those releases.
