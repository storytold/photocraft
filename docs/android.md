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

## Limits

- The app uses the desktop UI unchanged. Touch layout and keyboard entry are not validated.
- File Open reports that the Android system picker is not implemented.
- Save and Export write into the app-specific external files folder when available, or private storage otherwise. Access from other apps and USB depends on Android storage restrictions. Files are deleted when the app is uninstalled.
- Saves refuse to overwrite existing files, including a second Save to the same name. This avoids silent replacement without an overwrite dialog. It is not a complete document workflow.
- Preferences use atomic replacement. There is no Android autosave or recovery after process death.
- Clipboard images, pen pressure, the desktop control server, and display profile detection are absent.
- Android screen-reader support is disabled. eframe requires GameActivity for that backend; this prototype uses NativeActivity. Desktop and web accessibility remain enabled.

## Entry-point exception

This prototype requests maintainer approval for one exception to the unsafe-code rule: `#[unsafe(no_mangle)]` on `android_main` in `apps/photocraft-android/src/android.rs`. Android's native glue calls that symbol by name with the Rust ABI and the same pinned `android-activity` type. No unsafe memory operation was added. The rest of the crate keeps `unsafe_code = "deny"`.

## Checks

The APK derivation runs Clippy on the Android target and checks package metadata, ABI, ZIP alignment, both native entry symbols, and ELF load-segment alignment for 16 KiB pages. The signing command verifies its generated signature. These are artifact checks, not device tests.

Run `scripts/check-android-features.py` to check that eframe accessibility stays enabled on desktop and web but is excluded from the NativeActivity build.

Host tests in `apps/photocraft-android/src/storage.rs` exercise actual temporary files, overwrite refusal, path containment, preference replacement, and panic conversion. The dependency-layer checker includes the new app. Building only `photocraft-android` is intentional: a whole-workspace Android build would also try the unsupported desktop executable and can unify desktop-only Cargo features.
