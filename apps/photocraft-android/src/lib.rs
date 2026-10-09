//! PhotoCraft on Android.
//!
//! Runs the same [`photocraft_ui_egui::PhotocraftApp`] as the desktop app through eframe's
//! native runner on an `android.app.NativeActivity` (wgpu: Vulkan, GLES as a fallback). Build the
//! APK with `nix build .#photocraft-android`; see `docs/development.md` ("Android build").
//!
//! Differences from the desktop app:
//! - no TCP control server and no MCP bridge;
//! - no File › Open dialog yet (Android's Storage Access Framework needs JNI);
//! - File › Save writes a new file in the app-specific files folder and refuses overwrites;
//! - no Android screen-reader support or lifecycle recovery yet;
//! - no clipboard images, pen tilt/eraser integration, or display profile reader;
//! - automatic palm rejection in the Nix APK's patched native input backend;
//! - logs and panics go to logcat with the tag `photocraft`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
mod android;

#[cfg(any(target_os = "android", test))]
mod storage;

// Nix compiles this same policy into patched winit, before tool identity is lost.
#[cfg(test)]
mod palm_rejection;
