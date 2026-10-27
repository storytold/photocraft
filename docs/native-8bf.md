# Desktop 8BF filters

PhotoCraft desktop has experimental support for Windows x86 and x64 `.8bf` filter libraries through a separate Rust worker. Windows launches the worker directly; Linux and macOS require an external compatible Wine runtime. The browser remains WASM-only. No Adobe SDK, headers, libraries or installed application are required.

## Install and run

Build PhotoCraft normally, then prepare both Windows worker architectures for the platform:

- **Windows:** run `compat/build-8bf-workers.ps1` in PowerShell with Rust and the MSVC build tools installed. It installs the `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc` Rust targets, builds both workers, and compiles the original test fixtures. Windows packaging invokes this builder when its default worker payload is missing. The editor launches workers directly, regardless of its own architecture; the operating system must support executing each requested worker architecture.
- **Linux:** run `compat/build-8bf-workers.sh` with the Rust targets `i686-pc-windows-gnu` and `x86_64-pc-windows-gnu` and matching MinGW-w64 linkers. Install a Wine runtime capable of running the requested Windows x86/x64 workers.
- **macOS:** build the native editor for Intel or Apple Silicon and supply both Windows worker executables from a worker build or the `native-8bf-workers` CI artifact. The app bundle stores them in `PhotoCraft.app/Contents/Resources/8bf/`. Windows x86/x64 filters require a compatible Wine runtime, including WoW64 and the x86 translation support needed by that runtime and Mac architecture. Native macOS `.plugin` bundles are not handled by this Windows PE bridge. Mac runtime compatibility has not been verified by the checks described here.

For standalone executables, including the CLI, put `photocraft-8bf-worker32.exe` and `photocraft-8bf-worker64.exe` next to the executable. `PHOTOCRAFT_8BF_WORKER32` and `PHOTOCRAFT_8BF_WORKER64` override those paths individually.

For Linux development in this checkout, `cargo build -p photocraft --release` followed by `compat/build-8bf-workers.sh` builds the editor and both workers. Launch with `compat/run-8bf-dev.sh`. This launcher uses separate preferences at `target/8bf-profile` and Wine files at `target/8bf-wine`, leaving another installed PhotoCraft profile untouched. Existing environment overrides still apply.

The platform packaging scripts accept `PHOTOCRAFT_8BF_WORKER_DIR`, a directory containing both named worker executables. An explicit directory is strict: a missing worker fails packaging. macOS and Linux also include the default `compat/8bf-worker/dist` payload when present; if absent, they warn and package without native 8BF workers. The current external reusable release workflow does not download the worker artifact into these packaging jobs, so automatic worker inclusion in those releases is still pending integration. A prepared local package and an automatically published release are distinct states.

Flatpak may carry the worker files, but its current sandbox does not provide a supported Wine runtime or access to host Wine. Native 8BF execution in Flatpak remains unsupported. No Wine distribution or additional runtime license payload is bundled.
Open an RGB or Grayscale document at 8 or 16 bits per channel and select an unlocked raster layer. The filter itself must support that format. Choose **Filter → Plug-ins → Install 8BF Plug-in…**, enter the filter's full path, then select its `(8BF)` menu item. Its native dialog opens in a separate process. Applying produces one undo step; cancelling or a worker failure preserves the document. Keep companion DLLs and help files alongside the filter.

An explicit install lasts for the current session. For discovery after restart, open **Preferences → Plug-ins**, enable additional plug-in folders, and add folders to the list. All subfolders are included. Applying a changed folder list scans again; `plugin.reload {}` rescans unchanged folders, for example after adding files. Existing single-folder preferences remain supported. Overlapping folders and symbolic-link loops do not duplicate a canonical file. Unreadable folders and rejected libraries are reported while other folders continue loading. Native discovery examines metadata only and never executes a filter. A scan is bounded to 64 roots, 32 subfolder levels, 100,000 directory entries, and 4,096 matching files per registry; reaching a bound is reported and keeps what was found before it. Removing a folder or turning folders off unregisters the filters that folder provided; filters installed explicitly stay for the session. When two discovered filters share a name, their menu items add the architecture (and, for two builds of one architecture, the folder name), for example **Zombie (8BF x86)…** and **Zombie (8BF x64)…**.

Linux uses a dedicated Wine prefix at `$XDG_CONFIG_HOME/photocraft/8bf-wine` (or `$HOME/.config/photocraft/8bf-wine`). macOS uses `$HOME/Library/Application Support/Photocraft/8bf-wine`. `PHOTOCRAFT_8BF_WINEPREFIX` selects another absolute prefix path; PhotoCraft does not select the personal default `~/.wine` prefix. Wine's first initialization can take longer than subsequent filter launches.

`PHOTOCRAFT_8BF_WINE` selects an explicit Wine executable or launcher wrapper. macOS discovery checks PATH first, then `/opt/homebrew/bin/wine`, `/usr/local/bin/wine`, and `/opt/local/bin/wine` for Homebrew/MacPorts installations, allowing launches from Finder. Selecting a runtime does not establish that it can execute both worker architectures; verify the required architecture on the actual machine.
## Current compatibility profile

Supported: RGB and Grayscale raster layers at 8 or 16 bits per channel, with or without alpha; original alpha preservation; selection clipping and feathering; native filter dialogs with RGB or grayscale 8-bit previews; thread-safe basic buffer and handle callbacks; the watch-suspension property used around previews; both Windows architectures. Document and raster-layer mode/depth must match. Large buffers use bulk binary IPC with a 512 MiB pixel limit and a 32767-pixel limit on either dimension. The host transfers encoded image data in row bands to bound temporary memory.

The 16-bit wire format carries full-range `0..65535` unsigned little-endian samples. The classic native filter interface uses `0..32768`; the worker converts requested tiles at that boundary. If a filter leaves a native sample unchanged, PhotoCraft retains its exact original full-range value. Modified samples have the precision of the native interface. Output outside its native range is rejected, not silently wrapped or clipped. Alpha is never sent to the filter. No color-model conversion is performed to accommodate an unsupported filter.

Unsupported inputs are rejected explicitly: 32-bit float/HDR documents, CMYK/Lab/Indexed and other color modes, smart objects, masks or individual channel targets. Native filter results do not alter alpha. Native smart-filter replay and parameter persistence are not implemented. The record and pixel-advance callback are owned by the entry thread; allocator callbacks support plug-in processing threads. Modern suite acquisition and unverified property services remain unavailable. Filters requiring them may fail; not all Photoshop-compatible filters are guaranteed to work. Metadata discovery accepts regular library files up to 64 MiB. Libraries exporting multiple distinct filter entries are rejected until a filter-entry chooser is implemented. The current x86 ABI supports cdecl entry points, not decorated stdcall entry points.

`show_ui: false` omits the Parameters selector; a filter may still open its own dialog during Start. It is not an unattended-execution guarantee.

The worker advertises no native selection mask; the editor applies selection clipping and feathering to the returned pixels. Some filters populate the unused mask rectangle anyway. The worker ignores that rectangle and continues supplying input/output pixels; a mask-only rectangle does not keep the processing loop running. Native selection-mask delivery remains unsupported. After a filter's Start selector succeeds, the worker calls Finish once even if the filter ignored a failed host callback. A later cleanup error does not replace the original processing error. A failed or cancelled Start does not call Finish.

A worker is discarded after every invocation. It uses the Windows [process error mode](https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-seterrormode) to prevent system crash dialogs from stalling the job when an unhandled native fault occurs. Process separation contains ordinary filter crashes; it is **not a security sandbox**. Native plug-ins run with the user's permissions. Only install plug-ins you trust. The editor never loads a native DLL into its own process.

## Automation

These commands are available to trusted local engine/CLI sessions and the interactive desktop. The capability-scoped control channel, MCP, and untrusted sessions reject native installation and execution: a native DLL cannot be confined to their authorized file roots.

- `plugin.8bf.install {"path":"/absolute/path/filter.8bf"}` examines and registers a filter.
- `plugin.8bf.list {}` returns installed IDs, architecture, path and entry point.
- `plugin.8bf.run {"id":"…","show_ui":true}` runs on the active pixel layer using the existing job/cancellation system.
- `plugin.8bf.remove {"id":"…"}` removes its in-memory registration; it does not delete files.
- `plugin.reload {}` scans every configured folder recursively; `{"path":"/one/folder"}` and `{"paths":["/first/folder","/second/folder"]}` explicitly select scan roots. These forms cannot be combined.

The protocol uses a versioned 20-byte frame header, bounded JSON metadata, and raw RGB or grayscale samples. Pixel data is not base64-encoded or sent through per-pixel calls. The client concurrently drains pipe output and monitors cancellation and process exit. It retains bounded worker diagnostics when startup, transport or processing fails, including when a Wine child process keeps the diagnostic pipe open. It validates output dimensions, format, length and worker completion before changing a document.

## Development and provenance

Wine is an external runtime under [LGPL-2.1-or-later](https://github.com/wine-mirror/wine/blob/master/LICENSE); this project neither embeds nor redistributes it. The original bridge code follows PhotoCraft’s MIT OR Apache-2.0 licensing.

The host and client contain safe Rust. `compat/8bf-worker` is a separate Rust workspace containing the narrowly scoped native FFI boundary, explicitly authorized for this subsystem. See [ABI provenance](../compat/8bf-worker/ABI-PROVENANCE.md) for independent interoperability evidence. Third-party test filters are local inputs and are not distributed with PhotoCraft.

The original conformance fixture can be compiled and tested through Wine on Linux:

```sh
compat/build-8bf-workers.sh
xvfb-run -a python3 compat/8bf-worker/tools/validate-workers.py --prefix /absolute/test-wine-prefix --large
```

On Windows, use the native MSVC workers and prebuilt fixtures without Wine:

```powershell
./compat/build-8bf-workers.ps1
python compat/8bf-worker/tools/validate-workers.py --runner native --workers-dir compat/8bf-worker/dist --fixtures-dir compat/8bf-worker/fixtures
cargo run --locked -p photocraft-plugins --example native_8bf_smoke -- compat/8bf-worker/dist compat/8bf-worker/fixtures
```

The builder writes `photocraft-8bf-worker32.exe` and `photocraft-8bf-worker64.exe` under `compat/8bf-worker/dist`. Prebuilt fixture directories contain `i686/` and `x86_64/`, each with `photocraft-conformance.8bf`, `photocraft-host-services.8bf`, and `photocraft-handle-services.8bf`. Supplying `--fixtures-dir` prevents the Python validator from rebuilding fixtures. `--runner wine` is the default and requires `--prefix`; `--runner native` launches Windows executables directly and needs no prefix. The Rust smoke example exercises the safe host IPC client; the Python validator exercises worker conformance directly. On Unix, configure an isolated prefix/runtime before running the Rust example against compatible worker and fixture builds.

The `native-8bf.yml` CI workflow is configured to build and link the native editor and run plug-in library tests on Windows, Intel macOS, and Apple Silicon macOS. Its Windows job builds both workers, runs Python conformance, and runs the safe Rust IPC smoke example. These configured CI jobs have not been executed as part of this documentation update; they are not evidence of verified Windows or macOS runtime compatibility.

It checks exact inversion and identity in Gray/RGB at 8/16 bits, 16-bit subset-plane processing, plug-in stdout isolation, cancellation, native access violations, nonzero process exits, invalid rectangles, ignored callback errors, allocation from a processing thread, unused mask rectangles, the complete legacy handle table and foreign Windows handle disposal, Start/Finish sequencing, cleanup-error precedence, and compiled property-service calls. With `--large`, it runs 50 cases (46 without `--large`) across both architectures, including 24-megapixel RGB8 inversion and RGB16 identity. The native-fault case has a shorter deadline to catch debugger dialogs. Unit tests additionally round-trip all 65,536 full-range 16-bit values through an unchanged native tile.

The worker's Windows integration test `tests/preview.rs` draws into real offscreen GDI bitmaps and checks the resulting pixels. It covers one-sample grayscale, cropped grayscale with nonzero origins and padded/strided samples, and planar/interleaved RGB channel order. Run it for both Windows targets on Windows or through Wine. Actual third-party filter dialog checks are also necessary; passing the original fixture alone does not establish compatibility with other vendors.

## Verified examples (2026-10-14)

Local Wine tests exercised the original fixture on x86 and x64, including exact 24-megapixel output, stdout noise, cancellation, crashes, invalid rectangles, ignored callback errors and allocation from a processing thread. Both versions of [Vicanek Wire Worm](https://vicanek.de/imageprocessing/wireworm.htm) displayed their native preview and returned edited pixels. Wire Worm reports code `1` on Cancel; the bridge reports a filter error and applies no pixels. Filter Foundry on both architectures returned exact inverted pixels, and 32-bit Vignette Corrector returned an adjusted image. These are tested examples, not a guarantee for every 8BF filter or vendor version.

A further 32-bit Color Replacer 6.2 dialog test changed all 32,000 pixels in the chosen red patch to green while preserving the blue and gray patches byte-for-byte. Cancelling a second edited preview returned code `1` with no output pixels. Its window title reports incomplete host compatibility despite the tested controls working; this remains a compatibility limitation. These checks used the final worker in a separate Wine prefix and offscreen display; they exercised the actual vendor dialog and binary IPC, not a new end-to-end editor run. Original grayscale/RGB preview drawing tests and the conformance suite also passed for both worker architectures.

The subsequent format/service pass exercised Filter Foundry's actual `C-c` formula on x86 RGB8, Gray8 and RGB16, and x64 Gray16. All returned exact expected samples, including the native 16-bit scaling. The x64 Gray16 dialog preview was blank: this sample labels its one-channel Gray16 preview as CMYK, which the worker rejects. Correct processing does not establish correct preview rendering for that case. The original fixture covers every supported format on both architectures; the vendor matrix is narrower.

The supplied Flaming Pear collection contained 94 discoverable libraries, 47 per architecture, all accepted by metadata inspection. Aetherize 1.88, Melancholytron 1.64, and Mr. Contrast 1.64 were each tried on x86 and x64 RGB8: all six exited with code 5 before a usable dialog or pixel response. One Aetherize x64 runtime exception was a null read, but its cause is not established. Discovery is not execution compatibility; these three filters failed that earlier build. No proprietary plug-in code was disassembled or included in PhotoCraft.

The collection's Freebies ChromaSolarize 1.50 x64 returned a valid RGB8 result with all 40,960 test pixels changed. ChromaSolarize 1.20 x86 returned the handled error `filter requested pixels outside image bounds`, with no output applied. Neither invocation showed a filter dialog. These are different vendor versions, so this result alone does not isolate architecture as the cause of the difference.

That earlier eight-library pass produced one successful result and seven failures. It predates the legacy memory-table and unused-mask fixes described below.

## Crash-fix validation (2026-10-15)

The legacy handle table now supplies eight callbacks. The buffer table includes its
nullable sixth slot, which some filters require before accepting the allocation
callbacks. Ignoring unused mask rectangles also prevents an early host callback
error from leaving input pixels unavailable. The IPC client preserves the worker's
original exit status and bounded diagnostic output, and labels the incomplete
response-frame stage instead of presenting an unexplained buffer EOF.

Using a full 2048 × 994 RGB8 photograph that reproduced earlier failures:

| Filter | x86 | x64 |
|---|---|---|
| India Ink 1.997 | Dialog preview and Apply passed | Dialog preview and Apply passed |
| LunarCell | Dialog and Apply passed | Dialog and Apply passed |
| Zombie | Processing passed | Processing passed |
| SuperBladePro 1.80 | Dialog preview and Apply passed | Still crashes during Start |

Every passing case returned the complete 6,107,136-byte image with changed samples.
LunarCell's early captured preview was black; successful Apply does not establish
correct preview rendering. SuperBladePro x64 remains a known failure; use the x86
entry for the verified path. These tests exercised the actual plug-in DLLs, dialogs,
and worker IPC in isolated Wine sessions, not a fresh end-to-end editor UI run.

The updated original conformance suite passes 46 cases across both architectures,
including Gray/RGB 8/16-bit processing, unused masks, and handle disposal. Regression
replays against older workers fail on the targeted table/mask checks. This does not
claim CMYK/Lab, floating-point processing, native selection-mask delivery, modern
suites, or compatibility with all discovered filters.

## Platform verification (2026-10-16)

The desktop source passed `cargo check` for `x86_64-pc-windows-gnu`,
`x86_64-apple-darwin`, and `aarch64-apple-darwin`. The Windows x64 editor also
compiled and linked into an executable. This is a GNU Windows build; the MSVC
builder and native CI jobs are configured, but have not been run on CI in this pass.
A macOS cross-check does not verify Apple SDK linking, bundle signing, plug-in
windows, or Wine execution on a Mac. Those still require the native Mac jobs and
hardware/runtime testing.

On Windows 11, both freshly rebuilt worker architectures passed all
38 safe Rust IPC cases; the native Windows plug-in library passed 26 unit tests.
The editor opened and answered its authenticated control interface in the existing
interactive desktop session using CPU/software graphics. A final run opened the test
photograph, discovered all four test plug-ins (two per architecture), wrote a window
screenshot, and quit cleanly with a writable isolated profile. This did not exercise
interactive filter dialogs from inside the editor. A preceding service-session
attempt could not create a DX12 surface; service session 0 is not a desktop GUI test.

Linux passed 57 plug-in tests, the 46-case worker conformance suite, and the same
38-case safe Rust IPC suite against both fresh workers. Plug-in Clippy checks passed
on Linux and the Windows target; workspace layer and WASM checks also passed.
Persistent Wine services must be started with null/file standard streams before
using a pipe-EOF-based Python test runner: an initial harness attempt returned a
complete successful frame but waited on a diagnostic pipe inherited by Wine services.
The host client already handles retained diagnostic pipes without waiting indefinitely.

Four free vendor filters were tested on the same 2048×994 RGB8 photograph on native
Windows and Linux/Wine, with matching worker hashes across platforms. Zombie 1.40
x86 and 1.50 x64 returned complete changed images on both. Swap Red and Blue 1.20
x86 and 1.50 x64 returned the exact expected channel swap, byte-for-byte identical
on Windows and Wine. Zombie output differed between platforms and between repeated
native runs; this establishes processing compatibility, not deterministic pixel parity.
These were `show_ui=false` processing tests, separate from editor startup checks.

Vendor binaries, test photos, Wine prefixes, and generated fixtures remain outside source history.
