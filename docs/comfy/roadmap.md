# Comfy PhotoCraft roadmap

Current engine (2026-10-29): native worker only; ComfyUI removed. Historical phases below describe the fork’s development. See [removal report](research/notes/comfyui-removal.md).

Hardware tiers (the owner's long-term goal, 2026-10-29; [`goals/hardware-tiers.md`](goals/hardware-tiers.md)):
🟡 RTX 5090 (the current focus: every feature clearly faster than ComfyUI) · ⬜ RTX 3090 (int8
only) · ⬜ 16 GB NVIDIA cards (4-bit, one model resident) · ⬜ Mac 16 GB (MLX 4-bit) · ⬜ Mac 8 GB
(a "super MOSS").

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Updated 2026-10-09.

Each phase ends with something a user can run. Every phase obeys upstream's gates
(`AGENTS.md` §5): tests, clippy, `cargo xtask layers`, `cargo xtask wasm`, `cargo xtask parity`,
`panic_hunt`, a dev-log entry. Design details are in [`architecture.md`](architecture.md); the
code map is in [`codebase-orientation.md`](codebase-orientation.md).

| Phase | Status | Ships |
|---|---|---|
| 0 Foundation | ✅ 2026-10-08 | This documentation, the Windows toolchain, a green release build, the `comfy-photocraft` branch |
| 1 Backend + headless Generative Fill | ✅ 2026-10-09: live against ComfyUI 0.39.0 with Qwen-Image-2.1, Krea 2 Turbo and Qwen-Image-Edit-2511 (numbers in the dev log) | `photocraft-genai` crate, ComfyUI client, `generate.fill` / `generate.image` (pulled forward from Phase 3) / `generate.health` / `generate.models` from the CLI and MCP |
| 2 Generative Fill in the app | 🟢 2026-10-09: menu rows, the task bar under the selection (prompt, template picker with installed badges, variations, progress, switcher), the Preferences page, generative layer metadata with Generate Similar, and the control-protocol test (`generative_bar_tests`) | Prompt bar, progress, variations, generative layers with masks, model picker, preferences |
| 3 The Firefly core set | 🟢 2026-10-09: Generate Image (with a transparent option), Generative Expand (also from the Crop tool), Generative Edit (`generate.edit`, the whole picture by instruction, masked to the selection) and Remove Background (`generate.removeBackground`: SAM 3.1 by default, Qwen-Image-2.1's soft RGBA matte with the research opt-in) live; the prompt enhancer (`generate.enhancePrompt`, `enhance`) rewrites prompts with the picture in view | Expand, Generate Image, instruction Edit, Remove Background |
| 3.5 Select by text (SAM 3.1) | 🟢 2026-10-09: `select.byText`, `select.subjectML` and `select.byPoint` live; SAM is Remove Background's permissive route; box prompts and soft-matte refinement would be refinements | `select.byText`, ML Select Subject, point/box object selection, SAM-backed mattes for Remove Background |
| 4 Models and workflows as data | ⬜ | Model catalogue, workflow template import, references, LoRAs, downloads with checksums |
| 4.5 Assistant | ⬜ | In-app assistant panel over a local LLM/VLM, driving the command registry; prompt enhancement, auto-naming |
| 5 Quality and depth | ⬜ | Harmonize, Generative Upscale, Generate Similar, generative smart objects, PSD interop |
| 6 Beyond ComfyUI | 🟡 in progress: [`plan-native-engine.md`](plan-native-engine.md) (phases 0–6). Phase 0 ground truth, sidecar and worker skeleton done 2026-10-18; **Phase 1 SAM 3.1 native at parity and under `auto` 2026-10-19** (16/16, our own export of ComfyUI's modules); **Phase 2 Qwen-Image-2.1 text to image native at parity and under `auto` 2026-10-20** (resident, ComfyUI's noise and schedule reproduced, 40 dB on the fox, faster than the ComfyUI route per picture) | Native inference backend behind the same trait (`genai-native`, worker process, sd.cpp and ComfyUI as day-0 lanes); web-build path |

## Phase 0: Foundation ✅

Done 2026-10-08 (see [`devlog.md`](devlog.md)): repository cloned, rustup + MSVC build verified
(release build 2 min 39 s), `cargo test --workspace` run for a baseline, research on the models
and the ComfyUI API, the design, and this documentation set. Open item: publish the branch (needs
`gh auth login`; `dev-environment-windows.md` (local to the development PC) › Publishing).

## Phase 1: Backend crate and headless Generative Fill

**Goal.** `photocraft-cli run photo.png --cmd select.all --cmd generate.fill --params '{"prompt":"a red bicycle"}' --out out.psd`
produces a layered file with a new "Generative Fill" layer, against a local ComfyUI.

1. New crate `crates/genai` (`photocraft-genai`), registered at **L4** in `xtask/src/layers.rs`
   (same layer as `plugins` and the reserved `ml`). Depends on `geom`, `color`, `raster`, `doc`,
   `codecs` (PNG encode/decode) only. Carries the never-crash lints.
2. `GenerativeBackend` trait: `capabilities()`, `models()`, `submit(request) -> JobTicket`,
   `poll/wait(ticket, progress, cancel)`, `interrupt(ticket)`, `health()`.
3. `comfy` module: blocking HTTP client (pure Rust, rustls) + WebSocket progress; the flow in
   [`comfyui-setup.md`](comfyui-setup.md) › API primer. Cancellation calls `/interrupt` and drops
   the ticket. Every failure is an `Err` with the server's message.
4. Workflow templates as API-format JSON with `{{placeholders}}` in `crates/genai/workflows/`
   (fill, expand, image, edit, matte), one per model family, validated at load.
5. Engine: `crates/engine/src/generate_cmds.rs` with `generate.fill` using `jobs::edit_job`
   (background, cancellable, one undo step). Params: `prompt`, `negative`, `model`, `seed`, `steps`,
   `variations` (1 in Phase 1), `margin` (context around the selection), `layer`, `target`.
   Result: the new layer id, the seed used, timing.
6. Preferences in the existing `integrations` section (Edit › Preferences › AI Integrations…):
   `integrations.comfyServer` (URL), `defaultGenerateModel`, `defaultEditModel`,
   `generativeTimeoutSecs`, read by the command (so the "settings that do nothing" audit stays at
   its number).
7. Tests: unit tests for template filling and response parsing; an in-process fake ComfyUI
   (a `std::net::TcpListener` serving canned `/prompt`, `/history`, `/view`, `/ws`) for the
   command tests; one `#[ignore]` live test against a real server (env `PHOTOCRAFT_COMFY_URL`);
   `panic_hunt` coverage; graceful failures (server down, bad JSON, cancelled mid-run).

**DoD.** The CLI line above works on the development PC with Qwen-Image-Edit-2511; `cargo xtask
layers`, `wasm` (the crate compiles for wasm with the client behind `cfg(not(target_arch = "wasm32"))`,
returning "unsupported on the web"), `parity`, `panic_hunt` and clippy are green; VRAM and timing
numbers recorded in the dev log.

**Status 2026-10-09: done.** Items 1–7 are implemented (`crates/genai`,
`crates/engine/src/generate_cmds.rs`, the `integrations.*` preferences, 25 crate tests and 21
command tests against the in-process fake server), and the live DoD was met on the dev PC against
ComfyUI 0.39.0: Generative Fill with Qwen-Image-2.1 (4.7 s warm) and Qwen-Image-Edit-2511, text to
image with Krea 2 Turbo (9.8 s cold) and 2.1; timings and VRAM in [`devlog.md`](devlog.md).

## Phase 2: Generative Fill in the app

**Goal.** Select, type a prompt in the contextual bar, press Generate, pick one of three
variations, keep it as a masked layer. Feels like Photoshop.

1. Menu: **Edit › Generative Fill…** in `menu_catalog.rs` / `menus.rs` (upstream's catalogue has
   no generative entries, so this is an addition, not a parity row), plus the selection context
   menu entry (`canvas_tool_menu.rs` already lists it as Photoshop's order).
2. A **Generative task bar** widget (prompt field, model picker, Generate, variations strip,
   Cancel) anchored under the selection; UI state in `state.rs` so the control channel can drive
   it (`ui.set`), strings through `tl!` with catalogue coverage.
3. Progress: the job's `JobInfo` drives a progress bar with the ComfyUI step count; binary preview
   frames shown as a dimmed overlay inside the selection (optional, preference).
4. Variations: `variations: 3` runs one prompt with three seeds (batched in one workflow when the
   model allows); results are kept as hidden sibling layers in a group "Generative Fill" with the
   chosen one visible, or as a layer with stored alternates (decision in Phase 2 design).
5. Generative layer metadata (prompt, negative, model, seed, workflow hash, server version) stored
   on the layer (a `doc` field → also `format` manifest + PSD passthrough as an unknown block).
6. Preferences UI: Edit › Preferences › Generative (server URL, default model, timeouts, previews,
   NSFW filter toggle for community-licence models that require moderation).
7. Model picker reads the catalogue and the server's `object_info`; research-only models carry a
   badge and are never pre-selected.

**DoD.** Visual check with the offscreen snapshot example; a control-protocol script that selects,
fills and screenshots is committed as a test; `ui.inspect` exposes the task bar state.

**Status 2026-10-09.** Items 1, 2, 3 (progress in the bar, no preview frames yet), 4 (variations
as hidden sibling layers switched by `generate.variation`), 6 (generated Preferences page) and 7
in its first form (the bar's template picker, research templates gated) are in; see the dev log
entries of 2026-10-09. The generated schema dialog remains for Generate Image, Select by Text and
for a hidden bar. **Closed 2026-10-09 (late):** item 5, generative layer metadata: every layer
a generative command makes carries a `GenerativeInfo` (command, prompt as typed, resolved
template, seed, steps, guidance, edge, request rect, name; for images the size and the
transparent flag) in a PSD additional-layer-info block (`cpGn`, so it survives PSD round trips
and duplication); `generate.info` reads it and `generate.similar` (Edit › Generate Similar,
enabled on such a layer) runs it again with a new seed in the same place, the layer's mask as
the area, the result above it. The picker's installed badges: the bar starts a background
`generate.models` probe (`async: true`) and marks templates whose files the server lacks "(not
installed)". The control-protocol script test of the DoD is
`generative_bar_tests::the_menu_opens_the_bar_and_generate_runs_the_fill_with_variations`
(select → `ui.set` → generate → `ui.inspect`, against the fake server). ~~Still open: preview
frames.~~ **Closed 2026-10-29:** live previews from the native worker, Preferences › AI
Integrations › "Preview while generating: Off / Fast / Sharp" (default Fast), drawn over the
result's place (fill/edit/expand area, the canvas, each Upscale tile) and as the Generations
row's thumbnail; branch `feat/live-preview` (genai-native `feat/live-preview-2`). Open: MOSS-Image
(its own branch), a split's layers side by side, a new document's canvas preview.

**Performance pass 2026-10-09** (numbers in [`benchmarks.md`](benchmarks.md)): Lightning tiers
for 2511 with `defaultFillTemplate = auto` picking the 8-step tier when its LoRA is installed, a
1 MP request cap with Lanczos resampling, content-addressed uploads (the encoder is cached across
variations), prompt wrapping and an outward-feathered request mask against seams, `--fast
fp8_matrix_mult` on the server, then native-size sampling (the latent at the request's own size,
16-px alignment, a 512-px floor) and Edit › Purge › Generative Models. A Lightning-8 fill is
9–11 s on the 5090 at the test selection, a small selection 10 s, a Qwen-Image-2.1 re-roll
2.6 s. Next levers are listed in `benchmarks.md` §6 (a working FP8-tensor-core checkpoint,
SageAttention, step caching, purge suggestions from the timings). Edges: since 2026-10-09 the
result layer's mask is by default the soft, noise-dithered re-rendered band (`edge: soft`;
`hard` = the exact selection), for fills and expands alike.

## Phase 3: The Firefly core set

- **Generative Expand** (`generate.expand`): hooks Image › Canvas Size and the Crop tool's
  "expand" state; pads the composite, masks the padding with an inward feather.
- **Generate Image** (`generate.image`): new document or new layer; width/height from the
  document; Krea 2 Turbo or Z-Image Turbo; optional reference image.
- **Instruction Edit** (`generate.edit`): edit the active layer or the composite with an
  instruction; optional mask; references.
- **Remove Background** (`generate.removeBackground`): produces a layer mask from a matte
  (BiRefNet) or from Qwen-Image-2.1's RGBA output; never destroys pixels; also exposed as
  Select › Subject (generative) feeding the ordinary selection.

**DoD.** Each command documented, tested, in `docs/parity.md`-style generated listings, drivable
over MCP; the ten-task MCP acceptance test gains two generative tasks.

**Status 2026-10-09.** `generate.expand` landed: `left/top/right/bottom` in pixels or a larger
`width/height` placed by Canvas Size's anchors; one undo step (the canvas grows, the picture
moves by the left/top pads, the result is a layer whose soft mask blends the re-rendered edge
band into the picture); its own templates (`qwen-edit-2511/expand-lightning-8`, `expand`) show
the model only the picture as a reference while the padded canvas is the sampling latent; an
empty prompt continues the scene. Shares the fill pipeline (`fill_region`: Lightning tiers,
`auto`, sizing, feather, variations). Later the same day: soft, dithered edges by default
(`edge: soft|hard`), the edge-replicating pre-fill, a 4 % band; the empty prompt is the most
faithful continuation (`benchmarks.md` §3). Then **Remove Background** landed on Qwen-Image-2.1's
RGBA output (`generate.removeBackground`, Edit › Remove Background (Generative)…: the model's
alpha becomes the layer's mask or, with `asSelection`, the selection; `prompt` names what to
keep; pixels are never changed) together with `generate.image`'s `transparent` option (the
layer keeps the alpha the model returns). Both are research-licence opt-ins until a permissive
matting model (BiRefNet) is wired. Then **Generative Edit** (`generate.edit`, Edit › Generative
Edit…): the composite goes to the official 2511 edit graph with no noise mask (its own
templates, `edit-lightning-8` and `edit`, `auto` between them), the result is a layer masked
to the selection when there is one; and the **automatic purge** before a run whose model files
differ from the last run's (`run_switching`, every generative command), which replaces the
31–131 s thrash after a graph switch with one reload. The **Crop tool's expand state** followed:
the Crop options bar has a Generative Expand checkbox; with it on, a frame dragged beyond the
canvas crops to the part inside (when that is not the whole canvas) and then has the model
paint the overhang through `generate.expand` (`canvas::commit_crop`). Remove Background's
permissive route landed last: `template: auto` takes SAM 3.1 (the prompt or "the main
subject", a hard-edged mask) unless the research opt-in and the 2.1 files make the soft matte
possible; the detector route softens its mask with the classical edge refinement. **Split
into Layers** (`generate.splitLayers`, Qwen-Image-Layered, Apache-2.0) decomposes the picture
into RGBA layers, background first, each a new layer. The **prompt enhancer**
(`generate.enhancePrompt`, the task bar's wand button, `enhance` on fill/edit/image, Preferences ›
Enhance prompts) answers the first user session's finding that prompts saying "this" give the
model nothing to hold on to: the Krea 2 text encoder (Qwen3-VL 4B) rewrites the prompt with the
picture in view, naming what is there; the layer remembers both prompts. The **NVFP4 policy**
(Preferences › Model precision, template slots' `nvfp4` alternatives) loads Comfy-Org's 4-bit
repacks on Blackwell GPUs: the same pictures, less memory, Krea 2 18 % faster. Open: a prompt
study for Expand, the layered model above its 640 px working size, transparent fills.

## Phase 3.5: Select by text (SAM 3.1)

Design in [`ai-integration-proposal.md`](ai-integration-proposal.md) §2.1. Adds a SAM 3.1
template to the genai crate (same client, no diffusion model), the commands `select.byText`,
`select.subjectML` and `select.byPoint`, Select › Select by Text… and the Object Selection tool's
ML mode, and swaps the matte source of Remove Background when the server is up. Licence: SAM
License (commercial allowed, military/weapons uses banned, licence passed on; the app ships no
weights). **DoD.** "select the dog" on a public-domain test image yields one undoable selection
per instance in under a second on the 5090; classical fallback when the server is down; `panic_hunt`
green; the MCP acceptance test gains a select-by-text task.

**Status 2026-10-09.** `select.byText` and `select.subjectML` landed and ran live ("the
lighthouse" in 4.6 s cold, "sky" in 1.8 s warm; see the dev log). Select › Select by Text… opens
its dialog in the app since the Phase 2 slice. `select.byPoint` followed, and then the **Object
Selection tool's ML mode** (night): with a server configured a click asks SAM 3.1 for the object
under the pointer (⇧ adds, ⌥ subtracts; a drag stays the classical rectangle), the options bar
has a "Select by text" field, and Select Subject goes through the detector. Open: the BiRefNet
soft-matte refinement, the MCP acceptance task, box prompts.

## Phase 4: Models and workflows as data

- `docs/comfy/models.toml` → compiled catalogue: id, family, task, licence class + URL, files +
  sha256, VRAM class, default settings, template id.
- Template import: a user can drop an API-format workflow exported from ComfyUI with named
  placeholder nodes; PhotoCraft validates it against `object_info` and lists it in the picker.
- Downloads: a "Download models" helper that fetches the files for a template into the ComfyUI
  folder with checksum verification and progress (a background job; native only).
- LoRA and style reference selection (Krea 2 style-reference workflow), reference images for
  Qwen-Image-2.1 (up to 10).
- Health panel: server version, free VRAM, loaded models, queue length; one-click `/free`.

**Shape (2026-10-10).** [`artcraft-study.md`](artcraft-study.md) §5 orders this phase after
ArtCraft's patterns: a catalogue with capability flags the UI reads, a persistent task queue
with typed failures and a queue popover, a prompt panel with a reference deck and the time (their
cost) on the Generate button, then providers chosen late behind the user's own keys (Fal, the
ArtCraft Omni API) with the local server as the default and a fallback order.

**ShotoPhopEX build (2026-10-11), following [`plan-artcraft-experience.md`](plan-artcraft-experience.md):**

- ✅ Plan Phase 1, the persistent task queue: the **Generations library** (every generative job
  recorded with typed failures, results and thumbnails stored under the config folder, kept
  across restarts, shared with the CLI; 500 results / 5 GB) and **Window › Generations** (in
  progress, failed, completed; Place, Generate Similar, Save…, Copy prompt; Recreate). Commands
  `generate.jobs`, `.jobs.dismiss/.pin/.clear`, `generate.recreate`, `generate.place`,
  `generate.export`.
- ✅ Plan Phase 2: the Generate panel (Window › Generate: mode strip, reference deck, prompt,
  model / aspect / size / count, the time estimate on the button), template capability flags
  (`pages`, `references`, `sizes`, `counts`, `estimatedSeconds`), reference images on
  `generate.edit` / `generate.image` (2511 edits take two; new `qwen-edit-2511/image-ref`,
  `krea2-turbo/image-style`, research `qwen-2.1/edit`), image variations, the task bar as the
  panel's compact form (`showGenerativeBar` auto / always / never, `enterToGenerate`).
- ✅ Plan Phase 3: the model picker (one component for the Generate panel, the task bar and the
  generated dialogs): family glyphs, newest first, family submenus from eight rows
  (`groupModelsByFamily`), the estimate, `research` and `not installed` badges, an Auto row that
  names the model it resolves to; `ui.inspect.modelPickers`.
- ✅ Plan Phase 4: the Variations strip at the canvas edge (`integrations.variationsStrip`): the
  last five runs of the document as tiles from the Generations library, click to show, × to
  drop, ✓ to keep, ✦ for more, placeholders while running, Retry when failed;
  `generate.dropVariation`, `generate.keepVariation`, `ui.inspect.variations`.
- ✅ Plan Phase 5: the Generate hub (Home-screen cards, Help › Generative Features…), completion
  alerts (`integrations.alerts`, the `alerts` cargo feature, rodio; on in the Windows and macOS
  packages), saving results (`saveResultsDir` / `saveResultsNaming`, select mode with Save
  selected in Window › Generations), and Generative Angles (fal's Multiple Angles LoRA, Apache-2.0:
  `qwen-edit-2511/angles`, `generate.angles`, an orbit window).

- ✅ 2026-10-16: **Quality** (`integrations.quality`, best by default: full-step templates, no
  NVFP4 under auto) after the native-inference study (`docs/comfy/research/`), and the **Object Selection
  tool's SAM 3.1 mode**: object finder on hover, box prompt, Refine mode (include / exclude
  spots), refine edge, Select › Subject through the detector. ✅ Native inference
  (`docs/comfy/plan-native-engine.md`): the app-managed ComfyUI sidecar first, then the native
  worker in the same executable: SAM 3.1 (2026-10-19), Qwen-Image-2.1 text to image, edit,
  fill, expand, matte and transparent output (2026-10-20) at parity and under `auto`; Krea 2
  Turbo text to image ported the same night (served under `native`, at the model's own level of
  agreement, 4.3–4.85 s a picture resident) but kept on ComfyUI under `auto` until the fp8 GEMM
  lands; ✅ 2026-10-27 **Krea 2 text to image under `auto`**: ComfyUI's fp8 files on its own
  fp8 recipe, exact flash attention, fused small ops (19 of 28 at the gate, ComfyUI's two routes
  agree on 20; 4.0 s a picture against 4.0–4.7); ✅ plan Phase 5 started (the CUDA libraries
  delay-loaded, the runtime folder script, `package.ps1 -Native`); ✅ GPU detection (a card
  the build has no kernels for, or no NVIDIA driver, is refused with a sentence and `auto` stays
  on ComfyUI) and the engine page (the native worker's state, card, memory and loaded models in
  the status-bar dot and Preferences › AI Integrations; Start / Stop / Purge models / Show log);
  ⬜ the models-folder first run, the clean-profile install; ✅ 2026-10-27 the owner retires
  Qwen-Image-Edit-2511 as the default: with the research opt-in and the native worker, `auto`
  runs Qwen-Image-2.1 for fill, the removals, expand and edit (no ComfyUI for them); ⬜ Krea 2's style reference,
  Qwen-Image-Edit-2511 and the layered model still go to ComfyUI.

**Phase 4 status after the ShotoPhopEX build (2026-10-12):** ✅ capability flags the UI reads
(kept in the templates' metadata rather than a separate `models.toml`), ✅ LoRA and style
reference selection (Krea 2 style reference, the Multiple Angles LoRA) and reference images
(two on 2511 edits, nine plus the picture on Qwen-Image-2.1), ✅ one-click purge
(Edit › Purge › Generative Models); ⬜ template import from a user's ComfyUI export, ⬜ the
model download helper (deliberately not started: the app ships and fetches no weights), ⬜ a
health panel beyond Test connection.

## Phase 5: Quality and depth

**Status (2026-10-12):** ✅ Generate Similar (and Recreate from the Generations library);
✅ Generative Fill from a reference image (2026-10-21: `fill-ref` templates, `grow`, `erase`,
the side-by-side rewriter; learnt from AusBoss's Krea 2 clothing workflow, see
`research/notes/ausboss-krea2-clothing-workflow.md` for what is still to take);
✅ Content-Aware Fill, the Remove tool and Spot Healing through the fill model (2026-10-21,
`generative_remove`: the classical PatchMatch fills stay as the fallback);
⬜ everything else below.

- Harmonize, Generative Upscale, Generate Similar.
- Generative smart objects: the generation becomes a smart object whose source records the
  request; "regenerate" re-runs it non-destructively; smart filters apply on top.
- PSD interop: generative layers export as ordinary raster layers with masks; metadata survives as
  a private tagged block so re-import keeps it.
- ✅ 2026-10-23 Batch: `generate.*` in actions and `photocraft-cli batch` (e.g. remove backgrounds
  for a folder): a generative step in a batched action runs to completion on every file and the
  runs land in the Generations library as `run`'s do; the server is a `prefs.set` step of the
  action (the CLI loads no preferences of its own). Tested against the fake server.
- Measured quality: a small public-domain test set with prompts and expected properties (mask
  respected, seams within tolerance, no change outside the mask), run nightly like
  `perf-nightly.yml`.
- ✅ 2026-10-26 **LoRAs on the native engine**: `loras: [{file, strength}]` on the fill, edit,
  image and removal commands, applied by the worker as side paths on the resident Qwen-Image-2.1
  and Krea 2 DiTs (plan-native-engine §9); the installed set (AusBoss's Consistency, Outpaint,
  Restore and Outfit Swap, AnyPaint, Viggle Turbo, the MIT BFS swaps) in `models.md`.
  ✅ 2026-10-27: the Generate panel's LoRAs rows (`generate.loras`), the ComfyUI route applies
  `loras` too, parity through the app measured (edits at the no-LoRA agreement). Open: the
  Krea 2 reference path AnyPaint needs.
- ✅ 2026-10-27 **Generative Upscale** (`generate.upscale`): bicubic, then the edit model per
  tile with AusBoss's restore recipe; SeedVR2 as a non-redrawing "Enlarge 2×" still open.
- ✅ 2026-10-27 **Remove with its shadow** on the 2.1 route (the change band) and the worker's
  chat key/value cache.
- ✅ 2026-10-29 **MOSS-Image natively** (Apache-2.0, `moss-image/image` and `-base` text to image
  from Comfy-Org's files, `models.md`) level with ComfyUI 0.43 (11.5–12.0 s vs 11.1 s of sampling
  at 1024², 50 steps, same GPU state); ✅ 2026-10-30 live previews (Fast, Sharp with TAEF2;
  genai-native PR #10); ⬜ an opt-in NVFP4 Fast mode (owner's call), its edit and control LoRAs,
  the JSON layout prompt in the Generate panel.
- Taken from AusBoss's catalogue, in order (`research/notes/ausboss-catalogue-2026-10.md` §4):
  the 2.1 route for removals (measuring), realign to source, the differential composite
  ("remove with its shadow"), remove to its own layer, the rewriter on the resident encoder,
  LaMa as Spot Healing's first tier, Generative Upscale with the restore recipe, 2.1 Turbo.

## Phase 6: Beyond ComfyUI

🟡 **The Rust rebuild** (opened 2026-10-27, its own task:
[`goals/rust-rebuild.md`](goals/rust-rebuild.md)): every generative feature in ShotoPhopEX's
own Rust code, one executable, no Python and no ONNX Runtime; the queue R0–R6 (SAM 3.1 on
candle, the 2511 holdouts, Qwen-Image-Layered, one models folder, packaging, native by
default) and its progress log live there. As of 2026-10-28: R0 ✅, R2 ✅ (fill and image from
references, Angles natively), R4 ✅ for 2.1 and Krea 2 (ComfyUI's folder as it is; ComfyUI's
int8 repacks in progress), R1 ✅ (SAM 3.1 on candle, f16 on the GPU, as fast as ONNX Runtime
through the app; no `ort` in the -Native build; on 2026-10-29 the ONNX Runtime route was deleted from both
repositories, `sam-onnx` and `gn-perception`'s `onnx` feature with it), R3 ✅ at 640 px (57 s against ComfyUI's 58 s;
the 1 MP size slower), R5 ✅ (the -Native zip, 557 MB, runs every generative command from a
fresh profile with no ComfyUI, no Python and no ONNX Runtime or cuDNN), R6 🟡 (`auto` prefers
the worker already; the permissive defaults wait on the owner's licence decision).

- A second `GenerativeBackend`: in-process inference for small models (candle or burn, pure
  Rust, feature-gated, native only), starting with matting (BiRefNet-class) and a small
  text-to-image model; keeps the UI identical.
- Web build: the genai crate's HTTP path on wasm via `fetch` (`ehttp`-style), talking to a user's
  own loopback server when the browser allows it; otherwise the commands stay "unsupported on the
  web".

## Risks and decisions to make

| Risk / decision | Mitigation |
|---|---|
| Model licences (Qwen-Image-2.1 research-only; Krea 2 revenue cap and moderation duty) | Catalogue carries licence class; permissive defaults; moderation hook for community models |
| ComfyUI API drift (node names change per release) | Templates pinned per model family and validated against `object_info` at load; version check in `health()` |
| Blocking HTTP inside a job worker | Jobs already run on worker threads with cancellation; the client polls `ctx.cancelled()` between requests and uses short read timeouts |
| wasm gate | Client behind `cfg(not(target_arch = "wasm32"))`; the crate and the commands compile everywhere |
| Large transfers (2K RGBA PNG round trips) | Crop to selection bounds + margin; stream to disk when over a size cap; measure |
| Brand licence on publication | [`upstream.md`](upstream.md) checklist before the first published build |
| Upstream merges touching `menus.rs`, `commands.rs`, `state.rs` | Keep fork edits to those files surgical; put commands in a new module (`generate_cmds.rs`). Last merge: upstream 0.11.1 on 2026-10-21 (dev log) |
