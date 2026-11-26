# Contributing

- **Language:** Rust only. No JavaScript or TypeScript. On the web, `wasm-bindgen` generates a small loader; never hand-write JS.
- **Licence:** contributions are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The ArtCraft logos in `docs/brand/` are not open source (`docs/brand/LICENSE-brand.txt`).
- **Clean-room:** do not copy code, shaders, icons, ICC profiles or other assets from proprietary software (such as Photoshop). Match behaviour and look by observation and public specs. Only use assets with permissive licences, keep their license next to the asset (e.g. `assets/fonts/OFL-*.txt`, `assets/icons/LICENSE-lucide.txt`), and list every asset in `ATTRIBUTION.md`. **Fonts** live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this repo: add new fonts there, and build with them through the optional `CRAFT_FONTS_DIR` build input (`docs/development.md` › Fonts).
- **Never crash:** non-test code must not panic: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` and no `unsafe`. Errors go through `Result` and `?`, input-derived indices and sizes are checked, and every crash fix ships with a regression test. See *Never crash* in `AGENTS.md`.
- **Commands, not handlers:** new features are engine commands, and the UI calls them (checklist below).
- **Layering:** `cargo xtask layers` must pass. Register new crates in `xtask/src/layers.rs`.
- **Tests:** follow the test rules in `AGENTS.md` and [standards/testing.md](https://github.com/storytold/craft-contrib/blob/main/standards/testing.md). Format code can use round-trip and malformed-input tests. Pixel code can be tested at 8, 16 and 32-bit. If you touch psd, io, codecs, compose, gpu, text or format, also run the real-file corpus tests: `cargo xtask test-corpus` (fetches the pinned corpora, including our Photoshop oracles from https://github.com/storytold/photocraft-corpus, then runs the `corpus`-feature tests; CI always runs them). Never commit corpus files; see `docs/development.md` › Test corpora. Tests never require a string to be translated: a missing translation shows in English, so no test may fail because a catalog lacks a row (`cargo xtask i18n-report` lists the gaps instead).
- **Style:** `cargo fmt`, and `cargo clippy -- -D warnings`. Match surrounding code. Comments explain *why*.
- **UI:** use `theme::Tokens` and the framework components (`Button`, `NumberField`, `Select`, `Slider`). Verify visually (offscreen `snapshot` example or the control channel) before submitting, and attach before/after screenshots to PRs.
- **Commits:** small, focused, with a clear subject line.

## Adding a command

1. **Find the id.** Search `crates/ui-egui/src/menu_catalog.rs` for the Photoshop menu item. Using its id makes the menu item live with no UI work. Commands without a Photoshop menu entry use a descriptive id in the same style (`layer.smartFilter.delete`) and an empty menu path.
2. **Algorithm** goes in the lowest crate that fits (`algo` for imaging, `paint`, `vector`, `text`; colour management is `craft-cms` in craft-libs), with unit tests. It takes depth-agnostic surfaces (`photocraft-raster`), works per tile, and is deterministic (seeded randomness).
3. **Command** goes in an engine module (`crates/engine-core/src/<area>_cmds.rs`, or `crates/engine-<crate>/src/cmd/<area>_cmds.rs` for a crate cut out of core, such as `engine-paint` or `engine-vector`) exposing `specs()`, registered by adding `("<module>", crate::<module>::specs)` to `TABLES` in that crate's `install.rs`, and its ids to `crates/engine/src/install/command_ids.txt` after the command it follows in the menus: that file sets the registry order (the top crate's order test says how). A command in a core menu goes in the owner fragment `cmd/table/<owner>/<menu>.rs` of its crate instead. Fill in:
   - `id`, `label`, `menu` path and Photoshop's default `shortcut`,
   - a params doc string such as `{"radius":px=4,"mode":"a|b"="a"}` (this is what agents read through `commands` / `command_list`),
   - an `enabled` predicate (greys the menu item out),
   - `run`, which returns a JSON result and records exactly **one** history step (use `coalesce` for drags and typing sessions).
4. **Respect the context:** active selection (feathered), layer vs mask target, locks, the colour model and depth.
5. **Tests** in the module for behaviour that could regress, such as undo/redo and several depths. Add the command to the `panic_hunt` sweep (`crates/engine/tests`) instead of writing a bad-params test per command.
6. **Dialog** (if it has parameters): filters and adjustments get schema-driven dialogs with live preview (`ui-egui/src/filter_dialog.rs`, `dialogs.rs`).
7. Run `cargo xtask parity` and commit the updated `docs/parity-checklist.md`.

## Repository guide

### Documents

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, dependency layers, the engine/UI seam, document model |
| `docs/development.md` | Build, test, run, drive the app programmatically, debug tricks |
| `docs/contributing.md` | Rules: clean-room, tests, layering, style, commits; the "add a command" checklist |
| `docs/control-protocol.md` | JSON control channel: how agents drive and screenshot the running app |
| `docs/ui-design.md` | Design tokens, themes, widgets, and how to match Photoshop's look |
| `ROADMAP.md` | One-page summary: stage, the two parity numbers, every dimension, languages, what's next |
| `docs/target-app-parity.md` | The authoritative parity assessment against Photoshop 2026: numbers, weights, evidence, hours |
| `docs/gaps.md` | Every known shortfall, ranked: the work list to pick from |
| `docs/roadmap.md` | Milestones, **current focus**, what's next with estimates |
| `docs/ui-parity.md`, `docs/brush-parity.md`, `docs/context-menu-parity.md`, `docs/file-format-parity.md`, `docs/hardware-parity.md`, `docs/localization-parity.md` | Per-dimension parity checklists |
| `docs/parity-checklist.md` | Generated list of every Photoshop menu item, live or missing |
| `docs/scorecard.md` | Generated scorecard: performance budgets and numbers, corpus floors, per-area checklists (tools, files, UI, type, automation, reliability, distribution), settings that do nothing |
| `crates/<name>/README.md` (where present) | Public API of that crate |
| [photocraft-corpus](https://github.com/storytold/photocraft-corpus) + `docs/development.md` › Test corpora | Real-file test oracles (our Photoshop-authored PSDs and GIMP-authored XCFs); with psd-tools, ag-psd and PngSuite fetched into `corpus/` by `cargo xtask corpus --all` at the pins in `xtask/src/corpus_pins.rs` |

### Workspace map

```text
crates/
  geom color raster          L0 foundation (geometry, pixel formats + blend math, COW tiles); ICC colour management is craft-cms (craft-libs)
  ipp raw                    L0 standalone crates (no workspace deps): CUPS printing client, camera-raw develop
  tablet                     L0 standalone pen tablet input (macOS AppKit, X11 XInput2, Windows barrel button); the one isolated unsafe crate
  doc                        L1 document model (layers, masks, adjustments, effects, smart objects: pure data)
  ops paint algo text vector L2 history, brush engine, imaging algorithms, type engine, paths/shapes
  compose gpu format         L3 CPU compositor (the oracle), wgpu compositor, .pcraft native format
  io plugins                 L4 document <-> PSD / flat formats; sandboxed WebAssembly plug-ins
  engine                     L5 Session + command registry (every action is a command)
  ui-egui automation         L6 egui shell (thin: all actions go through the engine); MCP server
  testkit oil-round-proof    test helpers
apps/
  photocraft                 desktop app (eframe/wgpu), TCP control server
  photocraft-cli             headless CLI (convert/info/run/batch/commands/mcp)
  photocraft-web             the same app in the browser (trunk + wasm-bindgen)
  photocraft-web-boot        starts the packaged browser app from compressed parts of its wasm (no file over a host's size cap)
xtask/                       cargo xtask layers | wasm | ci | stats | corpus | test-corpus | parity | perf | scorecard
```

**Layering is enforced** by `cargo xtask layers`. A crate may depend only on lower layers. `psd` and `codecs` depend on nothing in the workspace. Nothing below `ui-egui` may use egui, eframe, winit or rfd. A new crate must be registered in `xtask/src/layers.rs`.

### Picking work

Priorities: important infrastructure first, then low-hanging parity, then the long tail.

0. **Read `ROADMAP.md`, then `docs/gaps.md`.** `docs/target-app-parity.md` says, dimension by
   dimension, where PhotoCraft is lacking; `docs/gaps.md` ranks the work. `docs/parity-checklist.md`
   (menu wiring) is not a measure of behaviour. When your work moves a number or closes a gap,
   update the parity doc and gap entry it belongs to, with the date, and bump their status line
   and revision history.
1. **Check `docs/scorecard.md`** before picking work: each area's `missing` and `partial` rows,
   the performance scenarios that are over budget or not measurable yet, and the count of
   settings that do nothing. Its numbers are measured; prefer them to estimates.
2. `docs/roadmap.md` → **Current focus**.
3. `cargo xtask parity` → `docs/parity-checklist.md` lists every missing menu item, grouped by menu. Low-hanging fruit is usually a missing command whose algorithm already exists in `algo`, `paint`, `vector` or `text`.
4. `log/devlog.md` → the "Still open" bullets of recent entries.

When parity rises, raise `FLOOR` in `crates/ui-egui/src/parity.rs` (never lower it).

### Checks by area

```sh
cargo test -p <crates you touched>
cargo clippy -p <crates> --all-targets -- -D warnings
cargo xtask layers
cargo xtask panel-commands  # if you touched ui-egui: click handlers dispatch command ids (AGENTS.md rule 14)
cargo xtask wasm            # if you touched L0–L6
cargo xtask parity          # if you added commands; commit the regenerated docs/parity-checklist.md
cargo test -p photocraft-engine --test panic_hunt -- --ignored   # hostile-input sweep over every command
cargo xtask scorecard       # if you moved a number: flip the checklist row in scorecard/*.toml, raise a
                            # corpus floor, fix a dead preference, or meet a budget (then set enforce = true
                            # in perf/budgets.toml); commit the regenerated docs/scorecard.md (CI checks it)
cargo xtask perf --quick    # if you touched a hot path; `cargo xtask perf --update-baseline` publishes a full run
cargo xtask test-corpus     # if you touched psd, io, codecs, compose, gpu, text or format (or: --changed decides)
```

While iterating, run less and keep the full commands above as the final gate: `cargo test -p <crate> --lib --tests` skips the examples and doctests plain `cargo test` builds every time (ui-egui has 11 examples), and `cargo test -p <crate> --test <file>` runs one integration-test file. `cargo nextest run` runs tests in parallel across binaries; see `docs/development.md` › Faster test loops.

**MCP in tests and agent sessions: `photocraft-cli mcp --headless`.** Plain `photocraft-cli mcp` works in the open PhotoCraft window while the ArtCraft Launcher's agents switch is on, and then may start PhotoCraft on the developer's machine when it is closed. Scripts, tests and agents that want a session without a window pass `--headless` (or use `photocraft-cli serve`); a test that runs the CLI also gives it its own settings, launcher and flags folders (`PHOTOCRAFT_CONFIG_DIR`, `CRAFT_MCP_DIR`, `CRAFT_FLAGS_FILE`, as `crate::command()` in `apps/photocraft-cli/tests/it/main.rs` does). To drive the running app on purpose, use `--control` (`docs/development.md`).

**Test corpora.** Real-file corpora live in `corpus/` (gitignored, never committed), fetched at pinned commits and sha256-verified by `cargo xtask corpus --all`: our Photoshop- and GIMP-authored oracles from https://github.com/storytold/photocraft-corpus plus psd-tools, ag-psd and PngSuite from their upstreams. Pins: `xtask/src/corpus_pins.rs`. The corpus tests are opt-in (cargo feature `corpus`): plain `cargo test` skips them, and with the feature on a missing corpus fails ("run `cargo xtask corpus --all`"). `cargo xtask test-corpus` fetches and runs them all. `cargo xtask test-corpus` runs each directory's fixed sample; `PHOTOCRAFT_FULL_SWEEP=1` runs every file. CI always runs them: the samples on pull requests and main, every file nightly. Never commit corpus files; new oracles go to photocraft-corpus (its `AGENTS.md`), then a pin bump here. Details: `docs/development.md` › Test corpora.

The dev log is `log/devlog.md` (local, gitignored).

### Where things are tracked

- `docs/roadmap.md`: milestones M0–M12, status and the current focus.
- `ROADMAP.md`, `docs/target-app-parity.md`, `docs/gaps.md` and the `docs/*-parity.md` checklists: where we stand against Photoshop.
- `docs/parity-checklist.md`: generated Photoshop menu coverage.
- `docs/scorecard.md`: generated scorecard (sources: `scorecard/*.toml`, `perf/budgets.toml`, `perf/baseline.json`, corpus floors, prefs audit).
- `docs/releasing.md`: cutting a release (`cargo xtask version`, the `release` branch), signing secrets, packaging scripts in `packaging/`.
- `plan/` is an optional gitignored notebook (research, parity notes, estimates). Do not ask for it and do not stop when it is absent.
- `log/` (local, gitignored): the dev log.

### Native format

`photocraft-format` deliberately fails to compile when a `photocraft-doc` struct gains a field, so
nothing is silently dropped from `.pcraft` saves. When you add a doc field, add it to
`crates/format/src/manifest.rs` and `convert.rs` with `#[serde(default)]` so older files still load.
If the field has a PSD equivalent, map it in `crates/io` too, and keep unknown PSD blocks verbatim.


### Contributor credits (About window)

About, Contributors and Models are compiled into the binary from `contributors/contributors.json` (generated, never hand-edited) and `contributors/people.toml` (names people chose for themselves). See `docs/contributors.md`.
