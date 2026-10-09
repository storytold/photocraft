# Contributing

- **Language:** Rust only. No JavaScript or TypeScript. On the web, `wasm-bindgen` generates a small loader; never hand-write JS.
- **Licence:** contributions are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The ArtCraft logos in `docs/brand/` are not open source (`docs/brand/LICENSE-brand.txt`).
- **Clean-room:** do not copy code, shaders, icons, ICC profiles or other assets from proprietary software (such as Photoshop). Match behaviour and look by observation and public specs. Only use assets with permissive licences, keep their license next to the asset (e.g. `assets/fonts/OFL-*.txt`, `assets/icons/LICENSE-lucide.txt`), and list every asset in `ATTRIBUTION.md`. **Fonts** live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this repo: add new fonts there, and build with them through the optional `CRAFT_FONTS_DIR` build input (`docs/development.md` › Fonts; rules in `../craftrules/standards/fonts.md`, a sibling checkout — `storytold/craftrules` is not public, so ask a maintainer for the rules relevant to your change).
- **Never crash:** non-test code must not panic: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` and no `unsafe`. Errors go through `Result` and `?`, input-derived indices and sizes are checked, and every crash fix ships with a regression test. See *Never crash* in `AGENTS.md`.
- **Commands, not handlers:** new features are engine commands with tests, and the UI calls them (checklist below).
- **Layering:** `cargo xtask layers` must pass. Register new crates in `xtask/src/layers.rs`.
- **Tests:** required for every change. Format code needs round-trip and malformed-input tests. Pixel code is tested at 8, 16 and 32-bit. If you touch psd, io, codecs, compose, gpu, text or format, also run the real-file corpus tests: `cargo xtask test-corpus` (fetches the pinned corpora, including our Photoshop oracles from https://github.com/storytold/photocraft-corpus, then runs the `corpus`-feature tests; CI always runs them). Never commit corpus files; see `docs/development.md` › Test corpora.
- **Style:** `cargo fmt`, and `cargo clippy -- -D warnings`. Match surrounding code. Comments explain *why*.
- **UI:** use `theme::Tokens` and `widgets::*`. Verify visually (offscreen `snapshot` example or the control channel) before submitting, and attach before/after screenshots to PRs.
- **Commits:** small, focused, with a clear subject line.

## Reporting issues

Use **Report an Issue** beside Discord in the app's title bar, then choose the
issue type from the dropdown to open its GitHub form.
Opening the dropdown does not open a browser or send diagnostics.
Selecting a type prefills the version, operating system, and a limited set of system
diagnostics. Review those fields before submitting; GitHub does not create an
issue until you submit the form.

The dropdown uses this priority order:

| Type | Use for |
|---|---|
| [Bug report](https://github.com/storytold/photocraft/issues/new?template=bug_report.yml) | Broken editing/UI behavior, crashes, hangs, or slow operations |
| [Compatibility issue](https://github.com/storytold/photocraft/issues/new?template=compatibility.yml) | File/Photoshop interchange or platform/package incompatibility |
| [Feature request](https://github.com/storytold/photocraft/issues/new?template=feature_request.yml) | Missing tools, formats, capabilities, or workflow improvements |

This is a fixed, evidence-informed order, not a live GitHub query. A
2026-10-08 sample of 45 open reports contained 21 bugs, 4 compatibility issues,
and 20 feature requests; 10 recently closed reports were bugs. Existing
breakage comes first, followed by interoperability/platform blockers, then
new capabilities. Crashes and performance problems use Bug report rather than
overlapping forms.

The detected platform includes the OS family and architecture; add your OS
release or Linux distribution manually, and the browser/version for web reports.

For bugs and compatibility problems, include steps to reproduce, the actual
result, and the expected result. For feature requests, explain the problem the
feature would solve and the desired workflow instead. For
file-specific problems, describe the format, dimensions, colour mode, and bit
depth. Attach a minimal test file, screenshot, or relevant error text if it is
safe to share; remove personal information and private file paths first.
Reports opened directly on GitHub need their version and system details filled
in manually. Search both open and closed issues for related reports.

The issue forms must be merged into the default branch of
`storytold/photocraft` before GitHub can display them. Their `version`, `os`, and
`system-info` field IDs are shared with the in-app prefill URL; keep them in sync
when editing `.github/ISSUE_TEMPLATE/*.yml`.

The fields reflect recurring gaps and useful details in open and closed reports:
[PSD interoperability](https://github.com/storytold/photocraft/issues/1281)
needs exact product versions and file details;
[macOS clipboard behavior](https://github.com/storytold/photocraft/issues/1287)
needs platform and reproduction steps;
[display scaling](https://github.com/storytold/photocraft/issues/1249)
needs monitor/scaling details; and
[video export](https://github.com/storytold/photocraft/issues/1288) and
[Slice Options cancellation](https://github.com/storytold/photocraft/issues/910)
need precise steps and expected versus actual results.

## Adding a command

1. **Find the id.** Search `crates/ui-egui/src/menu_catalog.rs` for the Photoshop menu item. Using its id makes the menu item live with no UI work. Commands without a Photoshop menu entry use a descriptive id in the same style (`layer.smartFilter.delete`) and an empty menu path.
2. **Algorithm** goes in the lowest crate that fits (`algo` for imaging, `paint`, `vector`, `text`, `cms`), with unit tests. It takes depth-agnostic surfaces (`photocraft-raster`), works per tile, and is deterministic (seeded randomness).
3. **Command** goes in an engine module (`crates/engine/src/<area>_cmds.rs`) exposing `specs()`, registered with `v.extend(...)` in `commands.rs`. Fill in:
   - `id`, `label`, `menu` path and Photoshop's default `shortcut`,
   - a params doc string such as `{"radius":px=4,"mode":"a|b"="a"}` (this is what agents read through `commands` / `command_list`),
   - an `enabled` predicate (greys the menu item out),
   - `run`, which returns a JSON result and records exactly **one** history step (use `coalesce` for drags and typing sessions).
4. **Respect the context:** active selection (feathered), layer vs mask target, locks, the colour model and depth.
5. **Tests** in the module: behaviour, undo/redo, disabled states, bad params, several depths.
6. **Dialog** (if it has parameters): filters and adjustments get schema-driven dialogs with live preview (`ui-egui/src/filter_dialog.rs`, `dialogs.rs`).
7. Run `cargo xtask parity` and commit the updated `docs/parity.md`.
