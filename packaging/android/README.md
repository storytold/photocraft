# PhotoCraft on Android

PhotoCraft on Android brings the native Rust engine and `egui` interface to Android phones and tablets using `eframe` and `android-activity` on the `wgpu` Vulkan backend.

---

## 1. Phone-First Adaptations

PhotoCraft is designed for desktop monitors by default. Running on a 6-inch phone screen requires distinct mobile ergonomics:

* **Canvas-First Layout:** On phone screens in portrait (typically 360–430 pt wide), the right-hand panel dock (Layers, Channels, Navigator) is collapsed by default. The bottom status bar is hidden, leaving the entire screen for the image canvas.
* **Finger-Friendly Touch Targets:** Button paddings, touch hitboxes, and sliders are enlarged to at least 44 pt to meet touch accessibility standards.
* **On-Screen Modifier Keys:** Sticky Shift, Ctrl, and Alt buttons are enabled (`app.ui.shell.modifier_keys`), allowing touch users to add/subtract selections and duplicate objects without a physical keyboard.
* **Stylus & S-Pen Pressure:** Active pen input (such as Samsung S-Pen) passes pressure and coordinates directly to the brush engine.
* **App Sandbox Storage:** Preferences, autosave, and recovery logs live safely inside Android's internal app sandbox (`/data/user/0/ai.storyteller.photocraft/`).

---

## 2. Prerequisites

1. **Rust target:**
   ```bash
   rustup target add aarch64-linux-android
   ```
2. **Android NDK:**
   * NDK r25c or newer installed.
   * Set the environment variable:
     ```bash
     export ANDROID_NDK_HOME=/path/to/android-ndk
     ```
3. **Packaging Tool (`cargo-apk`):**
   ```bash
   cargo install cargo-apk --locked
   ```

---

## 3. Building the APK

### Build with packaging script:
```bash
./packaging/android/package.sh --release
```
The resulting `.apk` will be written to `dist/release/photocraft-android-<version>.apk`.

### Debug Run on Connected Device / Emulator:
```bash
cargo apk run -p photocraft-android --target aarch64-linux-android
```

---

## 4. Installing on Device via ADB

```bash
adb install -r dist/release/photocraft-android-<version>.apk
adb logcat -s PhotoCraft:V
```

