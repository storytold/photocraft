## Purpose

This capability lets Windows users with an MSI-managed PhotoCraft installation see whether an official stable release is newer and install its matching Windows package without losing unsaved work.

## ADDED Requirements

### Requirement: Show official release status in About
When Help > About opens, the Windows desktop app SHALL check the latest published stable release from the official PhotoCraft GitHub repository without blocking the UI and show the available version beside the installed version.

#### Scenario: A newer stable release is available
- **WHEN** the latest official stable release version is newer than the installed version
- **THEN** About SHALL show the new version beside the installed version and enable the Update action next to the confirmation button

#### Scenario: The installed version is current
- **WHEN** the latest official stable release version is equal to or older than the installed version
- **THEN** About SHALL show that the installed version is current and SHALL NOT offer an update to an older release

#### Scenario: A prerelease or draft is newer
- **WHEN** a newer prerelease or draft release exists
- **THEN** the app SHALL ignore it and compare against the latest published stable release

#### Scenario: The release check fails
- **WHEN** release metadata cannot be reached or parsed
- **THEN** About SHALL show a recoverable error, keep PhotoCraft usable, and provide a link to the official releases page

#### Scenario: Release metadata is loading
- **WHEN** the release check is still running
- **THEN** About SHALL show a checking status without blocking document editing or other UI interaction

#### Scenario: The user retries a failed check
- **WHEN** the user activates the update action after a release check failed
- **THEN** the app SHALL retry the check and keep the official releases page link available

### Requirement: Select and verify the matching MSI package
The app SHALL enable direct updates only for an MSI-managed PhotoCraft installation, select the release MSI for the running Windows architecture, and verify its SHA256 digest against the official release checksum manifest before applying it.

#### Scenario: MSI-managed installation has an update
- **WHEN** the current executable belongs to the installation directory registered by the PhotoCraft MSI and a newer stable release is available
- **THEN** the app SHALL download the MSI for the running architecture and verify its checksum before offering installation

#### Scenario: Installation is not MSI-managed
- **WHEN** the current executable does not match the MSI-registered installation directory
- **THEN** the app SHALL NOT attempt an in-place update and SHALL provide a link to the official releases page

#### Scenario: Matching package is missing or checksum is invalid
- **WHEN** the official release has no matching package or the downloaded package does not match the release checksum
- **THEN** the app SHALL NOT install or extract the package and SHALL show an actionable error with the official releases page link

#### Scenario: Package is downloading
- **WHEN** a matching update package is being downloaded
- **THEN** About SHALL show download progress and keep the app responsive

### Requirement: Apply updates without losing user work
The app SHALL obtain the user's confirmation and honor the existing unsaved-document close safeguards before starting the Windows Installer.

#### Scenario: Update is confirmed with unsaved documents
- **WHEN** the user starts an update while documents have unsaved changes
- **THEN** PhotoCraft SHALL use its existing save, discard, and cancel protections and SHALL start the update only after the user permits the app to close

#### Scenario: User cancels the update
- **WHEN** the user cancels the update or its close confirmation
- **THEN** the running app and its documents SHALL remain available and no installation or file replacement SHALL occur

#### Scenario: MSI installation
- **WHEN** a verified MSI update is confirmed
- **THEN** PhotoCraft SHALL start the matching MSI through Windows Installer, allow Windows to request required elevation, and SHALL NOT silently elevate

#### Scenario: Update cannot be applied
- **WHEN** the verified MSI installer cannot be started or installation fails
- **THEN** the current user data SHALL remain intact and the user SHALL receive an error and a link to install manually from the official releases page
