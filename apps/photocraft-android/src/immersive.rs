//! Copied into android-activity's native backend by Nix. Uses its owned JNI activity reference
//! on the Java UI thread, not raw-pointer conversion or additional unsafe-code exceptions.

#![forbid(unsafe_code)]

use jni::{errors::Result, objects::JObject, refs::Global, JavaVM};

jni::bind_java_type! {
    Activity => "android.app.Activity",
    type_map { Window => "android.view.Window" },
    methods { fn get_window() -> Window }
}

// Keep the API 30 methods in a separate binding. Resolving the legacy window's method cache
// must not look up newer methods on API 28/29.
jni::bind_java_type! {
    Window => "android.view.Window",
    type_map { View => "android.view.View" },
    methods { fn get_decor_view() -> View }
}

jni::bind_java_type! {
    ModernWindow => "android.view.Window",
    type_map { InsetsController => "android.view.WindowInsetsController" },
    methods {
        fn get_insets_controller() -> InsetsController,
        fn set_decor_fits_system_windows(fits: bool),
    }
}

jni::bind_java_type! {
    InsetsController => "android.view.WindowInsetsController",
    methods {
        fn set_system_bars_behavior(behavior: i32),
        fn hide(types: i32),
    }
}

jni::bind_java_type! {
    InsetsType => "android.view.WindowInsets$Type",
    methods { static fn system_bars() -> i32 }
}

jni::bind_java_type! {
    View => "android.view.View",
    methods { fn set_system_ui_visibility(visibility: i32) }
}

/// Must be scheduled with AndroidApp's Java-main-thread dispatcher. Failures are reported by
/// the caller and retried on the next native-window, resume, or focus-gained event.
pub(crate) fn apply(activity: Global<JObject<'static>>, sdk: i32) -> Result<()> {
    JavaVM::singleton()?.attach_current_thread(|env| -> Result<()> {
        let activity = env.cast_global::<Activity>(activity)?;
        let window = activity.get_window(env)?;
        if sdk >= 30 {
            let window = env.cast_local::<ModernWindow>(window)?;
            window.set_decor_fits_system_windows(env, false)?;
            let controller = window.get_insets_controller(env)?;
            if controller.is_null() {
                return Err(jni::errors::Error::NullPtr("window insets controller is not attached yet"));
            }
            // WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE.
            controller.set_system_bars_behavior(env, 2)?;
            let bars = InsetsType::system_bars(env)?;
            controller.hide(env, bars)?;
        } else {
            // IMMERSIVE_STICKY | LAYOUT_STABLE | LAYOUT_HIDE_NAVIGATION | LAYOUT_FULLSCREEN |
            // HIDE_NAVIGATION | FULLSCREEN. Only for the API 28/29 fallback.
            window.get_decor_view(env)?.set_system_ui_visibility(env, 0x1706)?;
        }
        log::info!(target: "photocraft.immersive", "immersive system bars requested (API {sdk}); edge swipes reveal transient bars");
        Ok(())
    })
}
