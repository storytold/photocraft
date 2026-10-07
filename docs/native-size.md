# Native release size

Native releases ship separate Apple silicon and Intel Mac builds, Windows x64/x86/ARM64
builds, and Linux x86_64/aarch64 builds. GUI packages and CLI archives are separate. Windows
x86 means 32-bit; Linux aarch64 and Mac Apple silicon mean ARM64.

## Implementation and validation contract

Packaging work was split among Mac, Windows and Linux agents with exclusive ownership of
their respective `packaging/<platform>` files. They returned source changes without builds.
The coordinator owns profile settings, workflows, documentation, integration, builds and PR.

No image formats, fonts or glyph coverage, accessibility, GPU fallbacks, automation commands,
or native panic recovery may be removed to meet a size target. Preserve licenses, signing,
notarization, minimum OS versions and architecture checks. Diagnostic symbols stay outside
release downloads. Keep regular `release` unchanged as the same-source performance oracle.

Validation batches:

1. Packaging fixture tests, shell/XML/workflow/profile checks and normal release baseline.
2. Build `native-release`; compare stripped sizes and representative image-processing timings
   against the same-source baseline, not an older published release.
3. Package/verify each Mac architecture centrally; PR jobs verify Mac packages, Windows
   MSI/portable/CLI payloads and both Linux architecture archives. Windows ARM64 has its
   existing install/run CI lane. Release CI still creates every Linux installer and Flatpak.

`python3 scripts/test_release_packaging.py` exercises package boundaries, architecture mapping,
licenses, executable mode, diagnostics separation, failed stripping and invalid Mac arguments
without Rust builds. `.github/workflows/native-packaging.yml` checks real compiled artifacts.

## Choices

`native-release` uses fat LTO and `opt-level="s"`, with speed optimization retained for pixel,
codec, vector and text crates. It keeps `panic="unwind"` and line-table diagnostics. Packaging
extracts dSYMs/ELF debug files before stripping; Windows retains matching PDBs. CI diagnostics
are separate 30-day Actions artifacts, not GitHub Release assets; preserve them longer for
long-term crash support. They are not secret storage.

ZIP compression is Optimal, gzip tarballs use level 9 and DMGs retain zlib level 9. Installer
compression/runtime requirements remain compatible. Native bundled font bytes already share
one static source between UI and text; verify their copy count when measuring new binaries.
The pinned Japanese craft fonts are gzip-compressed at build time, verified byte-for-byte,
and decoded once into shared cached immutable bytes. Decoding rejects malformed/truncated
streams, mismatched lengths and fonts over 32 MiB. This adds one-time decompression and decoded
memory stays resident; no glyphs are subsetted. Original font bytes are compiled into tests
only, where exact equality and pointer sharing are checked. Default fallback fonts remain: blindly removing them loses symbols/emoji/script coverage.
GPU, SVG, plugin and codec dependencies provide supported functionality and are retained.
`opt-level="z"`, native panic abort and executable packing are not used.

## Baseline evidence

The SHA256-verified published v0.2.0 GUI executable was 78.10 MiB Mac universal, comprising
37.23 MiB Apple silicon and 40.85 MiB Intel; Windows x64 was 44.14 MiB and x86 36.95 MiB;
Linux x86_64 was 42.50 MiB and aarch64 36.98 MiB. These are historic shipped measurements,
not a same-source compiler comparison. Windows/Linux GUI downloads included the CLI then.

Stripping local symbols from a copy of the universal Mac executable saved 8.27 MiB installed,
but only 1.28 MiB in a gzip proxy because symbol names compress well. Every shipped GUI
architecture contained two copies of the four bundled fonts (1.45 MiB redundant payload per
architecture); the existing source sharing fix predates this packaging change. The Windows
x64 ZIP's CLI entry alone contributed 10.33 MiB compressed. Installed size and download size
must be reported separately; app bundle resources/signatures are additional to executable size.

Mac DMG verification includes the app inside the mounted image, not just the DMG signature.
`makehybrid` synthesizes FinderInfo on resources; packaging clears only that attribute inside
the app on a writable intermediate image, verifies the existing signature, then compresses and
signs the DMG. Notarization metadata and the volume's Finder presentation are preserved.

## Measured results (2026-10-06)

Same upstream application/font baseline, local Rust 1.95, Apple M1 Max; both executables
stripped and ad-hoc signed consistently:

| Executable | Before | After | Reduction |
|---|---:|---:|---:|
| Apple silicon GUI | 63,141,712 bytes (60.22 MiB) | 44,636,592 bytes (42.57 MiB) | 29.3% |
| Apple silicon CLI | 49,240,848 bytes (46.96 MiB) | 34,634,016 bytes (33.03 MiB) | 29.7% |

The GUI machine-code section fell from 23,641,616 to 16,841,296 bytes. Its main constant-data
section fell from 33,733,304 to 23,831,056 bytes. Japanese font payloads fell from 24,141,500
to 14,514,827 bytes (9.18 MiB saved), preserving all original bytes. These combined profile/
font savings are compared against the current application with those fonts, not old v0.2.0.

Measured Rust 1.99 CI executables and GUI downloads (unsigned verification builds):

| Platform | GUI executable | Separate CLI executable | GUI download |
|---|---:|---:|---:|
| Windows x64 | 55,315,968 bytes (52.75 MiB) | 40,287,744 bytes (38.42 MiB) | MSI 28,778,496 bytes (27.45 MiB) |
| Windows x86 | 48,683,008 bytes (46.43 MiB) | 36,492,800 bytes (34.80 MiB) | MSI 28,499,968 bytes (27.18 MiB) |
| Linux x86_64 | 52,750,464 bytes (50.31 MiB) | 36,994,848 bytes (35.28 MiB) | tar.gz 32,963,731 bytes (31.44 MiB) |
| Linux aarch64 | 48,195,064 bytes (45.96 MiB) | 34,523,576 bytes (32.92 MiB) | tar.gz 31,741,010 bytes (30.27 MiB) |

The local Apple silicon DMG is 30,462,452 bytes (29.05 MiB). Actual sizes vary with compiler,
architecture and release signing. Intel builds are separate; CI verifies the exact architecture.

Validation: 77 text and 529 UI tests pass locally, including bounded/corrupt gzip handling,
exact original-font equality, shared cached pointers and Japanese rendering without system
fonts. A before/after Japanese PNG export is byte-identical. An offscreen Japanese UI snapshot
was inspected. Five packaging fixture tests, shellcheck, actionlint and formatting pass. Fork
CI passes wasm, corpus tests and Mac workspace tests/clippy/layering. Windows x64/x86 and Linux
x86_64/aarch64 packages passed real payload, executable and installer checks. Mac disk-image
metadata correction passes local mounted-image signature/architecture/runtime verification;
its final CI matrix remains visible on the PR.

24 MP CPU blur operations were exercised. Initial r4 medians were 828 vs 1004 ms, and r40
1309 vs 1342 ms. A repeat became severely oversubscribed (load average rose to 143), so these
runs cannot certify performance budgets or isolate a compiler regression. No speedup or
budget compliance is claimed; hot-crate opt-level 3 and native panic recovery remain intact.
A dedicated idle-machine profile comparison remains the performance follow-up.
