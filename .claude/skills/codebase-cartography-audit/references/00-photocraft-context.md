# Module 0 - PhotoCraft context (read first)

This skill was written for .NET line-of-business apps and adapted to **PhotoCraft**: a
Rust-only, native image editor (egui/eframe on wgpu, plus the same code compiled to
WebAssembly). Read `AGENTS.md` and `docs/architecture.md` before anything else; they
are the project's own map and rules, and the audit measures the code against them as
well as against the general principles in Modules 1-8.

## 0.1 Shape of the codebase

| Part | Where | Notes |
|------|-------|-------|
| L0 foundation | `crates/geom cms color raster psd codecs tablet` | `psd`, `codecs`, `cms` have no workspace deps; `tablet` is the only crate allowed `unsafe` |
| L1 document model | `crates/doc` | pure data: layers, masks, adjustments, effects, smart objects |
| L2 domain services | `crates/ops paint algo text vector` (+ `raw heif affinity`) | history, brush engine, imaging algorithms, type, paths |
| L3 rendering + native format | `crates/compose gpu format` | CPU compositor is the oracle; wgpu compositor must match it |
| L4 I/O + plug-ins | `crates/io plugins` | document <-> PSD/flat formats; sandboxed WASM plug-ins |
| L5 application | `crates/engine` | `Session` + command registry: **every action is a command** (`*_cmds.rs`, `specs()`) |
| L6 delivery | `crates/ui-egui automation` | egui shell (should be thin), MCP server |
| Apps | `apps/photocraft photocraft-cli photocraft-web` | desktop + TCP control server, headless CLI, browser build |
| DevOps/Test | `crates/testkit`, `crates/*/{tests,benches,examples,fuzz}`, inline `#[cfg(test)]`, `xtask/`, `scripts/`, `packaging/`, `.github/`, `perf/`, `scorecard/`, `Dockerfile` | |

Layering is enforced by `cargo xtask layers` (`xtask/src/layers.rs`): a crate may depend
only on lower layers. That is the **Onion/Clean dependency rule (Modules 2-3)** made
executable; run it and report its result, then look for violations it cannot see
(e.g. UI logic that should be an engine command, engine code that knows about egui
concepts, L0 crates reaching for policy).

## 0.2 How each lens reads in Rust / PhotoCraft

| Module | Look for here |
|--------|---------------|
| 1 SOLID | SRP: god modules (`ui-egui/src/canvas.rs`, `panels.rs`, `engine/src/atn_cmds.rs` are the biggest). OCP: giant `match` on command id / tool / format instead of a registry or trait. LSP: trait impls that `Err("unsupported")` for most methods. ISP: fat traits. DIP: concrete types instead of traits at seams (filesystem, clock, GPU, fonts); platform code not behind the platform services. |
| 2/3 Onion & Clean | `cargo xtask layers` result; behaviour living in `ui-egui` that is not reachable as an engine command (AGENTS rule 1); `doc` gaining logic (should stay pure data); wasm-unsafe code above L0 not behind `cfg(not(target_arch = "wasm32"))`. |
| 4 Fowler smells | Long functions (per-file CC from the rollup, then the function), large modules, `Option`/`bool` parameter clumps, primitive obsession (raw `f32` for units, `String` ids), duplicated code (jscpd), shotgun surgery (one feature touching `menus.rs`, `state.rs`, `commands.rs`, `menu_catalog.rs` and docs), dead code (`#[allow(dead_code)]`). |
| 5 Enterprise/domain | Command pattern (engine), Memento/undo (`ops` history), Unit of Work (atomic `.pcraft` saves in `format`), value objects (`geom`, colour types), anemic vs rich model in `doc`. |
| 6 IoC/persistence (Miller) | Rust has no IoC container: check constructor/trait injection at the seams instead; global state (`static`, `OnceLock`, `lazy_static`, thread-locals) used as a service locator; persistence ignorance = `doc` knowing nothing about PSD/`.pcraft` (`io`/`format` do the mapping). |
| 7 AI anti-patterns | The project's own **never-crash** rule makes these **Critical**: `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, unchecked `[i]`/slicing on input-derived indices, unchecked arithmetic on lengths, unbounded recursion, `lock().unwrap()`. Also: magic numbers, stringly-typed command params, swallowed errors (`let _ =`, `.ok()` dropping an error a user needs), `unsafe` outside `crates/tablet/src/macos.rs`, hard-coded colours instead of `theme::Tokens`, `u8`-only pixel paths in public APIs, sRGB assumptions bypassing `photocraft-cms`. |
| 8 Testing | Unit = in-crate `#[cfg(test)]` + doc-tests + pure `tests/*.rs`; integration = GPU, real files/corpora, `photocraft_io` round trips; acceptance = tests that spawn an app binary (`CARGO_BIN_EXE_*`), drive the TCP control channel, or render the full egui shell. Also `panic_hunt` (fuzzes every command, `--ignored`), the opt-in `corpus` feature tests, fuzz crates, and perf budgets (`perf/budgets.toml`). |

Do not report something as a defect when it is a documented, deliberate project
decision (e.g. `unsafe` in `crates/tablet/src/macos.rs`, CPU compositor duplicated by
the GPU one as an oracle, generated `docs/parity-checklist.md`). Do report it when the
code breaks the project's own written rule.

## 0.3 Function-point basis for this product

PhotoCraft is a desktop editor, not a CRUD app; map IFPUG types like this and write the
basis into `sizing-inputs.txt`:

- **EI** (state-changing transactions): engine commands that modify the document or
  session. Source of truth: the registry (`cargo run -q -p photocraft-cli -- commands --json`,
  ~700 commands). Count business transactions, not variants: collapse families that
  differ only by a parameter (e.g. one EI per filter, not per preset; one per
  adjustment type) and say how you collapsed them.
- **EQ** (retrievals): read-only commands / queries (info, histogram, list layers,
  `ui.screenshot`-style reads, MCP read tools).
- **EO** (derived outputs): exports and renders with computed content - each export
  format writer, print/proof outputs, generated reports.
- **ILF** (data PhotoCraft maintains): logical files, not structs - the document
  (`.pcraft`), preferences/settings, presets libraries (brushes, swatches, gradients,
  patterns, styles), actions, workspace layouts, history snapshots.
- **EIF** (data maintained elsewhere, read here): foreign document formats grouped
  logically (PSD/PSB, flat raster codecs, camera raw, HEIF, Affinity, Krita/OpenRaster),
  ICC profiles, fonts (craft-fonts), plug-ins.

Backfiring ratio for Rust: no SPR/QSM edition publishes one. Use **55 LOC/FP
(analyst-assumed; nearest SPR peers: C++ 55, Java 53)** and widen the sensitivity band.
WGSL shaders: assume ~50 (analyst-assumed). TOML/YAML/shell in DevOps/Test: `ratio.default`.

## 0.4 Running things on this workspace

- Use `CARGO_TARGET_DIR=target/agent-audit` (AGENTS.md section 6); a full test build is
  ~10 GB and tens of minutes on a cold cache. Delete it when the audit is done.
- Default workspace members exclude `apps/photocraft-web` (wasm); `cargo xtask wasm` is
  the wasm check if you need it.
- The `corpus` cargo feature is off by default, so `cargo test -- --list` excludes the
  real-file corpus tests; say so, or fetch them (`cargo xtask corpus --all`) and list
  with `--features corpus` for the crates that have it.
- Never commit the report or tool output unless asked; never add tools to the repo.
