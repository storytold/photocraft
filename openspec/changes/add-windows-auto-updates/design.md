## Context

The About dialog is rendered by the shared `photocraft-ui-egui` crate and currently displays `photocraft_engine::build_info::long_version()`. The desktop executable injects operating-system services through `Services`; the web build does not have filesystem or installer authority. Official Windows stable releases provide x64, x86, and ARM64 MSI assets plus `SHA256SUMS.txt`. PhotoCraft's MSI records its installation directory in the machine registry and installs per-machine.

## Goals / Non-Goals

**Goals:**
- Check official stable release metadata asynchronously and show update status in the About dialog.
- Match the package to the running Windows process architecture and verify release asset integrity before installation.
- Protect unsaved documents before starting the installer.

**Non-Goals:**
- Automatic updates for macOS, Linux, FreeBSD, or the web build.
- Installing draft or prerelease builds.
- Running a silent installer or elevating without the user's consent.
- Direct updates for portable ZIP or manually copied installations; provide the official release link for those installations.
- Updating files outside the PhotoCraft MSI-managed installation.

## Decisions

- **Use the official GitHub Releases API and assets as the update source.** The official website points users to these releases; using one structured source avoids scraping website markup. Keep the official release page link as the recovery path.
- **Check from the About view on a background worker.** Keep blocking HTTP, metadata parsing, downloads, and hashing off the egui frame loop. Represent checking, available, current, download/apply progress, and failure as explicit UI state delivered back through the injected desktop service boundary.
- **Use only the latest stable release and compare parsed versions.** Drafts and prereleases must not be selected. Match the MSI asset suffix to the running process architecture. Enable the Update action only when the executable's normalized path is inside the PhotoCraft install directory registered by the MSI; otherwise keep the app unchanged and provide the release page link.
- **Stream to a temporary file, then verify against `SHA256SUMS.txt`.** Do not extract or launch any package until its exact asset name has a valid checksum match. Bound network time and handle missing assets, HTTP failures, and malformed metadata as ordinary recoverable errors.
- **Keep application-specific update mechanics in the Windows desktop app.** The shared About UI receives status and update callbacks, while the desktop service knows the install directory and platform. This keeps update authority out of `ui-egui` and leaves web behavior unchanged.
- **Install the verified MSI via Windows Installer after the normal close safeguards.** A temporary copy of the running executable acts as a helper: it waits for PhotoCraft to exit, invokes `msiexec` with the downloaded MSI, and cleans up temporary update files. Use the standard interactive installer so Windows can obtain elevation. Do not request credentials or invoke a hidden elevated process.
- **Preserve the visible installer flow.** Do not pass `/qn` or `/quiet`; launch `msiexec` from the helper so Windows Installer can display its wizard and UAC prompt.

## Risks / Trade-offs

- **A release may be temporarily incomplete while publishing** → treat missing architecture assets or checksum entries as unavailable and leave the current installation unchanged.
- **The release endpoint may be offline or rate-limited** → make failure non-fatal, keep the app usable, and link to the official release page.
- **The MSI install directory may be customized or registry views may differ by architecture** → compare the current executable with the install location from the appropriate registry view and otherwise use the manual release-page path.
- **A temporary update helper cannot be deleted while it runs** → schedule its deletion for the next reboot after Windows Installer exits.
- **A new updater dependency adds supply-chain and build surface** → use a maintained Rust HTTPS client with a background blocking worker; keep the dependency and TLS features minimal and review its supported API when implementing.

## Migration Plan

No persisted data migration is required. Older PhotoCraft builds and non-MSI installations remain manually updatable from the official release page. A failed MSI update leaves the installed application and its settings untouched; the helper opens the official release page if Windows Installer fails.
