## 1. Release metadata and package verification

- [x] 1.1 Add Windows-target-only HTTPS, semantic-version, and SHA256 dependencies.
- [x] 1.2 Parse the latest stable release and select the MSI asset matching the running architecture; cover tags, prereleases, missing assets, and architecture mapping with tests.
- [x] 1.3 Download release metadata and the MSI on a background worker, validate HTTPS GitHub release hosts, stream into a temporary file with bounded timeouts, and expose progress/errors.
- [x] 1.4 Parse `SHA256SUMS.txt` and verify the exact MSI asset before allowing installation; cover malformed manifests and digest mismatch with tests.

## 2. MSI install detection and handoff

- [x] 2.1 Read the MSI-recorded `HKLM\Software\PhotoCraft\InstallDir` using the appropriate registry view and enable direct update only when the running executable belongs to that install directory.
- [x] 2.2 Add a temporary helper mode that waits for the app process to exit, starts the verified MSI with interactive `msiexec`, and cleans up staged update files.
- [x] 2.3 Route the update through the existing app close flow so save, discard, and cancel safeguards run before the helper installs the update.
- [x] 2.4 Cover install-path matching, dirty-work confirmation and cancellation, helper argument validation, and installer launch failure with focused tests.

## 3. About dialog integration

- [x] 3.1 Add updater status and actions to the desktop service boundary without adding network or installer behavior to the Web build.
- [x] 3.2 Show the available version beside the installed version and add the Update button beside OK, with checking, current, download progress, and error states.
- [x] 3.3 Add English source messages and translations in all complete language catalogs; provide the official release page for non-MSI installs and recoverable failures.
- [x] 3.4 Cover About status rendering and button actions with UI tests.

## 4. Documentation and validation

- [x] 4.1 Document the Windows MSI update behavior and manual update path for other installation types.
- [ ] 4.2 Run the applicable formatting, Windows build, focused tests, and OpenSpec validation; inspect the About dialog on Windows.
