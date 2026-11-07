# photocraft-mobile

PhotoCraft on iPad and Android tablets: the same app as the desktop and web builds, hosted in a
native shell by [`craft-mobile`](https://github.com/storytold/craft-libs/tree/main/crates/mobile)
(SwiftUI with CraftKit on iPadOS, Jetpack Compose with craftkit on Android). `src/lib.rs` is the
Rust side; `ios/` and `android/` are the shells.

## The craft-libs checkout

The Rust crate comes through Cargo like the other craft-libs crates (the `craft-mobile` pin in the
workspace `Cargo.toml`). The shells also compile craft-mobile's Swift (`swift/CraftKit`) and
Kotlin (`android/craftkit`), so they need craft-libs checked out at that same commit. By default
they look beside the Craft repos:

```text
craft-libs/          git clone https://github.com/storytold/craft-libs, at the pinned commit
photocraft/          this repository
```

Elsewhere, set `CRAFT_MOBILE_DIR` to the crate (`.../craft-libs/crates/mobile`) for `ios/build.sh`
and Gradle.

## iPad

From the repository root:

- Simulator app: `apps/photocraft-mobile/ios/build.sh sim` (or `device`), then
  `xcrun simctl install booted <app>`. `CARGO_TARGET_DIR` is honoured.
- Xcode project (signing, running on a device, debugging): `apps/photocraft-mobile/ios/build.sh xcode`
  generates `ios/PhotoCraft.xcodeproj` with [XcodeGen](https://github.com/yonaskolb/XcodeGen);
  its "Build Rust" phase builds the library (`CRAFT_TARGET_DIR` overrides the target directory).

## Android

From the repository root, with the NDK and [cargo-ndk](https://github.com/bbqsrc/cargo-ndk):

```sh
cargo ndk -t arm64-v8a -P 26 -o apps/photocraft-mobile/android/app/src/main/jniLibs build --release -p photocraft-mobile
cd apps/photocraft-mobile/android && gradle assembleDebug
```
