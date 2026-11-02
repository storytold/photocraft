# PhotoshopEX dev log (continues the Comfy PhotoCraft log)

Newest entry first. Terse: what landed, numbers, what is still open. Upstream keeps its log in
the gitignored `log/devlog.md`; this one is tracked so the next session (any machine, any agent)
can pick up.

## 2026-11-01 (night): Split can try several seeds and keep the best-separated

- With the dialog fixed, the owner's picture still split badly at most seeds. ComfyUI's own
  Qwen-Image-Layered graph (shift 1.0, euler/simple, 20 steps, CFG 2.5, the same sizes and prompt)
  did no better: a foreground layer at 2 of 10 seeds on his picture (3 of 10 with a caption) and 7
  of 10 on the portrait, against 7 and 10 of 10 through the native worker. So the model, not the
  port; the native route's optimisations (INT8, the MLP fusion, the VAE GEMM, sparse attention)
  changed nothing either (switched off through their variables, which do reach the worker).
- `generate.splitLayers` takes `tries` (1–4; 1 by default, the owner's choice, with a Tries slider
  in the dialog defaulting to 1): seeds in a row, the
  split whose layers above the background hold the most of the picture kept (`split_separation`;
  each try's score in the result's `separation`). On his picture from seeds 1, 4 and 7: 8.2 s for
  one try, 24.0 s for three (about 8 s a further try, the conditioning and the DiT resident); the
  kept split was visibly better twice (nothing → a person layer; fragments → head and shoulders on
  their own layers) and arguably worse once (a clean face cutout gave way to a larger
  shoulders-and-dress layer, the score preferring the layer that holds more).

## 2026-11-01: upstream PhotoCraft v0.19.0 merged

Merged storytold/photocraft `v0.19.0` (231 commits since v0.18.0) into `photoshopex` (26
conflicts, 22 of them i18n TSVs, merged and deduplicated). Kept the fork's README, app icon and
generative tools; no ComfyUI code came back.

- Remove tool: upstream now turns off "Create a new layer" when painting into a mask or a
  channel (`remove_params(o, composite)`); the fork's Generative and Best of two options stay
  in `remove_options`, with the new rule applied there.
- `crates/engine/Cargo.toml`: both `photocraft-genai` and upstream's new `base64` dependency.
- `docs/scorecard.md` regenerated (11 of 206).

**Still open**

- Merge the PR into `photoshopex`, then rebuild and run the GPU tests on the 5090.

## 2026-11-01 (late): Split into Layers from the menu split nothing

- The owner: "the layers were not split basically". The generated dialog (Edit › Split into
  Layers, the Generate Hub card) read Split's optional `maxPixels` (`4096..4194304?`, no default)
  as a slider at its minimum and sent 4096: every Split from the menu sampled a 64-px thumbnail
  (48×64 for his 452×678 picture), the three layers overlapping blurs (87–93 % of the covered
  pixels shared). The engine call without `maxPixels` (the CLI, automation, the earlier checks)
  used the 640-px working size, which is why they looked fine. `maxPixels` now sits in braces like
  `seed`, so the dialog leaves it out; a ui test fails any generative dialog's optional range
  without a default. In the app on his picture: 416×640, a face layer, a body layer and the
  background plate. Contact sheets: `photocraft-tests\remove-2026-11-01\shots\split-*`.
- Fast draft (step cache, 8 of 20 steps) did not change the separation (portrait, seed 11: the
  person on one layer either way). At about 1 MP (`maxPixels` 1000000) the portrait came back
  worse (a whole-picture layer and red blobs): 640 px, the model's size, is the right default.
- genai-native pinned at cf85edf (#21, #22, from the owner's "look for more things like this"):
  SAM 3.1 keeps its read checkpoint in host memory when it gives the device back, so each Remove
  stroke's SAM reload is 0.3 s instead of 0.7–0.8 s (the first after launch 1.1 s); the Layered
  and 2511 prefetch faults pages on every core, so Split's DiT is ready about 2 s sooner (warm,
  after a 2.1 edit: 10.9–11.8 s to 8.8–10.4 s; Split 20.2–21.3 s to 18.2–20.3 s). Measured and
  dropped: caching Krea 2's low-rank split (its load is I/O), previews (no real cost), MOSS mapped
  loads (I8 tensors candle cannot name). #23 (drop the chat head before the encoder) stays open:
  no gain shown in the app.

## 2026-11-01 (evening): Remove after an image generation, 25 s to 13 s

- The owner: "remove tool is still really slow". Measured in his app (his preferences, generative
  `paint.remove` on the stock photos, seed 42, `remove-2026-11-01\` beside the reliability pass):
  a warm Remove is 4.6–5.7 s (bicycle 11.7 s: naming 0.3–0.7 s, SAM 0.2 s a find plus a 0.7 s
  load, two 2.1 edits of about 1 s, the judge 0.2 s). The first Remove after an image generation
  was 24.7–27.3 s: the swap from Krea 2 to Qwen-Image-2.1 reloads the text encoder (5–7 s), the
  DiT (3 s), the vision tower and VAE (1 s), SAM (1.1 s) and the chat head (10.2 s).
- genai-native #19, pinned here: the chat head's Q8_0 is quantized on every core with candle's own
  blocks (candle quantized the CUDA copy on one thread). Head 10.2 s to 1.5 s; Remove after a Krea
  image: hat 24.7 to 19.1 s, watch 27.3 to 17.8 s; same names, byte-identical results.
- genai-native #20, pinned here too: model loads fault their pages in on every core, then upload
  from the mapping (candle's mapped VarBuilder soft-faulted each page as it copied: 3.4 GB/s on a
  file already in the cache). 2.1 text encoder 4.5–6.1 to 2.6–2.8 s, DiT 3.0–3.4 to 2.4 s, vision
  tower 0.65 to 0.45 s; Remove after a Krea image 12.2–13.6 s (hat, watch, cone), byte-identical
  removals and seeded Krea images. The first run after a launch can still find the files out of the
  cache (encoder 5.3 s once: about 57 GB of models share his 62 GB).
- The heavy jobs in his log (6–7 samplings at about 1 MP, 24 reference encodes) were Upscale tiles.
- Left: a swap-Remove is now encoder 2.7 + DiT 2.4 + SAM 1.1 + head 1.35 + VAE and vision 0.75 s
  of loads and about 4.5 s of removal.

## 2026-11-01: in-app reliability pass; Split no longer pages on an INT8 cache miss

- The owner's rule: reliability is measured in the running app. 27 cases through the control
  channel, the way the panel is used (prompt, Ctrl+Enter), with his real preferences. Table,
  screenshots and raw rows in `C:\Users\5090\ComfyUI\photocraft-tests\reliability-2026-11-01\`
  (`RESULTS.md`). Fill, Edit (Auto, 2511, 2.1, Krea 2), Image (Krea 2, 2.1, MOSS), live preview
  Off/Fast/Sharp, Fast draft, Expand, LoRAs, a layer reference, Upscale, cancel, switching
  documents mid-job, undo/redo, layer edits and the PSD round-trip passed. Three failures, two bugs:
  - The panel kept a hand-picked mode for the whole session (fixed in #28, see below).
  - **Split into Layers timed out at 600 s** on every INT8 cache miss: the first split after a
    build that changes the entry key, or on a fresh cache. The converting load left the CUDA pool
    fragmented, the DiT loaded with 0 MiB driver-free and Windows paged every step (about 45 s
    instead of 1.5). genai-native #18: write the entry, drop, trim, reload from it; 2511's
    converting load too. Pinned here. In the app, forcing a miss: 80.4 s (10.7 GB driver-free
    after the reload); the next split, a hit, 40.2 s.
- Open, the owner's call: Expand's soft dithered edge shows a grainy ghost where the model
  redraws the blend band differently from the original (the grain was his 2026-10-09 request).
  Options in `RESULTS.md`.

## 2026-11-01: the Generate panel opens on the mode the canvas asks for; no licence gates

- The owner selected one eye, typed "close her eyes" in the Generate panel and got a whole new
  picture: the panel always opened on Image (text to image, the canvas ignored). It now shows
  Fill over a selection, Edit with a picture open, Image with nothing open. A mode picked by the
  person, a menu or a script holds until the canvas asks for another one (`GeneratePanel::context`:
  a selection made or dropped, the last document closed or one opened), so picking Image once
  cannot leave a later eye selection on Image. Found by the in-app reliability pass: with the pick
  sticking for the whole session, closing every document left the panel on Edit and Ctrl+Enter did
  nothing.
- The owner's rule (2026-11-01): every model is his own files, so nothing is refused, hidden or
  greyed out for its licence. The research-only error is gone from fill/image/edit/similar, the
  model listing allows every template (the licence stays a badge and a note), and
  `allowResearchModels` (on by default) only says whether Auto may pick a research model.
- Standing rules from the owner: measure speed in the running app and run reliability passes in the
  app itself, not only the CLI and cargo tests.
- Owner's machine: tools started from the Claude desktop app write `%APPDATA%` into the package's
  `LocalCache`; the owner's real preferences are reached through `\\localhost\C$\Users\5090\AppData`.
  The "empty Models folder" was that: his real file had none.

## 2026-10-31: speed pass 3 merged; genai-native pinned at e53bab0

- genai-native #11 (INT8 disk cache), #13 (Split step cache), #14 (block-sparse attention,
  opt-in, off), #15 (MLP fusion), #16 (VAE decoder implicit GEMM) and #17 (Krea 2 INT8 + rank-64
  correction, on by default) merged; main `e53bab0` verified on the 5090 (CUDA clippy, qir lib,
  moss_parity 7/7, layered, preview_live). Numbers: benchmarks.md, "Speed pass 3".
- The app pins `e53bab0`; Fast draft also runs Split into Layers (step cache 0.10). An empty
  Models folder falls back to `PHOTOCRAFT_MODELS`, `GN_MODELS` or `<exe dir>/models`.
- Open: why the owner's running app saw an empty Models folder although preferences.json sets one
  (the file loads with it set); block-sparse attention stays a measurement.

## 2026-10-30 (night): Fast draft; Krea 2's full-precision layers measured

- **Fast draft** (the owner's call; `integrations.fastDraft`, off by default, in all 20 languages):
  Generative Fill and Edit on a Qwen-Image-Edit-2511 template of 20 steps or more send the
  worker `params.step_cache = 0.05` (genai-native `0118813`; no environment variable). Through the
  app, the 40-step edit at 864x1152 went from 37.9 s to 24.6 s warm, 32.5 dB against off, with 18
  of 40 steps cached per branch. Repeats are byte-identical. Lightning tiers and other models are
  untouched.
- **Krea 2's three full-precision layers** (attention gate, `wo`, `down`), measure-only, on the
  local branch `perf/krea2-fp-int8`, 28 t2i cases at the 30 dB / SSIM 0.90 gate:
  - native fp8 against ComfyUI fp8: 19 of 28
  - INT8 against ComfyUI: **17**, and 21 against native fp8
  - fp8 with per-token activation scaling against ComfyUI: 14, and 18 against native fp8
  - warm 1024² time: fp8 3.68 s, INT8 2.75 s, token-scaled fp8 3.90 s

  INT8 is 25 % faster but two cases short of today's parity, so it is not landed; the worst four
  are the same street and architecture scenes that native fp8 already misses.

## 2026-10-30: speed pass 2, follow-ups: an opt-in step cache; three dead ends

Branch `perf/step-cache`, stacked on `perf/2511-int8` (genai-native `0f7bc5f`). There's an opt-in
first-block step cache for 2511 (`QIR_STEP_CACHE=<threshold>`, one per CFG branch). On the 40-step
edit: 39.4 s off, 24.8 s at 0.05 (32.5 dB, hard to tell apart), 16.9 s at 0.1, 12.3 s at 0.2 (the
snow mostly gone). It's measure-only; the default path is unchanged. Checked and not landed, details
in benchmarks.md §0: Krea's bf16 weight cache (−2 % for 9.6 GB), a cuBLASLt autotune (candle's GEMM
is at the bf16 ceiling already), and a device-built causal mask for 2.1's prefix (already ~0.2 s).
Next: the activation quantize fused into its producing kernels, with a MOSS regression.

## 2026-10-30: speed pass 2, item 3: 2511 on INT8 linears

Branch `perf/2511-int8` in both repos, stacked on `perf/split-int8` (genai-native `6a35b91`).
2511's resident DiT, its LoRAs merged, runs ConvRot INT8 linears by default (`LayeredDit::load_int8`).
Lightning-8 edit 7.15 to 4.41 s, 40-step 65.9 to 38.1 s warm. Same seed against fp8: 22.1 and 36.3 dB,
against seed bands of 14.4 and 16.3 (benchmarks.md §0). The CFG pair batched in one forward was
measured and dropped (4 % slower, 35.6 dB). `QIR_2511_INT8=0` opts out.

## 2026-10-29 (night): speed pass 2, item 2: Split on INT8 linears

Branch `perf/split-int8` in both repos, stacked on `perf/split-resident` (genai-native `16adfc4`).
Split's DiT runs ConvRot INT8 linears by default. Warm 86.3 to 61.9 s, composite against the input
37.25 dB against 36.91 (benchmarks.md §0). The paging that parked INT8 earlier was an unsynchronized
conversion queue (the freed bf16 temporaries were still pending), not the weights; a synchronize per
converted linear fixed it. The flag is scoped to `LayeredDit::from_resident_file`, because 2511
shares `LayeredLinear` and binds LoRAs, which INT8 refuses. Checked: a 2511 Lightning-8 edit on the
same worker still runs. `QIR_LAYERED_INT8=0` opts out.
Open: the INT8 resident set is ~7.6 GB above fp8's (free after load ~3 GB against 11 GB). If larger
splits page, that's where to look.

## 2026-10-29 (night): speed pass 2, item 1: Split keeps its DiT resident

Speed pass 2 started from genai-native main `aa5ccdf` and photoshopex `72efc7dd`. Branch
`perf/split-resident` in both repos (genai-native `9e10f13`, pinned here). `LayeredPipeline` keeps
the DiT and the VAE resident between splits, plus a one-entry conditioning cache for the prompt.
The app sends `SPLIT_DEFAULT_PROMPT`, so warm splits hit it. A cache miss drops the DiT before
the text encoder loads. Warm Split 91.9 to 86.0 s, layers byte-identical (benchmarks.md §0).
`QIR_LAYERED_RESIDENT=0` restores per-split loads.
The denoise (84.4 s of 86) is what's left; item 2 (INT8 linears without paging) is next.

## 2026-10-31: upstream PhotoCraft v0.18.0 merged

Merged storytold/photocraft `v0.18.0` into `photoshopex` (37 conflicts). Kept the fork's Pc
branding (README, app icon) and generative modules and tests; took upstream's new modules
(`brush_mode_menu`, `startup_report`), memory-usage prefs and packaging changes. No ComfyUI code
came back.

- Upstream's new `windows_and_linux_icons_keep_the_round_corners` test (#4478) failed on the Pc
  tile: `packaging/icon-tile.py` cropped 22 units for Windows and Linux. It now renders the full
  tile, as upstream does; the hicolor PNGs and `photocraft.ico` were re-rendered from
  `photocraft-1024.png` (same letterforms, no font change).
- `docs/scorecard.md` regenerated; i18n TSVs merged and deduplicated.
- Tests (cloud, no GPU): failures are wgpu, missing CJK glyphs, and three
  `font_commit_tests` cases that pick the container's system "Inter Display" over "Inter"; the
  fork does not touch the type tool.

**Still open**

- Merge the PR into `photoshopex`, then rebuild and run the GPU tests on the 5090.

## 2026-10-31: a new document's first save starts in the last folder saved to

Open dialogs already start in the most recent file's folder (#1889) and save dialogs where the
document was last saved, or beside it (#1826). An untitled document had neither, so its first
save started wherever the OS chose. `Preferences.last_save_folder` (`lastSaveFolder`, not in the
dialog) now keeps the folder of the last save dialog and `file_dialog.rs` `pick_save` falls back
to it; it survives a restart. Scorecard: 201 preferences, 12 unread (unchanged).
## 2026-10-31: App Icon (any image, one click) and the owner's local files out of the tree

Ported from `claude/project-thread-c7hno9` (8883a1af, ddbe96ae), which branched before the v0.17.0
merge, onto photoshopex. The title-bar look (no Discord button, wheel zooms) was already here from
9e75bf43, so only these two parts came over:

- Edit › Preferences › App Icon… (`edit.appIcon`, `app_icon_ui.rs`), also a click on the title
  bar's mark: Built-in plus the user's own icons (click to use, right-click to remove), Choose
  Image…, Paste Image, and an address with Download (`Services::download`, the system `curl`,
  30 s and 20 MB, http/https only). Pictures are fitted into a square of at most 256 px and kept
  as PNGs in the preferences (`appIcon`), at most 12; applied live to the mark and the
  window/taskbar icon. The exe's file icon stays the built-in "Pc" tile. Automation:
  `edit.appIcon {use | remove | add}`, `ui.inspect.appIcon`. Strings in all 21 catalogs (the 8
  catalogs added since were translated here).
- Untracked and gitignored, kept on the owner's PC: `docs/comfy/bench/`, `packaging/icon-px.py`,
  `CLAUDE.md`, `docs/comfy/dev-environment-windows.md`. Links to them in docs now name them as
  local files; older devlog entries still mention them by path, as history.

Open: not tried by hand in the running app. The Ps tile's PNGs remain in photoshopex's history
(9e75bf43, before 70ad043a redrew it as "Pc"); scrubbing them would mean rewriting photoshopex.

## 2026-10-31: Generate panel redesign (UI/UX pass)

Window › Generate's full form was a loose stack: a mode strip of pills, a prompt with two icons
stacked beside it, an unlabelled row of dropdowns that wrapped unpredictably (at a 320 pt dock the
Expand pads ran off the panel's right edge), and a 150 pt Generate button floating at the right.
Now (`generate_panel.rs`, `full`):

- A segmented mode switch (icons from the Generate hub), full width; what blocks the mode is a
  tinted callout, not faint text.
- Sections with small-caps titles: References (with "n / max"), Prompt, Settings.
- The prompt is one framed composer (accent frame while focused) with its toolbar inside: Enhance,
  the large editor, the counter. The negative prompt is behind a chevron disclosure.
- Settings are labelled rows in a fixed label column: Model, Quality (segmented Best · Balanced ·
  Fast), Aspect, Size (segmented, with the resulting pixel size under it), Count (segmented),
  Output (Document · Layer), Widen and Denoise for Fill. Expand's four sides sit around a to-scale
  diagram of the picture inside the grown canvas.
- One full-width Generate with the estimate; while it runs, Cancel with the percent and a progress
  line. Clear all and the shortcut hint under it.

Behaviour, commands and `ui.set`/`ui.inspect.generatePanel` are unchanged; the one-row compact form
is unchanged. `capture_generate_panel_snapshots` renders every theme × mode to PNGs when
`PHOTOCRAFT_GENERATE_SNAPSHOTS` names a folder (skipped otherwise).
## 2026-10-30 (overnight): live previews for MOSS-Image (review PRs, not merged)

The owner's overnight item 3. genai-native#10 (`moss-preview`, c2737f2) and this repo's #15
(the pin and `models.md`). MOSS samples in the FLUX.2 VAE's normalised, 2×2-packed 128-channel
space (`qir::preview::LatentSpace::Flux2`): Fast unpacks to 32 channels at 8× and applies
ComfyUI's `Flux2` factors; Sharp decodes every third step with madebyollin's TAEF2 (MIT, 10.7 MB,
`vae_approx/taef2.safetensors`, installed on this PC; 35.1 dB against the full decode on a final
latent). On a clean GPU slot at 1024², 50 steps, CFG 3: off 10.56 s, Fast 10.62 s (+0.6 %, 49
frames), Sharp 11.77 s (+11 %, 16 frames), the picture byte-identical in all three; moss_parity
unchanged. Earlier runs that night overlapped the speed thread's Krea timings (VRAM full, decode
7–84 s): Sharp then fell back to Fast as designed. Open: merge genai-native#10 first, then
re-pin #15 to its merge commit.

## 2026-10-29 (later): upstream PhotoCraft v0.17.0 merged

Merged storytold/photocraft `v0.17.0` (from v0.11.1+172; about 4,000 commits: egui 0.37, engine
and UI modules moved into folders, native panel docking, clap CLI). Generative modules stay
top-level; their hooks moved into upstream's new files. No ComfyUI code came back.

- Upstream replaced the old dock, so `heights_for`'s generative rules are gone. In the native
  dock the Generate and Generations groups give way to their tab strips before Layers shrinks
  (`panel_docking.rs`: min and compact 0 for `Group::yields`, body skipped when squeezed);
  folded, the task bar under a selection stands in for the Generate panel.
- New upstream tests that measure ⌥ + wheel turn off the fork's wheel = zoom default; docking
  tests expect the generative groups in Essentials, or hide them where upstream geometry is
  measured.
- Clippy now denies `cognitive_complexity` (30): `fill_region` and the big task-bar test were
  split.
- Tests (cloud, no GPU, no CJK fonts): every failure is a wgpu or missing-glyph one, and those
  fail on plain v0.17.0 too.
- Scheduled check "PhotoCraft release merge check" runs daily at 18:23 UTC: a new upstream
  vX.Y.0 tag gets merged on a branch and PR'd into `photoshopex`.

**Still open**

- Merge the PR into `photoshopex`, then rebuild on the PC.
## 2026-10-29: MOSS-Image natively, level with ComfyUI

The owner asked for "moss" on the native stack (nothing was in Downloads; MOSS-Image, OpenMOSS,
Apache-2.0, was the only MOSS on the PC, in stable-diffusion.cpp). Weights from Comfy-Org/MOSS-Image
(SFT int8 DiT, Qwen3.5-9B int8 encoder, FLUX.2 VAEs) and Qwen3.5's tokenizer.json in
`D:\PhotoCraftModels\moss-image`, junctioned into the ComfyUI `models` folder (`models.md`).
- genai-native `moss-image`, merged to main as aa5ccdf (on 92dc472): `qir::moss`: the Qwen3.5 hybrid encoder
  (Gated DeltaNet in its chunked form, tested against the recurrence), the 5B DiT in one-pass
  kernels (Krea 2's fused norms/RoPE/gates, SiTU inside the down projection's ConvRot quantize,
  SageAttention2, both guidance branches batched), the official snapped shift-5 schedule with
  the block-22 negative skip, the FLUX.2 small decoder in bf16; gn-diffusion templates
  `moss-image/image` (SFT) and `moss-image/image-base`. Here: the two templates (metadata),
  genai-native pinned at aa5ccdf. No live preview for MOSS yet (its latent is the FLUX.2 VAE's).
- Parity against the official pipeline on the same weights (dequantized by ComfyUI; reference
  script outside the repo): encoder cosine 0.9996, DiT 0.9999 (t=1000) / 0.9990 (t=500), VAE
  60.0 dB in bf16, a 512² 20-step picture 25.3 dB (the same picture to the eye).
- Speed at 1024², 50 steps, CFG 3, warm, against ComfyUI updated to v0.43.0 (the owner's ask;
  0.39 had no MOSS): 35.2 s first port → 17.7 s one-pass blocks → 11.0 s SageAttention2 (05:40);
  back to back with ComfyUI at 07:05 native 11.5–12.0 s against ComfyUI's 11.1 s (the GPU ran
  ±4 % between runs that day). Dropped: cuBLASLt int8 GEMMs (no faster than the fused CUTLASS
  epilogue). Through the app: a `native-diffusion` photocraft-cli made the 1024² picture from a
  throwaway profile.
- Open: an opt-in NVFP4 Fast mode (~20 % faster, visibly different samples; the owner's call),
  the edit and colour/depth control LoRAs, the JSON layout prompt in the Generate panel.

## 2026-10-29 (evening): live preview while generating (Off / Fast / Sharp)

The owner asked for both preview kinds as a setting. Branches: PhotoshopEX `feat/live-preview`
(on 40f7ad15) and genai-native `feat/live-preview-2` (`df81a61`, on eeb4eb9; the first
`feat/live-preview` holds only the protocol commit from before the speed-pass rebase and is not
used).
- Protocol: `RunRequest.preview` (left out of the JSON when off, so older workers read the same
  request), `Reply::Preview` with the RGBA picture as the blob before it, `Engine::run_previewed`,
  `EngineClient::run_previewed`.
- Worker (`qir::preview`): after a step the loop hands x0 = x − σ·v to a `StepPreview` (2.1 and
  its Upscale batch, Krea 2 and style, 2511 through its `step_i.denoised` observation, Layered
  from the CFG-denoised estimate, one picture per layer). Fast: ComfyUI's latent-to-RGB factors
  (Wan21, QwenImage21) at the latent grid, Layered's own fitted ones. Sharp: madebyollin's
  taew2_1 (TAEHV, one latent frame) and taeqi2_1 (TAESD 16×), MIT, from `vae_approx/` under the
  model root (models.md), every 3rd step; Fast without the file or without room. The pictures
  never change: byte-identical Off/Fast/Sharp on every family (benchmarks.md §0c). Fast is free;
  Sharp costs 0.15–0.2 s a run.
- Editor: `integrations.previewWhileGenerating` (default `fast`), "Preview while generating" in
  Preferences › AI Integrations (20 languages); the job keeps the latest picture with its place
  (the fill/edit/expand area, the canvas, each Upscale tile of a batch); the canvas draws it
  scaled over that place until the result lands; the Generations row shows it as a thumbnail.
- Open: MOSS-Image (branch `moss-image`) is not hooked (its loop can take the same
  `StepPreview`; taef2 if it matches flux2-vae_small); a split shows the latest layer, not the
  layers side by side; a text-to-image run that opens a new document has only the row
  thumbnail; no look at it in the running GUI yet (drawn in tests only); the ui-egui test
  `readme_tool_list_matches_tool_all` fails on the 40f7ad15 base (its README lost the tools heading).

## 2026-10-29 (evening): Upscale on fewer, larger tiles by default

The owner chose the larger tiles after the seam crops (`photocraft-tests\bench\upscale-tiles`).
On `perf/upscale-tiles`, `auto_grid` picks the fewest equal tiles of at most 1.15 MP each, never a
strip of three or more, up to 16 per side (64 MP takes 9x9). An explicit `grid` overrides it,
`tileMegapixels` asks for the old square tiles, and the result reports `grid`. The portrait, cat
and bird at 2x take 2x2 instead of six tiles: 30.3 to 24.2 s, 30.0 to 24.3 s and 29.7 to 24.8 s
(benchmarks.md §0). Recreate needs nothing: Upscale is not in the Generations library
(`is_generative` leaves it out), so no Upscale record exists to replay.

## 2026-10-29 (evening): warm up the AI models at launch; fewer Upscale tiles (the tiles prototype)

The owner chose "try both" for the two optional speedups.
- `feat/warmup` (genai-native `ae35245`, PhotoshopEX the same name). The preference
  `integrations.warmUpAtLaunch` ("Warm up AI models at launch", on by default, in all 20
  languages) makes the editor start the native worker after the first frame and load the
  **default image template's** family. That's Krea 2 by default, not 2.1: the first text to
  image uses the default, and only one family is resident. It runs `generate.engine.warmUp`
  (`wait` for scripts), a worker run with `params.warmup` that loads and generates nothing.
  - The VRAM guard is in the worker (`need_free`: the family's resident size plus 2 GB, asked
    of the driver with nothing resident; the health reports no free memory before a pipeline
    holds the context).
  - Any other run cancels a warm-up in flight. A load already under way (Pipeline::load is
    atomic) finishes first.
  - Measured on fresh workers, first text to image at 864x1152: Krea 2 **12.6 to 3.6 s**
    (the warm-up takes 6.1 s and holds 18.2 GB), 2.1 **15.3 to 3.7 s** (8.0 s, 25.0 GB).
- `perf/upscale-tiles`: an opt-in Upscale `grid` ([columns, rows]). 2x2 is 28 to 32 % faster on
  landscapes (cat 35.6 to 25.6 s, bird 34.0 to 23.2 s) and ties on the 3:4 portrait. Strips
  of three (1.5 MP tiles) take 82 to 154 s. The crops are in
  `photocraft-tests\bench\upscale-tiles`; the owner then made larger tiles the default (entry above).

## 2026-10-29 (late afternoon): Upscale in one batched request; Upscale back in the background

Branches: genai-native `perf/upscale-batch` (`dce1ce3`, on `perf/gemm-plan-5090`) and PhotoshopEX
`perf/upscale-batch` (pinned to it, on `perf/vae-decode`).
- 1A: the app sends all of Upscale's tiles in one request to the native 2.1 edit (`params.batch`,
  each tile's seed). `Pipeline::generate_batch` encodes every tile first, with the transient vision
  tower loaded once instead of per tile, then samples each one. The result is byte-identical, warm
  32.5 to 30.8 s and cold 33.8 to 31.8 s, measured in interleaved ABCCBA order with cooldowns
  (benchmarks.md §0). It falls back to tile by tile when a worker refuses the batch or answers with
  another count. Cancel and timeout are not retried. The time limit covers each tile.
- Found along the way: Upscale had always run inline (blocking the UI) because its nested
  `image.imageSize` reset `jobs.spawn`. `Session::dispatch` now restores the caller's flag. An
  audit found no other command that runs a nested command before its job. A background job also
  drops its pre-job nested journal entries on apply, so a recorded action replays Upscale once, not
  Image Size first as well. The re-apply recipe is in commit `bfa7c493`'s message, since the
  v0.17.0 merge moved those lines to 844/863.
- The sm120 GEMM plan (boundary fixed in `a84ab70`) is byte-identical but not a consistent win
  across three slots, so it stays opt-in.
- Bench gotcha: dot-sourcing `native-worker.ps1 -Stop` resets its `$Models` default in the
  caller's scope, and PowerShell names are case-insensitive, so name the caller's variable
  something else (`$modelRoot`).

Next (the owner's "try both"): `perf/upscale-tiles` (4 or 5 larger tiles, seam crops for the
owner first) and `feat/warmup` (warm up the AI models at launch).

## 2026-10-29: README around the generative features; the app icon reads "Pc"

The owner asked for "Pc" in place of "Ps" (trademark), and for a README about the generative
additions. The README now leads with the features (model and 5090 warm time each), a native
vs ComfyUI chart (`docs/images/genai/bench-5090-{light,dark}.svg`) and table from
`benchmarks.md` §0/§0b, install guides (build from source, and a prompt for a coding agent),
the models table, the hardware-tier roadmap and the editor underneath. The README calls the product
"PhotoCraftEX". The ArtCraft logo and community links are gone (`docs/brand/LICENSE-brand.txt`
asks forks to remove them). The app icon, titlebar mark and exe icon are a navy/blue "Pc" tile
(`packaging/icon-tile.py . photocraft Pc "#001E36" "#31A8FF"`); `assets/app-icon/ps/` became `pc/`.

Still open: the 3090 column (nothing measured yet); screenshots and GIFs of Select Subject and a
Qwen edit captured in the app; a published release for a one-click install. CI on GitHub can't
fetch the private genai-native, so every Rust job fails there before compiling.

## 2026-10-29 (day): the speed pass on the 5090

Branches (verified, not merged; the owner's OK pending): genai-native `perf/layered-attention`
then `perf/vae-decode` (stacked, `e9a4ea1`), PhotoshopEX the same names (pinned).
- Split into Layers on SageAttention2: 137.2 to 92.6 s warm (ComfyUI 124.8 s). Judged by
  compositing the layers back onto the input (36.91 dB, inside flash's two-seed band): the
  background layer's invented detail moves with any numeric change. Blocks 0/52/58 keep flash.
- Upscale 2x 56.3 to 30.6 s warm, 60.3 to 31.7 s cold, byte-identical: the 2.1 f32 vision tower
  gives way instead of the text encoder (which reloaded on every tile once the desktop held
  ~3.4 GB); 256 MiB driver headroom against Windows paging; `warn_if_paging`.
- 2511 on SageAttention2 too: Lightning-8 edit 8.09 to 7.25 s, the 40-step edit 94.4 to 86.8 s.
- Rejected or skipped: fp8 GEMM for Layered (no gain once activations are scaled), int8 ConvRot
  from the fp8 file (165 effective TOPS at M≈40k and a 12 s re-conversion per split), Q8_0 text
  encoder as default (Edit moves 38 dB for no gain), NVFP4 (Krea 2 9 % faster, another sample per
  seed), CUDA graphs (the GPU is busy ~97 % of a 2.1 step).
- The card runs power-capped (575 W, SW Power Cap in 98 of 105 busy samples, 61 C).
- Also on branches: `fix/model-names` (template file names, the picker's installed badges),
  `cleanup/remove-onnx` (both repositories), genai-native `fix/cuda-test-build`.

## 2026-10-29: ONNX Runtime route removed (branch `cleanup/remove-onnx`)

The owner's call: no `ort` anywhere. genai-native `cleanup/remove-onnx` deletes gn-perception's
`onnx` feature, its `ort` dependency, the ONNX modules (runtime, sam3, sam31, tokenizer,
resample), the `gn-sam` tool, `export_sam31.py`, gn-harness's `sam-ort` engine and gn-segment's
ONNX comparison example and dev-dependencies; SAM 3.1 runs on candle exactly as before. Here:
the `sam-onnx` features and `default`s of `photocraft`, `photocraft-cli` and `photocraft-genai`
are gone (the apps have no default features, so `package.ps1 -Native` drops its
`--no-default-features` with the same feature set), `runtime.ps1` loses `-Ort` / `-Cudnn` (CUDA
libraries only), comments and `dev-environment-windows.md` no longer describe ONNX Runtime. A
build without `native-diffusion` now has no SAM 3.1 backend (the worker answers ModelMissing);
before, it ran the ONNX graphs. Gates against the local genai-native worktree (cargo `--config`
patches): workspace check and clippy `-D warnings` clean, genai 14 + engine lib 1,928 pass,
`xtask layers` / `wasm` OK. Open: bump `crates/genai/Cargo.toml`'s genai-native rev to the
cleanup branch's commit once it merges (the pin here is unchanged).

## 2026-10-29: ComfyUI engine removed

The owner's call. Every generative command runs on the native worker (`photocraft --engine`);
the ComfyUI client, sidecar, route preferences, graph templates and the NVFP4/purge policies are
gone, and the tests run against a fake native worker (`photocraft_genai::fake`) on the real
gn-engine protocol (Codex K, checked). Old preference files load: retired keys are dropped, and a
`comfyPath` with no Models folder becomes that install's `models` folder once; with no Models
folder at all the Generate panel shows "Set Models Folder…". Bench scripts stay, comparison only
(those using retired preferences need `cdcedbd8`). Gates on the real GPU: engine lib 1,928 pass;
the workspace failures are the known GPU-preview ones (full_res_preview 3, pattern_preview_canvas
1) and `photocraft-gpu` timing (unchanged crate, timing under another job's load). Native 2511
has no cold glitch; empty-prompt Remove Background 5 of 5 on the GPU. Report:
[`research/notes/comfyui-removal.md`](research/notes/comfyui-removal.md).
Later the same night: Upscale 2× rerun natively 30.8 s warm against ComfyUI 0.43's 44.1 s with
the Restore LoRA on both (0.70×, `benchmarks.md` §0); genai-native pinned at `1b0b152`
(clippy clean, the eight old warnings; the Cargo.lock without the stale `patch.unused`).

## 2026-10-28 (overnight): Rust rebuild: no ONNX Runtime, the -Native zip from a clean profile

Details and every commit in `goals/rust-rebuild.md` §5; the morning summary in
`goals/rust-rebuild-morning.md`.
- **R1 done**: ONNX Runtime behind `gn-perception`'s `onnx` / PhotoshopEX's `sam-onnx` features,
  off in `package.ps1 -Native` (no `ort` in that graph). SAM 3.1 runs f16 on the GPU by default
  (`--sam-precision`; `PHOTOCRAFT_SAM_PRECISION=f32` reverts): by text on a new picture 273–278 ms
  through the app against ONNX Runtime's 249–346 ms (f32 508–515 ms).
- **R5 done**: the -Native portable zip (557 MB; `-NoMsi` without WiX; CUDA kernels `/MT` for
  the static CRT) runs all 11 generative cases from a fresh profile with ComfyUI not
  configured: no python.exe, no onnxruntime / cudnn / python / torch module in the worker.
- The Layered RoPE kernel moved from run-time NVRTC to a build-time `.cu` (the package has no
  nvrtc); gn-segment's f16 example needs `cuda`.
- Open: Remove Background with an empty prompt (2.1 matte) keeps nothing on about half the
  seeds; R6's permissive defaults (2511 on ComfyUI) wait on the owner; the split at 1 MP.
- Pitfalls: a PC crash at 23:47 (Kernel-Power 41, no driver event) with a CPU build and another
  session's GPU benchmark running; Windows PowerShell 5.1 unrolls `if` returning a one-item
  array (splat it as `@(if …)`), and turns native stderr into terminating errors under `Stop`.

## 2026-10-27 (night): Rust rebuild: SAM 3.1 in the worker, the Layered split natively

- **SAM 3.1 on candle in the worker** (genai-native `7619fd6`; PhotoshopEX `41b6d0ef` builds it
  with `native-diffusion`): the perception engine runs it whenever the checkpoint is there
  (ComfyUI's `checkpoints/`), the ONNX graphs otherwise; CLIP tokenizer from ComfyUI's
  `sd1_tokenizer`. GPU speed pass (`7eefc85`, Codex): by text on a new picture 515 ms f32 (f16
  opt-in 260 ms; ONNX Runtime 200 ms), points 9–13 ms. Candle equals ComfyUI's own f32 SAM 3.1
  on the picture where it disagreed with ONNX (`8404ffd`): the exported graphs drift.
- **Split into Layers natively** (`ec668ba` pipeline by Codex, `3056cda` serving, `7f5abb8` fp8
  on CUDA through qir's kernels): at parity with ComfyUI through the app (1–3/255, alpha IoU
  ≥ 0.994), 86 s against 58 s, so `engine = native` only for now (PhotoshopEX `29308319`).
- Models folder empty → the ComfyUI install's `models` (`8872dbcf`); `runtime.ps1` without
  ONNX Runtime and cuDNN (581 MB, `130e4b1b`).
- Pitfall: a `$env:CARGO_TARGET_DIR` set earlier in the same PowerShell command sends
  `build-native.cmd`'s build into that folder, and the old `photocraft-cli.exe` keeps running.

## 2026-10-27 (evening, later): Rust rebuild: SAM 3.1 on candle, models from ComfyUI's folder

Task file `docs/comfy/goals/rust-rebuild.md` (progress log there). Codex wrote the model ports in
its own worktrees from Claude's briefs; Claude reviewed, landed and integrated. genai-native
commits, all local:
- **R1, SAM 3.1 without ONNX Runtime**: the vision trunk and pyramids (`155aa14`), the
  interactive heads (`e0538e3`), box geometry and CLS (`45d76f3`), the detector (`181f623`), each
  against ComfyUI's own modules on the real checkpoint (1e-6 to 6e-4), then composed with one
  checkpoint read and the worker's thresholds and refinement (`987636b`): masks at IoU
  0.996–0.9999 against the ONNX route, same instances and order, on CPU (8× slower than ORT's
  CPU route there; the GPU switch is next).
- **R4, one models folder** (`58f7a15`): 2.1 and Krea 2 load from ComfyUI's `models` folder as it
  is (`qir::comfy_files`): the fused `gate_up`, the `model.` text root, the VAE's Wan names (a
  bijection on the real headers), the Qwen tokenizer assembled from ComfyUI's
  `qwen25_tokenizer` (not embedded: our 11 MB `tokenizer.json` comes from a research snapshot or
  Krea's repo; Qwen3-VL's own is a 7 MB file we do not have). Caches (prequantized DiT, Q8_0
  text GGUF built on all cores in 16 s) in `%LOCALAPPDATA%\genai-native\qir-cache`, never in
  ComfyUI's folder. `r4-live.ps1`: Krea 2 image, 2.1 image and a 2.1 edit pixel-identical to the
  snapshot route; 2.1 cold load 15 s cached (129 s before the text cache).
- **R3, Qwen-Image-Layered**: the Qwen2.5-VL 7B encoder and the files' fp8 linear (`9994e39`, ids
  exact, 9e-4) and the 60-block DiT streamed from the fp8mixed file (`9941269`, velocity 9.2e-4,
  1.1 GB RAM), on CPU. Codex on the noise, schedule and pipeline next.
- **R2, Generative Angles natively** (PhotoshopEX `da4ce52b`): the 2.1 edit with a camera
  instruction under `auto` (6 s); naming the view only turned the subject, naming the camera's
  move orbits it. Krea 2's General Edit V2 cannot move the camera.
- Open, the owner's calls: the Multiple Angles LoRA (to compare Angles with 2511) and Ostris's
  Krea 2 style-reference LoRA are not installed; embedding a Qwen tokenizer file (licence).
  ComfyUI's `int8_convrot` repacks (another Hadamard basis) are not read yet.

## 2026-10-27 (late night): swaps measured; Krea 2 edits natively

- **Swap benchmark** (`swap-cases.json`, `swap-run.ps1`, `swap-sheets.ps1`): 15 item and outfit
  swaps at two seeds, the object selected by name with SAM 3.1. Clean of 30: Krea 2 edit 27
  (items 22 of 22), 2.1 edit 27, 2.1 fill 23.5, 2511 fill 14.5 (it keeps the old object on
  items). Numbers and faults in `benchmarks.md`.
- **Krea 2 edits by instruction, natively** (genai-native `f56189b`): the community krea2_edit
  LoRAs (conradlocke's General / Identity Edit V2, 2026-10-16; downloaded on the owner's go into
  ComfyUI's `loras`, which the native folders share; personal and non-commercial licence, BYOM)
  condition on the picture as clean VAE tokens on RoPE frame 1 plus a picture-grounded prompt.
  Built from the comfyui-krea2edit nodes and the trainer's GEOMETRY.md (Apache-2.0, read; the
  repos and licence files in `_eval\krea2edit`): a Wan 2.1 VAE encoder, `Krea2Dit::forward_edit`,
  a `ref_boost` score bias in the flash kernel (GPU-tested against f32), the 4B vision tower and
  `forward_with_images_taps`. Template `krea2-turbo/edit` (the nodes' API graph for the ComfyUI
  route, where the two custom nodes are reported missing; the worker serves it when
  `krea2_general_edit_v2.safetensors` is in its loras). First run: the denim jacket to a
  yellow raincoat, person and street kept, 1.6 s encode + 9.3 s sampling at 832×1216.
- Open: the garment-longer-than-the-selection band on both edit routes (the result's layer mask
  is the selection); Krea 2 edit is not in `auto` (2.1 edit is, and is faster); Identity Edit
  V2 (person into a new scene) is downloaded but has no feature yet.

## 2026-10-27 (night): Qwen-Image-2.1 replaces 2511 under `auto`

- **The owner's decision**: 2511 is "an older model, and way slower"; not used where 2.1 runs.
  `generate_cmds::native21_auto`: with Allow Research-Only Models on and the native worker
  running the family, `auto` runs `qwen-2.1/edit` for the removals (Content-Aware Fill, Remove,
  Spot Healing; the crop at 0.75 MP as measured), `qwen-2.1/fill` for prompted Generative Fill,
  `qwen-2.1/expand`, `qwen-2.1/edit` for Generative Edit; Generate Similar of an expand too, and
  it now runs on the engine that serves its template (it used the ComfyUI route even for a
  native template). Without the opt-in the 2511 orders stay the permissive default, so the
  licence rule holds. `generate.models` reports `autoRemove` beside `autoFill` / `autoExpand` /
  `autoEdit`, and the Generate panel's Auto row names the 2.1 template.
- Still on 2511 (2.1 does not take them yet): a fill from a reference, Generative Angles, an
  image from references.
- **Evidence**: the removal numbers of 2026-10-26 (2.1 15 of 20 hard strokes at 6–28 s, 2511 16
  at 33–58 s); a `-p22` interim run had never been written up (superseded by `-p21v`). Prompted
  fills compared for the first time (`benchmarks.md`): level with 2511 on the hats, the cloud,
  the compass (cleaner); 2.1 missed the stormy sea's rowing boat; 1.6–5.8 s a case.
- Gate: engine 1930 and UI 3227 tests, clippy, wasm OK.

## 2026-10-27 (evening): GPU detection and the engine page (plan Phase 5)

- **GPU detection** (genai-native `182e897`): `qir::device::probe` asks the driver about the
  card with no context made (looks for `nvcuda.dll` first: cudarc aborts the process when the
  driver library is missing); the diffusion engine refuses a card older than the build's
  kernels (`CUDA_COMPUTE_CAP`, 12.0 here) with a sentence, lists no templates (so `auto` stays
  on ComfyUI) and says it in its health (`Health.detail`, new, defaulted for older peers).
  Missing cuBLAS / cuRAND gives the same kind of sentence (`native::Refused`).
- **The native worker in `generate.engine.status`** (`native`: state, card, total and free
  memory, loaded model folders, reason, log), asked without waiting for a running request and
  without starting a worker; `generate.engine.start / stop / restart {engine: "native"}` and a
  new `generate.engine.purge {engine}`. The worker's stderr is its log
  (`%TEMP%\photocraft-native-engine.log`, `RUST_LOG=info`: load and phase timings).
- **UI**: the status-bar dot follows the engine the commands go to (native under `native`, and
  under `auto` once the worker runs), its tooltip has both engines, its menu a section each;
  Preferences › AI Integrations › Native engine shows the same lines with Start / Stop / Purge
  models / Show log. 11 strings × 20 languages.
- **Live** (the CLI, only the runtime folder on `PATH`): stopped → a Krea 2 picture → ready,
  "NVIDIA GeForce RTX 5090", `qir+ort`, `krea2-turbo` loaded, 14.6 of 34.2 GB free → Purge →
  nothing loaded, worker kept. The log shows the DiT load 6.4 s, sampling 3.5 s.
- Also: `generate_engine_cmds.rs` was committed double-encoded (`â€º` for `›` in its error
  sentences since `467a8ae0`); repaired.
- **Open, the owner's question "do we need 2511?"** (answered from `benchmarks.md` 2026-10-26):
  its value is the Apache licence (2.1 is research-only, never a shipped default) and the hard
  removals with two candidates (14 of 20 against 2.1's 12, at 43–62 s a stroke against 10–38 s;
  with one candidate 2.1 was level or ahead, 13–14 against 12.5). The gap was mostly the 8B
  naming the person, since reworded but not re-run on 2.1. Proposed: no 2511 port now; re-run
  the removal suite on 2.1 with the new wording.
- Commits: genai-native `182e897`; PhotoshopEX `06cb0878`. Gate: the 5 known GPU-driver
  failures; clippy, wasm, layers, parity, scorecard, native genai tests OK.

## 2026-10-27 (day): Krea 2 native under `auto`; plan Phase 5 started

- **Krea 2 from ComfyUI's own fp8 files** (genai-native `6354b13`, `qir::fp8`): the worker
  reads `krea2_turbo_fp8_scaled` and `qwen3vl_4b_fp8_scaled` as they are and runs ComfyUI's
  recipe, read from `comfy/ops.py` and `comfy_kitchen` (the five fp8 linears of a block through
  cuBLASLt's FP8 GEMM with the activation cast at scale 1; the three the file marks
  `full_precision_matrix_mult` dequantized to bf16; the encoder in full precision, since ComfyUI
  only takes fp8 matmuls there to generate text). Measured on the way: the encoder on fp8
  matmuls 12 of 28; every DiT layer fp8 6 of 28; SageAttention2 costs three to four cases on
  this 8-step Turbo against exact attention.
- **Exact attention and fused small ops** (`qir::flash`, `qir::krea2_fused`): an FA2-style bf16
  kernel of our own (PV in fp16, strided BSHD inputs, grouped KV heads), and Krea 2's norms,
  modulation, RoPE, SwiGLU and gates in one pass each. Sampling 4.54 → 3.20 s (1024², 8 steps);
  `QIR_PROFILE=1` showed the ≈ 0.17 s a step candle's op chains took.
- **Result**: 19 of 28 at the gate against the app's ComfyUI route (20 against its bf16 route;
  ComfyUI's own two routes agree on 20), **4.0 s a picture through the app** against ComfyUI's
  4.0–4.7, LoRAs still applied (Anygles: the fox reposed). `krea2` joins
  `NATIVE_AUTO_FAMILIES`; the style reference stays on ComfyUI. `benchmarks.md` §8 has every
  variant.
- **Plan Phase 5** (packaging without Python): a `native-diffusion` build delay-loads cuBLAS,
  cuBLASLt and cuRAND (the editor and CLI start with no CUDA on `PATH`, checked with `dumpbin`
  and a stripped `PATH`); the worker checks for them before serving the diffusion models
  (`missing_cuda_libraries`, a sentence in its log); `packaging/windows/runtime.ps1` assembles
  the runtime folder (1.33 GB, over the 1 GB target: cuDNN is 542 MB of it) and the Krea 2 suite
  ran from that folder alone; the app takes a `runtime` folder next to the exe when the
  preference is empty (`Integrations::runtime_folders`); `package.ps1 -Native` ships it in the
  portable zip (not run end to end: WiX and signing are not set up on this PC).
- **A regression found by clippy** (`1a2a2b46`): the 0.11.1 merge had dropped the desktop app's
  `set_generations_dir`, so since 2026-10-21 the GUI's Generations library (Window ›
  Generations, the Variations strip, Recreate) was in memory only, not kept across restarts.
  Attached again; the workspace is clippy-clean (`--all-targets`).
- Commits: genai-native `6354b13`; PhotoshopEX `88127c3f` (packaging), `32685bc9` (Krea 2 under
  `auto`), `1a2a2b46` (the Generations fix, clippy). Gate: the 5 known GPU-driver failures,
  wasm, layers, parity and scorecard OK.
- **This PC**: `_eval\native-models\krea2-turbo` now points at `_eval\weights\krea2-turbo-comfy`
  (ComfyUI's files, hard-linked). The gn-weights test `the_local_comfyui_models_are_all_identified`
  fails on the Q8_0 GGUF of Qwen3-VL-4B in ComfyUI's folder (machine-dependent, not this work).
- Open: Krea 2's style reference and NVFP4 file, the rest of Phase 5 (models-folder first run,
  GPU detection sentence, engine page, clean-profile install), cuDNN's share of the runtime.

## 2026-10-27 (overnight goal): wearers, shadows, the chat cache, LoRAs, Generative Upscale

An unattended run of `docs/comfy/goals/overnight-2026-10-27.md` (its progress log has the
details and the numbers per task).

- **The wearer's candidates** (`47dc76ed`): a removal named "the man wearing the cap and mask"
  asks the detector for each worn thing, all of them and the wearer, and takes the mask that
  matches the stroke best. The mask's whole stroke on 2.1 is clean. `PHOTOCRAFT_DUMP_VIEWS`
  (bench only) saves the exact picture the rewriter sees.
- **The change band** (`0005d577`): on 2.1 (whole-picture) removals the pixels the model
  changed beside the hole join it, so a shadow or reflection the model removed goes too
  (`shadow`, on by default); a band that runs into the request's edge is refused, since it would
  cut the shadow in a straight line. Stock hard set, 2.1 two judged on the worker: **16 of 20**
  (15 the night before); robustness set 27 of 28 with and without it. The dog's long shadow
  runs past the request and stays.
- **The worker's chat cache** (genai-native `2f3cd2a`): prefill once, a K/V cache per layer,
  one row a token. A 28-token rewrite with the picture 5.6 s → 0.95 s, the removal naming
  0.8–2.1 s → 0.3–0.8 s.
- **LoRAs** (`6e6e1d6a`): `generate.loras` (files and their family from the tensor names), the
  Generate panel's LoRAs rows (native route), and the ComfyUI route applies `loras` too
  (`LoraLoaderModelOnly` after the model loader). Parity through the app: edits with
  Consistency and Restore agree as closely as without (38.5–44.7 dB); BFS head on text to
  image 22.6 dB (same person and framing, the 8B-scale text-to-image drift). The parity run
  found a worker bug, fixed in genai-native `53256a7`: a request without LoRAs after one with
  them failed (`CUDA_ERROR_INVALID_CONTEXT`: the adapters were freed on a client thread before
  the context was bound to it).
- **Generative Upscale** (`9994874a`, Edit › Generative Upscale): bicubic, then the 2.1 edit
  per ~1 MP tile with AusBoss's restore wording and the Restore LoRA, tiles realigned and
  colour-matched, strips faded. An old print halved and enlarged back: crisper than bicubic,
  the face kept, no seams; 89 s for six tiles on the worker.
- **Denoise on fills** (`72a4ec89`): the 2511 fill templates bind `denoise`, the fill page has
  the field (2511 only). On the 40-step template 0.9 keeps the old sheen with the new colour,
  0.8 barely changes; the 8-step tier hardly moves below 1.
- **The LoRA list live** (`b81db474`, `loras-live.ps1`): in the release app the folded panel
  never loaded the list; it now loads on `ui.set` and before a Generate.
- **Measured and not shipped:** the seam tone match on Generative Fill (the native 2.1 fill
  already meets the picture at its edge: a no-op on four of six fills, invisible on two); a
  wider removal request (`removeMargin` 0.5: the 2.1 edit keeps the dog's shadow itself);
  naming the shadow in the prompt (`nameShadow`: +1 on the stock set, −1 on the robustness
  set). Both parameters stay, off / at their defaults (`df01a051`).
- **The removal default, measured the same night**: 2.1 on the worker (two judged) 16 of 20 on
  the stock hard scenes, the shipped 2511 Best of two 12 of 20 (16 two nights ago: its random
  seeds swing it by four). On this set 2.1 is ahead and needs no ComfyUI; the default is the
  owner's call.
- Skipped without the owner: LaMa (a download), 2.1 Turbo (a download). Open: the dog's long
  shadow, the cup's patch on 2.1, the logo's whole stroke naming the shirt, Upscale's text
  encoder reloading per tile, `native_menu`'s order-dependent test.

## 2026-10-26 (night): the rewriter's views per model; 2.1 on the worker passes 2511

- **A naming probe** (`naming-probe.ps1` + `crates/genai/examples/chat_probe.rs`): the two
  views a removal's rewriter sees, made from each stock scene's half and whole stroke, asked
  of the running worker (or of ComfyUI with `PHOTOCRAFT_COMFY_URL`) in a second each. Wording
  and crop changes are now seconds, not a 20-minute run.
- **The views were the cause, not the wording.** For an object under ~300 px the
  "surroundings" (at least 384 px) and the "close-up" (a quarter of the grown stroke around
  it) were the same crop, so the 8B named the most salient thing in it: the man. A close-up
  with a tenth around it names the mask and the glasses; the 4B names parts from that one
  (the dog's chain, the bicycle's handlebar), so the margin follows the model that answers
  (`naming_views`: 0.1 when the worker's own 8B answers, 0.25 for ComfyUI's 4B). Wider
  surroundings made the 4B name the beard; a dimmed spotlight on the stroke made the 8B talk
  about "the selected area". Both dropped.
- **The close-up keeps its shape**: `side_by_side` held the right half to the left's width,
  which squeezed a wide close-up (the boat's hull at two thirds). **The name parser** cut "the
  black silhouette of the standing person" at "standing" (a word after an article is now part
  of the name) and cuts at "leaning".
- **Measured** (`views-run.ps1`, the ten hard scenes, half and whole, my count of clean
  removals of 20): 2.1 edit on the worker, one candidate 14 (was 13), two judged **15** (was
  12); the shipped 2511 Best of two 16 (13 on its last run, random seeds). The 2.1 worker
  route, no ComfyUI, 6–28 s a stroke, now matches or beats the shipped one on this set.
- Open: the mask's whole stroke still names "the man wearing the cap and mask" (the real
  crop's few extra pixels of face flip the 8B; the probe's nearly identical crop says "mask"
  16 times of 16); the cup's dark patch on 2.1 is the table's own shade continued, so the
  differential composite would not fix it; the dog's cast shadow stays on every route (that
  one is the differential composite's case). The chat KV cache needs the encoder's
  incremental pass with M-RoPE image positions: the names already take 1–2 s.

## 2026-10-26: removals stay on the Lightning tier under Quality best

- The user reported Spot Healing and image generation "not working" in the app with the
  server up. `app-prefs-probe.ps1` (the CLI with the app's own preferences: engine `auto`,
  the sidecar `comfyPath`, Quality at its default `best`) showed both working, the image in
  7 s and a Spot Healing dab in **98 s**: two 40-step fills (Best of two) where the bench
  strokes, all run with `-Quality balanced`, take 20–28 s. The server log had a 40-step fill
  cut by a "Global interrupt" (the user giving up). So a removal now runs at `balanced`
  whatever Quality says (`removal_quality`; `fast` stays `fast`): the 40-step tier was
  measured no better on removals and worse under the smoothed hole (`benchmarks.md` §5i,
  "The fill's own settings"). Quality still governs generations, edits and expansions. The
  classical-tools test asserts the Lightning template under the default Quality.
- **Where the rest goes** (`same-graph-probe.ps1`, `flag-probe.ps1`): of a 25 s Lightning
  fill, 6 s is sampling and 10–12 s is ComfyUI 0.39's dynamic loader re-staging the 2511
  with its LoRA patches before the first step, on every prompt, the same graph twice in a
  row included. Three launch flags measured against it, all worse (`comfyui-setup.md`'s
  table, `benchmarks.md` §5i); the session-0 tenants (`sd-server`, four pythons) came back
  during the last two and hold 5 GB again. The server runs the measured flags again.
- **The identical graph twice in a row** (`bare-twice`, no rewriter, one candidate) stages in
  1.5 s the second time and the fill takes 10 s: the re-staging is the vision-language
  model (5 GB) evicting the 2511 between dabs. **Skipping the rewriter measured and
  rejected** (`noname-run.ps1`, both sets, one candidate): without the name the half strokes
  leave the object on most scenes and the whole strokes over-take (a man replaced by a wall);
  5 s saved a stroke. `benchmarks.md` §5i.
- Open: the ComfyUI route's floor is 22–25 s a dab on this card. The way past it is a
  resident model, which means the native engine; the owner's decision (plan §15.6) has
  2511 native out of scope, so the next step is the owner's call.

## 2026-10-26 (afternoon): the 2.1 route, AusBoss's catalogue, the LoRA engine

- **Direction.** Asked for the next lever, the owner first picked the 2511 port, then asked
  whether Qwen-Image-2.1 would do (AusBoss's 2.1 workflows on Civitai) and set **Krea 2 and
  Qwen-Image-2.1 as the main models of the build**, the app being open-sourced, not sold (the
  research licence is the installer's to accept; BYOM stays). 2511 stays the permissive
  fallback; the port is off the list (plan §15.6).
- **AusBoss's whole catalogue read** (31 models through Civitai's API, the pages' own
  measurements): `research/notes/ausboss-catalogue-2026-10.md`. The concepts worth taking, in
  order: realign to source, the differential composite ("remove with its shadow"), remove to
  its own layer, the rewriter on the resident encoder, LaMa for small marks, tiled generative
  upscale with the restore recipe and per-tile colour matching, 2.1 Turbo, LoRA loading. His
  LoRAs fetched on the owner's go (Consistency, Outpaint v1/v2, Restore, Outfit Swap; AnyPaint;
  Viggle Turbo v0.3; the MIT BFS swaps), checksums in `models.md`; the Civitai workflow JSONs
  need his login (links given).
- **A worker many processes join.** The CLI spawns its own worker per process, so a bench
  stroke on the native route paid the whole cold load (368 s). `photocraft-cli --engine
  --stay` (gn-engine `serve_with(stay)`, genai-native `e91865b`) outlives its clients and
  `PHOTOCRAFT_NATIVE_ENGINE=port:token` makes `native_backend` attach to it
  (`NativeBackend::attached`); `bench/scripts/native-worker.ps1` starts and stops it.
  genai-native is consumed from the local checkout through a `[patch]` in the user-level cargo
  config (`CARGO_HOME\config.toml`) until the revision is pushed and bumped.
- **The rewriter kept off the worker** (`thinker_for`): a job's vision-language questions go
  to the backend serving the enhancer template, the ComfyUI route, while the worker serves the
  picture; with both on the worker the name was lost and the detector fell back to the point
  prompt (the first native strokes). And `release_thinker`: on the native route the ComfyUI
  server drops its models after each answer (unless purges are off), because the 5 GB
  vision model resident beside the worker's 2.1 set and the card's other tenants paged a stroke
  for eight minutes (31.5 GB in use).
- **The LoRA engine** (the owner's standing ask), in `qir::lora` with slots on both DiTs,
  `params.loras` through the worker and `loras: [{file, strength}]` on the fill, edit, image and
  removal commands: plan §9's status paragraph has the design. Tests: the side path equals the
  merged weight; both file layouts load and the fused gate-up splits as ComfyUI does; the
  worker's request parsing and path rules; the app's validation on both routes. Not yet
  measured against ComfyUI with the same file.
- **The native 2.1 removal bench** (`native21-run.ps1`) ran four times today and was stopped
  four times: once without the worker shared, once without the rewriter's name, twice with a
  second stroke of 8 minutes. The worker's log (a tracing subscriber under `RUST_LOG`, new)
  named it: the card full beside ComfyUI's vision model and the other tenants, `free_vram`
  reads 0 and the pipeline evicts the text encoder after every prompt, and **loading it again
  took 114 s** (Q8 on load; about 60 s in bf16) of a 135 s stroke, 492 s for the whole-object
  one. Fix: `qwen-image-rs prequantize-text` makes a Q8_0 GGUF of the encoder beside the
  snapshot (`text_encoder.q8_0.gguf`, the owner's weights, nothing shipped) and `gn-diffusion`
  uses it when there (genai-native `7d85a88`): a reload in seconds. Two more causes, fixed:
  the app's purge rule counted a removal's SAM run between two 2.1 fills as a model switch
  and freed the whole worker (the next fill reloaded everything: `run_switching` now never
  purges the native key, `3fcb1462`); without that accidental purge the resident detector
  beside the 2.1 working set paged for 12 minutes, so a light engine now gives the device back
  before another engine of the worker runs (`Engine::yields`, the perception engine yields;
  genai-native `937c149`). Result, two strokes on one worker: **25 s and 60 s** (the 2.1 fill
  itself 10–15 s: encode 3.7–9.7, sampling 5.1, decode 0.6), against 135 s and 492 s before.
- **The rewriter on the resident encoder** (genai-native `9e856bf`, PhotoshopEX `3999b2d4`):
  the 2.1 snapshot's text encoder is Qwen3-VL 8B Instruct with its `lm_head`, so
  `Pipeline::chat` answers as the vision-language model (chat template, the picture through
  the vision tower, sampling through the resident encoder plus the final norm and a Q8 head
  loaded on first use; no key/value cache yet). The worker serves `enhance/edit` and
  `enhance/image` with it, and a native job's rewriter and judge go to the worker
  (`thinker_for` with the job's server key; the worker is never told to unload after
  answering). ComfyUI then has no part in a native removal. Live (`chat-smoke.ps1`): sensible
  rewrites, 6.4 s for a 28-token edit instruction with the picture, 4.7 s for a 120-token image
  prompt, the head's first load 10 s. But on removals the 8B names the person rather than the
  thing under the stroke ("the man's face" for glasses): its naming needs its own pass before
  it replaces the 4B's.
- **The 2.1 removal measurements** (`benchmarks.md` §5i): the plain `qwen-2.1/fill` is worse
  than 2511 on every hard scene; the 2.1 edit route as first run (smoothed hole, 8B names) is
  unusable. 2511 stays the removal model. A poisoned CUDA context now ends the worker
  (`gn-diffusion` `562b413`) and the app tries the run once more on a fresh one
  (`retry_after_stop`); the bench restarts a shared worker that stopped.
- **LoRAs, the owner's question "did you get all of them":** AusBoss's five Qwen-Image-2.1
  files (Consistency, Outpaint v1 and v2, Restore, Outfit Swap) are in; not taken: his FLUX 3
  Klein adapters (Klein is not a model of the build) and the step checkpoints of each repo
  (alternates). Also in: yijunwang2's four Krea 2 adapters (AnyPaint, Registered Outpaint,
  Anygles, ReID), Viggle's Turbo, the MIT BFS swaps. `models.md` has them with checksums.
- **Evening: the 2.1 edit route AusBoss's way** (`benchmarks.md` §5i): the real picture in
  view (a 2.1 template never gets the smoothed hole now: `qwen21` in `fill_region`, a test),
  the crop at 0.75 MP. One candidate: 2.1 14 of 20 against 2511's 12.5. Best of two: the
  shipped 2511 14, 2.1 on the worker alone 12, at 10–38 s a stroke against 43–62. **The memory
  fix that made the native route fast**: the CUDA pool's retained memory goes back to the
  driver after each request and counts as free inside (genai-native `da60b3c`); strokes went
  from 100–420 s to 9–30 s. The rewriter's wording now names the object, not its wearer: better
  names from the 4B, the 8B still names the man on two scenes. `PHOTOCRAFT_THINKER=comfy`
  (bench only) keeps the questions on ComfyUI.
- Open, in order: the 8B's naming of worn things (or the 4B ported for the worker); the cup's
  smudge on 2.1 (the differential composite, catalogue §2.2); a key/value cache for `chat`.
- Also taken from the catalogue today: **realign to source** (`generate_realign`, on edits and
  every 2.1 template: the result moved back onto the picture by the translation that matches
  it outside the mask) and **`keepObject`** on the removal commands (the removed thing as its
  own masked layer above the fill, the result's `objectLayer`).

## 2026-10-22: the removal adapter, installed and measured

- **Downloaded on the user's go**: `Qwen-Image-Edit-2511-Object-Remover-v2-9200.safetensors`
  (225 MB, Apache-2.0, sha256 verified against the Hugging Face manifest) into the portable
  install's `models/loras` (not `C:\Users\5090\ComfyUI\models`, which does not exist: the
  server's `argv` in `/system_stats` gives the real root). New template
  `qwen-edit-2511/remove-lightning-8`: the Lightning fill graph with a second
  `LoraLoaderModelOnly`, `highlight: "red"` in its meta; the engine paints the hole red
  (`highlight_covered`) and sends the trigger sentence. Template meta gained `highlight`;
  the ComfyUI client binds `sampler`/`scheduler` to the official pair when a request leaves
  them out.
- **As the named template** on both sets: removes what the plain fill cannot (the fresco's
  earring, the tattoo, the bicycle, the Liddell print, the umbrella, the beach walkers) and
  leaves red in the hole wherever the hole was grown far past the object (every 96-px hole
  on the generated set, the glass, the motorcycle). `benchmarks.md` §5i, "The removal adapter".
- **Shipped path**: under `auto` with two variations a removal now runs the adapter on the
  selection grown 6 px (`HIGHLIGHT_GROW`) as the first variation and the smoothed plain fill
  as the second; the judge picks. First round (`auto2-run.ps1`): a wash in count, the adapter
  winning the tattoo, the Liddell print, the bicycle, the umbrella and losing where the judge
  kept a crow, a boat and a red-tinted glass as "natural". So the judge is told what was to be
  removed (`judge_question` with the rewriter's name; the system turn says a version where it
  is still there loses, and flat paint is a flaw), and an adapter result with saturated red on
  more than 5 % of its hole is out before judging (`paint_left`). Second round
  (`auto3-run.ps1`): 63 of 76 against the smoothed-only 61, the generated set 28 of 28, the
  stock set a draw in count (whole 17 against 19, half 18 against 16) and a win in kind (the
  tattoo, the fresco's earring, the Liddell print, the umbrella's head, no red anywhere);
  the judge still kept transparent-framed glasses it was told to remove. Ships as the
  default. `benchmarks.md` §5i, "The judged pair".
- Open: the blurred-face seed on the daguerreotype; the detector asked by the adapter's own
  name convention; FLUX.2 klein 4B with fal's adapter as a second family (`models.md`).

## 2026-10-23 (next steps, continued): generative steps in `photocraft-cli batch`

- The GPU is still held by the two SSH tenants (8.7 GB with ComfyUI stopped), so the clean
  re-measure of the purge policies waits; the roadmap's offline-testable item instead:
  **Batch** (Phase 5). `photocraft-cli batch` already waited for a generative step's job
  (`Headless::command_run` runs `Session::execute`); what was missing was the Generations
  library hookup `run` has (`generations_dir_for`, now shared, looking through the played
  set's steps too) and a test: an action `[prefs.set {comfyServer, engine, allowResearchModels},
  generate.removeBackground]` over a folder of two PNGs against the fake server, both outputs
  transparent outside the matte, one run per file. The CLI gained the fake-server
  dev-dependency for it. Roadmap Phase 5: Batch ✅.
- Recipe, for the record: an actions file with a `prefs.set` step naming the server, then any
  `generate.*` / `paint.spotHealing {type: generative}` / `paint.remove {generative}` step,
  then `photocraft-cli batch --actions it.json --in photos --out done --format png`.

## 2026-10-23 (next steps): the purge policy, Best of two, and the driver resets explained

- **Faster removal on ComfyUI** (the user's pick of the four directions): a stroke's time
  went on the server purging and reloading between the detector, the fill and the fill
  again. `integrations.purgeModels` (`switch` | `heavy` | `never`, Preferences › AI
  Integrations › Unload models between tasks; `generate_cmds::purge_mode`, threaded through
  every `run_switching` call: fills, image, matte, split, the detector): `heavy` purges only
  when the diffusion model or checkpoint differs, so the adapter and the plain fill, sharing
  the 20 GB 2511, no longer purge between them. Default `heavy`. `purge-run.ps1` timed the
  three (`benchmarks.md` §5i): 8 % off a stroke for `heavy`, 10 % for `never`.
- **Best of two** on the Remove tool's and Spot Healing's options bars (`remove_pick_best`,
  on; off = one candidate, `variations: 1`, about half the time). Two strings ×20; the
  preference's label and choices ×20.
- **The driver resets, corrected**: `nvidia-smi` with ComfyUI stopped showed 8.4 GB of the
  card held by two session-0 tenants from SSH logins to this PC: an `sd-server.exe` (4.5 GB,
  since 22:26 on the 22nd, 48 minutes before the first reset) and a python tree under
  `sshd-session.exe` (2.5 GB, since 00:35). With them, 2511 and its encoder no longer fit,
  every run loaded partially, a stroke took 48–77 s instead of 20–28, and the driver gave
  out. The presence check and the judge's picture size were correlates, not the cause; the
  smaller judge pair stays (it costs nothing). Not killable from the user session; reported
  to the user.
- Open: re-measure the purge policies and the third round on a clean card.

## 2026-10-23: the judge's view enlarged; the driver reset under model churn

- **The judge on thin things**: the judged crop is now enlarged to at least 384 px on its
  shorter side (`judge_view`) before the side-by-side ask, and the user's "turn off the other
  stuff" is honoured: ComfyUI runs only for a measurement (`judged-run.ps1` starts it with its
  output logged and stops it after).
- **Tried and gated off**: asking each result on its own whether the named object is still
  there (`still_there`, `PRESENCE_CHECK = false`). It got the old man's glasses off in the 18
  strokes that ran, but its two extra loads of the 5 GB vision-language encoder per stroke,
  on top of the rewriter's and the judge's, had the **GPU driver reset** twice in two runs
  (System log `nvlddmkm` 153, the server killed mid-answer, 2026-10-22 23:14 and 2026-10-23
  00:05, no such event during the earlier hours-long runs). The bench driver now restarts
  the server and retries a stroke that found it gone, and counts the restarts.
- Bench scripts: `-Base` on `robust-sheets.ps1`; `judged-run.ps1`; `one-stroke.ps1`;
  stroke errors reach the log (the native call in its own function, stderr merged at the
  call: after `--%` a redirection is an argument to the CLI).
- **The driver reset again** with the presence check off (00:30, 00:34): the remaining
  change was the judge's enlarged pair, up to a megapixel for the vision-language model with
  the 20 GB fill model resident. The judge view is 320 px on the shorter side now, 512 at
  most on the longer, the pair under half a megapixel (`JUDGE_PAIR_MAX_PIXELS`); the GPU
  itself was fine (49 °C, no ECC errors). The rerun: 78 of 78, no restart, no driver event.
- **Third round** (`benchmarks.md` §5i): 64 of 76 against the smoothed-only 61 and round
  two's 63; stock whole strokes 21 of 24 (the best yet: the tattoo, the fresco earring, the
  bicycle, the umbrella's head won; the glasses' eye band smeared, the museum hall blurred),
  half 17, generated 26 (a speck on the wire, marks on the gingham). Ships.
- Open: the judge choosing a smeared band over clean eyes on the old man; a half stroke that
  covers under a tenth of a big object (bench, motorcycle) stays a dab by design; the
  mislocated heart sign is the test's, not the tool's.

## 2026-10-21 (late night): the fill's own settings, measured

- **"The Qwen settings can be refined"**: the fill templates run ComfyUI's official 2511
  pair (euler/simple, shift 3.1, CFGNorm, Lightning 8 at CFG 1 or 40 at CFG 4) at 0.75 MP.
  `generate.fill` and the removal commands take `sampler`, `scheduler` and `maxPixels` now
  (`Tuning`, bound into the three plain fill templates), and `tune-run.ps1` measured eight
  whole-object strokes under 1 MP, 1.5 MP, `res_multistep`, `beta`, and the 40-step tier at
  1 MP (`benchmarks.md` §5i, "The fill's own settings"). No sampler, scheduler or step count
  gave a consistent gain; `res_multistep` and the 40-step tier were worse (a kite invented, a
  face erased). 1 MP gave a small consistent gain for 15 % more time with the server still
  resident, so the request cap is 1 MP now; 1.5 MP put a seam in a lawn.
- Open: the realism ceiling on a removal is the model continuing from the edge; the
  removal-trained 2511 LoRA (`models.md`) is the untried lever.

## 2026-10-21 (night): the hole smoothed over, and what a stronger fill model would be

- **"50 % unusable is unacceptable"**: three model-side changes measured on the nine failing
  stock scenes gave the same nine outcomes (the 40-step tier; Qwen-Image-2.1's fill, which won
  only the motorcycle; the whole-crop edit instruction with no noise mask, the user's
  suggestion). An edit model copies what its reference shows, however it is asked. **Smoothing
  the hole over in the picture the model sees** (`smooth_covered`: a Poisson membrane from the
  hole's edge, at request size, `prefill` on the three removal commands) takes the object out
  of the reference, and the glasses, the tiara, the beach walker, the kite and the motorcycle
  went. Both sets again: stock half 13 → 16 of 24, whole 14 → 19 of 24, the generated set
  26 of 28 either way (`benchmarks.md` §5i). `prefill` is on by default now.
- **Stronger fill models** (searched 2026-10-21, `models.md` to follow): removal-trained
  Apache-2.0 LoRAs on 2511 itself (`prithivMLmods/QIE-2511-Object-Remover-v2`, 225 MB; the
  object painted red, "Remove the red highlighted object from the scene"), FLUX.2 [klein] 4B
  (Apache-2.0, ~13 GB, 2 s at 4 steps) with fal's Apache-2.0 object-removal LoRA on the same
  convention, FLUX.1 Fill dev (non-commercial), LaMa (Apache-2.0, fails on large objects). The
  2511 LoRA is the smallest change and awaits the user's go on the download.
- Bench scripts: `-Extra key=value;…` (quotes do not survive the outer shell), `-Template`,
  `-Compare`, `smooth-run.ps1`; two PowerShell traps found and documented in the scripts
  (a [string] parameter shares its name, case ignored, with the variable the parsed set or
  list is put in, and turns it back into a string).

## 2026-10-21 (the stock set): real photographs, and the generated set's bias

- **Why**: the generated set is made by a model of the same family as the fill model, so its
  scenes look the way the fill model expects. Twenty-six public-domain and CC0 photographs
  from Wikimedia Commons (`stock-fetch.ps1`, licences read from each file's metadata, sources
  in `bench\stock\st-sources.txt`; the user approved the downloads), objects located by Select
  by Text (`stock-objects.ps1`), the same two strokes (`robust-strokes.ps1 -Dir … -Prefix st`).
- **Result on the Lightning tier**: 27 of 45 judged strokes clean, against 26 of 28 on the
  generated set (`benchmarks.md` §5i, "The stock set"). Clean: a mask, a ring, a tattoo, a
  latte cup, a dog, a lamp post, a bicycle on cobbles, a cone, a top hat, a trash can, a cow,
  a tiny bird in a flock. Failed: transparent-framed glasses on an old man, a tiara in a
  stereograph, a heart sign at night, two people on a beach, the Liddell sisters' print, the
  Wall St sign, a museum motorcycle; one of two for a boat silhouette, an umbrella, a kite.
- **Two failure kinds**: (1) the rewriter or detector losing the object on old framed prints
  (the frame named instead of the tiara, "the selected object", no name for the fresco's
  earring): names that are a frame, border, "object" or "selected" are rejected now, so the
  generic path runs; (2) the eight-step fill model leaving, redrawing or blurring on grainy and
  busy real scenes, which no amount of masking fixes. The 40-step tier on the nine failing
  scenes gave the same result on every one at five times the time: the step count is not the
  lever, the model is. Lightning stays the default.
- The bench scripts take `-Quality`, `-Suffix` and `-Kinds` now, so a tier can be compared on
  the same strokes. The photographs' sources are tracked in `bench/stock-sources.txt`.
- Open: the real-photo ceiling is the fill model (2511 at its working size); the native Krea 2
  port with AnyPaint's inpainting recipe (plan §9) is the next thing to measure on this set;
  Select by Text's object choice for the bench (a 66-px "poster" beside a heart sign) is the
  test's weakness, not the tool's, which a user's stroke does not have.

## 2026-10-21 (the robustness set): fourteen scenes, six rounds, what held up

- **The set** (`robust-make.ps1`, the app's own text-to-image, 1024², seed 31): a watch on a
  wrist, glasses, a hoop earring, a rose tattoo, a shirt logo, a parked car, a distant person
  on a beach, a lamp post, a street sign on bricks, a crow on a wire, a mug on a checkered
  cloth, a diamond ring, a poster on a brick wall, a dog on a lawn. Two strokes a scene
  through the tools' own commands (`robust-strokes.ps1`): Spot Healing along the first half
  of the object's long axis (the hard case), the Remove tool in a serpentine over its whole
  box. Contact sheets by `robust-sheets.ps1`; every run's rewrite, growth and time in
  `bench\rb-log.txt`. `benchmarks.md` §5i has the round-by-round tables.
- **What the rounds found and fixed**, in order:
  1. The "quarter of the object" rule rejected a half stroke over any thick object (22 % of
     the watch): now a tenth. The rewriter, seeing one crop and no selection, named the part
     under the stroke ("the window" of the car, "the dog's head", "the hand" for a ring).
  2. Outlining the stroke in green for the rewriter made it talk about the outline ("the green
     circle"). Dropped. The Remove tool's serpentine on a logo or on glasses had the point
     prompt return the shirt or the face, which the stroke covered enough of: the point prompt
     is now the fallback only, for an object no bigger than the stroke's box and a half.
  3. The rewriter now sees two views side by side (the surroundings, a close-up of the painted
     part) and names the whole object; the detector is asked by that name, then by its last
     two words, then its last word; an object's enclosed gaps go with it (the coffee inside the
     cup's rim). All fourteen named right.
  4. The rewriter's "what lies behind" was wrong often enough ("the beige wall" behind glasses,
     "light blue sky" for a grey one, "the wooden table" over a wrist) and the model painted
     it. The rewriter now gives the name alone; the fill's wording is the measured generic one
     with the name in it. A crow taken off the sky left a lighter patch: the result's tone is
     now matched to the picture at the hole's edge (`tone_matched`, seamless cloning).
  5. The fill model's own guess is a coin flip on the hard ones (wood or skin for the watch,
     a blur over the eyes, a second wire): a removal now runs **two seeds** and keeps the one
     whose fill matches its edge (`edge_mismatch`: colour and texture), the other a hidden
     variation (`variations` 1–4 on the three commands, default 2). The rewriter named "the
     painted-over area" once: names that are the selection are rejected.
  6. The edge score preferred a smeared skin-coloured blob over a man's eyes to the eyes (its
     colour and smoothness matched the cheeks): the vision-language model now **judges the two
     results side by side** ("which looks like an untouched photograph", `judge_pair`), the
     edge score the fallback. It picked the eyes over the blob and the sky over the second wire.
- **Where it stands**: the last run of the set is in §5i; a stroke is 20–24 s wall on the
  Lightning tier (name, detector, two fills, judge; the native engine will keep the three
  models resident). `seed` on the generative strokes now sets the fill's seed too, for
  reproducible benches.
- Open: the Remove tool's serpentine makes a rectangular hole (its `closeLoops`) that a real
  stroke would not; a third seed when the judge is unsure; the same judge for Generative
  Fill's own variations; faces are the fragile case (a blur or mosaic over the eyes at some
  seeds).

## 2026-10-21 (later still): the detector asked again by the name the rewriter found

- **Closed the open item**: a stroke over half a thin necklace now removes the whole of it.
  `generative_remove` runs the rewriter before the request (with the picture around the hole
  in view), `named_object` takes the object's name out of its sentence ("Remove the gold
  necklace and replace it…" → "gold necklace"; up to six words, or nothing), the detector is
  asked by that name through the text template (`ObjectUnderStroke::find_by_text`) and every
  instance the stroke covers a quarter of joins the hole; the fill then takes the rewritten
  prompt as is (`FillJob::rewritten`, so the layer still remembers it as the rewrite). Point
  prompt first, name second: the point ask is what a compact object needs, the name ask what
  a thin one needs. Tests: the fake names "the red hat", the name ask widens the hole, the
  graph carries the name; `named_object` on six sentences.
- **Tried and dropped**: a rewriter note of its own for removals (name precisely, then only
  what is behind), against one boat run in seven that put a rock where the boat was. It named
  the pendant instead of the necklace and the half-necklace stroke regressed; the fill note
  stays (`benchmarks.md` §5i, "Asked again by name").
- Through the tools: the half-necklace stroke, the pendant dab, the Remove tool along the
  chain and the boat's three strokes all clean; 14–17 s wall a stroke on a fresh server.
- Open: a one-in-seven hallucination at the seed (the Generations panel's retry covers it;
  a second variation by default would double the time); the rewriter's wording lists the
  scene's neighbours, and a removal-specific note has to be found that keeps the whole-object
  naming.

## 2026-10-21 (late): a necklace, and the rewriter names what a stroke removes

- **The test**: "removing a necklace or something like that". A portrait with a 6-px gold chain
  and a pendant, made by the app's own text-to-image (`necklace-make.ps1`). Unnamed ("the
  object in the selected area"), the 2511 fill left or redrew the chain at half the seeds
  however far the hole was grown, even 123 px: the portrait prior wins. Named by the existing
  vision-language rewriter (`enhance`; "Remove the gold necklace and replace it with the
  surrounding skin, collarbone, and dark grey fabric…") every run was clean, and the boat
  then goes with the hole grown only 15 (`benchmarks.md` §5i, the necklace tables).
- **What changed**: removals run through the rewriter by default (`enhance`, default true,
  on `edit.contentAwareFill {generative}`, `paint.remove`, `paint.spotHealing`; the layer
  remembers the rewritten instruction); the growth is 15 % of the hole's larger side, 16–96
  px (was 30 %, 32–192); a removal's request is at least 384 px a side, since a 40-px dab's
  108-px request sent at 512 came back as a lighter, smoother patch of fabric.
- **Through the tools**: the Remove tool along the whole chain is clean; a dab on the pendant
  removes it and joins the chain; Spot Healing along half the chain removes that half with a
  loose end, because SAM's point prompt returns only the brushed half of a thin chain (the
  "whole object" rule can only be as whole as the detector's answer). 23–25 s wall a stroke on
  Lightning (detector, rewriter, fill, each loaded in turn).
- Open: ask the detector again by text with the name the rewriter found ("gold necklace"), so
  a thin object goes whole from half a stroke; the 20 s of model loading between the three
  models (the native engine keeps them resident); more pictures.

## 2026-10-21 (night): a generative stroke removes the object whole

- **The report**: the generative Spot Healing Brush "just recreates the selected item slightly
  different". Reproduced through the tool's own command on the boat (`spot-generative-live.ps1`):
  a stroke over the middle of the boat came back as a redrawn, ribbed boat; a stroke over the
  whole boat left a smudge of its reflection. 2511 keeps whatever of an object stays in view,
  and an object whose hole hugs it.
- **Tried and dropped**: pre-filling the hole classically (PatchMatch) before the model, the
  LaMa-then-diffuse trick. Worse here: the model keeps the patch fill's copied rocks as a smear
  (`benchmarks.md` §5i). Code removed, the measurement kept.
- **What fixed it** (`generate_cmds::generative_remove`, measured step by step in §5i):
  the detector (SAM 3.1, the point template, the stroke's spots as the Quick Selection tool
  sends them: `select_ml_cmds::object_under_stroke`) finds the object under the stroke and
  the hole takes it whole when the stroke covers ≥ 25 % of it and it is < 40 % of the canvas
  (a dab on a blemish must not remove the face under it); the hole is all-or-nothing wherever
  the brush touched (a soft brush's partial coverage kept the boat's rim); the hole grows by
  30 % of its larger side, 32–192 px (grown 15 the boat stayed at both seeds, 30 at one, 45
  and 60 gone at both: `caf-grow-live.ps1`; the shape does not matter: `caf-shape-live.ps1`).
  `wholeObject` (default true) on `paint.spotHealing` and `paint.remove` turns the detector
  ask off. Content-Aware Fill's generative switch takes the growth, not the detector.
  The three strokes are clean now, 20–22 s wall each on Lightning (the server switches
  between SAM and 2511). Test: the fake "finds" an object; a stroke across it grows by 45
  and the request takes it all in; a dab on it grows by 32 and stays near the dab;
  `wholeObject: false` never asks.
- Open: the Remove tool's classical `closeLoops` is not applied to the detector mask (a
  stroke around an object still removes the enclosed area through the classical hole first);
  a tap on a soft-edged object's shadow; the 20 s of model switching, which the native engine
  can avoid by keeping both models resident. The app's binary for this is in `target-app`
  (the running app pinned `target\release\photocraft.exe` during the build).

## 2026-10-21 (evening): Content-Aware Fill, the Remove tool and Spot Healing through the model

- **The ask**: "content aware fill that works for anything, including complex patterns and
  textures". Upstream's Content-Aware Fill is already PatchMatch with the Wexler vote, Image
  Melding's colour adaptation and the Photoshop-measured sampling window; the Remove tool is
  Newson's non-local completion with texture features; Spot Healing has Proximity Match and
  Efros–Freeman texture synthesis (`crates/algo/{content_aware,inpaint,nonlocal,remove}.rs`).
  Patch copying has a ceiling: it cannot invent what a pattern does behind an object. The
  diffusion fill can, so the three tools got a **generative switch**.
- **What landed**: `generate_cmds::generative_remove` (the hole, widened by `grow`, as a
  Generative Fill with `REMOVE_PROMPT`, a soft edge, the `auto` fill template; a masked layer
  above the active one) behind `edit.contentAwareFill {generative}`, `paint.remove
  {generative}` and `paint.spotHealing {type: generative}`; `generative_configured` (a server
  URL or the native folders) decides, and the classical fill runs as before otherwise. UI: a
  **Generative** checkbox on the Remove tool's options bar (on by default; `remove_generative`,
  in Reset Tool), a **Generative** type on Spot Healing's, a **Generative** checkbox in the
  Content-Aware Fill workspace (the preview stays classical and says so; OK runs the model).
  Three strings ×20. Tests: the three tools against the fake (a layer each, the instruction, the
  pixels kept), then the fallback with the server cleared (in place, nothing asked).
- **Measured on the lighthouse's boat** (`benchmarks.md` §5i, `caf-generative-live.ps1` and
  `caf-matrix-live.ps1`): the first version flattened the hole to grey, as the reference swap
  does, and **2511 painted the grey rectangle back as a panel** (40 steps and Lightning, plain
  and mask-guided alike; the Remove tool's smaller organic hole happened to work). With the
  pixels kept, the generic "Remove the object in the selected area and fill its place with the
  surrounding scene continued seamlessly…" removes the boat and continues the sea in **11.6 s**
  on Lightning (56 s on the 40-step tier); naming the object ("the red boat") does no better,
  the guided template matches it at twice the time. So `REMOVE_PROMPT` is that wording and
  `erase` is off for removals (still a param). The classical fills: 0.4 s, boat gone, rock
  copies in the water.
- Open: more pictures and seeds; a "next meaning" cycle; the stitch tone match; whether the
  Lightning tier should be the removal default under Quality `best` (56 s is long for a
  retouch stroke); the native 2.1 fill under `auto` serves the same path once its folders are
  set (not timed here).

## 2026-10-21 (later): Generative Fill from a reference, learnt from AusBoss's Krea 2 clothing workflow

- **The study** (`docs/comfy/research/notes/ausboss-krea2-clothing-workflow.md`): the owner's
  `krea2ClothingFrom_v12.json` read node by node with the AusBoss help pages and the AnyPaint
  model card. What makes it work: SAM 3.1 picks the clothes; the mask is grown, hole-filled
  and softened with every pixel number scaled by picture size; the crop takes the mask's box
  ×2 plus context to 1 MP; **the picked area is flattened to mid-grey** so the model paints a
  hole instead of copying the old garment; the reference is **stitched beside the hole** and
  the noise mask covers the hole's half only (an in-context pair for a text-to-image model);
  **the Krea 2 encoder itself writes the prompt** from the pair ("describe the finished
  picture… the clothing exactly as the right photo shows it… never call anything gray"); the
  AnyPaint LoRA plus a model patch register a ≤ 384-px reference over the canvas with its KV
  computed once per run; stitch back with a tone match. The LoRAs are Krea 2 Community
  License and the nodes are not installed: nothing of it was run.
- **Taken into PhotoshopEX the same day, on the Apache-2.0 2511 route:**
  `qwen-edit-2511/fill-ref-lightning-8` and `fill-ref` (one or two references on `image2`/
  `image3` of the fill graph; `eraseSelection: true`; a wrapper that names the grey hole and
  says "exactly as the references show it"); `generate.fill` takes `references[]` (Auto
  switches to the reference template; Quality `best` → the 40-step one), `grow` (px, `dilate`
  of the coverage; the result's mask grows with it) and `erase` (the covered pixels of the
  request flattened to grey, `erase_covered`; default from the template); the enhancer for a
  fill with references sees the grey-holed picture and the first reference **side by side**
  (`side_by_side`, `EnhanceKind::FillRef`) and names the thing as the reference shows it; the
  Generate panel's fill page has the reference deck, its estimate counts references, and Auto
  names the reference template once a reference is on the deck. `GenerativeInfo` remembers
  `grow`/`erase`, so Generate Similar repeats them. Tests: `generate_refs_tests` (the switch,
  the grey request, the mask grown, `erase: false`, Similar with the reference; the count
  checks), `generate_panel_tests` (the fill deck and request), the template parse. No new UI
  strings.
- **Live** (`benchmarks.md` §5h, `fill-ref-live.ps1`): the cardigan became the reference's
  red cable-knit pullover on both tiers (Lightning 28 s, 40 steps 150 s), while the plain fill
  with the same words only recoloured the cardigan and kept its buttons. The silhouette is the
  selection's: the V neck stayed because the white top inside it was not selected (select what
  the new piece covers, `grow` for the rest). The rewriter's phrase for "the red sweater":
  "red cable-knit sweater, crew neck, long sleeves, ribbed cuffs and hem, loose fit, textured
  pattern running down front and arms, worn over white top." (The first pass of the script
  printed `generate.info`'s `enhanced` from the wrong key; fixed.)
- **The mask stack on the tool (the same evening).** `select.byPoint` / `byText` / `subjectML`
  take `grow` (−256..256 px, negative shrinks: `sel::expand` / `contract`), `fillHoles` (a
  flood from the canvas edge; what the mask encloses becomes selected, `fill_enclosed`) and
  `soften` (0..64, `sel::feather`), applied in that order after the edge refinement
  (`finish_coverage`, on the probe path too). The Object Selection options bar gets **Widen**,
  **Fill Holes** and **Soften** (`object_widen` / `object_fill_holes` / `object_soften`, in
  Reset Tool; `object_select::mask_stack` adds only what is set, so a plain click sends what it
  did); the Generate panel's fill page gets a **Widen** field (`generatePanel.grow`, the fill's
  `grow`). Six strings ×20 ("Grow" was taken by Photoshop's Select › Grow, hence "Widen";
  Russian lacked "Soften"). Tests: the engine's bounds after grow ±4, the feathered edge, the
  ring with a notch for `fill_enclosed`, the parameter checks; the tool's query and widened
  bounds; the panel's `grow`. The `retouch_ui` and `object_finder` test helpers now clear the
  server URL: with ComfyUI running on this PC their Object Selection tests had turned into SAM
  runs (the same trap `panic_hunt` hit on 2026-10-12).
- **Still to take** (the note's §4): a "next meaning" cycle on the tool; a stitch tone match; a
  hard sampling mask separate from the feathered layer mask; Krea 2 inpainting natively by the
  AnyPaint recipe (added to `plan-native-engine.md` §9; the LoRA stays BYOM); size-scaled widen
  and blend defaults; more seeds for the measurements.

## 2026-10-21: upstream merged (PhotoCraft 0.5.0 → 0.11.1, 2411 commits)

- **The merge.** `git merge --no-commit --no-ff upstream/main` at `b097800e` (storytold/photocraft,
  2026-10-20): 1449 files, 42 in conflict. Conflicts were resolved hunk by hunk with two
  scratchpad scripts (`hunks.py` dumps every hunk as ours/theirs, `resolve.py <file> ours|theirs|
  both|theirs-ours…` applies one decision per hunk); the fork's rule held: upstream's version
  of a shared file, the fork's lines re-added where they sat. The manual fix-ups after the
  markers were gone are the interesting part:
  - **Upstream APIs that moved under the fork's code:** file dialogs are asynchronous now
    (`Services::pick_open/pick_open_paths/pick_save` are gone; `ask_file`/`pick_save`/
    `pick_file_bytes` answer on a later frame) → the Generate panel's File…, the Generations
    panel's Save (`export_many` runs in the dialog's continuation) and the hub's "open a
    picture first" (`hub.pending` waits while the dialog is open, `file_dialog_open()`);
    `layer_thumb` returns `(TextureId, Rect)`; `jobs_ui::cancel` returns bool;
    `reveal_command` returns `Result`; `ToolOptions.sample_all_layers` became
    `sample_all.<tool>`; `after_command` takes `nested_from`; a missing layer is explained
    (`UnknownLayer`); `select.object` checks for pixels under the sampled layers (the fork's
    test paints on an empty layer: `sampleAllLayers`); upstream now has its own Object
    Finder (#4087): the fork's `object_select::finish` keeps the SAM 3.1 route and falls back
    to upstream's classical click/dab/rectangle when no server answers (`classical_finish`);
    `inspect`'s `json!` grew past serde_json's recursion limit → the fork's keys are inserted
    after the macro; `UI_COMMANDS` is `shell_commands::SHELL_COMMANDS` (the Generate /
    Generations toggles and Help › Generative Features… live there now); `dock::Group::ALL` has
    nine groups (upstream's Brushes); workspace presets: Essentials shows Generate and
    Generations, the focused presets hide them (`apply_workspace`).
  - **Upstream tests the fork changes:** the Navigator-preview and Properties-column layout
    tests measure with the two generative groups hidden (they squeeze the dock at 1000 pt);
    the ⌥+wheel zoom tests set Zoom with Scroll Wheel off (the fork's default is on); the
    Space-reposition snap test leaves out Object Selection (the fork never snaps its box,
    `snap_ui::is_point_tool`); the SVG icon test skips the vector-mask check when the icon
    has no clip path (PhotoshopEX's "Px" tile); the workspace-presets table names the fork's
    Essentials. Each carries a one-line comment. A detached worktree of pristine
    `upstream/main` with its own `CARGO_TARGET_DIR` settled which failures were the merge's
    and which the machine's (the ⌥+wheel, snap, SVG-icon and CLI font-warning tests pass on
    upstream, so those were ours to fix; the GPU ones fail there too).
  - **i18n.** Upstream has 20 languages and all count as complete: the fork's ~190 strings
    were added in Ukrainian, Serbian (Latin), Greek, Hungarian, Dutch, Turkish and Vietnamese
    (`i18n_fix.py` in the scratchpad dedupes a catalog and appends the rows a language
    lacks), the Select by Text dialog's labels (Instance, Intersect, Prompt, Template) in all
    twenty, and 60-odd rows upstream added that the fork already had were deduplicated
    (`bundled_catalogs_are_consistent`). The MCP `ui_set` description names the fork's fields
    again (#1548's guard).
- **Gates.** `cargo check --workspace --tests` clean; targeted suites (ui-egui generative,
  object selection, dock, presets, i18n, engine generate, automation mcp, cli, io svg, xtask)
  green; `cargo xtask layers`, `parity` (`docs/parity-checklist.md` regenerated),
  `scorecard` (`docs/scorecard.md`: settings that do nothing 26 of 194); `cargo xtask wasm`
  green. **Full workspace run: 9838 passed, 5 failed, 83 ignored**; the five are this
  machine's GPU family (`gpu --test timing`, `full_res_preview` ×3, `pattern_preview_canvas`),
  not the merge's: all five fail the same way on the pristine `upstream/main` worktree (the
  new baseline on this PC; the old one's 7 egui-wgpu canvas tests and the eyedropper test
  pass now). The release build of the app could not replace
  `target\release\photocraft.exe` while the pinned app was running (os error 5): rebuild it
  after closing the app; a build into a separate target dir checked that it compiles.
- **What upstream brought that matters here** (worth a read in their `docs/`): async file
  dialogs, the Brushes dock group, the Object Finder, slices and Save for Web, Camera Raw,
  a 20-language UI, the MCP server's `ui_set` schema, `shell_commands.rs` as the one table
  of UI commands, workspace presets per layout, many Photoshop-parity items (parity checklist
  now 642 menu items, all live). Nothing generative; `docs/roadmap.md` still defers it (#41).
- Open after the merge: the native-diffusion feature build (`nvcc`) was not rebuilt this
  session (the genai crate compiles without the feature; the worker is a separate process);
  Krea 2 under `auto` (fp8 GEMM), style reference, LoRAs, task-bar previews, as before.

## 2026-10-20 (night): plan Phase 4, Krea 2 Turbo native

- **Weights.** Krea 2 Turbo and its Qwen3-VL-4B encoder exist on this machine only as
  ComfyUI's `fp8_scaled` repacks (`comfy_quant` float8_e4m3fn, one `weight_scale` per linear).
  `qwen-image-rs convert-fp8` rewrites them as bf16 safetensors tensor by tensor (streaming
  writer; 12 GB in 21 s, 5 GB in 7 s), the rest copied, the markers dropped. The weights
  folder `_eval\weights\krea2-turbo\{tokenizer,text_encoder,transformer,vae}` is a junction
  under `_eval\native-models` for the worker (`<models>/krea2-turbo/`). BYOM stays: nothing
  ships, the converter is the user's.
- **The architecture, read from ComfyUI's model code and stable-diffusion.cpp's `krea2.hpp`
  (MIT), each piece checked against ComfyUI's running objects:** the **encoder** is the 4B's
  language model (width 2560, 36 layers, 32/8 heads, SwiGLU 9728; `TextConfig::qwen3vl_4b`,
  the loader takes any key root since the repack keeps `model.*`) tapped at twelve hidden
  states (`hidden_states[2, 5, …, 35]`, transformers' numbering, `forward_taps`), the prefix
  through `<|im_start|>user\n` dropped (`krea2::template_end`): the conditioning
  `(tokens, 12, 2560)` is **1 % from ComfyUI's** (cosine 1.000, 59 tokens → 25). The **DiT**
  (`model/krea2.rs`): a single-stream transformer, 28 blocks of 6144 with GQA 48/12 heads of
  128, per-head q/k RMSNorm with the `(1 + scale)` weight, a sigmoid gate on the attention
  output, SwiGLU 16384, AdaLN-single (one `tproj` of the timestep plus a per-block bias,
  six chunks), Flux's 3-axis RoPE with axes `[32, 48, 48]` at theta 1000 (text at position 0,
  pixels at `(0, y, x)`), the `txtfusion` adapter (two blocks across each token's twelve states,
  a learned 12→1 mix, two blocks along the sequence) and `txtmlp`; one forward on ComfyUI's
  latent and conditioning is **1.6 % from its fp8 route** (cosine 0.99988). The **Wan 2.1
  VAE** decoder for one frame (`model/wan_vae.rs`, Wan's own layout; every causal 3-D
  convolution sees two zero frames so only its last temporal tap matters, the temporal
  resample never runs on a first frame): **0.24 % from ComfyUI's decode**, first try. The
  **sampling**: a `FLUX`-type model with `shift 1.15` (read off the loaded model's
  `model_sampling`; the 8 `simple` sigmas are in a unit test), the noise ComfyUI draws on the
  `(1, 16, h, w)` latent, the Wan21 statistics on the way to the VAE, no timestep rounding
  (Flux's embedding takes the f32 sigma).
- **Speed on the way.** The first picture took 14 s (plain f32 attention over 4121 tokens × 48
  heads, an f32 VAE at 1024²) and the plain path ran out of memory on one case of 28. With
  SageAttention2 over the joint sequence (the GQA handled by the kernel), ConvRot INT8 on the
  block and text-fusion linears (`KLinear`: the 24 GB bf16 model becomes 12 and the encoder
  stays resident) and the VAE in bf16 (norms and attention scores in f32): the fox in
  **3.8 s** (0.09 s prompt, 3.0 s sampling, 0.6 s decode) at **35.6 dB** against the app's
  ComfyUI route (fp8), ComfyUI 4.0–4.4 s. The sage path is as close to ComfyUI as the plain one
  (1.6 % on the forward).
- **Parity, 28 cases** (`runs/2026-10-20-qir-krea2-t2i` against
  `references/2026-10-18-comfy-t2i`, the app's ComfyUI route with the fp8 files): **16 of 28
  at the gate, 17 against a bf16 ComfyUI route** (`references/2026-10-20-comfy-t2i-krea2-bf16`,
  the converted files through ComfyUI's API with the template's graph) — and ComfyUI's two
  routes agree with each other on only 20 of 28: an 8-step Turbo at CFG 1 is sensitive to its
  arithmetic, and the misses are the same scenes everywhere (street ×3, food, vehicle,
  architecture-s7, interior-s7), the same picture with details shifted. A run with the bf16
  DiT (no ConvRot; SageAttention still) reaches **21 of 28 against the bf16 route**, one more
  than ComfyUI's fp8 route does, at 8.6–15 s a picture (candle's bf16 GEMMs; the encoder is
  reloaded per prompt because the 24 GB DiT leaves it no room): the INT8 linears cost four
  cases here, the fp8 GEMM of the kernel track (§11) is what keeps both. Engine times with
  INT8: 3.5–4.9 s a picture (ComfyUI fp8 4.0–4.7, bf16 5.5–6.7).
- **The worker** (`gn-diffusion`, `4ab8f4f`): `krea2-turbo/image` beside the 2.1 templates,
  one family resident at a time (the card holds either set): the other family's first request
  swaps them. PhotoshopEX pinned at `4ab8f4f`; through the app with `engine = native`: the
  first picture 15.7 s (loads, quantises), then **4.85 s and 4.31 s per 1024² picture**
  (ComfyUI warm 4.0–4.7), a swap to 2.1 and back 18 s and 23 s. **Krea 2 stays on ComfyUI under
  `auto`** (the rule is faster at parity: this is neither clearly faster nor at the precision
  ComfyUI's own routes reach with INT8); `native` serves it, the GUI and CLI carry it.
- **The ConvRot cache for Krea 2** (`genai-native` `f2bf496`, pinned): `convrot_cache::Policy`
  names which linears a model rotates and tags the entry; Krea 2's entry
  (`<cache>/krea2-turbo-<hash>/transformer_convrot.safetensors`, 256 linears, 13 GB) builds
  once in 27 s and the DiT then loads in **4 s instead of 11**, the picture bit-identical
  (99 dB). The worker keeps it under `<models>/.qir-cache/` with the 2.1 entry.
- **A trap for timing on this PC (23:00):** four `python.exe` processes in session 0
  (services, not killable from the user session, no command line visible) appeared and hold
  about 10 GB of the card; with them resident the same cold start sampled in 14 s and decoded
  in 19 s (0.6 s before). Every number above was measured before they appeared. Check what
  started them before trusting a timing (`nvidia-smi`, `tasklist /v`). Open: the style
  reference (6 cases), LoRAs, the fp8 GEMM.

## 2026-10-20 (later): plan Phase 3, Qwen-Image-2.1 edit and fill native, under `auto`

- **The pieces, each checked against ComfyUI's running objects before the next was built**
  (`genai-native` `c0a598f`, `d170a38`, `1c43110`): the **VAE encoder** (Encoder3d at T = 1 with
  the AvgDown3D shortcut's zero-padded earlier frame: 0.8 % from ComfyUI's latent on the fox,
  38 dB round trip); the **Qwen3-VL vision tower** in f32 (27 ViT blocks, the learned 48×48
  position table interpolated in bf16 as transformers does, which took its drift from 4.8 % to
  1.4 %; DeepStack mergers after blocks 8/16/24); the **multimodal text encoder** (image rows
  at `<|image_pad|>`, interleaved mRoPE `[24, 20, 20]`, DeepStack features added after decoder
  layers 0–2; spans and slots exact, the text after the picture at cosine 0.9998); the DiT's
  **`build_sequence`** (reference latents spliced at their slots, frame positions, grids centred
  on the target with the half-token parity offset, prefix rows modulated at t = 0, text stretches
  causal, each picture block attending to everything before its end). The edit prompt template
  and `TextEncodeQwenImage21`'s sizing at `resolution 0` (sides to multiples of 32, Lanczos).
- **The first joint forward ran out of memory** (a dense f32 `(1, 32, S, S)` score tensor at
  S ≈ 5100): attention is now computed per segment in 1024-row query chunks with f32 scores and
  a small causal mask where a text stretch needs one. The fox's "make it winter" then matched
  ComfyUI at **40.0 dB** (bf16 route) and 39.4 dB (the app's INT8 route; ComfyUI's two routes
  are 43.9 dB apart on it), at 47 s of sampling.
- **The prefix cache** (`dit::PrefixCache`, what ComfyUI's `QwenImage21Cache` node keeps):
  the text and reference rows never attend to the target and modulate at t = 0, so their keys
  and values are the same at every step. They are computed once per generation (the plain
  segment attention, 1.5 s on the fox) and every step runs the target's 1024 rows alone, with
  SageAttention over the joint keys and no mask. **Sampling 47 s → 3.8 s**, 44 dB against the
  uncached picture, 39.9 dB against ComfyUI. `--no-prefix-cache` keeps the plain path as the
  reference.
- **The worker and the app.** `gn-diffusion` serves `qwen-2.1/edit` (the picture, then the
  references) and `qwen-2.1/fill` (the picture, then the mask as a grey picture) the way the
  app's ComfyUI graphs feed `TextEncodeQwenImage21`; alpha is made opaque the way ComfyUI's
  `LoadImage` hands pictures over (the colours as they are, the VAE's alpha padded opaque, no
  compositing over white). PhotoshopEX: `run_fill`, `run_expand` and `run_edit` resolve their
  backend per template through `backend_for` like `run_image` does, the native route wraps the
  prompt with the template's `promptFormat` as the ComfyUI route does (a fill's wrapper names
  the mask image), the fake-engine test runs an edit and a fill through the channel. Pinned at
  `317f1e2` (the eviction policy included), both executables built with `native-diffusion`.
- **Parity, measured through the app's own CLI** (`gn-bench --engine comfy --pref
  integrations.engine=native …`: the same request building on both routes, the composite, the
  feathered mask, the 880 px working size; references
  `references/2026-10-20-comfy-{edit,fill}-qwen21`, runs `runs/2026-10-20-native-{edit,fill}-qwen21`):
  **edit 8 of 8 at the gate** (36–40 dB on six; the boat 21.2 dB / 0.94 SSIM and the fox in the
  boat 23.8 dB / 0.97), **fill 8 of 8** (17–56 dB, SSIM 0.91–1.00 on the whole picture with the
  fill masked in). A direct `--engine qir` run of the edit suite is *not* comparable (2 of 8):
  the app resizes a 1024² picture to 880² before the model sees it and the CLI does not.
- **Speed.** Through the app, one CLI process per case (so each case loads the models): edits
  12.6–13.5 s against ComfyUI's 5.4–5.9 s warm. **Resident, in one CLI process**
  (`native-edit-live.bat`): the first edit 12.3 s (loads), then **4.19 s and 4.21 s per 880²
  edit against ComfyUI's 5.4–5.9 s**, and a 608×464 fill in **1.8 s** (ComfyUI 1.9–3.5 s). The
  two-reference cases had run 30 and 68 s: the bf16 text encoder (16 GB) + INT8 DiT + VAE +
  vision tower + an 8 GB prefix cache oversubscribe the 32 GB card and the driver pages. Fixed
  in `genai-native` by the policy §8 of the plan asked for: `Options::evict_text_encoder` (on)
  drops the encoder after the prompt when `free_vram` says the cache will not fit (free 2.3 GB,
  needed 9.0 GB on the fox-in-the-boat) and loads it again for the next prompt (about 7 s): that
  edit now takes **9.0 s** (3.6 s encoders and eviction, 4.7 s sampling, 0.7 s decode) against
  ComfyUI's 8.2–8.6 s. Through the pinned app, two such edits in one process: 14.7 s (loads)
  then 10.9 s (the encoder's 7 s reload is paid on every two-reference edit; keeping its weights
  in host memory for a sub-second upload is the next lever). No worker lingers after the CLI
  exits (the server quits when its client goes).
- **Matte and transparent output, native** (`genai-native` `02f428c`): the worker serves
  `qwen-2.1/matte` (the picture alone; the 2.1 VAE decodes RGBA and the alpha is the cut-out)
  and `generate.removeBackground` resolves its backend per template too; `resolve_auto` counts
  a template the native worker lists as installed (the worker names templates, not model
  files), so `auto` with research models allowed takes the native matte. On the fox the native
  matte and the ComfyUI route's agree at **soft IoU 0.992** (alpha mean 0.337 vs 0.335): the
  same noisy background matte on both, the model's weakness on a busy background (§5b), not the
  route's. `generate.image {transparent}` natively carries a real alpha (the boat prompt the
  ComfyUI route was verified with: alpha mean 44/255 both through the engine and through the
  app; an exported composite over the white Background layer is opaque, use `target:
  document` to see it).
- **Generative Expand with 2.1, native** (`genai-native` `37be64a`, template
  `qwen-2.1/expand`). Two approaches failed first: the fill graph with the whole canvas as
  image 1 and the mask as image 2 painted the replicated-edge prefill as streaks, and a grey
  prefill came back as black canvas (an in-context model copies whatever it is shown). What
  works: the picture alone as the reference, **placed at its own position inside the canvas**
  (`RefImage::offset`: its latent tokens take the positions of the target tokens under them,
  cropped to whole 16 px tokens, never resampled), and the target is the whole canvas; the
  editor's layer mask then confines the result to the new area as it does for 2511. The worker
  cuts the picture out of the request at the engine's `ref_x/ref_y/ref_w/ref_h`. Symmetric
  expands were seamless at once; a one-sided 384 px expand painted a second fox when the request
  carried only a strip of the picture (the engine's 25 % margin), so the template says
  `expandMargin: 1` (a new template property: the whole picture goes along) and both one-sided
  cases (left 384, top 320) are seamless at 11.9 s per process through the app. The ComfyUI
  graph of the template does the same with ImageCrop and EmptyLatentImage, but
  `TextEncodeQwenImage21` centres every reference on the target, so through ComfyUI it lines
  up for symmetric expands only (noted in the template). `auto` still prefers the 2511 expand
  templates (`AUTO_EXPAND_ORDER`); the 2.1 one is picked by template or as the default edit
  model. Open: the `<|im_start|>` sink row's precision; previews for the task bar.

## 2026-10-20: plan Phase 2 done, Qwen-Image-2.1 native at parity and under `auto`

- **ComfyUI's sampling conventions, reproduced** (`genai-native/crates/qir/src/comfy.rs`,
  `4cfc445` then `992231a`): PyTorch's CPU generator (MT19937 seeded as `torch.manual_seed`, the
  16-wide Box–Muller `normal_fill` with its last-chunk redraw) gives `torch.randn([1, 64, H/16,
  W/16])` bit for bit (unit tests carry the numbers ComfyUI's own python printed for seeds 3, 7
  and 11, square and 48×84); the `simple` scheduler over the model's sigma table; the DiT's
  timestep rounded as ComfyUI rounds it (`bf16(bf16(σ·1000)/1000)`); the prompt tokens checked
  against `TextEncodeQwenImage21` (the system turn is 14 tokens, dropped). `generate --comfy`
  uses them; `--width`/`--height` make the non-square cases; a `timings` log line carries the
  phases and `gn-bench --engine qir` reads it (its `tracing` goes to stdout, not stderr).
- **The sigma table was the day's detour.** The first table was `ModelSamplingDiscreteFlow`'s
  (the SNR shift `0.69·t/(1+(0.69−1)t)` over 1000 steps): the routes agreed after one step
  (42 dB, sigmas `[1, 0]` both sides) and parted from the second (31.5 dB at 2 steps, 21 dB at
  25; 6 of 28 cases at the gate). Ruled out one by one on the fox: the attention kernel, our DiT
  quantisation, the DiT weights themselves (ComfyUI's INT8 ConvRot repack converted into the
  port's cache, its 256-column groups permuted between the two Hadamard seeds: 21.1 dB, same as
  before), the text encoder (ComfyUI's INT8 one is 8.8 % from the bf16 oracle, ours 2.7 % after
  moving its attention scores to f32, and ComfyUI's own INT8 and bf16 routes agree at 24–42 dB
  anyway), the VAE (ComfyUI's own final latent through our decoder: 49 dB), a single DiT forward
  on identical inputs (1 % apart). Stepping ComfyUI's real `KSampler` next to a plain Euler loop
  over the same model showed the second sigma at 0.9795 against the formula's 0.9430: **the 2.1
  model is a `FLUX`-type model in ComfyUI, its sampling is `ModelSamplingFlux`** (the exponential
  shift `e^0.69/(e^0.69 + 1/t − 1)` over 10 000 timesteps), and the script that had printed the
  "ground truth" had instantiated the other class. With that table: **the fox 39.9 dB** against
  the app's reference and 40.6 dB against a bf16 one. Lesson: take a model's sigma table from the
  loaded model object, never from a formula read off a class. (A raw-latent-space theory
  entertained for an hour on the way was wrong too: ComfyUI's `QwenImage21` latent format does
  carry the VAE statistics; `SaveLatent` writes the denormalised tensor, which is what misled.)
- **Parity, measured** (`runs/2026-10-20-qir-t2i-comfy-flux`, 28/28 cases, SageAttention2,
  INT8 ConvRot): **24 of 28 at the 30 dB / 0.90 gate against the app's INT8 reference, 25 of
  28 against ComfyUI run with its bf16 encoder and DiT** (`references/2026-10-20-comfy-t2i-qwen21-bf16`,
  through ComfyUI's API with the template's graph); ComfyUI's INT8 route against its own bf16
  route is 26 of 28 (24–42 dB), so the native route agrees with ComfyUI as well as ComfyUI agrees
  with itself. The misses are the same cases for everyone: the brutalist facade (both seeds;
  ComfyUI vs itself 24.8/25.8 dB), the vehicle at seed 11 (23 dB; ComfyUI vs itself 26), and the
  BAKERY sign at seed 7, where the INT8 encoder flips the background to transparent on the app's
  route (6 dB between ComfyUI's two routes; ours matches the bf16 route at 37.9 dB).
- **The resident pipeline and the worker.** `qir::pipeline` (encoder, DiT and VAE loaded once,
  `generate` with progress and a cancel check between steps, ComfyUI's conventions by default);
  `gn-diffusion` serves `qwen-2.1/image` over it from `<models>/qwen-image-2.1/` (Hugging Face's
  layout) with the INT8 DiT cached in `<models>/.qir-cache/`; `gn_engine::Multi` puts it beside
  the perception engine in one worker, each request to the engine whose index lists the template;
  `EngineClient::spawn_with_env`. PhotoshopEX (`crates/genai`, feature `native-diffusion`, off by
  default: nvcc needed): the worker serves both engines, the runtime folders go first on its
  `PATH` (candle links `cudart`/`cublas`; without them the worker dies at start with
  STATUS_DLL_NOT_FOUND), `generate_cmds::NATIVE_AUTO_FAMILIES` is the parity table (`sam3.1`,
  `qwen-image-2.1`: **`auto` routes Generate Image with the 2.1 template natively**; `run_image`
  now resolves its backend per template through `backend_for`, as the `select.*` commands do,
  while fill, edit and expand keep the process backend until their families go native), the
  fake-engine test covers it. Built with the feature (3 min 19 s on top of the usual build) and
  run live through `photocraft-cli`: health `native/ort+qir`, the fox at 1024², 25 steps:
  **11.7 s the first picture (loads), 3.7 s and 3.65 s the next two, resident** — against ComfyUI's
  4.4–5.0 s engine time. With ComfyUI's models still resident the same run took 11.9 s a picture
  (32 GB oversubscribed): stop one engine before timing the other.
- **Also on disk now** (downloaded for the bf16 reference, kept as BYOM files): Comfy-Org's
  `qwen_image_2.1_bf16.safetensors` and `qwen3vl_8b_bf16.safetensors` in ComfyUI's model folders.
  `_eval\native-models\qwen-image-2.1` and `.qir-cache` are junctions to the snapshot and to
  `_eval\qir-cache`; the real preferences keep `engine = auto`, which now means native 2.1 text
  to image in a `native-diffusion` build (the pinned app is one).
- **Open after this**: the §7.1 restructure (`gn-kernels` fatbins, `gn-blocks`, `gn-models`);
  loading the ComfyUI repack weights as they are (the converter proved the Hadamard-seed
  permutation; it saves the 31 GB snapshot and 20 s of quantisation); a VRAM policy for the
  resident encoder (Q8 on 24 GB cards: `--text-quant` exists, no preference yet); the GUI's
  engine page (resident models, free); Phase 3 (edit, fill, expand natively).

## 2026-10-18: the native-engine plan

- **Owner's direction** (2026-10-16 → 18): "this needs to leave the lab and become real
  production software", "it's always BYOM", "my biggest concern is that there will be new
  frontier models by then", "I want to do this once and do it right". Scope they chose for a
  native engine: Krea 2 Turbo, Qwen-Image-2.1 (generate + edit), SAM 3.1; everything else rented.
- **Landscape check** (web, 2026-10-16): `stable-diffusion.cpp` (C++/ggml, MIT, ~7.7k stars)
  already runs Krea 2 (2026-06-25), Qwen-Image-2.1 (day-0, 2026-09-20), the Qwen Image Edit
  series, Z-Image, FLUX.2/3, with LoRA and GGUF; no FP8/NVFP4 path and no published 5090 number
  vs ComfyUI. `umag/qwen-image-rs` (Rust + CUDA on candle, Apache-2.0, one developer) ports
  Qwen-Image-2.1 t2i from diffusers at parity and self-reports 0.62 → 0.148 s/step on a 4090
  with INT8 ConvRot GEMMs + SageAttention + fused kernels (sm89 only, no edit/LoRA/masks).
  `SceneWorks/inference` merged 2.1 on candle and MLX with reference-image editing
  (2026-09-23). SAM 3 community ONNX/TensorRT: 24 ms text, 85 ms point/box on a 5090 FP16; no
  official export. Reading: speed is in CUDA kernels whatever the host; a component engine
  with thin definitions is how sd.cpp keeps day-0 pace; a Rust host with its own kernels can
  beat ComfyUI per model at per-model, per-GPU cost.
- **Sizing against the owner's Halo week** (`Documents\BloodGulch-Web`, no git: ~55 k lines JS +
  ~34 k Python export tools in 8 days by a lead model plus a swarm of bounded workers, 65 agent
  reports, 1,663-test baseline, README: "1:1 milestone not achieved"): that pace comes from
  hundreds of independent deterministic tasks with an x86 oracle and instant feedback; the
  engine serializes on one GPU and a dependency chain, so the estimate is ~6–8 weeks to "works
  end to end", ~12 weeks to the acceptance walkthrough, kernel track ongoing.
- **Written: [`plan-native-engine.md`](plan-native-engine.md)**: the four brief sentences as
  design commitments; the ground (trait, templates, commands, research verdict, landscape
  table); the target (editor ⇄ `GenerativeBackend` ⇄ {native worker, sd.cpp FFI, hidden ComfyUI
  sidecar}); `genai-native`'s five one-way layers (kernels / blocks / thin model definitions /
  BYOM loader / perception via `ort`) on candle plumbing; routing `auto: native → sdcpp → comfy`
  with per-model overrides and badges; the engine as a **worker process** (`photocraft.exe
  --engine`) so a CUDA fault never kills the editor; phases 0–6 with gates (ground truth +
  harness + sidecar; SAM 3.1 native; skeleton + 2.1 t2i parity; 2.1 edit/fill/expand; Krea 2;
  Python-free packaging; kernel track SageAttention sm120 → FP8 → NVFP4 → CUDA graphs); the
  parity harness spec and thresholds; failure taxonomy; commands and preferences; BYOM file
  security; budgets; tests by crate; live scripts; licence checklist (Qwen Research License
  covers "model code" → port from diffusers/ComfyUI's Apache-marked file, never Qwen's repo);
  the owner's walkthrough; timeline by track; risks; six decisions for the owner (worker
  process, separate repo, a second GPU box, ~46 GB of test-weight downloads, the one surviving
  Python step for the SAM export, 2511 out of scope). Roadmap Phase 6 → planned; README index row.
- **Owner, same day**: "let's build it out", then "this needs to be cross platform… or at least
  first Windows and Linux" → plan §3.5 (Windows and Linux first, macOS as a later kernel backend;
  the second GPU box should be Linux). Phase 0 started.
- **Toolchain facts on this PC**: no CUDA toolkit, no nvcc, no CMake (installed through scoop,
  4.4.4), VS 2019 Build Tools 16.11 only (CUDA 13 wants VS 2022; sm_120 needs CUDA ≥ 12.8),
  driver 617.42, 1 TB free. Installing CUDA + VS 2022 Build Tools needs the owner's go.
- **Two review notes** (subagents, read-only): [`research/notes/sd-cpp-survey.md`](research/notes/sd-cpp-survey.md)
  — sd.cpp's CMake recipe (`-DSD_CUDA=ON`, shared libs, `GGML_BACKEND_DL`), **a prebuilt Windows
  CUDA 12.8 release zip includes arch 120** (so the §5.3 benchmark needs no local toolkit; Linux
  CUDA is Docker-only, we build), the full C API (ctx params, txt2img/img2img/inpaint/ref-edit,
  21 samplers, `sd_cancel_generation`, process-global callbacks, lib-malloc'd images), Krea 2 and
  Qwen-Image-2.1 file/format details (Comfy-Org repacks and INT8 convrot load directly; GGUF
  Q8/K-quants; LoRA at runtime), SageAttention 2++ on CUDA ≥ 12.8, a declared NVFP4 GGUF type
  with no documented model path, `--rng cpu` for ComfyUI-matching noise, FFI risks (abort on
  GGML_ABORT, exceptions across the C boundary, unstable appended struct fields → pin + bindgen),
  and the sd-cli benchmark lines. [`research/notes/qwen-image-rs-review.md`](research/notes/qwen-image-rs-review.md)
  — candle 0.11 + cudarc, kernels compiled by `cc` + nvcc for one `sm_89` at build time (no
  fatbins), CUTLASS from an external dir, cuDNN via a Linux pip wheel: **Linux/WSL2-only as
  built**; architecture map (single-stream 32-block DiT t2i only, Qwen3 text tower only, VAE
  decoder only, 2 000-line CLI monolith, official diffusers tensor names only); kernels (CUTLASS
  Sm80 EVT INT8 GEMM with fused dequant, in-register Hadamard rotate+quant, SageAttention v1/v2,
  fused norms) all gated `>= 800/890`, so sm_120 is an arch-flag change for correctness and a
  re-tune for speed; parity scripts reusable but the oracle tensors are not in the repo; 69
  `unsafe` at the FFI boundary; a licence inconsistency (Cargo says MIT OR Apache-2.0, LICENSE
  says Apache-2.0); nothing from Qwen's research-licensed repo. Verdict for week 1: bring
  qwen-image-rs up in WSL2 (its author's own setup, CUDA 13.3) or on the Linux box, not on
  Windows.
- **The sidecar landed (plan §5.4)**: `photocraft_genai::sidecar` (a `Launch` built from the
  preferences each call; one child per process; `resolve` finds the interpreter per layout —
  `python_embeded`, `venv`/`.venv`, system Python; `command_line` = `python [-s] main.py --listen
  host --port port --disable-auto-launch <flags>`; `CREATE_NO_WINDOW` on Windows, a process group
  on Unix; stdout+stderr to `%TEMP%/photocraft-engine.log`, restarted past 10 MB; `status` polls
  the child and probes the URL at most once a second; `ensure_running` waits up to 180 s with
  progress and cancel; an external server at the URL is used as it is), `ComfyClient::probe`,
  `ComfyBackend::with_sidecar` (a run brings the server up first), `generate.engine.status/
  start/stop/restart` (`generate_engine_cmds.rs`), preferences `comfyPath`, `comfyArgs`
  (`--fast fp8_matrix_mult`), `engineStartOnLaunch`, `engineStopOnQuit` (eframe `on_exit`), the
  status bar's engine dot with a Start/Stop/Restart/Show log menu (`engine_ui.rs`, polled every
  2 s through `session.execute` so the status line is never touched), 15 strings ×13. Tests:
  4 in `sidecar`, 1 engine (`engine_commands_follow_the_preferences`), 2 UI. Live
  (`bench/scripts/sidecar-live.ps1`, ComfyUI stopped first): unconfigured → stopped → `start`
  returns `starting` at once → **ready after 29.2 s, console window hidden** → `select.subjectML`
  through it (the fox, 1 instance) → `stop` → a detector call with the engine stopped **started
  it by itself and answered after 21.4 s**; a second run against the real config dir set
  `comfyPath`, started the engine and quit through `app.quit`: the engine process was gone 4 s
  later (`engineStopOnQuit` through eframe's `on_exit`). The pinned app now starts ComfyUI itself.
  Suite: genai 38, engine 920 + integration, UI all
  green but the baseline `eyedropper` failure; clippy, fmt, wasm engine build. Three UI tests
  (`alt_scroll_zooms_*`, `clicking_outside_a_dialog_…`) assumed upstream's wheel default and
  broke with the ported wheel = zoom default (`9e75bf4`); they now pin the preference.
  `append-translations.ps1` pointed at the retired checkout; fixed. `CLAUDE.md`, README's
  how-to, `architecture.md` (commands, preferences) updated.
- **The harness (plan §5.1) exists**: a separate repository `C:\Users\5090\Documents\genai-native`
  (the recommended split; `gn-harness` first, the other crates to come): fixtures as JSON with
  geometry in fractions of the picture (`t2i` 28 cases: 12 prompts × 2 seeds at 1024² + 4 other
  aspects; `edit` 8, two with references; `fill` 8; `sam` 16: text, point, negative point,
  box over the bench pictures), an `Engine` trait with `ComfyCli` (PhotoshopEX's CLI with the
  same templates and seeds the product uses; `--comfy-path` sets `integrations.comfyPath` at the
  head of every invocation so the sidecar brings the server up), metrics (PSNR, SSIM on luma,
  mask IoU, boundary F at a pixel tolerance, bounds IoU for the SAM suite while engines report
  bounds), `Verdict` with the plan's thresholds, run folders (`<case>.png` + `<case>.json` +
  `summary.md`), `gn-bench run|compare|summary`. 7 tests; clippy clean under the same never-crash
  lints. Docs: README, ARCHITECTURE, PARITY, PORTING, NOTICE, licences.
- **A real bug the harness found in the sidecar.** On Windows `std::process::Command` creates
  children with handle inheritance on, so ComfyUI started from `photocraft-cli` inherited the
  shell's (and `gn-bench`'s) stdout pipe and held it open for as long as it lived: `gn-bench`
  waited for EOF on the first case forever, and the same would hang the MCP bridge on stdio and
  any script. The GUI was unaffected. Fix: on Windows the sidecar launches through PowerShell's
  `Start-Process` (ShellExecute, no inheritance) running a hidden `cmd.exe /d /c "<python> …
  >> log 2>&1"`, and tracks the child by PID (`tasklist`, `taskkill /T`; the liveness check runs
  only while the URL does not answer, so the status poll stays cheap); Unix keeps the plain
  spawn in its own process group (Rust's descriptors are close-on-exec). A Windows unit test
  starts a `ping` that way, sees it in `tasklist`, reads its redirected output and ends it.
  Re-verified live after the change: `sidecar-live.ps1` ready in 10.1 s (warm disk), auto-start
  before a detector call in 14.7 s; and the **first reference run**, the SAM suite on the ComfyUI
  route: 16 of 16 cases, 8.0 s wall in total (0.3–0.8 s per case warm, engine time 0.2–0.7 s),
  the shell pipeline returning at once. Reference folder:
  `genai-native\references\2026-10-18-comfy-sam\` (ignored; `summary.md` there).
- **The reference baseline is in** (`genai-native\references\2026-10-18-comfy-{t2i,edit,fill,sam}`,
  44 pictures, 52 MB, ignored by git; `benchmarks.md` §8): t2i 28/28 on `krea2-turbo/image` in
  142 s (4.0–4.4 s engine time per 1024² image warm, 20.6 s for the first, cold); edit 8/8 in
  652 s (`auto` under Quality best = `qwen-edit-2511/edit`, 40 steps: 67 s each, 120 s with a
  reference picture); fill 8/8 in 383 s (44–48 s each, 60 s for the removal case); sam 16/16 in
  8 s. Every later engine is compared against these folders with `gn-bench compare`.
- **2026-10-19, `gn-weights` (plan layer 4, the BYOM loader), in `genai-native`** (`69d1367`):
  a hardened safetensors reader (header length capped, every offset and byte count checked
  against the file and the dtype, unknown dtypes indexed but refused on read; only the header is
  read on open, tensors come as checked ranges on demand through a seekable `RangeFile`, so a
  20 GB repack costs milliseconds and no `unsafe` mmap), a GGUF v2/v3 reader (growing-prefix
  header parse, all metadata value types, ggml block-type sizes incl. the NVFP4 id sd.cpp
  declares, alignment, ranges checked), `DType` across both, writers for fixtures, `ModelIndex`
  over ComfyUI's folder layout, and `identify` from the tensors alone (never the file name). The
  `gn-inspect` tool read the owner's 16 real files and **all are identified**: Krea 2 (fp8-scaled
  686 tensors, NVFP4 942), Qwen-Image 20B (2511 and Layered fp8mixed), Qwen-Image-2.1 (INT8
  convrot), Qwen3-VL 4B/8B and Qwen2.5-VL 7B encoders by `model.embed_tokens` shape, the Wan,
  Layered (RGBA head) and 2.1 (`modelspec.architecture`) VAEs, SAM 3.1 (`detector`/`tracker`),
  the Lightning LoRAs. What the files taught, in `genai-native/docs/MODELS-ON-DISK.md`:
  ComfyUI's repacks are self-describing (`_quantization_metadata` per layer in the header, a
  `comfy_quant` JSON tensor per layer): `float8_e4m3fn` = F8 weight + per-tensor F32
  `weight_scale` (+ `input_scale` in "fp8mixed"); `int8_tensorwise` + `convrot` group 256 = I8
  weight + per-row `[out, 1]` scales (the 8B encoder's embedding too); `nvfp4` = U8 `[out, in/2]`
  + F8 block scales `[out, in/16]` + F32 `weight_scale_2`. Krea 2's own naming (`blocks.N.attn.
  {wq,wk,wv,wo,gate}`, `txtfusion`, width 6144, text width 2560), 2.1's `modulation` prefix and
  hidden 4096 against 2511's 3072. Tests: 5 (+ a local-files test that runs only on this PC).
- **`gn-engine` (plan §3.4, the worker process), in `genai-native`**: the protocol (`Request`/
  `Reply` as JSON, pixels as binary blob frames preceding the message that names them, 256 MB
  frame cap, `check_blobs`), the server loop (loopback TCP with a random port printed as the
  first stdout line and a token from `GN_ENGINE_TOKEN`; one client at a time with a reader
  thread so `Cancel` reaches a running engine; a second client is refused at once instead of
  waiting in the backlog), the client (`spawn`/`connect`, a resumable `FrameReader` that keeps
  partial bytes across the 200 ms cancel polls — `read_exact` under a read timeout would lose
  them —, a 10 s greeting timeout, `quit`/`kill`/`alive`, a `Drop` that never leaves a worker
  behind), a `FakeEngine` (solid pictures from the seed, tinted edits, a disc for segment,
  typed `ModelMissing`), the `gn-engine` binary. Integration tests drive the real child
  process: progress and residency, free, index, a typed error without a dead worker, cancel
  within a second of a 10 s run, a killed worker noticed on the next call, a 1 MP picture both
  ways, a mismatched blob refused before sending, a wrong token turned away while the first
  client stays connected (the case that hung the first version). Transport note: TCP on
  loopback was chosen over named pipes / Unix sockets because it is one code path on all three
  platforms with the standard library alone and is what the app's control channel already
  does; the protocol is transport-agnostic. The editor side (`NativeBackend`) waits on how
  PhotoshopEX depends on `genai-native` (a path dependency breaks clean clones; a git dependency
  needs the repository published — the owner's call).
- **Owner: "yes" to all four** (publish, installs, downloads, decisions). Done in this order:
  - `genai-native` pushed to **github.com/CodyChuGit/genai-native (private)**; PhotoshopEX
    depends on `gn-engine` by git rev (`crates/genai/Cargo.toml`), fetched with the system git
    (`.cargo/config.toml` `[net] git-fetch-with-cli = true`, so the gh credential helper serves
    the private repo). Pinned at `4a06b15`.
  - **CUDA Toolkit 13.4.3 installed through scoop** (user-level, `~\scoop\apps\cuda\current`,
    on PATH for new shells). **VS 2022 Build Tools failed with 1602** (= the UAC prompt was not
    accepted; winget's installer needs a click): rerun
    `winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`
    with the owner at the keyboard. nvcc 13 wants VS 2022 as its host compiler.
  - **Downloads** (into ComfyUI's model folders, so both engines see them): the Wan 2.1 VAE
    (`wan_2.1_vae.safetensors`, 254 MB), `Krea-2-Turbo-Q8_0.gguf` (13.0 GB, `realrebelai/
    KREA-2_GGUFs`), `Qwen3VL-4B-Instruct-Q8_0.gguf` (4.1 GB, Qwen's official GGUF), and
    sd.cpp's prebuilt `sd-master-95b3090-bin-win-cuda12-x64` + its `cudart` zip into
    `Documents\_eval\sdcpp-bin` (CUDA 12.8, arch list includes 120: no local toolkit needed).
    The ~30 GB Qwen-Image-2.1 bf16 oracle download waits until the qwen-image-rs bring-up has a
    Linux/WSL2 home.
  - **The first sd.cpp number (plan §5.3)**, `benchmarks.md` §8: Krea 2 Turbo GGUF Q8 at
    1024², 8 steps, flash attention on: **0.59 s per step (1.69 it/s), sampling 8.0 s, encode
    1.1 s, decode 0.57 s, 9.7 s per `generate_image`**, 17.3 GB in VRAM; the fp8 repack through
    sd.cpp 15.7 s (dequantised, no tensor cores). Against ComfyUI's ≈0.41 s/step: **1.45× per
    step**, the plan's "the lane, flagged for the kernel track" band; the 2.3× per picture is
    mostly sd-cli loading the DiT inside every call (3.1 s) and a slower encoder, which a
    resident worker removes. The fox is correct. sd.cpp loaded both the GGUF and the ComfyUI
    fp8 repack files as they are.
  - **The editor talks to a worker (plan §3.4 editor side):** `photocraft_genai::native::
    NativeBackend` implements `GenerativeBackend` over `gn-engine`'s client (spawns
    `photocraft[-cli] --engine` on first use, respawns after a death, maps `Request` → blobs +
    `RunRequest`, progress and `Progress::cancelled` → `Cancel`, blobs → `Rgba8`, coverage →
    grey pictures as the ComfyUI route delivers masks, `ClientError` → `Error`); `engine_main`
    makes either executable the worker (serving the fake engine until the native models land,
    "installed" = the built-in template ids); both `main`s branch on `--engine` first.
    `integrations.engine` = `auto | native | comfy` (`auto` is the ComfyUI route until a native
    model is faster at parity; "Engine", "Native", "ComfyUI" ×13); `generate_cmds::backend`
    routes, one process-wide `NativeBackend` shared by every command; `generate.health` reports
    `native/fake`. Tests: `native::tests` (in-process fake worker: health, image, edit round
    trip, coverage, typed missing model, cancel) and the engine's
    `the_engine_preference_routes_to_the_native_worker` (`prefs.set` validation, health,
    `generate.image` into a new document, `generate.edit`, a missing template as a sentence,
    back to ComfyUI). 187 generate/select tests still green.
  - `gn-engine` fix on the way: a worker whose greeted client leaves without `Quit` (a crashed
    editor) exits by itself; a refused greeting does not end it; tested with a hand-spawned
    worker.
  - **Live through the real binaries**: `photocraft-cli run … prefs.set engine=native …
    generate.image` spawned `photocraft-cli.exe --engine` as its worker, `generate.health` said
    `native/fake`, the picture came back as a layer, no worker left behind. Suites after the
    wiring: engine 921 (+1), UI all but the baseline `eyedropper`, wasm engine build green.
  - **VS 2022 Build Tools installed on the second attempt** (17.14.42, MSVC 14.44; the UAC
    prompt accepted this time) and **nvcc 13.4 compiles for `sm_120a` with it**: a one-kernel
    smoke test built in 3.1 s and ran on the 5090 (cc 12.0). The kernel toolchain for
    `gn-kernels` exists on this PC (`dev-environment-windows.md` › CUDA kernels); the
    qwen-image-rs bring-up can be attempted here before WSL2 (its build script is POSIX-leaning,
    so WSL2 stays the fallback).
  - **The sd.cpp runner in the harness** (`gn-bench run --engine sdcpp`, `97b3d86` there) and
    the whole t2i suite through it: 28/28, 0.41× ComfyUI's wall per picture, **9 of 28 at
    seed-level parity**, the misses being the same scene with detail drift (encoder-level
    divergence: Q8 GGUF Qwen3-VL-4B + sd.cpp's chat template vs ComfyUI's fp8 repack + template).
    `benchmarks.md` §8 and `bench/sdcpp-t2i-{summary,compare}-2026-10-19.md`. Lesson for the
    native engine: ComfyUI-identical pictures need ComfyUI-identical conditioning; the encoder
    path is ours to port, not to borrow.
  - **SAM 3 ONNX on Hugging Face (for Phase 1)**: no HF token on this PC and `facebook/sam3.1`
    is gated, but **`Comfy-Org/sam3.1` is not** (the same `sam3.1_multiplex_fp16.safetensors`
    we already have), and community SAM 3 exports exist: `kishanstar2003/SAM3_ONNX_FP16`
    (vision 1.76 GB, text 1.35 GB, geometry 31 MB, decoder 92 MB, tokenizer; Meta SAM 3 licence;
    downloaded to `_eval\sam3-onnx-fp16`), `onnx-community/sam3-tracker-ONNX` (vision + prompt
    encoder/mask decoder, the point/box tracker), `danilobukvic/sam3-text-onnx` (text path with
    its export and validation scripts, saved to `_eval\sam3-export-scripts` as the reference for
    our own 3.1 export later), `wkentaro/sam3-onnx-models-v0.3.0` (image/language encoders +
    decoder). ONNX Runtime 1.31.0 ships a **CUDA 13** Windows GPU build (295 MB) that pairs with
    the toolkit here, and cuDNN 9 already lives in ComfyUI's torch (`torch\lib\cudnn*64_9.dll`,
    `cudart64_13.dll`); the `ort` crate is at 2.0.0-rc.15.
  - **Phase 1, SAM 3.1 through ONNX Runtime, at parity (2026-10-19; `genai-native` `6e87b2a`,
    `b1642d5`; `gn-perception`).** The runtime is loaded like the weights: `ort` with
    `load-dynamic`, `init_from(<dir>/onnxruntime.dll)`, the CUDA and cuDNN libraries preloaded
    from directories the caller names (here scoop's `cuda\bin\x64` and ComfyUI's `torch\lib`;
    nothing from `PATH`). Two findings on the way: cuDNN 9.24 loads
    `cudnn_engines_tensor_ir64_9.dll` by name and `ort` rc.15's preload list predates it (first
    convolution failed with `CUDNN_STATUS_SUBLIBRARY_LOADING_FAILED`), so every `cudnn*_9.dll`
    in the named directories is preloaded; and the "FP16" community set is fp32 inside (header
    census). The CLIP tokenizer is the `tokenizers` crate without its C parts (`fancy-regex`),
    with ComfyUI's 32-token window (start, ≤ 30 pieces, end, zero pad; the mask through the end
    token). **Round one, the community `kishanstar2003` SAM 3 export** (detector only):
    11 of 16 cases at parity, every text prompt at bounds IoU 0.98–1.00 even across SAM 3 vs
    3.1 weights and without the node's refinement, box prompts split, point prompts fail (the
    detector's geometry encoder is not a point segmenter). **Round two, our own export of
    ComfyUI's SAM 3.1** (`tools/export_sam31.py`, run with ComfyUI's Python against the user's
    `sam3.1_multiplex_fp16.safetensors`; `torch.onnx.export(dynamo=True)`, opset 18, fp32,
    `in_training = True` so RoPE takes ComfyUI's plain-torch path instead of its `comfy_kitchen`
    op, which has no ONNX translation): five graphs (`vision` 1.87 GB with the detector FPN, the
    tracker's interactive FPN and the geometry CLS token in one trunk pass; `text` with the
    language resizer; `geometry` for boxes; `decoder`; `heads` = the tracker's prompt encoder +
    mask decoder returning all four mask tokens, IoUs and the object score, taking a 1008² mask
    prompt and a `has_mask` switch so one graph serves points, boxes and mask refinement). The
    small graphs verified against torch in the exporting process (text 4e-6, geometry 1e-4,
    decoder 2e-2, heads 5e-3 max abs). `gn_perception::sam31` then reproduces the `SAM3_Detect`
    node step by step (ComfyUI's unnormalised 0..1 input, prompt = text ⊕ boxes ⊕ CLS,
    `sigmoid(score) > 0.5` and the best 1, the 288 → picture → crop → 1008 interpolation chain
    with the node's `int()` truncations in f64, two tracker passes on a 10 %-padded crop, OR with
    the coarse mask; points as `[pts, pad]` with the best of three multimask heads for a single
    point, boxes as `[pad, tl=2, br=3]`, the −1024 no-object rule). **16 of 16 at parity**
    (`bench/sam-ort-comfy-compare-2026-10-19.md`): bounds IoU 0.99–1.00 everywhere; the fox
    masks within 0.03 % of the reference pixel counts (point 344 133 vs 344 125, text 344 151 vs
    344 045, box 142 239 vs 141 856). Engine times, fp32 through ORT's CUDA provider after a
    one-time autotune: vision 470 ms, text route 750 ms (text 209 + decoder 146 + crop vision
    264 + two heads 109), box 140 ms, point ≈ 200 ms warm (the 1.4 s first heads call is cuDNN
    plan building); ComfyUI fp16 does the whole text node in 154 ms and a point in 94 ms, so the
    levers are an fp16 export, keeping the FPN tensors on the device (IoBinding) and CUDA
    graphs; parity came first. Harness: `gn-bench run --engine sam-ort` (`SamOrt` drives
    `gn-sam run`, whose JSON line has PhotoshopEX's `select.*` shape). Also learned: ComfyUI's
    node sends points and boxes through the tracker heads and text through the detector, and
    refines detector masks through the tracker; the Hugging Face layout stays in `sam3.rs` as a
    bring-up fallback. Python additions to ComfyUI's embedded interpreter for the export:
    `onnx` 1.23.2, `onnxscript` 0.7.2, `onnxruntime` 1.31.0 (CPU, verification only).
  - **The app runs SAM 3.1 natively (same day; `genai-native` `2b2b141`).**
    `gn_perception::engine::PerceptionEngine` is the worker-side `Engine` (feature `engine`):
    it answers `sam3.1/segment`, `-point` and `-box` with the params PhotoshopEX's `select.*`
    commands already send (`threshold`, `points` / `negatives` as JSON strings of `{x, y}`,
    `bx by bw bh`), loads the runtime and the graphs on the first request, keeps them resident,
    caches the last picture's embedding by content hash (the probe that follows the pointer pays
    only the prompt), returns one 1-channel coverage blob per instance, and maps every failure
    to the protocol's `ErrorKind`s. PhotoshopEX: `engine_main` serves it when the worker gets
    `--runtime <dir;dir> --models <dir>` (the fake engine otherwise); two preferences,
    `integrations.runtimePath` (`;`-separated folders holding ONNX Runtime and the CUDA / cuDNN
    libraries; nothing from `PATH`) and `integrations.modelsPath` (the model root, `sam3.1/`
    inside), become those flags through `native::worker_args`; `generate_cmds::native_backend`
    keeps one `NativeBackend` per flag set and stops the worker when the folders change
    (tests route through `NATIVE_OVERRIDE` instead). `select_ml_cmds.rs` is untouched: a
    coverage blob arrives as the grey picture the ComfyUI route delivers. Labels "Native runtime
    folders" / "Native models folder" ×13; `architecture.md` rows; README how-to. **Live through
    `photocraft-cli run` with `engine = native` and the two folders set**, on the fox:
    `select.byPoint` [69, 123, 747, 877] / 344 133 px in 4.4 s (sessions + first-call autotune),
    `select.byText "fox"` [75, 123, 741, 877] / 344 151 px in 0.51 s, `select.byPoint` with a
    box [90, 125, 617, 759] / 142 239 px in **34 ms** (the cached embedding: this is the probe's
    cost from now on), `generate.health` = `native/ort` on `cuda:0`; the reference bounds exactly,
    the pixel counts within 0.03 %; no worker left behind. Tests: genai 25 + 12 + 3, engine
    native/prefs/select/generate subset 88, `prefs_usage` 4, clippy clean. Model files live at
    `_eval\native-models\sam3.1` on this PC.
  - **fp16 and warm numbers** (`export_sam31.py --fp16`: vision, geometry, decoder and heads
    traced in half precision with fp32 inputs and outputs, the text graph fp32; `gn-sam run
    --repeat 3` for warm timings; `genai-native` `b5d7104`, `98fb1a7`): 16/16 at parity again
    (`runs/2026-10-19-sam-ort-comfy-fp16`, fox text 344 142 px). Warm, fp32 → fp16: vision
    198 → **133 ms**, text route after the embedding 276 → **202 ms**, point / box **22 ms**
    (the first call in a process pays 0.3–0.5 s of cuDNN planning once). Against ComfyUI's node
    (94 ms point, 154 ms text, trunk every time) a prompt on an embedded picture is 4× faster,
    a fresh picture 1.6–2.2× slower: ORT runs the exporter's unfused attention. Opset 23 emits
    the fused `Attention` op but ORT 1.31's CUDA provider took 2.7 s on it; dropped. The fp16 set
    is now `_eval\native-models\sam3.1` (fp32 beside it as `sam3.1-fp32`). `benchmarks.md` §8.
    In the app with the fp16 set (`photocraft-cli run`, same fox): point 344 125 px (the
    reference's count exactly), text 430 ms, box 33 ms, point with a negative on the tail
    [90, 125, 724, 873] / 287 133 px against the reference's 287 283, a second point 30 ms.
  - **Object Selection with the native engine**: `server_configured` is engine-aware (under
    `native` it asks for the two folders, not a ComfyUI URL), the finder's online check is keyed
    by engine + folders, and the rest delay before a probe drops from 0.25 s to 0.08 s when the
    engine is native (the worker keeps the picture's embedding, so a probe is ≈ 22 ms).
  - **`auto` routes SAM 3.1 natively** (plan §3.3's rule applied for the first time):
    `generate_cmds::backend_for(s, template)` answers per template: `native` and `comfy` as the
    preference says; under `auto` the native worker when the two folders are set, the template's
    family is `sam3.1` and the worker's index lists it, the ComfyUI route otherwise. Native runs
    remember their model set under their own key (`native`), so they never count as a model
    switch that would purge ComfyUI. `select_ml_cmds` (`run_select`, `run_probe`) asks for the
    template's backend; `generate.health {template}` reports the backend one template would use,
    and the selection tools' online check asks about `sam3.1/segment-point`, so Object Selection
    works under `auto` with ComfyUI off and the worker on (`server_configured` / `native_engine`
    follow the same rule). The engine test extends to `auto` (no folders → ComfyUI; folders →
    `select.byText` through the fake worker). A first attempt let the test override stand in for
    the folders and every `select_ml` test in the binary went native: the opt-in is the
    preferences, as in production.
    Live with the real binaries and ComfyUI stopped, `engine = auto` + the two folders:
    `select.byPoint` native (344 125 px), `generate.health {template: sam3.1/segment-point}` =
    `native/ort`, `{template: krea2-turbo/image}` and plain health = ComfyUI (unreachable, as it
    should say). **The real preferences now carry the two folders** (`%APPDATA%\Photocraft\
    preferences.json`, backup `.bak-2026-10-19`; `engine` left at `auto`), so the pinned app
    selects with native SAM 3.1 and still fills and generates through ComfyUI.
  - **Phase 1's "done when" (plan §6.4), ComfyUI stopped**: `object-select-live.ps1 -Native`
    (the switch sets the two folders in the script's own session; `engine` stays `auto`) passes
    all five steps through the native worker: the finder's probe 3.3 s after the pointer rests
    (worker start + sessions + the first-call autotune; later probes are the 22 ms kind), the
    click applies it without a new detection (bounds 75,123,742,878), Alt-click refinement
    excludes the tail (90,124,726,876), the box prompt selects the head, Select › Subject runs
    through `select.subjectML` natively. Screenshots in the session's scratch folder.
  - **Device-resident features and fused attention** (`genai-native` `5202e61`): ORT's per-node
    profile of the trunk (`gn-sam --profile`) showed ≈ 37 ms of node time inside an 82 ms run and
    2 010 kernels, i.e. launch-bound, plus 25 ms of copying 220 MB of outputs to the host and
    ≈ 10 ms per prompt uploading them again. Two changes: the vision session runs through an
    `IoBinding` with its seven feature maps bound to the device's memory (they stay there as
    `Tensor<f32>` values in the `Frame`; the decoder, geometry and heads sessions read them in
    place; only the 256-number CLS token comes to the host), and `export_sam31.py --mha`
    translates `scaled_dot_product_attention` to ORT's contrib `MultiHeadAttention` (a custom
    translation table; the contrib op's schema is registered on the fly so onnxscript resolves
    it). ORT's offline transformers optimizer could not fuse the attention itself (RoPE and the
    window layout defeat its patterns; it fused BiasGelu and SkipLayerNorm only, no gain).
    Warm: **vision 66 ms** (was 133), **heads 10 ms** (was 22), text route after the embedding
    **132 ms** (was 202); a fresh picture by point 79 ms against the node's 94, by text 198
    against 154. 16/16 at parity again (`runs/2026-10-19-sam-ort-comfy-fp16-mha`). The fused
    fp16 set is now `_eval\native-models\sam3.1`; PhotoshopEX pinned to `5202e61`.
  - **CUDA graph for the trunk: tried, parked** (`genai-native`, next commit after `5202e61`):
    the vision session can be opened with `enable_cuda_graph`, fixed device buffers for its
    input and eight outputs, and replayed (`Sam31::open_with(…, true)`, `gn-sam --graph`), but
    ORT 1.31's CUDA provider fails the capture on this model ("operation failed due to a previous
    error during capture") with the exporter's attention and with the fused one, with cuDNN's
    exhaustive search on and off. The code falls back to the plain session automatically (one
    slow picture, 1.4 s) and is **off by default**; the remaining ≈ 20 ms of launch overhead in
    the 56 ms trunk run waits for a later ORT or for our own kernels.
  - **Tracker-only vision graph for the crops: measured, left out.** `export_sam31.py --parts
    tracker` writes `vision-tracker.onnx` (trunk + tracker FPN) and `sam31` uses it for the
    refinement crops when present: 66 → 61 ms per crop, for 0.9 GB more on disk and in VRAM (the
    trunk is the cost either way). Not in the default set. `gn-perception/docs/EXPORT.md` holds
    the export, licence and reproducibility notes.
  - **Open after this**: a Preferences screenshot scrolled to the two new fields (the page
    renders; the fields sit below the fold at 900 pt).
- **Plan Phase 2 opened (2026-10-19, evening): qwen-image-rs runs on this PC, imported as
  `genai-native/crates/qir`.**
  - **Weights**: the official `Qwen/Qwen-Image-2.1` snapshot (ungated, Qwen Research licence;
    7.1 B parameters: DiT ≈ 14 GB bf16, Qwen3-VL-8B text encoder ≈ 16 GB, the VAE) downloaded
    to `_eval\weights\qwen-image-2.1` in 6 min (31 GB; `huggingface_hub` from ComfyUI's Python).
  - **The ComfyUI reference for the 2.1 template** (`gn-bench run --engine comfy --template
    qwen-2.1/image --pref integrations.allowResearchModels=true`, a new `--pref` flag): 28/28,
    4.4–5.0 s engine time per 1024² picture (25 steps, euler/simple, CFG 1, the INT8 ConvRot
    repack), `references/2026-10-19-comfy-t2i-qwen21`.
  - **Building qwen-image-rs on Windows** took four environment fixes and no source change
    (table in `dev-environment-windows.md`): `CUDARC_CUDA_VERSION=13030` (cudarc 0.19 stops at
    13.3), `NVCC_APPEND_FLAGS=-Xcompiler /Zc:preprocessor -Xcompiler /MD -std=c++17` (CUDA 13.4's
    CCCL under MSVC; the dynamic CRT so candle-kernels' `libmoe.a` links beside `cc`'s objects,
    after `cargo clean -p candle-kernels --release`), the VS Installer folder on PATH before
    `vcvars64.bat` (vswhere), and no quoted `NVCC_PREPEND_FLAGS` (nvcc keeps the quotes, "Access
    is denied"). Features `cuda,convrot,sage,sage2,fusednorm`, `CUDA_COMPUTE_CAP=120`, CUTLASS
    from a shallow clone; no cuDNN (the wheel has no import library; candle's own conv serves).
  - **First picture on the 5090** (`generate`, the fox prompt, 1024², 25 steps, seed 7, CFG 1,
    `--convrot`): **0.14 s per step, 25 steps in 3.5 s**, VAE decode 2.1 s (tiled: the bf16
    text encoder's 16 GB left 11 GB free), 45 s wall of which ≈ 40 s loading (16 GB text encoder
    from disk, the DiT rotated and quantised on load because the cache write failed:
    `sync_all` on a read-only handle is "Access is denied" on Windows, fixed in the import). The
    per-step time equals the authors' 4090 number, i.e. the kernels run correctly on sm_120 but
    nothing is Blackwell-tuned yet (plan risk 1); ComfyUI's INT8 ConvRot + SageAttention path
    does 25 steps in ≈ 3.7 s on the same card, so the Python-free port is at speed parity with
    ComfyUI today and not ahead. The fox is photographic (scratch `qir-out/fox-s7.png`).
  - **The square t2i cases through it** (`gn-bench run --engine qir`, a new runner; 24 of 28,
    the four non-square sizes refused since its CLI takes one `--size`): every picture correct,
    0 of 24 at seed-level parity (PSNR 5–17 dB, SSIM 0.1–0.7: candle's CUDA RNG and diffusers'
    schedule against ComfyUI's CPU noise and `simple` sigmas, as the review predicted), 26–40 s
    wall each (loads every time; the engine's residency is Phase 2's first saving).
  - **Imported** (`genai-native` `1180ef2` by `git subtree`, history kept; `be7bfcf`): a workspace
    member with its own Apache-2.0 licence and lints, a Windows-aware `build.rs` (conforming
    preprocessor, the target's CRT from `CARGO_CFG_TARGET_FEATURE`, no rpath), `tokenizers`
    without `esaxx_fast` (its C++ is built with the static CRT whatever the target and cannot
    link beside the CUDA objects), the cache-write fix; the root `NOTICE` lists the import, the
    vendored SageAttention, the SGLang reference file, CUTLASS, cuDNN, candle.
  - **A stall and its cause.** Built inside the workspace the same crate sat at "denoising
    step=0" for minutes at 100 % GPU, alone, with and without the cache, with one codegen unit,
    with cudarc pinned to upstream's 0.19.9 (the standalone build in `_eval` kept working). The
    difference was the workspace profile's `debug = 1`: the `cc` crate turns it into nvcc `-G`
    (device debug, every optimisation off) for the kernels it compiles. `build.rs` now sets
    `.debug(false).opt_level(3)` whatever the profile (`genai-native` `dfd35b4`). With that and
    the cache write fixed, one process per picture is **11–13 s**: cache hit, DiT loaded in 3 s,
    the bf16 text encoder in ≈ 5 s from the page cache, **25 steps in 2.9 s (0.116 s/step)**,
    VAE decode 0.43 s untiled. Lesson for `gn-kernels`: device code gets its own flags, never the
    Rust profile's.
  - **Next (plan §7.1)**: kernels into `gn-kernels` behind the `Kernels` trait with prebuilt
    fatbins, the DiT/encoder/VAE into `gn-blocks` + `gn-models/qwen_image_21.rs`, ComfyUI's
    `simple` schedule and CPU noise for seed-level parity with the reference, residency in the
    worker (no per-call loads), then the Blackwell tuning pass.

## 2026-10-16: the native-inference question, Quality mode, and the Object Selection tool's SAM 3.1 mode

- **Research** (five researchers, report in `docs/comfy/research/Native inference for PhotoCraft on RTX
  5090.md`, notes in `research/notes/`): no Rust runtime runs this model set; candle reaches ~0.65× ComfyUI's
  bf16 speed stock, burn's attention kernel is an open blocker, TensorRT has a FLUX path only, and
  in-process Python breaks the no-crash rule. Recommended: keep the ComfyUI protocol as the backend
  trait, ship an app-managed sidecar for zero setup, go native per model only after same-machine
  benchmarks. **NVFP4** is a GEMM and VRAM lever (1.6–2.6× per DiT step, 1.2–1.8× per image on a
  5090, with a quality cost under post-training quantisation), not a general speed-up; Lightning
  distillation dwarfs it. The owner chose **highest quality first, then optimise**.
- **Quality** (`integrations.quality`: `best` default, `balanced`, `fast`): what `auto` optimises
  for. `generate_cmds::auto_order(command, quality)` replaces the fixed orders (`best` = the
  full-step base templates; `balanced` = the 8-step Lightning tier first, the old behaviour;
  `fast` = the 4-step fill tier first), `likely_auto` takes the quality, and `backend()` keeps
  `modelPrecision: auto` off the NVFP4 files under `best`. A Best / Balanced / Fast menu sits beside
  the model in the Generate panel (writes the preference); the picker's Auto row follows.
  `generate.models` reports `quality`. Ten engine tests that pin the Lightning tier now set
  `balanced` in their session helpers.
- **Object Selection with SAM 3.1** (`object_select.rs`, `select_ml_cmds.rs` reworked):
  - **Object finder**: the pointer resting a quarter second over the picture runs `select.probe`
    (new: a `jobs::run` without a document lock, not journaled, no Generations row, quiet in the
    status bar; the request goes out at ≤ 0.6 MP) and the answer tints the object in the accent
    colour; the click applies the kept probe (`select.byPoint {probe}`) with no second detection
    while the picture and its history are unchanged (probes live in `Session::select_probes`, eight
    kept). The health check behind it is cached ten seconds **per server URL** (a first version
    cached per app and sent clicks to a dead server after the preference changed).
  - **Box prompt**: a drag is `select.byPoint {box}` through the new `sam3.1/segment-box` template
    (stock `PrimitiveBoundingBox` → `SAM3_Detect.bboxes`); `sam3.1/segment-point` gained
    `negative_coords`. Checked live before writing them: an empty `[]` list is accepted, a
    negative point trims the mask, a box around the whole fox finds it as the point does.
  - **Refine mode** (the options bar's New | Refine): a click adds a spot the last object must
    include, ⌥-click one it must not; the last query runs again with every point so far after
    `edit.undo` takes its result back, so one undo step stays. Ctrl was tried first and is the Move
    tool's override, hence a mode. **Refine edge** (on by default) runs the detector's hard mask
    through the classical edge refinement, as Remove Background does; Select › Subject and the
    Properties' Select Subject go through the detector when it answers.
  - Object Selection no longer snaps its presses (`snap_ui`): a prompt snapped to a selection edge
    landed off the object (a test caught it as a 20,20 click arriving as 23,13).
  - Automation: `select.byPoint {points, negativePoints, box, refine, probe}`, `select.probe`,
    `ui.inspect.objectSelect`, `ui.set {toolOptions}` (any field, spelled as inspect shows it),
    `ui.click` / `ui.move` modifiers, `ui.inspect.canvasRect` + `activeDocument` for document →
    screen mapping in scripts.
- **Live** (`docs/comfy/bench/scripts/object-select-live.ps1`, release GUI, SAM 3.1 warm): the
  finder's probe for the fox arrived 0.8 s after the pointer rested; the click applied it with no
  new detection (selection 75,123 742×878); Refine's ⌥-click on the tail re-ran with the negative
  point (344k → 289k pixels in a CLI check: the tail region went); a box around the head found a
  patchy face region (39 % of its box filled at any threshold; `refine_iterations` 0 gives 51 %,
  5 gives 13 %); Select › Subject ran `select.subjectML`. **Model limits seen**: SAM 3.1's box
  prompt is good at framing a whole object and poor at cutting a part out of one (the head of a
  fox), and a negative point on a whole limb trims rather than removes it; the Refine mode's
  islands in the screenshot are SAM's own low-confidence pixels, not the edge refinement.
- **Tests**: engine `select_ml_cmds_tests` (+2: box / negatives / refine; probe kept, applied,
  stale, unknown, nothing-found, downscaled) and the quality test (auto orders, Blackwell NVFP4
  under best vs balanced, `generate.models.quality`); UI `object_select_tests.rs` (3: box prompt,
  Refine mode and its undo depth, mode switching; the finder in a kittest harness with the inline
  fake; no server and the menu's Select Subject) and `retouch_ui`'s updated test. Gates: clippy,
  fmt, layers, wasm, parity, scorecard 57 of 156 unread (the new preference is read).
- **Select Subject, after the owner said it "sucks"**: a sweep of seven phrasings over seven
  pictures (through the CLI) showed "the main subject" finding the fox, a sideways face, a
  balloon and a lighthouse tower, while "subject" alone failed on three pictures with the
  server's raw `SaveImage: index 0 is out of bounds` (an empty mask batch), and "person" on a
  seascape returned an 83-pixel speck. So `select.subjectML` now asks in turn: "the main
  subject", "the foreground object", "person", "animal", "object" (one detector call each,
  ~0.5 s warm, stopping at the first hit); instances under 2 % of the largest are dropped unless
  one is asked for by number; the server's empty-batch error reads as "nothing matching"; and
  every Select Subject entry point (the Select menu, the Properties quick action, both options
  bars) goes through the detector with Refine edge. Before this build the menu and the Quick
  Selection bar still ran the classical `select.subject`, which on the lighthouse picture selects
  the whole lower half: most likely what the owner saw. The fake server gained `segments_for`
  (per-prompt answers) to test the chain.
- **Quick Selection through SAM 3.1** (owner: "make quick select use sam also"): a stroke of the
  Quick Selection tool, when the server answers, is one `select.byPoint` with at most 4 spots
  spread evenly along the stroke's samples (`retouch_ui::stroke_prompt`: whole pixels inside the
  canvas, no repeats), `mode` add (⌥: subtract, so the object under the stroke comes out) and
  `refine` = the bar's Enhance Edge; the classical `select.quick` stays for no server, a server
  that is down, or a stroke off the canvas. The options bar says "Paint over an object to select
  it" when a server is configured (×13). Tests: `quick_selection_strokes_ask_the_detector_when_a_server_answers`
  (fake server; no server / down server → `select.quick`) and `stroke_prompt_spreads_a_few_whole_pixel_spots_inside_the_canvas`.
  The cap was 16 first; the live script (`bench/scripts/quick-select-live.ps1`) showed a hatched,
  fragmented fox, and a CLI sweep of the same stroke with 1, 2, 4, 8, 16 spots gave 344 k, 335 k,
  337 k, 292 k and 164 k pixels: SAM 3.1's mask degrades as positive points pile up, so 4 it is
  (whole fox, 0.9 s warm; a second stroke adds, an ⌥ stroke over the tail subtracts it).
- **One checkout from now on.** The owner had been testing the taskbar-pinned app, which was
  `C:\Users\5090\Projects\comfy-photocraft\target\release\photocraft.exe` built 2026-10-15 02:12,
  i.e. none of this week's work; that explained "Select Subject is really bad". PhotoshopEX
  (`C:\Users\5090\Documents\PhotoshopEX`, branch `photoshopex`) is the main line; the other
  checkout was fast-forwarded to the same commit and its three uncommitted local-build tweaks
  (wheel = zoom default, no Discord button in the title bar, the Ps-style tile in
  `assets/app-icon/ps/`, from its 2026-10-15 entry below) were ported here by patch. The pin now
  points at this checkout's binary. The old checkout is to be deleted by the owner.
- **Open**: the sidecar (report step 2) is not started; SAM's box-on-a-part weakness could be
  helped by sending the box crop at full resolution; the finder could pre-warm the checkpoint on
  tool selection (the first probe after a cold server takes ~19 s, the model load).

## 2026-10-12 (night): plan Phase 5, the finish (hub, alerts, saving, Angles) and the walkthrough

Built [`plan-artcraft-experience.md`](plan-artcraft-experience.md) §7 and ran its §9 walkthrough.

- **Generate hub** (`generate_hub.rs`): eight cards (Create: Image; Edit: Fill, Edit, Expand,
  Remove Background, Split into Layers, Select by Text, Angles) under the Home screen's recent
  files and in **Help › Generative Features…** (`help.generativeFeatures`, first in Help). A card
  opens the Generate panel in its mode or runs its menu command; an Edit card with nothing open
  shows the open dialog and carries on once the picture is open (a cancel leaves nothing waiting).
  The Home card scrolls on a short window. Window › Generate now also focuses the prompt.
  `ui.inspect.hub` lists the cards with their drawn rects.
- **Alerts** (`alerts.rs`): `integrations.alerts` (`off` default, `done`, `doneAndFailed`) picks a
  synthesized blip in `generations_panel::on_event` (880 → 1320 Hz done, 220 Hz failed, no sound
  files). Playback is the `alerts` cargo feature (rodio 0.20.1, a new dependency; one blip at a time
  on its own thread). It is not a default feature, because CI builds the workspace on Linux without
  the ALSA headers, so "on in release" means the Windows and macOS packaging scripts build with
  `--features heif,photocraft/alerts`; the Linux tarball, Flatpak (offline vendored crates) and
  FreeBSD packages leave it off for now (ALSA would be a new build and runtime dependency there).
- **Saving results**: `integrations.saveResultsDir` (empty = ask) and `saveResultsNaming`
  (`{model}_{date}_{seed}`, validated: at least one token, no folders) are what `generate.export`
  uses without `dir` / `name`; automation may not set the folder (`workspace.rs`, however the path
  is written). Window › Generations gained a select mode (✓ in the header, or Select in a card's
  menu) with **Save selected**.
- **Generative Angles**: the research step found fal's Multiple Angles LoRA for
  Qwen-Image-Edit-2511, Apache-2.0, with the authors' own ComfyUI graph (stock nodes only, checked
  against our server's `/object_info`). Template `qwen-edit-2511/angles` (angles LoRA, then the
  4-step Lightning LoRA, ModelSamplingAuraFlow 3.1, CFGNorm, 4 steps CFG 1; page `angles` so no
  other picker lists it); `generate.angles {rotation, tilt, zoom, seed, variations}` builds the
  LoRA's `<sks> [azimuth] [elevation] [distance]` prompt and runs Generative Edit with the new
  `wholePicture` option (a selection does not mask a camera move); Edit › Generative Angles… opens
  `angles_ui.rs`: an orbit widget (the subject, the ground ring with 8 stops, the camera dragged
  around and up, snapping to the LoRA's 96 cameras), Direction / Height / Distance / Variations,
  Generate with its estimate, and a warning naming the LoRA when the server lacks it. **Not run
  live**: the LoRA is not on the dev server and the app downloads no weights; tested against the
  fake (the graph, the words, the layers in the Variations strip).
- **Live walkthrough** (`docs/comfy/bench/scripts/walkthrough-live.ps1`, release GUI with
  `alerts`, ComfyUI up), all ten steps of §9: Essentials shows both panels; Image × 4 with Krea 2
  Turbo into a new document in 17.5 s (estimate ~22 s), four tiles, a tile click and Keep leave one
  layer; Fill "a red kite" 15.7 s with the task bar away; Edit "make the sky stormy, keep the kite"
  with the kite layer as reference 18.5 s; a dead server URL gives a `serverUnreachable` row and
  Recreate succeeds; Place after close; the CLI's export writes
  `krea2-turbo-image_2026-10-12_061608_<seed>.png`; after a relaunch 4 completed and 1 failed rows
  with 4 thumbnails; the Home Image card and Help's Expand card set the panel's mode; Enter to
  generate, `lastAlert = done`, the strip at the bottom; CLI and GUI both list 6 records.
  Screenshots (and the Phase 3/4 ones) in `C:\Users\5090\Documents\PhotoshopEX-walkthrough-2026-10-12`.
- **Fixes on the way**: hub card sentences were clipped mid-line (now two lines with an ellipsis,
  cards 64 pt); "CREATE"/"EDIT" and Russian "Direction"/"Distance" were missing from catalogues
  (`every_tl_literal_is_translated` caught them).
- **Tests**: `generate_hub_tests.rs` (4), `angles_ui_tests.rs` (3), `alerts.rs` (2), the
  Generations select mode and alert choice (2), engine export preferences and Angles (graph,
  words, validation, 96 cameras), automation's save-folder refusal. Gates: clippy (also with
  `alerts`), fmt, layers, wasm, parity (the two menu items), scorecard 57 of 155 unread.
- **Open**: Angles needs a live run once the LoRA is installed (and its estimate a measurement);
  Linux packages without alert sounds; the hub's Edit cards cannot be driven by scripts with
  nothing open (they show the native open dialog).

## 2026-10-12 (evening): plan Phase 4, the Variations strip

Built [`plan-artcraft-experience.md`](plan-artcraft-experience.md) §6.

- **`variations_strip.rs`** (new): the active document's last five generative runs as 56-pt tiles
  at the canvas's right edge (`integrations.variationsStrip`: `right`, `bottom`, `off`; new
  preference, read by the strip). The plan suggested a new `app.ui.bundles` map fed by job
  events; the runs are instead read from the Generations library each frame (records whose `doc`
  is the active document, fill / edit / expand / image / similar, with the result layers that
  still exist). Runs started by a dialog, a script or the CLI-shared library show too, and undo,
  delete and close need no bookkeeping. A tile is the layer's pixels over the run's area (cached
  by pixel fingerprint, four builds per frame). Click shows it (`generate.variation`), × drops it,
  ✓ on the shown one keeps it, ✦ in the header runs Generate Similar for as many again; a running
  job shows placeholders with a sheen and its percent; a failed one a red tile with the reason,
  Retry (`generate.recreate`) and Dismiss.
- **Engine**: `generate.dropVariation {layers, index}` (deletes one; when it was on screen and the
  others are hidden the next one shows and becomes active) and `generate.keepVariation {layers,
  index}` (keeps one, deletes the others), one undo step each; the variation commands share their
  parsing and now refuse a layer listed twice.
- **Task bar**: the 1/n switcher, its `results` / `shown` state and `show_variation` are gone
  (the strip replaces them; `ui.inspect.generativeBar` no longer reports `results`).
- **No modal over the canvas**: a fill started from a dialog or a script showed the modal progress
  dialog in the middle of the canvas. It now stays away while the strip draws the job's
  placeholders, the task bar shows the job or the Generate panel shows its Cancel (that last case
  was a Phase 2 gap).
- **Automation**: `ui.inspect.variations` (placement, visibility, runs with status, layers,
  shown, failure, and the drawn rects of tiles, ×, ✓, ✦, placeholders, Retry, Dismiss).
- **Live** (`docs/comfy/bench/scripts/variations-live.ps1`, release GUI, ComfyUI up): a fill with
  four variations over open water ran in the background in 33.8 s; four placeholders and no modal
  while it ran, then four tiles each with a different red sailboat; `ui.click` on the third tile
  showed it in one undo step ("Variation 3"); ✓ left one layer (5 → 2 layers with the
  background). One run showed the tool switched from Brush to Path Selection after the click; it
  did not reproduce (the history showed only "Variation 3"), the window was on the desktop, and a
  whole-app test now checks that a tile click reaches nothing but the strip.
- **Tests**: `variations_strip_tests.rs` (5: tiles, click / × / ✓ each one step and undo; running
  placeholders then tiles, the modal exemption, a failed run's Retry and Dismiss; bottom and off;
  Similar adds a run, a text selection stays out, five runs at most; the whole-app click), engine
  `dropping_and_keeping_variations_are_one_undo_step_each`, the bar tests read the strip. Gates:
  clippy, fmt, layers, wasm, parity unchanged, scorecard 57 of 152 unread.
- **Open**: a run's thumbnails show the layer unmasked over its padded area (the request
  rectangle), not the masked composite; pending tiles of `generate.image` into a new document sit
  on the document it started from.

## 2026-10-12 (later): plan Phase 3, the model picker

Built [`plan-artcraft-experience.md`](plan-artcraft-experience.md) §5.

- **`model_picker.rs`** (new): one pill + popover for the Generate panel (`generate-panel`), the
  task bar (`generative-bar`) and the generated dialogs' `template` field (`flt-<command>`, rows
  carried as `__models` in the dialog's fields). Data-driven: callers pass `model_picker::Item`s
  built by `generate_panel::picker_items` from `generate.models`, so the plan's
  `pill(ui, app, page, …)` became `pill(ui, id, items, current, auto, group, width)`. Family glyphs
  from the plan's colour map; newest first by a numeric-aware compare; family submenus from eight
  rows when the new preference `integrations.groupModelsByFamily` (default on) allows; badges for
  the estimate, `research` (greyed, not selectable until `allowResearchModels`) and
  `not installed` (selectable). The Auto row names what it resolves to ("Uses: Qwen-Image-Edit-2511,
  Lightning 8 steps"; the preference's default first, as the engine does). Rows whose short names
  collide show full names (both Krea text-to-image templates shortened to "Krea 2 Turbo").
- **The probe moved**: Phase 3 first made loading the catalogue probe the server. The panel is on
  screen from startup, so every launch contacted ComfyUI and left "Checking generative models" in
  the status line; three UI tests caught it (a job they did not start). Now the catalogue loads
  offline, the bar probes when it opens (as before Phase 3), a picker probes when its popover
  first opens (`generate_panel::probe_once`), and a finished `generate.models` job stays out of the
  status line (`jobs_ui.rs`, one arm).
- **One-row form**: at 1440×900 the panel stays in its one-row form even with Generations hidden,
  which hid the picker, so that form now carries the pill as the glyph alone (30 pt).
- **Automation**: `ui.inspect.modelPickers` lists the pickers drawn with their rect, label and
  whether the popover is open; the pill and rows have accessibility labels ("Model: Auto").
- **Live** (`docs/comfy/bench/scripts/model-picker-live.ps1`, release GUI, ComfyUI up): each
  popover opened through `ui.click` on its inspected rect and screenshotted. These were checked:
  Fill lists the five fill templates with estimates (~6 s to ~54 s) and Qwen-Image-2.1 greyed
  `research`; Image marks the Krea style template `not installed` (its LoRA is absent); the task
  bar and the Generate Image dialog show the same component.
- **Tests**: `model_picker_tests.rs` (6: grouping threshold and preference, numeric newest-first,
  research gating and badge text, display names, every shipped family has a glyph, the panel's
  rows and the probe against the fake, clicking through the popover with kittest); the one-row
  panel test checks the glyph pill; `jobs_ui_tests` checks the quiet background check. Gates:
  clippy, fmt, layers, wasm, parity unchanged, scorecard 57 of 151 unread (the new preference is
  read). Workspace: the nine known environment failures plus the scorecard staleness test, which
  passes after regenerating `docs/scorecard.md`.
- **Open**: the dialogs show no estimates (their size lives in the dialog's own fields); the
  popover grows to the longest name (about 600 pt for the mask-guided fill).

## 2026-10-12: plan Phase 2, the Generate panel and reference images

Built [`plan-artcraft-experience.md`](plan-artcraft-experience.md) §4.

- **Template capabilities** (genai `template.rs`): `creator`, `description`, `badges`, `pages`,
  `references {max, min, placeholders, nodes}`, `sizes`, `counts`, `estimatedSeconds {base,
  perMegapixel}` on every shipped template (values from `benchmarks.md`), `Meta::estimate_seconds`.
  Reference placeholders a request leaves unbound are pruned with their nodes and the inputs that
  linked to them, so one graph serves zero to N references.
- **References**: `Request.references` (checked against min/max before any network call,
  uploaded, bound in order). The 2511 edit templates take two (`image2`/`image3`, wired as the
  official graph does). New templates, each checked against the live `/object_info`:
  `qwen-edit-2511/image-ref` (permissive: 1–3 references into an empty latent of the asked size),
  `krea2-turbo/image-style` (the official style-reference graph with Ostris's LoRA, Krea 2
  Community licence per its Hugging Face metadata; not run, the LoRA is not installed),
  `qwen-2.1/edit` (research opt-in, up to nine references). Engine: `references` on
  `generate.edit` / `generate.image` as `{"layer"}` / `{"path"}` / `{"job","index"}`
  (`generate_refs.rs`, ≤ 1 MP each), remembered on the layer for Generate Similar; `generate.image`
  switches Krea 2 to the reference template when references come with no template named, and
  gained `variations`. `generate.models {width, height, count}` reports capabilities and `eta`;
  `generate_cmds::estimate_seconds` / `likely_auto` are the shared formula. Automation refuses
  `{"path"}` references.
- **Window › Generate** (`generate_panel.rs`): mode strip, reference deck (+ menu: active layer,
  a layer, a file, a stored generation; drops onto the panel become references through a new
  `DropTarget::References`), the growing prompt with counter / larger editor / wand / negative
  disclosure, model / aspect / size / count / destination / expand pads, Clear all (confirmed with
  references), and **Generate · ~16 s** (Cancel with the percent while running). Ctrl+Enter
  generates; Enter too with the new preference `enterToGenerate`. One-row form under 150 pt.
- **The task bar is the panel's compact form**: prompt, per-mode template and count live in
  `UiState::generate`; `ui.set generative*` stay as aliases and `ui.inspect.generativeBar` still
  reports them. `showGenerativeBar` became `auto` / `always` / `never` (a saved boolean reads as
  `auto` / `never`); in `auto` the bar steps aside while the panel is on screen and Edit ›
  Generative Fill… focuses the panel's Fill mode.
- **Dock**: Generate sits above Generations. With both open Layers asks for 320 pt instead of 500,
  the other groups give way first, and the generative groups fold to their tab strips when Layers
  would drop under 260 pt. Measured: full prompt box on a 1250-pt window; one-row forms and five
  Layers rows at 900 pt (ten without them); folded and three rows at 720 pt (four without).
- **Live** (`docs/comfy/bench/scripts/generate-panel-live.ps1`, release GUI on the RTX 5090,
  through the panel's own Generate path): a 2511 edit with two layer references ("put the fox
  from image 2 on the rocks … holding the compass from image 3") placed both, 39.5 s including
  the model load; an image from one layer reference into a new document 12.1 s against 14.9 s
  estimated; a fill 9.3 s cold and 8.7 s warm against 8.7 s estimated (the plan asked for
  within 50 %). Then warm CLI runs measured what a reference costs (`benchmarks.md` §5g): 11.8 /
  19.9 / 30.2 s with 0 / 1 / 2 references, so `estimatedSeconds` gained `perReference` (9 s on
  the Lightning tier) and the Lightning-8 family was refitted to `7.5 s + 5.5 s/MP`: the edits now
  estimate 11.7 / 20.7 / 29.7 s, the reference image 11.3 s, the fill 8.6 s.
- **Tests**: genai 19 + 12 client (pruning, estimate, bad reference metadata, references bound in
  order and limits checked first); engine `generate_refs_tests.rs` (5: edit references and
  Similar, limits and malformed specs, the image template switch and document variations, file
  and stored-generation references, the one formula); UI `generate_panel_tests.rs` (6) and the
  updated bar tests; dock tests for the new budget. Workspace gates: clippy, fmt, layers, wasm,
  `panic_hunt`; parity 637 / 637; scorecard 57 of 150 unread (the new preference is read).
- **Open**: `qwen-2.1/edit` and the Krea style template are not run live (their per-reference
  costs are guesses); the model picker is still a dropdown (plan Phase 3); the first run after a
  model switch takes 20–30 s longer than the estimate, which knows only warm servers.

## 2026-10-11 (night): plan Phase 1, the Generations library and panel

Built [`plan-artcraft-experience.md`](plan-artcraft-experience.md) §3 after the owner answered
§0: panels on by default, a **500-result / 5 GB** cap, sounds behind a cargo feature (Phase 5).

- **Engine** (`crates/engine/src/generate_jobs.rs`, new): `GenerationStore` on `Session`
  (`session.generations`). The dispatcher records every generative command (fill, image, expand,
  edit, similar, split, remove background, the three SAM selections, recreate): a background job
  gets a running row at start and its outcome in `end_job`; an inline run (CLI, tests, MCP with
  `wait`) is recorded when it returns; refused requests leave no row. Result layers are rendered
  over their request rectangle with the mask as alpha into `results/<id>-<i>.png` plus a 128-px
  thumbnail, on a writer thread for background jobs. Failures get a kind from the message
  (server unreachable, model files missing, out of memory, server error, timeout, bad input,
  discarded, interrupted). One correction to the plan: job ids restart per process, so record
  ids are unix µs + pid, bumped past known ids (plan §0 notes it).
- **Persistence**: `<config dir>/generations/index.json`, atomic, one-second debounce from
  `poll_jobs`, flushed on drop; another process's rows are merged (ours win, removed rows stay
  removed), so `photocraft-cli run … --cmd generate.fill` shows up in the app's panel. The CLI
  attaches the folder only for runs with a generative command. Rows left running by a closed
  process become "interrupted"; a corrupt index is ignored.
- **Commands**: `generate.jobs`, `generate.jobs.dismiss` / `.pin` / `.clear`,
  `generate.recreate` (Similar while the layer exists, else the original command on its
  document, else a fill/edit becomes Generate Image of its prompt with `fallback: "image"`),
  `generate.place` (a stored result back as a layer at its place, or a new document),
  `generate.export` (name tokens `{model} {template} {date} {seed} {prompt} {n}`, never
  overwrites; denied to automation as an ambient path). `generate.info` gains `job`.
- **UI** (`generations_panel.rs`, new): `Group::Generations` below Properties, on in Essentials
  (a saved layout without the field gets it too), rail button `grid-2x2` with a running/unread
  badge, Window › Generations, `ui.inspect.generations`, `ui.set {generationsView}`. In progress
  rows (real progress, Cancel), Failed rows (kind label, message tooltip, Recreate, Dismiss),
  Completed grid or list (hover Place, Generate Similar, Save…, Copy prompt; context menu with
  Place as Layer, Open as Document, Pin/Unpin, Dismiss; click selects the layer or opens the
  result as a document); overflow menu with confirmed Clear completed / failed / all.
  Only visible lines are laid out; thumbnails decode lazily, six per frame. When the dock
  squeezes the group (any column where Layers needs the room, which on this 1250-pt window is
  already the case), it draws a **compact strip**: one row of square tiles (progress, a warning
  sign with the reason in the tooltip, thumbnails) with context menus and the overflow menu.
  36 strings ×13 languages.
- **Layout cost** (#147): Generations sits between Properties and Layers at a compact 80 pt, so
  the default workspace shows **8 Layers rows at 900 pt** (10 without it) and 2 at 720 pt (4).
  `dock_tests` checks upstream's guarantee with the group hidden and PhotoshopEX's with it.
- **Tests**: 15 engine tests (`generate_jobs_tests.rs`: lifecycle inline and background, failure
  kinds against the fake (down, OOM, validation error), restart, interrupted rows, caps and pins,
  dismiss/clear, place after close, recreate fallbacks, export naming, two-process merge,
  no-folder mode, bad params); 6 UI tests plus an ignored bench (`generations_panel_tests.rs`);
  the automation deny test. `panic_hunt` now clears the server URL in its fresh session: with
  ComfyUI running, `generate.splitLayers {}` and `select.subjectML {}` ran real jobs (>4 s).
  Workspace: 4347 passed, the same 9 environment failures as the baseline (GPU parity rounding,
  7 egui-wgpu canvas tests, the eyedropper test). Clippy, fmt, layers, wasm green; parity 636 /
  636 (the Window › Generations row); scorecard moved only by that row. rustfmt also reformatted
  a few lines in the earlier generative files (`generate_cmds.rs`, its tests, `fake.rs`,
  `generative_bar.rs`) and one clippy fix in `generate_enhance_tests.rs`.
- **Numbers** (dev profile, `bench_generations_panel_with_200_rows`): `panel()` with 200
  completed rows **0.13 ms** in the grid, **0.08 ms** in the list (1.45 ms before laying out
  only the visible lines; budget 1 ms); the index for 200 rows is 278 KB and writes in 2.5–3.0 ms.
  Live, release GUI through the control channel (`docs/comfy/bench/scripts/generations-live.ps1`,
  RTX 5090, ComfyUI warm): fill 13.7–14.7 s, edit 10.7–13.3 s, a 768×512 Krea 2 image
  5.6–7.7 s, each a background job that showed as running then completed with its thumbnail;
  Place after File › Close All opened the 464×416 fill result as a document; a dead server URL
  gave a `serverUnreachable` row; a restart kept all four rows. A process killed mid-job had
  already flushed its running row, which the next start marks interrupted.
- **Open**: an ETA on running rows waits for the template `estimatedSeconds` of Phase 2; the
  Save… button uses the platform save dialog until Phase 5's export preference; record ids are
  looked up by layer only within the session that made them (`generate.info.job`); a process
  that still holds a row another process cleared writes it back on its next flush (the CLI is
  short-lived, so in practice only two GUI instances could do this).

## 2026-10-11 (later): the build plan

[`plan-artcraft-experience.md`](plan-artcraft-experience.md): the hand-off plan the owner asked
for so another agent can build the study's §6. Five phases, each with the engine commands (params
doc strings), the data shapes (`GenerationRecord`, the library on disk under
`<config_dir>/generations/`, template capability flags, `Request.references`), the UI modules
and the exact upstream files to touch (dock groups, rail, menus, control protocol, with line
anchors at `ce5c692`), the tests per file, the docs per phase, performance budgets, a failure
taxonomy, the ETA formula, live scripts, an acceptance walkthrough and a file map. Open
decisions listed in §0 (panels on by default, a 200-result / 2 GB library cap, sounds behind a
cargo feature). Nothing built yet.

## 2026-10-11: the ArtCraft study, refocused on the experience

The owner clarified: not the business model, the **UI and experience**, with the credits removed
(local compute). [`artcraft-study.md`](artcraft-study.md) was rewritten from the frontend
source: the one-window shell with Home + app tabs and the Task Queue button; the Create pages'
editorial empty state, feed and floating prompt box; the prompt box piece by piece (reference
deck, growing prompt with counter and fullscreen, model pill and family-grouped picker with wait
badges, aspect/resolution/quality/count pickers that follow the model's capabilities, Generate);
pending tiles, failed rows and completed cards with hover actions (Recreate, Make Video, Share,
Download); the Task Queue popover; the lightbox's prompt panel; the Library's folders, tags and
drag-to-canvas; the 2D editor's side toolbar, inpaint toolbar, History Stack of candidate bundles
and prompt editor; Angles' orbit sphere; settings (Enter to generate, sounds, themes, keybinds,
download naming). Then the PhotoshopEX translation: a **Generate panel**, a **Generations
panel** (feed + queue with real progress and time estimates), a **Variations strip** over the
candidate layers, a Generate hub, sounds and preferences, with wireframes and a five-step build
order. Write-up only.

## 2026-10-10: the ArtCraft study (write-up only)

The owner asked to look into ArtCraft's paid features and to make the implementation "more
like how they do things overall", write-up first. [`artcraft-study.md`](artcraft-study.md) is
that write-up, from getartcraft.com, the team's posts and a local read of the public
`storytold/artcraft` repository (a shallow clone in the temp directory, not kept in the tree).

Short version: ArtCraft's only paid feature is generation through its own hosted provider,
priced in credits (1 credit = $0.01; Basic $8 for 1,000, Pro $28 for 3,750, Max $48 for
6,600; refunded on failure); everything else is free, and users can bypass credits with their
own Fal/Replicate/Google keys or Midjourney/Grok logins. The patterns worth taking: a model
catalogue as data with capability flags that drive the UI, one request shape with the provider
chosen late and a priority/fallback list, a persistent local task queue with typed statuses and
failure reasons plus a Task Queue popover, a prompt box built from capabilities with a reference
deck and the cost (for us: the time) on the Generate button, and a history stack of candidates.
The study ends with an ordered adoption list (catalogue → task queue → prompt panel →
providers with BYO keys, the ArtCraft Omni API among them → failure labels → a gallery) and
three questions for the owner (cloud providers at all; queue or providers first; asking the
ArtCraft team for API access). Nothing in the app changed.

## 2026-10-09 (late night): the PhotoshopEX branch

The owner branched the fork into their own version: this checkout,
`C:\Users\5090\Documents\PhotoshopEX`, branch `photoshopex`, from `comfy-photocraft` at `9cd7165`
(the enhancer, the NVFP4 policy and the Object Selection tool's SAM mode included). Landed here:

- **The app icon**: a Photoshop-style tile (navy `#001E36`, the letters "Px" in `#31A8FF`, Segoe
  UI Bold) replaces the PhotoCraft kitsune everywhere the app and its packaging read an icon
  (`assets/app-icon/`: the hicolor PNGs 16–512, the 1024 macOS render, the `.ico` packed with
  `cargo xtask ico`, the `.icns` written by Pillow, the SVG masters). `packaging/icon-px.py`
  regenerates them (Pillow from the ComfyUI portable Python; the Rust toolchain packs the .ico).
  Adobe's own "Ps" mark is a trademark and is not copied; the letters are this version's.
  `app_icon.rs`'s tests (every shell size in the .ico, the window icon decodes) pass.
- **The session's dev files**: the PowerShell bench and verification scripts in
  `docs/comfy/bench/scripts/` (with a README), the bench images in `docs/comfy/bench/images/`
  (git-ignored, 171 MB).
- `CLAUDE.md` says what this checkout is; `ATTRIBUTION.md` and `brand.rs` name the new icon.

Open here: the product name (the window title and `--help` still say PhotoCraft; "Photoshop" is
Adobe's trademark, so the published name is the owner's call), and everything in the entries
below.

## 2026-10-15: local-build look and feel (wheel zoom, no Discord button, Ps tile)

The user's three asks for the local build. Hotkeys were already Photoshop's (`shortcuts.rs`,
no overrides in `preferences.json`); the one that felt off was the wheel: Photoshop's default
is wheel = scroll, Alt + wheel = zoom, which upstream copies, and the user wants wheel = zoom.

**Landed**

- `General::zoom_with_scroll_wheel` defaults to `true` in the fork (`prefs.rs`); the saved
  `preferences.json` on this PC was flipped by hand too, since serde writes every field and a
  stored `false` would have won over the new default. Alt + wheel now scrolls (`wheel_nav.rs`
  swaps the two; untouched).
- The title bar's Discord button is gone (`panels.rs`: the block and `DISCORD_ROOM`); Help ›
  Discord stays, so `links.rs` and its tests are untouched. The `titlebar.rs` width test now
  asserts the button is absent at every width; `docs/ui-design.md` says the same.
- A Photoshop-style "Ps" tile (`assets/app-icon/ps/`, README there) in place of the kitsune in
  the title bar (`brand.rs`), the window/taskbar icon (`app_icon.rs`, non-macOS) and the exe
  resource (`build.rs`). New files, three path edits; the kitsune assets are untouched for
  merges. Personal build only: it must never ship (`upstream.md` brand checklist).

**Open**

- The fork is 1,728 commits behind upstream (2026-10-15), which closes ~1,200 issues a week,
  many of them parity fixes. A merge is the next big lever on "feels unlike Photoshop".

## 2026-10-09 (night): the prompt enhancer

The user asked for "some kind of prompt optimization" after the doodle-to-food session, and
whether the settings are the best ones. Research first (`benchmarks.md` §5e): the 2511 and
Krea 2 graphs already match ComfyUI's shipped templates node for node; what the Qwen authors
recommend is a VLM rewrite of the instruction with the picture in view (their
`polish_edit_prompt`), which ComfyUI's own Qwen-Image-2.1 template does with `TextGenerate`.

**Landed**

- `generate.enhancePrompt {prompt, task: fill|edit|image, useImage, seed}` → `{prompt, enhanced}`:
  the Krea 2 text encoder (Qwen3-VL 4B, Apache-2.0, already installed for Krea 2) run as a
  vision-language model through `TextGenerate` (templates `enhance/edit` with the picture,
  `enhance/image` without; `Task::Text`, `Response.texts`, `PreviewAny` text outputs in
  `comfy.rs`). The engine's rules are the chat's system turn, the user's words the user turn;
  a fill is told where its selection sits ("the upper-left part"). Two tries with consecutive
  seeds; the answer is cleaned of a stray "assistant", quotes and markdown; nothing usable
  leaves the prompt as typed. `enhance: true` on `generate.fill`, `edit` and `image` does it
  on the way (an expand keeps its wording); Preferences › AI Integrations › **Enhance prompts**
  makes it the default; the layer remembers both prompts (`GenerativeInfo.enhanced`,
  `generate.info`), and Generate Similar rewrites anew. The task bar has a wand button by the
  prompt that replaces the text with the rewrite (job, "Enhancing…", the modal dialog stays
  away). Translated ×13 (4 strings). Tests: 6 engine (`generate_enhance_tests.rs`, the fake
  answers `Enhanced: ` + the user turn or `Options.enhanced_text`), the bar test.
- **The chat template.** Half the first live rewrites came back empty: `TextGenerate`'s
  `thinking: false` appends an empty `<think>` block that the non-thinking 4B Instruct model
  answers with an immediate end of turn (or a restarted "assistant" turn). With `thinking:
  true` the template ends at the plain assistant turn: 26 of 26 clean. Kept in the template
  notes so nobody "fixes" it back.
- **The doodle.** With the rules' first wording the rewriter asked to keep the doodle's outline
  and 2511 drew bread again; three hand-written sentences showed the model needs "a photograph
  of a real human face … replacing the black outline strokes with real skin, hair and facial
  features", which now is the rule for a change of medium, and the rewriter writes exactly that
  (`bench/doodle-sentence-A*.png`: a clean photographic face).

**Numbers.** A rewrite is 0.8–1.3 s (sentence) / 1.8–2 s (paragraph) after a 3 s first load.
End to end through the CLI, both of the user's doodle prompts give a photographic human face
(18.8 s and 17.2 s with the rewrite, `bench/enhance-doodle-*-final.png`). The 4B encoder stays
resident (5 GB); with 2511 alone the steady state is 13.5–14.6 s an edit. Answer to the
user's VRAM question in `models.md` §4: Qwen-Image-2.1 int8 + its 8B encoder + SAM 3.1 ≈ 18
GB, they fit together.

**NVFP4 (same session).** Comfy-Org's official repacks of the 2511 encoder (5.7 GB) and Krea 2
Turbo (7.2 GB) measured against fp8 (`benchmarks.md` §5f): the encoder's precision does not
change an edit's time (14.1 s either way) but saves 3 GB; Krea 2 is 18 % faster (5.7–6.0 s
against 7.0–7.1 s a 1024² image) and leaves 5 GB more free; the pictures are alike. Shipped
as a policy: template slots carry an `nvfp4` alternative, Preferences › AI Integrations ›
**Model precision** (`auto` | `default` | `nvfp4`, `auto` by default) loads it on a Blackwell
GPU (`is_blackwell` on the device name) when the server has it; `run_switching` computes the
model set from the prepared request so the purge sees a precision change; `generate.models`
reports `device`, `blackwell`, `nvfp4`, `nvfp4Installed`; tests in
`generate_precision_tests.rs` (the fake takes `Options.device`). Live: the server loaded both
NVFP4 files with nothing but the defaults. Lesson from the first benchmark attempt: ComfyUI
caches a repeated graph; vary the seed or the "warm" run takes 290 ms.

**The SAM 3.1 UI (same session).** The Object Selection tool: with a generative server
configured, a click (no drag) runs `select.byPoint` on the point under the pointer (⇧ adds, ⌥
subtracts, as the rectangle does), the options bar has a **Select by text** field (Enter →
`select.byText`, replacing the selection; `toolOptions.selectText`) and says "Click an object,
or drag a rectangle around it", and **Select Subject** goes through `select.subjectML`, falling
back to the classical `select.subject` when no server is configured or the command cannot
start. A drag stays the classical `select.object`. Test in `retouch_ui.rs` against the fake.
Translated ×13 (4 strings, the two job labels included).

**Open.** The GUI is the 8:56 PM build until the user closes it (the enhancer, the NVFP4
policy, the SAM UI and the temp-file model memory are in the CLI build only); SAM box prompts
(a drag through the detector) and hover highlighting are the next steps of the tool.

## 2026-10-09 (evening): first user session, two fixes

The user tried the app on a line drawing of a face: Generative Fill over a rectangle with
"make this hyper realistic", then "turn this drawing into a hyper realistic portrait that looks
similar". Both results were a plate of food; the second run took 229 s.

**What happened** (from the server's history, `/history`, and the uploads): the fills were
correct inpaints (the output outside the mask matched the input to within 1/765), but Fill
regenerates the selection from scratch with the picture as a reference, and "make this hyper
realistic" over a white rectangle with a doodle gave the model nothing to hold on to: it drew
a dish. The instruction was an *edit* of what is there, which is Generative Edit's job (the
whole picture by instruction, no noise mask). The 229 s: the server still held the 19 GB
layered model from the Split tests in another process, so the app's first run (which never
purged: the model-set memory is per process) loaded 2511 partially and streamed weights.

**Landed**

- `run_switching` purges on a process's first run too when the server reports less than a
  quarter of its VRAM free (one `/system_stats` call); the fake takes `Options.vram_free` and
  a test checks the purge precedes the first prompt and happens once.
- The task bar has a **Fill | Edit** mode (`generativeMode` over `ui.set`): Edit runs
  `generate.edit` with the same prompt, template picker (edit templates) and variations, the
  selection masking the result; and a Fill prompt that reads like an instruction to change
  what is there ("make this…", "turn it into…") shows "Changing what is there? Switch to Edit".

**Follow-up, the same evening.** Generative Edit with the user's own prompts still gave food
("turn this drawing into a hyper realistic portrait…", "make this hyper realistic", on
Lightning and on the 40-step base): the model does not read the doodle as a face, "this" is a
plate to it. Prompts that **name what the drawing shows** work: "turn this sketch of a face
into a realistic photo of a human face" and "redraw this simple line drawing of a smiling face
as a photorealistic portrait photo of a person with the same round face, two eyes, a nose and
a smile" both gave a face (`bench/user-drawing-*.png`). Noted for the UI copy: describe the
subject, not "this".

The slow runs: a fresh load of 2511 at 1088×704 ran three times at 14–17 s under logging
(ComfyUI 0.39 stages models for dynamic VRAM loading; no partial loads in the log), so the
209 s and 229 s runs were contention: the Split tests' 19 GB layered model in one case, and
in the other the server being used from its own web UI at the same time (its log shows Krea 2
attempts with the text encoder loaded as `qwen_image`, which fails: Krea 2 needs CLIPLoader
type `krea2`, which PhotoCraft's template uses; our Krea 2 run took 9.5 s). Landed on top:
the model-set memory behind `run_switching` is mirrored to a file in the temp directory, so
the app, the CLI and MCP agree on what a server holds across processes and only a real
change purges; the low-VRAM rule now applies only to a server no PhotoCraft process has
recorded; `generate.free` forgets the record.

## 2026-10-09 (late): Split into Layers (Qwen-Image-Layered), and a softer detector matte

**Landed**

- `generate.splitLayers` (Edit › Split into Layers (Generative)…, translated ×13, generated
  dialog): the composite (or the active layer) goes to **Qwen-Image-Layered** through
  `qwen-layered/split`, ComfyUI's official "Image to Layers" subgraph as a template (the
  picture as a `ReferenceLatent` on both conditionings, `EmptyQwenImageLayeredLatentImage` with
  `layers` slices, `LatentCut` dropping slice 0 and `LatentCutToBatch` making a batch, the
  layered VAE decoding RGBA). Each result becomes a new layer above the active one, background
  first, with the alpha the model gave it; `layers` 1..8 (default 3); the picture goes out at
  640 px on its longer side (the official size) and comes back resampled. A new `Task::Layers`;
  the fake answers such graphs with one banded RGBA image per layer. The model is Apache-2.0
  (`models.md`); the 19 GB fp8 repack and its VAE were downloaded here for testing only.
- Remove Background's detector route now softens its hard mask with the classical edge
  refinement (`refine: true` by default, the Quick Action's parameters), so hair and soft
  edges get partial coverage from the pixels themselves.
- Tests: two for the split (layer order, alpha bands, names, graph, sizes, validation, no
  "similar"), the refinement in the detector test.

**Live** (the 1024² lighthouse, `bench/split-layer-*.png`): three layers in **72 s** (model
load included) at 640²: layer 0 a near-white base carrying the boat's shadow, layer 1 the sky
and sea with the boat (soft where the rocks were), layer 2 the rocks and lighthouse cut out
with real soft alpha. The model chooses the decomposition; the composite is the picture. A
second run reuses the loaded model.

**Open**: the layered model's 640 px working size (a 1024 run is worth a try), transparent
fills (the model's alpha times the selection), preview frames.

## 2026-10-09 (late): Remove Background without the opt-in, and Select by Point

**Landed**

- `generate.removeBackground` works out of the box: `template` defaults to `auto`, which is
  the Qwen-Image-2.1 matte (soft alpha, research licence) when the preference allows it and
  the server has the files, else **SAM 3.1** (`sam3.1/segment`, SAM License, no opt-in): the
  prompt or "the main subject" (the phrase `select.subjectML` uses) goes to the detector and
  the union of its instance masks becomes the layer mask or the selection, hard-edged. The
  route is decided in the job (`resolve_auto` over the matte template with the detector as
  the fallback); a named template of another task is refused; `generate.models` reports
  `autoMatte`. The classical Quick Action `layer.removeBackground` remains the no-server path.
- `select.byPoint {"x","y"}` (Phase 3.5): a new template `sam3.1/segment-point` prompts
  `SAM3_Detect` with `positive_coords` (a literal JSON list in the API format, probed live:
  the boat under one point in 2 s) and no text; the engine scales the canvas point to the
  request's pixels. No menu row yet (it needs a click); scripts and MCP can use it, a tool
  modifier later.
- Tests: the detector route (prompt, no prompt, as selection, the fallback when the 2.1 files
  are missing, `autoMatte` either way), by point (coordinates sent, validation).

**Open**: a soft-matte refinement for the detector route (its masks are hard); box prompts;
preview frames; Qwen-Image-Layered.

## 2026-10-09 (late): the leftovers of Phases 2 and 3

**Landed**

- Generative layer metadata (Phase 2 item 5): every layer a generative command makes carries a
  `GenerativeInfo` (command, prompt as typed, resolved template, seed, steps, guidance, edge,
  request rect, name; images add size and transparent) as JSON in a PSD additional-layer-info
  block keyed `cpGn`, so upstream's `Layer` is untouched and the record survives PSD round
  trips and `Layer::duplicate`. `generate.info` reads it.
- `generate.similar` (Edit › Generate Similar, translated ×13, enabled on a generative layer):
  runs what made the layer again with a new seed in the same place, the layer's mask as the
  area (no selection needed), the result a layer above it. Fills and edits re-run as they were
  (the same resolved template); an expanded area is filled again with the expand's prompt
  through the fill templates (its canvas already exists); a generated image is generated
  again as a layer. Research gate as usual.
- The picker's installed badges: `generate.models` takes `async: true` and probes the server
  on a worker (`jobs::run`, result = the usual answer); the bar starts it when it loads its
  templates, keeps the job id, and marks templates whose files the server lacks "(not
  installed)" (translated). The bar never waits for the server.
- The Crop tool's expand state (Phase 3): a Generative Expand checkbox in the Crop options bar
  (`ToolOptions::crop_generative`); on ↵ with a frame beyond the canvas, `commit_crop` crops to
  the part inside first (when that is not the whole canvas) and then runs `generate.expand`
  with the overhang as pads, so the model paints the new area; an empty prompt continues the
  scene. Off, the canvas just grows as before.
- The control-protocol script test of the Phase 2 DoD already existed as
  `generative_bar_tests::the_menu_opens_the_bar_and_generate_runs_the_fill_with_variations`
  (select → `ui.set` → generate → `ui.inspect` against the fake); the roadmap now says so.
- Tests: two engine tests (metadata + similar for fills, duplication; images and expands), the
  crop test (beyond, partly inside, option off), the bar test checks the badges.

**Open**: preview frames in the bar; a permissive matting default; Qwen-Image-Layered.

## 2026-10-09 (late, Phase 3): Generative Edit, and the automatic purge on a model switch

**Landed**

- `generate.edit` (Edit › Generative Edit…, translated ×13, generated dialog): the whole
  composite goes to the official 2511 edit graph as the model is meant to be used, no noise
  mask, the instruction decides what changes (`qwen-edit-2511/edit-lightning-8` and `edit`,
  `auto` between them, `AUTO_EDIT_ORDER`, `generate.models` reports `autoEdit`). The result
  is a layer above the active one, masked to the selection when there is one (soft dithered
  edge or hard), so "turn the red boat blue" with a marquee on the boat changes the boat and
  nothing else on the canvas. Descriptions are wrapped as "Change this image so that it shows
  {prompt}, keeping everything else exactly as it is."; instructions get ". Keep everything
  else exactly as it is." Shares `fill_region` (variations, sizing, Lanczos, timings); the
  request carries no mask when the template takes none.
- `run_switching`: every generative run (fill, expand, edit, image, matte, select by text)
  goes through it. The engine remembers the model files (`folder/file`) of the last run per
  server URL (a process-wide static) and asks the server to unload its models before a run
  whose files differ: the Lightning run after the 40-step base went from 121 s to 20.2 s
  (`benchmarks.md` §5c). Never on a server's first run; a server that cannot purge runs as
  before. A test drives Lightning → base → base → Lightning against the fake and counts the
  `POST /free`s (1, then 2) and their position.
- Corrected a diagnosis: the 40-step base's 130–148 s is its own cost (two passes a step at
  CFG 4), not thrash; the thrash was the Lightning tier after it.
- Four engine tests for edit (whole canvas, no mask upload, wrapping; the selection confines
  the result; `auto` falls back to the base, validation, `autoEdit`), one for the purge.

**Live** (`bench/edit-*.png`): a dark storm with lightning and the lamp lit at 16 s; the boat
turned blue inside its selection and nothing else; a sunny day at 20 s after a switch.

**Open**: the Crop tool's expand state; a permissive matting default; Qwen-Image-Layered;
a 20-step middle tier for the base.

## 2026-10-09 (late, Phase 3): Remove Background on Qwen-Image-2.1's alpha channel

The user: "qwen2.1 supports alpha channels". It does, natively: the 2.1 VAE decodes RGBA.
Probed live before writing code (`bench/probe-rgba-s7.png`, `probe-t2i-alpha-s3.png`):
ComfyUI's official background-removal recipe returned a colour-type-6 PNG with a soft matte in
18 s, and a plain text-to-image prompt ending in "isolated on a transparent background, output
a PNG image" returned a boat with a clean matte in 10 s.

**Landed**

- `generate.removeBackground` (Edit › Remove Background (Generative)…, translated ×13, generated
  dialog): the layer (or the composite with `sampleAllLayers`) goes to `qwen-2.1/matte`, the
  official recipe as a template (the 2.1 edit graph, the picture as image_1 at its own size,
  "Remove the background, and output a PNG image"; `prompt` names what to keep and becomes
  "Remove the background, keeping only {prompt}, …"). The result's **alpha** comes back as the
  layer's mask (the Background becomes a normal layer, like the Quick Action) or, with
  `asSelection`, as the selection combined by `mode`. The pixels the model re-rendered are
  never used; the layer keeps its own. Requests go out at ≤ 1 MP with sides in multiples of 32
  (what `TextEncodeQwenImage21` rounds to), the matte comes back through the tent filter, the
  layer's own transparency goes to the model over mid-grey and multiplies the matte. One undo
  step; "kept nothing" is an error that changes nothing. Research licence gate as for the other
  2.1 templates.
- `generate.image` takes `"transparent": true`: the template's new `promptFormatTransparent`
  wraps the prompt (only `qwen-2.1/image` has one; others refuse with a clear error) and the
  layer keeps the alpha the PNG comes back with.
- Fake server: `Options.matte` gives generated images an alpha rectangle. Five engine tests
  (mask from alpha with pixels untouched and undo, subject wrapping + Background → layer,
  as-selection + add mode, validation + licence gate + "kept nothing", transparent image +
  refusal). Docs: models.md, architecture.md, roadmap; parity/scorecard regenerated.

**Live** (the 1024² lighthouse with the boat, `bench/matte-*.png`, `transparent-compass.png`):
16.8 s per matte (25 steps, 2.1 int8). The model's own reading kept the boat solid and the
lighthouse as a ghost; "the red boat" gave a perfect boat cutout (147×46 px); "the lighthouse"
kept the lighthouse with its rocks, the white tower semi-transparent against the bright sky
(a model quirk to note in the UI later: name the subject, re-roll, or refine the mask). A
transparent "vintage brass compass, product photo" came back in 10.8 s as a clean cutout with a
real soft alpha.

**Open**: a permissive matting model (BiRefNet) as the default so the command works without
the research opt-in; Qwen-Image-Layered ("image to layers": ComfyUI 0.39 ships
`EmptyQwenImageLayeredLatentImage` and official templates; needs `qwen_image_layered_bf16` +
its VAE, not installed) as a Layers › "Split into Layers" feature; transparent fills (the
model's alpha times the selection) for adding cut-out objects.

## 2026-10-09 (late): soft, dithered edges for fills and expands

The user: "the borders are harsh, is there a way to do like a noise opacity in the edges so
that there aren't hard cutoffs?" Yes; it took three changes, each found by measuring the
right-side expand's sky profile (`benchmarks.md` §3, "Edges").

**Landed**

- `generate.fill` and `generate.expand` take `"edge":"soft|hard"` (default soft; a choice in
  the generated dialogs, labels already translated). Soft: the result layer's mask is the
  feathered request band itself, its ramp **dithered** with per-pixel noise (`dither_edge`: a
  splitmix64 hash of pixel index and seed, amplitude `0.6 × (1 − |2m − 1|)`, so nothing changes
  inside or outside the band and the grain peaks mid-ramp). The result fades into its
  surroundings as grain, not a gradient or a line. Hard: exactly the selection (or the added
  canvas). The feather is now 4 % of the longer side for both (fill 6–48 px, was 2 %; expand
  8–80 px, was 8 %: at 8 % the model moved the horizon inside the band at one seed, a ghosted
  double horizon; at 2 % the old edge still read as a line).
- `feather_outward` starts its ramp at **full** coverage on the edge and falls to nothing at
  twice the radius (the GrowMask + FeatherMask shape). The old `max(selection, blur)` ramp
  started at half coverage: a 0.5 step at the edge in both the noise mask and the layer mask,
  which is half the tone difference left standing as a line.
- Expand's padding is pre-filled by replicating the picture's edge pixels instead of mid-grey:
  the grey never reached the model as content (cropped reference, full noise mask) but the VAE
  carried its tone into the latents of the edge band, a dark line right on the old edge
  (luminance 213 between 225 and 228). `PREFILL` grey remains only for an empty picture.
- Tests: the soft-edge test (full on the edge pixel, high just outside, falling off, grain in
  the row, hard = selection, `edge: fuzzy` rejected), `dither_edge` unit test, the prefill test
  now checks edge continuity, expectations for the new radii and ramp.

**Live** (`bench/edge-s11-*.png`, `bench/expand-right-{soft,hard}.png`, `xab*-*.png`): the
expand's sky goes 229 → 226 across the band with no step or dip, nothing visible in a
contrast-stretched crop; the hard variant keeps its step. The soft and hard variants of one
seed are the same request, so the second returns from ComfyUI's cache in 0.1 s. The three
placement cases (boat ×2, balloon) are unchanged by the wider feather. Back-to-back expands
run 13–20 s (13 s with the reference's encoding cached, 19 s after another reference evicted
it).

**Found on the way** (`benchmarks.md` §3): what no edge can hide is the new side being a
different photograph. With a described prompt the Lightning model painted a crisp, sharp-
horizon sea next to the hazy original at two seeds; the empty prompt (the default instruction)
continued the haze; the 40-step base (130 s) and several wordings copied the lighthouse into
the new area at one seed, and so did the empty prompt at the same seed when re-run later. The
server is bit-deterministic in a steady state and not across model-load states (pixel diffs
0.00 on repeats, up to 21/765 between states: a second lighthouse against none), so two-seed
wording comparisons prove little; the wrappers stay as they are and the dialog's note that an
empty prompt continues the scene is the advice. Also: the first run after a graph switch
(base ↔ Lightning, or purge → fill → expand) took 31–131 s with the server at 2.4 GB free
(partial model load), then 13–20 s; the engine should purge before a run that changes the
model set (roadmap, next).

**Open**: the automatic purge on a model-set change; a prompt study for Expand over more seeds;
the Crop tool's expand state; `generate.edit`, `generate.removeBackground`.

## 2026-10-09 (night, Phase 3): Generative Expand

**Landed**

- `generate.expand` (Edit › Generative Expand…, translated ×13, generated dialog): adds canvas
  (`left/top/right/bottom` px, or a larger `width/height` with Canvas Size's `anchor`) and has
  the model paint it, as **one undo step**: `translate_doc` + the new size inside the job, the
  picture moves by the left/top pads, the selection and vector geometry move along. The request
  is the bounding box of the added area (an L or a frame when several sides grow) plus the fill
  margin of the picture. An empty prompt becomes "extend the scene beyond its original edges,
  continuing it naturally". Result adds `canvas` and `offset`.
- Its own templates, `qwen-edit-2511/expand-lightning-8` and `expand` (task `expand`,
  `AUTO_EXPAND_ORDER`, `generate.models` reports `autoExpand`): the fill graph plus an
  `ImageCrop` so the text encoder's reference is **the picture alone** while the padded canvas is
  the sampling latent under the noise mask. Found the hard way: shown the padded canvas as its
  reference, the edit model reproduced the padding as content (replicated edge pixels came back
  as streaks; mid-grey came back as a grey frame at one seed). The empty canvas is still painted
  mid-grey before upload (`prefill_outside`), and the result layer's mask is the soft request
  mask with an 8 % feather into the picture (`outpaint_feather_radius`), so the re-rendered band
  carries the new area's tone across the old edge instead of meeting it at a line.
- The fill job body is now `fill_region` (shared by fill and expand): Lightning tiers, `auto`,
  grid alignment, the request cap and 512 px floor, the feathered request mask, variations. The
  cap went from 1 MP to **0.75 MP**: with 2511 fp8 and its 7.9 GB text encoder resident on the
  32 GB card, a 1 MP latent plus the 1 MP reference pushed ComfyUI into offloading part of the
  model (a 45 s run instead of 18 s).
- Tests: two engine tests (one-step L-shaped expand with pixel/mask/undo checks, the grey
  pre-fill and the crop rectangle; target size + anchor, selection follows, validation, disabled
  without a document).

**Live** (the 1024² lighthouse, Lightning 8, `bench/expand-*.png`): widened to the right by
384 px with "more open sea and evening sky" in 20.2 s (a 736×1024 request), and framed by 192 px
on three sides with the default prompt in 14.8 s (a 1408×1216 canvas sent at 944×816). Both
continue rocks, sea and the pink evening sky coherently; the old edges are faint at most. The
earlier variants (padded reference: streaks, a grey frame, a blue sky) are kept next to them with
suffixes for comparison.

## 2026-10-09 (performance pass): Lightning tiers, `auto`, request sizing, server flags

**Landed**

- `qwen-edit-2511/fill` now matches ComfyUI's official 2511 graph exactly (CFGNorm after
  ModelSamplingAuraFlow, `FluxKontextMultiReferenceLatentMethod index_timestep_zero` on both
  conditionings). Two Lightning tiers, `fill-lightning-8` and `fill-lightning-4` (Apache-2.0
  LoRAs by lightx2v, `LoraLoaderModelOnly` after CFGNorm, CFG 1), and the preference default
  `defaultFillTemplate = auto`: the job asks the server which files it has (`/models/<folder>`)
  and takes the first of `AUTO_FILL_ORDER` (`fill-lightning-8`, then the 40-step base) whose
  files are all installed. `generate.models` reports `autoFill`; the task bar's picker starts
  with Auto. BYOB stays: the 850 MB LoRA is a download the user makes.
- Request sizing: a fill request over 1 MP is sent downscaled (Lanczos-3 pixels, tent-filtered
  mask, multiples of 16) and the result is resampled back under the full-resolution layer mask;
  `resize_rgba8` is Lanczos-3 everywhere (was bilinear); segmentation requests are capped at 2 MP
  and their masks come back through the tent filter. `requestWidth/Height` and per-variation
  `timings` (`encodeMs`, `uploadMs`, `queueMs`, `runMs`, `downloadMs`) are in the result.
- Uploads: PNG at the fastest deflate level (loopback, read once) and **content-addressed
  names** (FNV-1a of the bytes), so ComfyUI's node cache keeps the loader, the 7 B vision text
  encoder and the VAE encode across variations and re-rolls of one selection.
- `docs/comfy/bench/bench-fill.ps1` (the live benchmark), `docs/comfy/benchmarks.md` (numbers),
  `start-comfyui-fast.ps1` on the dev PC with `--fast fp8_matrix_mult --highvram`, the flag table
  and a 24 GB / RTX 3090 section in `comfyui-setup.md`, Lightning facts in `models.md`.
- Tests: auto resolution with and without the LoRA on the fake (`Options.missing_files`),
  the 1 MP cap end to end (upload size, placement, timings), resampler behaviour, the 2 MP
  segmentation cap; 33 engine generate/select tests, genai 29.

- Prompt wrapping for the 2511 fills: a description ("a red boat") becomes "Add {prompt} to
  this image, fitting the scene's perspective, lighting and surroundings naturally. Change
  nothing else."; a prompt that already opens with an imperative verb (`template::is_imperative`,
  "remove the car") becomes "{prompt}. Fit the result…" (`promptFormatImperative`, also on the
  2.1 fill). The request mask is **feathered outward** (2 % of the longer side, 4–24 px, two box
  blurs then `max` with the original) so the latent noise mask blends the edge; the layer mask
  stays the selection. Opt-in `qwen-edit-2511/fill-guided` passes the mask as a second
  reference image for the best placement at 1.5–3× the time.

- **Native-size sampling** (second pass, same day): the 2511 templates VAE-encode the uploaded
  image instead of the Kontext-scaled one, so the sampling latent has the request's own size
  while the text encoder keeps its 1 MP reference. The engine grows the request rectangle to
  the 16-px grid (`align_to_grid`), caps it at 1 MP and floors it at 512 px on the longer side
  (`request_size`). 35–45 % faster and better placed (benchmarks §4). `generate.free`
  (Edit › Purge › Generative Models) asks the server to unload its models after a measured
  memory-pressure state made fills three times slower (benchmarks §5).

**Measured** (all tables and the method in `benchmarks.md`; the lighthouse, a 576×512 request):

| Fill, warm | default flags, unique uploads | `fp8_matrix_mult`, cached encoder (re-roll) |
|---|---|---|
| 2511 base, 40 steps CFG 4 | 162 s | 153 s |
| 2511 Lightning 8 | 18.6 s | 16.0 s |
| 2511 Lightning 4 | 12–16 s | 8.1 s |
| Qwen-Image-2.1, 25 steps | 4.5 s | 2.6 s |

A 2511 fill is sampling-bound (~2 s per Lightning step at 1 MP, ~4 s per base step with CFG 4);
the client's share is 2 ms encode, 5 ms upload, under 0.4 s download. With native-size sampling
the three A/B cases went from 16.2–16.5 s to 9.2–11.2 s, and a 120×90 selection fills in 10 s.

**Findings**

- Quality, three cases at fixed seeds (benchmarks §3): the plain prompt with a hard mask gave a
  cut-off boat with a rectangular seam at one seed; the shipped wrapper + feather gave no seam
  in any case and the boat on the water at seed 11; placement at seed 7 stays the model's
  choice (rocks) unless `fill-guided` or Qwen-Image-2.1 is used, both of which put it on the water.
- `--highvram` is harmful with 2511 fp8 on 32 GB (2 GB free, LoRA reload thrashed); dynamic
  VRAM already keeps a family resident. lightx2v's `fp8_e4m3fn_scaled` 2511 file ran 35 % faster
  but returned noise in the masked area with our graph; removed. `fp16_accumulation` does
  nothing for bf16/fp8/int8 models. Shipped flag: `--fast fp8_matrix_mult` (~5 %).
- Two benchmark runs overlapped on the GPU once (a background task kept running after I thought
  it had died); the numbers above come from clean, sequential foreground runs.
- PowerShell: variable names are case-insensitive (`$seed` is `$Seed`), `.NET`'s working
  directory is not PowerShell's (`[IO.File]` needs absolute paths), and `"$f:"` is a drive
  reference; all three bit the benchmark scripts once.

## 2026-10-09 (late night): Phase 2, the generative task bar and variations

**Landed**

- `generate.fill` takes `variations` (1..4): one backend run per variation with consecutive
  seeds, each its own masked layer, only the first visible, all in one undo step; the result
  lists `layers` and `seeds`, and the job's progress spans the runs ("Variation 2/3").
  `generate.variation {layers, index}` shows one of them, hides the others and activates it, as
  one undo step. `generate.models {probe: false}` lists the templates without touching the server.
- `crates/ui-egui/src/generative_bar.rs`: the task bar under the selection. Prompt (Enter
  generates), template picker (research templates only with the preference on), variations,
  Generate; a progress bar with Cancel while a fill runs on the document (its own or one a script
  started); a ‹ 1/3 › switcher after a multi-variation run; × hides it until the next selection.
  It sits centred below the selection, above it when there is no room, always on the canvas.
  Edit › Generative Fill… and the selection context menu open it; the generated dialog remains
  the fallback when the new `integrations.showGenerativeBar` preference is off or for scripts
  that pass params. `ui.set` gained `generativeBar`, `generativePrompt`, `generativeTemplate`,
  `generativeVariations`; `ui.inspect` reports `generativeBar`; the modal progress dialog stays
  away while the bar shows the job (`docs/control-protocol.md` updated).
- Tests: two engine tests (variations, the variation command and its validation), four UI tests
  (menu → bar → generate → switch against the fake server; the preference's dialog fallback; the
  bar's placement in a kittest harness; the placement arithmetic); six strings and one
  preference label translated in the 13 languages.
- Live: a two-variation Qwen-Image-2.1 fill from the bar on the lighthouse (a boat on the water),
  captured as `ui-genbar-idle/running/results.png` in `C:\Users\5090\ComfyUI\photocraft-tests`.

**Findings for the performance pass (next)**

- ComfyUI's official 2511 template chains UNETLoader → ModelSamplingAuraFlow 3.1 → CFGNorm 1.0 →
  (Lightning LoRA, 4 steps, CFG 1 | nothing, 40 steps, CFG 4) → KSampler euler/simple, with
  `FluxKontextMultiReferenceLatentMethod index_timestep_zero` on both conditionings. Our 2511
  fill lacks CFGNorm and the reference-method nodes. The 2511 Lightning LoRAs (lightx2v) are
  Apache-2.0: 850 MB bf16 files for 4 and 8 steps.
- Qwen-Image-2.1's few-step LoRA (Viggle turbo, 6 steps) carries the same research licence as
  the base model and needs the author's custom nodes (a sigma schedule and an unmerged LoRA
  loader); not a plain `LoraLoaderModelOnly`. `QwenImage21Cache` (already in our templates) is
  the other 2.1 speed lever.
- ComfyUI flags worth measuring on the 5090: `--fast fp16_accumulation` (ships in Comfy's own
  bat file; changes outputs slightly), `--highvram` (keeps models resident). SageAttention needs
  a wheel matching torch 2.14 / cu130 / Python 3.13 and is reported to break Qwen when enabled
  globally; left out.
- Photoshop's Generative Fill renders at most 1024 px on the longer side and upsamples; our
  2511 path does the same through `FluxKontextImageScale`, the 2.1 path keeps the input size.

**Still open (Phase 2)**: generative layer metadata, a committed control-protocol script test,
installed-model badges in the picker.

## 2026-10-09 (night): Phase 2, first slice: the generative commands are clickable

**Landed**

- Menu rows (fork additions in `menu_catalog.rs`, not parity rows): **Edit › Generative Fill…**
  and **Edit › Generate Image…** right after Content-Aware Fill, **Select › Select by Text…** (the
  command's own `menu`), and **Generative Fill…** first in the selection context menu
  (`canvas_tool_menu.rs`), where Photoshop puts it. Translated in the 13 complete-menu languages;
  `docs/parity.md` now counts 629 live items (the two Edit rows).
- Generated dialogs: the three commands are on `filter_dialog::has_dialog`'s allow-list (no
  preview). Their params docs were rewritten in the dialog notation, with 0 meaning "the
  template's default" for steps/guidance and "the document's size" for width/height, and
  `target: "auto|layer|document"`; the engine accepts those values from the CLI too (tests
  updated). OK runs the usual background job (`jobs_ui`: status-bar progress, Esc cancels), so
  the dialog itself needed no new UI code.
- Two fixes in upstream's `filter_dialog.rs`, both candidates for an upstream PR: `parse_spec`
  parses only the parameter object (a `:` in the notes after `→` leaked in as an `Int` field,
  visible as a stray `Ms"} (prompt` row), and ranges spanning ≤ 10 are stored with the two
  decimals the field shows (margin 0.25 was snapped to 0.3). Unit test added.
- Preferences › AI Integrations had been failing two ui-egui tests since Phase 1: the generated
  labels of the seven settings had no translations, and a test assumed the section was entirely
  unimplemented. Labels in 13 languages, `humanize` overrides ("ComfyUI server", "Generative
  timeout (seconds)"), the test now asserts the section is visible while agent control stays
  hidden.
- Visual check with the offscreen snapshot example, captures in
  `C:\Users\5090\ComfyUI\photocraft-tests\ui-*.png`: the three dialogs, the Edit and Select menus,
  the selection context menu, the Preferences page in English and German.

**Findings**

- The snapshot example panics on this PC's DX12 adapter inside egui-wgpu's staging belt
  (`renderer.rs:984`, the same failure as the 7 baseline canvas tests). `$env:WGPU_BACKEND =
  "vulkan"` works, and so does `WGPU_FORCE_FALLBACK_ADAPTER=1`; written into
  `dev-environment-windows.md` with the PowerShell quoting for `--script`.
- `input_tests::eyedropper_and_alt_sampling_show_a_pipette` fails the same way with
  `PHOTOCRAFT_CONFIG_DIR` pointing at an empty folder, so it is not reading machine preferences.
  Still unexplained, still in the baseline.
- The generated dialog is a stopgap: "0" for width/height/steps cannot explain itself, and there
  is no model picker. The task bar (Phase 2 item 2) replaces it.

**Numbers**: ui-egui lib 916 passed / 1 baseline failure; engine 859 + the generate/select
command tests; genai 29; i18n 1467/1467 in every language; parity 629/629; scorecard unchanged
(57 unread settings of 146); clippy, layers, wasm green.

**Still open (Phase 2)**: the task bar anchored to the selection, variations, progress preview,
a model picker fed by `generate.models`, generative layer metadata, and a committed
control-protocol script that selects, fills and screenshots.

## 2026-10-09 (later): Phase 3.5, Select by Text with SAM 3.1, live

**Landed**

- Template `sam3.1/segment` (community licence: the SAM License) from ComfyUI's official
  "Image Segment (SAM3)" template: `CheckpointLoaderSimple` (MODEL + CLIP from the 1.75 GB
  `sam3.1_multiplex_fp16.safetensors`), `CLIPTextEncode` on that CLIP, `SAM3_Detect`
  (threshold, 2 refinement passes, `individual_masks` on), `MaskToImage` → `SaveImage`, so the
  server answers with one PNG per instance. The server's `object_info` confirmed the node set.
- `Request.params` (template-specific placeholder values such as `threshold`) and
  `Error::NoOutput` (a detector that finds nothing) in the genai crate; the fake server answers
  `SAM3_Detect` graphs with one mask per configured rectangle.
- `crates/engine/src/select_ml_cmds.rs`: `select.byText` (phrase → instances → coverage →
  `sel::combine` with `replace|add|subtract|intersect`, `instance` picks one, `threshold`,
  `sampleAllLayers`; a background job and one undo step; reports each instance's bounds and
  pixel count) and `select.subjectML` (`what`: subject, person, face, hair, sky, animal, vehicle,
  text → a fixed phrase). Shared plumbing with `generate_cmds` (`plan_common`, backend, job
  progress, resampling), now `pub(crate)`.
- 7 command tests and 3 crate tests against the fake; clippy, wasm gate green.

**Live, on the 1024² lighthouse from the morning** (`photocraft-cli run … --cmd select.byText`):

| Phrase | Found | Bounds `[x, y, w, h]` | Pixels | Time |
|---|---|---|---|---|
| "the lighthouse" | 1 | `[695, 322, 111, 287]` (the tower, exactly) | 17 555 | 4.6 s, cold checkpoint load |
| "sky" | 1 | `[0, 0, 1024, 612]` (down to the horizon) | 601 550 | 1.8 s, warm |

Then `generate.fill` with `qwen-2.1/fill` on the lighthouse selection ("bold red and white
horizontal stripes", margin 0.4 → a 341×517 request) took 8.2 s and repainted only the tower
(`sam-lighthouse.psd/.png` in `C:\Users\5090\ComfyUI\photocraft-tests`). Select → describe →
generate, with no lasso, is now a two-command script.

**Still open**

- Point and box prompts (`select.byPoint`, the Object Selection tool's ML mode): `SAM3_Detect`
  takes `positive_coords`/`negative_coords` strings and `bboxes`; the coordinate string format
  needs reading from the node source before wiring.
- `panic_hunt` on a machine with ComfyUI running at the default URL will make real calls for
  `select.subjectML {}` (every other command fails validation first); run the hunt with the
  server stopped, as CI effectively does.
- Soft mattes for hair/glass (BiRefNet refinement) remain Phase 3.5's second half.

## 2026-10-09: first live runs against a real ComfyUI (Phase 1 DoD met)

**Setup on the dev PC** (nothing of this is in the repo; the app ships no weights, BYO models):
ComfyUI v0.39.0 Windows portable (NVIDIA build, 2.0 GB) at `C:\Users\5090\ComfyUI\ComfyUI_windows_portable`,
embedded Python 3.13.14, torch 2.14.0+cu130, RTX 5090 recognised. Start script
`C:\Users\5090\ComfyUI\start-comfyui.ps1` (loopback :8188, `--preview-method auto`). Model files
(Hugging Face, ~100 MB/s): Qwen-Image-2.1 int8 set 16.1 GB, Krea 2 Turbo fp8 set 17.4 GB,
Qwen-Image-Edit-2511 fp8mixed 19.1 GB + its 8.7 GB encoder (VAE shared with Krea 2). The server
listed the files without a restart.

**Findings that changed code**

- `TextEncodeQwenImage21`'s `latent` output is an **empty** latent sized to image 1, not an
  encoding of it (checked in `comfy_extras/nodes_qwen.py`): 2.1 edits in context and regenerates
  the whole picture, so `SetLatentNoiseMask` cannot inpaint with it. `qwen-2.1/fill` now sends
  the selection mask as a second reference image and wraps the prompt in a local-edit instruction
  (`meta.promptFormat`, new); PhotoCraft's layer mask confines the visible change. Result: correct.
- Grouped ("autogrow") inputs are spelled `images.image_1` in the API format (the server prefixes
  the group id; the first attempt with `image_1` failed with "unexpected keyword argument").
- `/object_info` on 0.39.0 has every class the four templates use; `CLIPLoader` offers both
  `qwen_image` and `krea2`.

**Numbers** (1024² text-to-image; fill = 384×320 selection, 576×512 request; `ms` is the
command's own timing, which includes upload, queue, sampling and download):

| Run | Template | Time | Notes |
|---|---|---|---|
| Text to image, cold | `qwen-2.1/image` | 28.4 s | model + encoder load from disk included |
| Text to image, warm | `qwen-2.1/image` | 10.8 s | 25 steps |
| Generative Fill, warm | `qwen-2.1/fill` | 4.7 s | boat placed exactly in the selection, seamless |
| Text to image, cold | `krea2-turbo/image` | 9.8 s | 8 steps; includes the 12 GB load |
| Generative Fill, cold | `qwen-edit-2511/fill` | 124.2 s | 40 steps + 19 GB load; see result note below |

Result note: the selection covered water on its left and rocks on its right. 2.1 (mask as a
reference image + instruction) placed a small boat on the water, matching the scene's light and
scale, with no visible seam. 2511 (latent noise mask) painted a larger boat on the rocks, inside
the selection but less plausible; worth retrying with the prompt describing the whole scene and
with the official template's `CFGNorm` node. Both results stayed inside the mask, as designed.
Krea 2 Turbo's text-to-image (8 steps) produced a different, photographic composition at the
same seed; Qwen-Image-2.1's fox test image is near photo quality.

VRAM after the 2.1 runs: 20.6 GB in use of 32 GB (ComfyUI keeps models resident); after all
three families had run: 21 GB (it swaps between them).

Outputs in `C:\Users\5090\ComfyUI\photocraft-tests\` (`q21-image.png`, `q21-fill.psd/.png`,
`krea2-image.png`, `q2511-fill.psd/.png`, `q21-fox.png`); the PSDs carry the masked
"Generative Fill: …" layer above the Background. Headless invocation, from the repo root:

```powershell
.\target\release\photocraft-cli.exe --% run in.png --cmd prefs.set --params "{\"values\":{\"integrations.allowResearchModels\":true}}" --cmd select.rect --params "{\"x\":320,\"y\":560,\"width\":384,\"height\":320}" --cmd generate.fill --params "{\"prompt\":\"a small red wooden rowing boat floating on the water\",\"template\":\"qwen-2.1/fill\",\"seed\":7}" --out out.psd
```

(PowerShell: one `--%` command per line; everything after the token is passed verbatim.)

**Still open**

- Judge the 2511 fill quality against 2.1 on more images; consider the official template's
  `CFGNorm` node and the Lightning 4-step LoRA for 2511.
- Variations, the task bar and menu rows (Phase 2); `generate.edit` / `expand` /
  `removeBackground` (Phase 3); `select.byText` with SAM 3.1 (Phase 3.5).
- The ComfyUI server was left running after the session (`Stop-Process -Name python` or close it
  from the start script's window to free 21 GB of VRAM).

## 2026-10-08 (later): Phase 1, the generative backend and `generate.fill`

**Landed**

- `crates/genai` (`photocraft-genai`, L4, registered in `xtask/src/layers.rs`): the
  `GenerativeBackend` trait (`health`, `run`, `model_files`), `Request`/`Response` over RGBA8 +
  Gray8 pixels, `Progress` for job progress and cancellation, `template` (API-format graphs with
  typed `{{placeholders}}`; built-in `qwen-edit-2511/fill` from ComfyUI's official Qwen-Image-Edit
  2511 template plus `ImageToMask` → `SetLatentNoiseMask` for inpainting), `comfy` (a blocking
  client on ureq 3.4 + tungstenite 0.30, no TLS: `/system_stats` version check → `/upload/image`
  ×2 → `/ws` → `/prompt` → socket progress + `/history` polling → `/view`; cancel = `/interrupt` +
  queue delete; deadline from the preference), `png` (via photocraft-codecs), and `fake` (feature
  `fake-server`: an in-process ComfyUI stand-in with HTTP + WebSocket, knobs for delay, failure,
  rejection, missing nodes, no-socket).
- `crates/engine/src/generate_cmds.rs`: `generate.fill` (validated params → composite crop with a
  25 % context margin + selection coverage → backend → result resampled if needed → **new raster
  layer in the document's own depth, above the active layer, with a layer mask equal to the
  selection**; one undo step; background job with progress and Esc-cancel), `generate.health`,
  `generate.models`. Commands have `menu: &[]` until Phase 2 (so parity and i18n are untouched).
- `generate.image` (pulled forward from Phase 3 on request): text to image with the built-in
  `krea2-turbo/image` template (from ComfyUI's official `image_krea2_turbo_t2i.json`: UNETLoader,
  CLIPLoader type `krea2`, CLIPTextEncode, ConditioningZeroOut negative, EmptyLatentImage,
  KSampler euler/simple 8 steps CFG 1, VAEDecode). `target: "layer"` covers the whole canvas
  (size defaults to the document, rounded down to the 16-px grid, min 64, resampled back),
  `target: "document"` opens a new RGB 8-bit document of the result's size (default 1024²).
  Krea 2 is a community-licence model: the catalogue says so and nothing pre-selects it for fill.
- Qwen-Image-2.1 templates (`qwen-2.1/fill`, `qwen-2.1/image`), from ComfyUI's official 2.1
  image-edit and text-to-image templates: `UNETLoader → QwenImage21Cache`, `CLIPLoader` type
  `qwen_image`, `TextEncodeQwenImage21` (positive, negative and the encoded latent of `image_1`,
  resolution 0 keeps the input size), KSampler euler/simple 25 steps CFG 1, the 2.1-specific VAE
  `qwen_image_2.1_vae_bf16`. Fill masks the encoder's latent with `SetLatentNoiseMask`. Both are
  **research-licence** templates: refused with a message until `allowResearchModels` is on, and
  listed as `allowed: false` by `generate.models` meanwhile.
- Preferences › AI Integrations: `comfyServer`, `defaultEditModel`, `defaultGenerateModel`,
  `defaultFillTemplate` (`qwen-edit-2511/fill`), `defaultImageTemplate` (`krea2-turbo/image`),
  `generativeTimeoutSecs`, `allowResearchModels`, each read by the commands and validated. Setting
  `defaultFillTemplate = qwen-2.1/fill` with the research opt-in makes 2.1 the everyday editor.
- Tests: 25 in the genai crate (unit + the client against the fake server: upload/queue/wait/
  download, polling without a socket, overrides, server failure, rejected prompt, cancellation
  interrupts within 3 s, deadline, unreachable server, old server version, health/model files,
  request validation) and 18 engine command tests (fill: masked layer geometry and pixels, undo,
  the uploaded crop and mask, background job parity with inline, cancel leaves the document
  alone, server failure, unreachable server, health/models, parameter validation, enablement,
  model overrides, a 16-bit document, resampling; image: canvas layer, new document, size
  rounding and validation, background job, template/task checks, model preference).
  `cargo test -p photocraft-engine --test prefs_usage` green.
- Gates on 2026-10-08: `cargo fmt`, `cargo clippy --all-targets -- -D warnings` on both crates,
  `cargo xtask layers` (29 crates, no violations), `cargo xtask wasm` (genai and engine check for
  wasm32; the client is native-only and seeds fall back to a counter there), `cargo xtask parity`
  (unchanged: 627/627, the commands have no menu rows yet), `cargo xtask scorecard` (57 unread
  settings of 144; the five new preferences are read), `panic_hunt --ignored` green in 25 s with
  the new commands, release `photocraft-cli` built (1 min 43 s); `photocraft-cli commands --filter
  generate` lists the four commands. Headless `generate.health` / `generate.fill` without a server
  return "cannot reach the ComfyUI server at http://127.0.0.1:8188 … start ComfyUI or change the
  server URL in Preferences › AI Integrations".

**Still open**

- A live run against a real ComfyUI (not installed on this PC yet) with the 2511 files; the
  template's node names come from the official template but have not executed here. First live
  run should also decide whether to add back `CFGNorm`.
- Phase 2: menu rows + translations, the generative task bar, variations, previews.
- `docs/comfy/models.toml` catalogue and `generate.image` / `generate.edit` / `removeBackground`
  (Phase 3) reuse the same crate; `select.byText` (Phase 3.5) needs a SAM 3.1 template.

## 2026-10-08: Phase 0, foundation

**Landed**

- Cloned `storytold/photocraft` (`main` @ `d836e34`, 642 commits, 49 MB) to
  `C:\Users\5090\Projects\comfy-photocraft`; branch `comfy-photocraft` created; local git identity
  set to the owner's email.
- Toolchain on the Windows PC: `scoop install rustup gh` → rustup 1.29.1, Rust stable 1.99.0
  (MSVC, using the pre-installed VS 2019 Build Tools), gh 2.102.0. No Python/Node on the machine
  (not needed for Rust; ComfyUI brings its own Python).
- Release build: `cargo build --release -p photocraft` **2 min 39 s** cold; `photocraft.exe`
  58.5 MB; `--version` → `photocraft 0.5.0 (dev build)`. `cargo build --release -p photocraft-cli`
  2 min 10 s (incremental on the same deps).
- Test baseline, `cargo test --workspace --no-fail-fast` (debug profile, 138 s after the build):
  **171 test binaries, 4227 passed, 9 failed, 32 ignored.** All failures are environment-related
  on this RTX 5090 / driver 617.42 / wgpu DX12 machine, none in engine or format code:
  - `photocraft-gpu --test parity adjustment_layers`: "Hue/Saturation Normal: max diff 1.00/255
    at (18,23)", i.e. exactly at the tolerance edge (GPU rounding differs from CI's software
    adapter).
  - 7 egui-wgpu canvas tests (`canvas_16f` ×2, `color_managed_canvas` ×2, `drag_preview_canvas`
    ×2, `live_stroke_canvas` ×1) panic inside `egui-wgpu 0.36.2 renderer.rs:984` "Failed to create
    staging buffer for index data" (a wgpu staging-belt issue on this adapter, not PhotoCraft code).
  - `photocraft-ui-egui --lib input_tests::eyedropper_and_alt_sampling_show_a_pipette`:
    "Precise keeps the crosshair" (unexplained; may read the machine's real preferences; retest
    with `PHOTOCRAFT_CONFIG_DIR` pointing at an empty folder).
  Everything else (engine 831 tests, ui-egui lib 915, io, psd, codecs, compose, automation…) is green.
- Research (web, dated 2026-10-08) on Krea 2 (open weights 2026-06-22, Krea 2 Community License,
  ComfyUI ≥ 0.26), Qwen-Image-2.1 (2026-09-20, **research-only licence**), Qwen-Image-Edit-2511
  (Apache-2.0), Z-Image Turbo (Apache-2.0), FLUX.2 variants, the ComfyUI HTTP/WebSocket API,
  Photoshop 2026 generative feature list, Krita's ComfyUI plugin, Rust ComfyUI crates.
  Finding worth repeating: **"Krea 2 Edit" is a community LoRA, not an official Krea model.**
- Codebase analysis delegated to Codex (`codex exec`, read-only, gpt-5.5) and verified by hand;
  key facts: `CommandSpec` fn-pointer registry, `jobs::edit_job` background jobs with progress
  and cancel, `ml` reserved at L4 in `xtask/src/layers.rs`, no HTTP client in the tree, an
  existing `integrations` preference section with an "AI Integrations…" menu entry.
- Documentation set in `docs/comfy/` (README, codebase-orientation, architecture, models,
  comfyui-setup, roadmap, dev-environment-windows, upstream, this log) and a root `CLAUDE.md`;
  a fork notice in the top-level README.

- `ai-integration-proposal.md`: the full catalogue of ComfyUI + local-LLM integrations (17
  generative, 9 perception, 8 language items, 8 infrastructure items) with a recommended order;
  adds "Select by text" and the in-app Assistant as new phases 3.5 and 4.5.
- SAM 3.1 researched and written in: Meta, 2026-03-27, text/point/box prompts, native in ComfyUI
  (PR #13408, one 1.75 GB checkpoint), SAM License (commercial OK, military/weapons banned, licence
  passed on). It is the model behind Phase 3.5 (`select.byText`, `select.subjectML`,
  `select.byPoint`); design sketch in the proposal §2.1.

**Numbers to carry forward**

| Metric | Value | How |
|---|---|---|
| Release build, cold | 159 s | `cargo build --release -p photocraft` |
| Test suite | 4227 pass / 9 fail / 32 ignored, 171 binaries | `cargo test --workspace --no-fail-fast` |
| Upstream menu parity | 627/627 | `docs/parity.md` |
| Generative commands | 0 | — |

- Published: fork **https://github.com/CodyChuGit/comfy-photocraft** created with `gh repo fork`,
  `comfy-photocraft` pushed (`5aed6c2`), `upstream` → storytold/photocraft. The push first hung on
  scoop git's `helper-selector` credential GUI; the repo now uses gh as its only credential
  helper (`dev-environment-windows.md` › Remotes and pushing).

**Still open**

- Install ComfyUI (portable build) and the Phase 1 model files; record versions and VRAM here.
- Decide the published product name and the brand-asset removal before any public build
  (`docs/comfy/upstream.md`).
- Check whether the 7 egui-wgpu staging-buffer failures and the 1/255 parity edge are known
  upstream (search issues for "staging buffer" / "parity 1.00/255"); if not, file them with the
  adapter/driver details above.
- Phase 1 starts with `crates/genai` (see `docs/comfy/roadmap.md`).
