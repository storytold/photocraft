# PhotoCraft development installers

These tools build unsigned x64 installers from a working tree on Linux. They do
not publish a release. Official releases still use `release.yml`, MSVC/WiX on
Windows and the release signing configuration documented in `docs/releasing.md`.

`Dockerfile` installs Ubuntu 22.04 build dependencies, MinGW, NSIS and nFPM 2.47.0.
Mount a Rust 1.95 toolchain at `/rust` and a writable Cargo cache at `/cargo`. Add
`x86_64-pc-windows-gnu` to the toolchain with rustup before mounting it read-only.
Mount the repository at `/src` as your normal user, and set
`CARGO_TARGET_DIR=/src/target/installers`. Run both builds with at most two jobs.
Optional `CRAFT_FONTS_DIR=/fonts` requires a read-only craft-fonts mount; use the
commit pinned by `CRAFT_FONTS_REF` in `release.yml` and set `CRAFT_FONTS_REQUIRED=1`.
Font files stay outside this repository; their licenses accompany each package.

```sh
docker build -t photocraft-installer-builder:20261009 -f packaging/development/Dockerfile packaging/development
```

Inside the builder container, set `PHOTOCRAFT_VERSION` to the package version
(for example `0.8.2-dev.20261013`), `DIST=/src/dist/installers`, the build date and
an honest `PHOTOCRAFT_BUILD_SHA` identifying the development source. The app's
internal version remains the workspace version. For Linux:

```sh
cargo build --release --locked -p photocraft -p photocraft-cli --features heif
PHOTOCRAFT_APPIMAGE_UPDATE_INFO='' bash packaging/linux/package.sh --skip-build --formats 'appimage deb rpm tar'
```

The empty update setting prevents unpublished AppImages from advertising an
official update feed. Omitting it retains the release feed. Building on Ubuntu
22.04 preserves the glibc 2.35 baseline; verify the resulting ELF requirements.

For Windows, `package-gnu.sh` builds both executables, embeds the app icon,
checks PE architecture/subsystems and compiler runtime imports, and produces a
per-user NSIS setup executable plus a portable ZIP:

```sh
bash packaging/windows/package-gnu.sh
# Or repackage already built executables:
bash packaging/windows/package-gnu.sh --skip-build
```

The setup installs into `%LOCALAPPDATA%\Programs\PhotoCraft-Dev` by default,
adds a Start Menu entry and an uninstall entry, and does not require elevation.
Uninstall removes only the package's known files. The ZIP contains `portable.txt`,
so settings and recovery data go into `PhotoCraftData` beside the executable.
Do not copy that marker into an installed copy.

Run the shell packaging regressions with:

```sh
bash packaging/linux/apprun-test.sh
bash packaging/linux/package-test.sh
```

`Dockerfile.verify` adds an Ubuntu runtime and virtual display for Linux GUI
checks; `Dockerfile.windows-test` adds Wine 10, including 32-bit installer support.
These images create no persistent installations on the build host. Use a
temporary RAM filesystem for the Wine container (`--tmpfs /tmp:rw,exec,size=2g,mode=1777`)
with a memory limit of at least 3 GiB: initializing a Wine prefix on a slow or
nearly full overlay filesystem can exceed the smoke test's timeout. All tests
use synthetic documents and temporary settings. To check a
DEB, install it inside the temporary Ubuntu container and run:

```sh
python3 packaging/development/with-display.py python3 packaging/development/gui-smoke.py /usr/bin/photocraft /src/dist/installers/linux-api-settings.png
```

To test a setup executable in the Wine container:

```sh
python3 packaging/development/with-display.py python3 packaging/development/windows-smoke.py /src/dist/installers/photocraft-0.8.2-dev.20261013-windows-x64-setup.exe /src/dist/installers/windows-verification.json
```

The Windows check installs, checks the uninstall registry command, exercises the
CLI and AI command registry, saves/reopens a synthetic `.pcraft`, and uninstalls
while preserving an extra user file. Wine checks do not replace a native Windows
GPU/driver test. The Linux check starts the real GUI, opens the German API
settings dialog, all eight AI forms and saves screenshots, using temporary settings and a virtual
display. An explicit third argument to `gui-smoke.py`, the ComfyUI base URL, also
tests direct asynchronous 4K generation without an upscaler, Save As identity,
separate native-source 4K-to-8K Upscale, result dialogs, layer apply, Undo/Redo,
native save/reopen and stale-result recovery. Set `PHOTOCRAFT_SMOKE_TIMEOUT=3600`
for that opt-in live check. Optional fourth/fifth arguments are the exact prompt
model ID and a Responses-compatible prompt endpoint; a local HTTP test peer
checks serialization without claiming that the user’s actual text backend works.
Run test containers with `--init` and use `with-display.py` to avoid
`xvfb-run` readiness-signal hangs in containers.
