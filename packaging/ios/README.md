# Native iPadOS build

This target runs the same Rust engine and egui UI as desktop PhotoCraft. Xcode creates and signs
the app bundle; the executable itself is built by Cargo and enters UIKit through eframe/winit.
There is no web view and no JavaScript runtime.

## Experimental community modification

This is an experimental modification of PhotoCraft, not an official iPadOS release or an
endorsement by the upstream maintainers. Upstream inclusion and any official distribution
require a pull request, maintainer review and approval. A local build or commit is not approval.

The contribution remains under the project's existing [MIT](../../LICENSE-MIT) OR
[Apache-2.0](../../LICENSE-APACHE) licenses; their notices and terms are preserved. To the extent
permitted by applicable law, this modification is provided **AS IS**, without warranties, and
its contributor disclaims liability for damage or loss arising from its use, including loss of
documents or data, as provided by those licenses. This notice does not override applicable law
or add restrictions to the existing licenses. Use copies of test documents while evaluating it.

For installation without Xcode on the tester's computer, see [distribution options](DISTRIBUTION.md).
No public installable release or TestFlight invitation is provided by this change.

## Prerequisites

- macOS with Xcode, its command-line tools selected, and an iOS SDK. Physical-device testing
  used Xcode 27; compatibility with older Xcode versions has not been verified.
- An ARM64 iPad running iPadOS 17 or later, or an Apple Silicon Mac with an iPad simulator runtime.
  Only the M4 iPad Pro on iPadOS 27.2 beta was tested physically. The deployment minimum does not
  imply that every older iPad/OS combination has been tested. Intel simulator builds are unsupported.
- Rust 1.95 with the device and simulator targets:

  ```sh
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim
  ```

- XcodeGen when regenerating the checked-in Xcode project:

  ```sh
  brew install xcodegen
  cd packaging/ios
  xcodegen generate
  ```

## Build

Use a complete checkout of this repository, not only `apps/photocraft-ios`. Cargo downloads the
locked dependencies, including the pinned public `storytold/craft-libs` Git dependency. No
private fonts checkout or local signing files from another contributor are needed. The Xcode
project is checked in, so XcodeGen is optional for building. Commands below run from the root
of the checkout unless stated otherwise.

Open `PhotoCraft.xcodeproj`, select the PhotoCraft scheme and an iPad destination, then Run. For a
physical iPad, copy `Signing.local.xcconfig.example` to `Signing.local.xcconfig` and fill in your
team ID and a bundle identifier available to your team. This file is gitignored; do not commit
personal team IDs, provisioning profiles, certificates or Xcode user settings. Regenerating the
project preserves local signing. `Signing.xcconfig` supplies portable defaults when no local file
exists. You still need to sign in under Xcode Settings → Apple Accounts.

To build for a connected device (from the repository root):

```sh
xcodebuild -project packaging/ios/PhotoCraft.xcodeproj -scheme PhotoCraft \
  -configuration Debug -destination 'id=YOUR_DEVICE_UDID' \
  -derivedDataPath target/ios-derived \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration build
```

The latter option lets Xcode register the selected physical device with your development team.
Install `target/ios-derived/Build/Products/Debug-iphoneos/PhotoCraft.app` with Xcode or
`xcrun devicectl device install app --device YOUR_DEVICE_UDID <app-path>`.
If iPadOS requires it, trust your own developer account under Settings → General → VPN & Device
Management. Keep the iPad unlocked during installation and launch. A successful install is not
a successful launch: verify the editor remains visible and responds to input.

A signing-free command-line build for the simulator is:

```sh
xcodebuild -project packaging/ios/PhotoCraft.xcodeproj \
  -scheme PhotoCraft \
  -sdk iphonesimulator \
  -configuration Debug \
  CODE_SIGNING_ALLOWED=NO \
  build
```

## Current scope

On the canvas, fingers navigate/zoom and Apple Pencil draws. The iOS input wrapper suppresses
the first finger's emulated mouse press before a second finger arrives, retaining raw touches
for pinch zoom. Buttons and popup menus remain finger-operable. This relies on winit's iPad
force metadata (Pencil: present, including zero; finger: absent); validate on other hardware.

The native app shell, Rust UI and Metal renderer build for iPadOS device and simulator targets.
On 2026-10-22, a signed Debug build was installed and launched on a physical M4 iPad Pro running
iPadOS 27.2 beta. The user confirmed visible editor UI, touch input and Apple Pencil pressure.
Pressure accuracy/tilt have not been instrumented. The user confirmed the safe-area menu fix.
Pencil hover now uses a Pencil-only `UIHoverGestureRecognizer` on the rendering view. Its
movement feeds egui pointer input; leaving proximity clears the pointer. Hover never
creates button events and yields to real touch input throughout a stroke and its release frame.
UIKit points are converted by egui zoom without subtracting the safe-area origin. The recognizer
does not cancel or delay drawing touches; its action target is retained on the main thread.
The tester confirmed hover movement works, but the brush outline is not visible. Treat the
brush preview as incomplete. Hover distance, tilt and barrel roll are not yet used to alter it.

Reference: [Apple's Pencil hover guide](https://developer.apple.com/documentation/uikit/adopting-hover-support-for-apple-pencil).
File > Open now offers Photos or Files (including iCloud providers). One selected image is
loaded into a new document using its contents, without retaining a temporary path or modifying
the original. Photos uses the system PHPicker, without broad photo-library authorization;
Files imports a local copy. Reads are limited to 128 MiB and errors/cancellation are reported
through the existing asynchronous dialog service. The native picker integration requires
physical-device confirmation. Save/export/share and a touch-adapted workspace remain follow-ups.

## UIKit scene compatibility

iPadOS 27 rejects apps built against SDK 27 that do not adopt `UIScene`. The original build
trapped in `UIApplicationEvaluateRuntimeIssueForNoSceneLifecycleAdoption` before rendering.
The manifest now declares one `UIWindowScene` and `PhotoCraftSceneDelegate`. The small Rust
adapter in `apps/photocraft-ios/src/scene.rs` registers that class before `UIApplicationMain`,
attaches eframe's window to its connected scene, and retains it for reconnection. Multiple scenes
are deliberately disabled: this is a single-editor-window bridge, not multi-window support.

UIKit continues to post application lifecycle notifications with scenes enabled; winit receives
its existing events without synthetic duplicate resume/suspend notifications. The adapter is
the only iOS module allowed `unsafe`, with documented ABI and borrowed-handle invariants. The
engine, shared UI and desktop app are unchanged. No SDK-version spoofing, registry-cache patches,
Swift sources or private UIKit APIs are used. Track
[winit #4224](https://github.com/rust-windowing/winit/issues/4224) and remove the adapter once the
pinned upstream stack implements scene attachment itself.

References: [Apple scene migration](https://developer.apple.com/documentation/uikit/transitioning-to-the-uikit-scene-based-life-cycle),
[scene activation notifications](https://developer.apple.com/documentation/uikit/uiscenedelegate/scenedidbecomeactive(_:)).

## Verification

The preview was synchronized with upstream `main` at `952c328b3` (PhotoCraft 0.17.0) on
2026-10-31. The iOS import service uses the updated `ImportOptions` contract, the Objective-C
runtime dependency matches the desktop application's pin, and the Xcode marketing version is
0.17.0. An unsigned generic-device Debug build passed. This synchronization has not yet been
installed or re-tested on a physical iPad; earlier device observations below refer to the
previous build. Distribution/signing requirements still apply.

The iOS shell refreshes egui safe-area insets from the attached UIKit window every frame.
This reserves the status bar and home indicator even when initial winit events arrived before
scene attachment, and follows rotation/resume. Insets are converted from UIKit points using
egui's zoom factor; pointer coordinates and the shared desktop editor are unchanged.
The shell also constrains the editor's child Ui to `content_rect`: egui 0.36.2 builds its
root Ui using `viewport_rect`, so supplying insets alone does not move root panels.
Five host unit tests cover inset conversion, zoom, rotation values, invalid inputs and actual
menu-panel bounds in portrait/landscape frames.
The tester confirmed menu placement is fixed. Full rotation/resume coverage remains incomplete.

```sh
python3 packaging/ios/test_packaging.py
python3 packaging/ios/check_staged_privacy.py # after staging, before committing
cargo test -p photocraft-ios
cargo clippy -p photocraft-ios --target aarch64-apple-ios --no-deps -- -D warnings
cargo xtask layers
```

The host tests guard the scene manifest/class registration contract and portable signing, not
UIKit execution. Physical-device checks must cover cold launch, touch/Pencil input, background
and resume, rotation and scene reconnection. Cold launch and basic input were verified; the
remaining lifecycle cases are not yet claimed as tested. A simulator was not used for this fix.

The privacy check examines staged file contents and rejects common home paths, email addresses,
device IDs, signing identities, keys and build outputs. Review the staged diff as well. Commit
author/committer metadata is separate: use a public username and your GitHub-provided noreply
email if you do not want your personal name/email published. Existing upstream authorship and
license notices should be preserved.
