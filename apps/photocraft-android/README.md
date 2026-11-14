# photocraft-android

PhotoCraft on Android: a small `apps/` crate that fills PhotoCraft's [`Services`] struct with Android
equivalents — the window, the file dialogs, the storage layout — and leaves the engine, the UI and the
document model untouched.

`apps/photocraft` is not compiled, so `rfd`, `arboard`, `open` and its config-directory code never
enter the graph.

## What the shell does

- **Entry point** — builds as `libmain.so`, loaded with `System.loadLibrary`; `android_main` is the
  usual `android-activity` entry.
- **Storage** — everything lives under `AndroidApp::internal_data_path()`: `config/`, `Recovery/`,
  `import/` for SAF copies and `export/` for scratch saves, next to PhotoCraft's own `app.ron`.
- **File dialogs** — the `Services` file-dialog seam is a request/reply channel: the UI asks, the
  shell starts `ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT` on a worker, and the answer comes
  back as a path on a later frame. Picked documents are copied into `import/`; a save writes to
  `export/` first and is published to the chosen `content://` destination afterwards. A document
  opened through SAF also remembers its source URI, so File ▸ Save can write it back.
- **Opening a file** — completed picks go through `Services::os_events` (`OsEvent::Open`), the seam
  the desktop uses for Finder's "Open With". Open Recent, the remembered path and every warning stay
  PhotoCraft's own code.
- **Phone layout** — the docked Color/Properties/Layers panels are hidden at start-up (a 250 pt dock
  leaves ~110 pt of canvas on a 411 pt phone) and are reachable from the touch command palette and
  PhotoCraft's own `Edit ▸ Search…`. Re-opened panels are remembered by the app itself.
- **Command palette** — long-press the title strip for 500 ms; the buttons are PhotoCraft's own
  command ids executed through the app's control channel, so they behave like the menus.
- **Self-test** — a background thread makes a document through the injected `Services`
  (`file.new → edit.fill → save_as(PSD) → save_as(PNG) → open_path`), proving the import, export and
  write seams on the device.
- **Fonts / touch** — the `/system/fonts` CJK face is appended to every family after the theme's
  fonts are live; 40 pt hit targets, no zoom.

## Known gaps

- The system clipboard and `open_url` are not wired.
- Commands that ask for *bytes* in the same frame (Place Embedded, load-brush, scripts) fall back to
  "open the picked file as a new document"; multi-file File ▸ Open is the supported path.
- `background_jobs` stays off, so a very large file occupies the UI thread while it opens.
- HEIF is not enabled, and pen pressure is not wired (`crates/tablet` has macOS/X11 back ends only).

## Build

```bash
cargo ndk -t arm64-v8a -o apps/photocraft-android/jniLibs build --release -p photocraft-android
cd apps/photocraft-android/android && ./gradlew assembleRelease
```

`jniLibs/` and the Gradle output are gitignored; the APK is not part of this change.
