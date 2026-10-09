# Experimental Android build

This target builds the existing Rust editor as a NativeActivity APK. It is a build experiment, not a supported mobile release. Do not use it for important documents. The Android UI, graphics drivers, soft keyboard, and activity lifecycle need runtime testing.

## Build

Run on `x86_64-linux` with Nix flakes enabled:

```sh
nix build .#photocraft-android --out-link result-android
```

The output contains `photocraft-0.3.0-android-unsigned.apk` and the ARM64 shared library. The output version follows `Cargo.toml`. `photocraft-android-x86_64` selects the emulator ABI instead; that output is separate from the phone build.

The flake uses Rust 1.98.1 from rust-overlay, NDK 29.0.14206865, build-tools 36.1.0, and Android platform 36. The manifest sets minimum API 28 and target API 36. The Nix expression explicitly accepts the [Android SDK licence](https://developer.android.com/studio/terms) for these tools. No Gradle, Java application code, or webview is used.

## Sign and install

Signing happens outside the Nix store:

```sh
nix run .#photocraft-android-sign -- photocraft-test.apk
```

This creates or reuses `~/.android/debug.keystore`. Set `ANDROID_DEBUG_KEYSTORE` to use another debug keystore. Keep the same key for later updates. The script refuses to overwrite an existing output APK.

With a compatible ARM64 device connected and authorized in ADB:

```sh
nix run .#photocraft-android-sign -- --install photocraft-device-test.apk
```

The installer uses `adb install -r` and launches `ai.storyteller.photocraft/android.app.NativeActivity`. It never uninstalls an existing app to resolve a signing conflict. Uninstalling deletes app-private documents and preferences. Back them up first.

Build and signature checks do not establish runtime compatibility. Check startup, painting, graphics, suspend/resume, rotation, and folding on the target device before relying on it.

## Full immersive mode

The APK requests full immersive mode on native-window creation, resume, and focus gain. Android's status, navigation, and caption bars are requested hidden. An edge swipe can reveal transient system bars; this is not kiosk mode and does not disable system navigation. Android can still show required system UI, such as the keyboard or window-manager controls.

On API 30 and later the native backend uses `WindowInsetsController` with `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`. API 28/29 use the legacy immersive-sticky flags. The calls run on the Java UI thread through the activity's existing owned JNI reference. No raw JNI pointer conversion or additional unsafe-code exception is introduced.

The source is `apps/photocraft-android/src/immersive.rs`, copied into the APK's patched android-activity 0.6.1 dependency. winit 0.30.13's Android fullscreen flag is a no-op, so setting a desktop viewport flag alone does not provide this behavior.

Verify on the tablet that the bars hide at startup, an edge swipe can reveal them, and the app returns to immersive mode after background/resume. Check keyboard entry and multi-window separately; operating-system policy can limit immersion there.

## Automatic palm rejection

The Android APK rejects finger and unknown touch contacts while Android reports a pen hovering or touching the screen. Touch resumes 250 ms after the pen leaves. A contact rejected during that interval stays blocked until it lifts; moving a resting palm after the delay does not restart a stroke. Android contacts explicitly marked as palms are always rejected. Mouse contacts are not rejected by this policy.

If a finger was already down when the pen arrived, the app cancels that contact before delivering pen input. Cancellation releases the held pointer without clicking a control. It does not undo marks made before the pen was detected. Focus loss and suspend clear pen proximity and cancel active contacts so input does not stay locked after returning to the app.

This applies across the app, including menus. Finger gestures are unavailable while the pen is nearby. Detection depends on Android reporting the correct tool type and hover lifecycle. No pressure threshold or contact-size guess is used. Pen pressure still passes through to the brush; tilt and eraser-tool switching are not added here.

### Dependency patch

`nix/android-input.nix` patches only the APK's vendored winit, egui-winit, egui, and android-activity sources. Desktop and web builds use the original locked crates. winit keeps pen identity until after arbitration. egui-winit then maps cancellation to an explicit egui pointer-cancel event, which releases without a click. The original `PointerGone` behavior remains unchanged for mouse drags.

The policy source is `apps/photocraft-android/src/palm_rejection.rs`. Nix copies that exact file into winit, and host tests compile the same file. The patch pins winit 0.30.13 and egui/egui-winit 0.36.2; dependency upgrades must rebase and retest it. File checksums are refreshed after the reviewed patch while retaining the locked package checksum.

Use the Nix APK build for this behavior. A direct Cargo build outside the patched vendor environment does not include palm rejection.

### Tablet acceptance test

Use a disposable document. Verify these on the physical tablet after installation:

1. With the pen away, finger input still works.
2. Hover the pen, rest a palm, and draw with the pen. The palm must not paint, pan, or activate controls.
3. Lift the pen but keep the palm resting for more than 250 ms. The resting contact must stay rejected.
4. Lift all contacts, wait 250 ms, then start a new finger gesture. Touch must work again.
5. Touch first, then bring the pen near. The old touch must stop without a click or a stuck drag.
6. Background and resume the app while the pen is nearby. Verify that neither pen nor touch stays stuck.

Logcat messages under `photocraft.palm` report pen proximity and the number of blocked contacts. They contain no drawing coordinates. Automated tests cover the policy and real egui pointer state, but cannot prove a physical digitizer's palm behavior.

## Limits

- The app uses the desktop UI unchanged. Touch layout and keyboard entry are not validated.
- File Open reports that the Android system picker is not implemented.
- Save and Export write into the app-specific external files folder when available, or private storage otherwise. Access from other apps and USB depends on Android storage restrictions. Files are deleted when the app is uninstalled.
- Saves refuse to overwrite existing files, including a second Save to the same name. This avoids silent replacement without an overwrite dialog. It is not a complete document workflow.
- Preferences use atomic replacement. There is no Android autosave or recovery after process death.
- Clipboard images, pen tilt and eraser-tool switching, the desktop control server, and display profile detection are absent. Pen pressure is forwarded through winit's touch events.
- Android screen-reader support is disabled. eframe requires GameActivity for that backend; this prototype uses NativeActivity. Desktop and web accessibility remain enabled.

## Entry-point exception

This prototype requests maintainer approval for one exception to the unsafe-code rule: `#[unsafe(no_mangle)]` on `android_main` in `apps/photocraft-android/src/android.rs`. Android's native glue calls that symbol by name with the Rust ABI and the same pinned `android-activity` type. No unsafe memory operation was added. The rest of the crate keeps `unsafe_code = "deny"`.

## Checks

The APK derivation runs the host policy/storage tests, tests the patched egui cancellation behavior, runs Clippy on the Android target, and checks package metadata, ABI, ZIP alignment, both native entry symbols, and ELF load-segment alignment for 16 KiB pages. The signing command verifies its generated signature. These are artifact checks, not device tests.

Run `scripts/check-android-features.py` to check that eframe accessibility stays enabled on desktop and web but is excluded from the NativeActivity build.

Host tests in `apps/photocraft-android/src/storage.rs` exercise actual temporary files, overwrite refusal, path containment, preference replacement, and panic conversion. The dependency-layer checker includes the new app. Building only `photocraft-android` is intentional: a whole-workspace Android build would also try the unsupported desktop executable and can unify desktop-only Cargo features.
