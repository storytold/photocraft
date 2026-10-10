//! PhotoCraft on Android.
//!
//! Runs the engine and egui shell through eframe's Android runner (wgpu on Vulkan,
//! driven by `android-activity`).
//!
//! Mobile adaptations:
//! - Responsive touch scale factor (touch targets adapted for phone screens);
//! - Canvas-first layout with collapsible right dock and hidden status bar by default;
//! - Touch gestures (two-finger pan and pinch-to-zoom);
//! - Stylus / pen pressure support;
//! - Android application storage sandbox for preferences and crash recovery.

#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod mobile;

#[cfg(target_os = "android")]
mod android_app;

#[cfg(target_os = "android")]
// SAFETY: Required by android-activity to export the C entry point for Android NativeActivity.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_app::start(app);
}

/// Helper function to query whether this build is compiled for Android.
pub fn is_android_target() -> bool {
    cfg!(target_os = "android")
}

