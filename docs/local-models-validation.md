# Optional local models: validation

Recorded 2026-10-08 on an Apple M3 Max with 36 GiB RAM, Rust 1.98.0, native CPU inference
and the immutable model exports listed in [local-models.md](local-models.md). No learned weights
are included in this repository. The integration/performance checks below are supplemented by a
fixed 22-case evaluation. Its 20 labelled COCO targets are exploratory evidence, not a representative
or held-out benchmark, a soft-alpha matting assessment, or evidence of a universal winner.

## CI follow-up: 2026-10-11

Integrated upstream `6ff2a03f33dbe84e726e9cc0c31787de8f91f50b`, including its
platform-independent save-dialog regression fix. The batch merge keeps upstream's called-action
state and nested-error handling alongside the explicit model backend, authorization and method
preferences. The extended batch regression verifies model inference inside a called action and
rejects the same call when the host has not granted a backend.

Full native suites with the optional backend and HEIF passed on macOS: 1,179 engine / 1,733 UI /
52 automation / 12 ML unit tests, plus CLI and integration tests (11 / 10 / 0 / 1 ignored).
The extended batch regression also passed. Final lint/platform results are linked from the PR;
this follow-up does not rerun or change the historical accuracy/performance evaluation.

## Review update: 2026-10-10

Rebased on upstream 0.6.0 (`ab76c242a1225efc680dc88bd761cc202e1e29df`). The optional model
workflow is manual-only (`workflow_dispatch`), with no push/PR triggers. The development guide
now identifies the ONNX Runtime build-time download and offline limitation. The
[export review rationale](local-models.md#why-these-community-exports) links exact official
checkpoint trees and community metadata, explains why ONNX re-exports remain selected, and
separates artifact identity from independently proven conversion equivalence.

All 18 model labels now cover Dutch and Ukrainian and the other 14 complete catalogs, guarded
by a runtime lookup regression. Existing catalog entries and all other contributors' credits
are preserved. New entries are kept away from append locations to reduce future conflicts.
The lockfile preserves all 728 existing upstream package versions; it adds only the optional
backend entries. Model revisions/bytes and the historical benchmark remain unchanged.

Fresh optional-native tests, lint, default builds, dependency layers, parity, scorecard,
adversarial command tests and wasm checks are linked from [#1199](https://github.com/storytold/photocraft/pull/1199).
Native engine tests passed 1,169 / 11 ignored; the final UI suite after the file-dialog update
passed 1,721 / 10 ignored, with all integration tests green. Five-package all-target Clippy
(`local-ml,heif`), the default build, layers, parity (628/628), scorecard, optional-feature
adversarial commands, full L0–L6 wasm and optional engine/UI wasm all pass. Ignored tests
are not counted as passes. An offscreen 1200×800 Preferences › Integrations capture was
inspected: both explicit downloads are offered while both selected methods remain Classical.
Historical screenshots below are not current 0.6.0 captures.

## Upstream integration check: 2026-10-09

Integrated upstream `2515fa7cee624cf232c873ba283aa0359ec159d7`. The integration keeps the
upstream Rotate View preference, Linux display-server selection and Wayland snapshot behavior alongside the optional
model controls. It adds model labels to the newly supported Greek catalog and keeps every
upstream translation and contributor entry. Additive catalog/self-credit blocks are placed
before existing entries to reduce conflicts with future upstream additions.

The native optional-backend suites passed on macOS: engine unit tests **945 passed / 11 ignored**,
UI unit tests **1,112 passed / 7 ignored**, plus ML runtime and CLI integration tests. Five-package
all-target Clippy with warnings denied, the default native build, layers, 627/627 parity,
regenerated scorecard, optional-backend adversarial panic hunt, full L0–L6 wasm and optional
engine/UI wasm checks passed. Ignored tests are not counted as passes. The native matrix now
also runs model Preferences and complete-language catalog tests; its current run is linked from
[#1199](https://github.com/storytold/photocraft/pull/1199).

The real-image measurements and screenshots below remain the earlier evidence at their stated
commits; rebasing does not turn them into a new accuracy or performance benchmark.

## Preferences before and after

Before, from the unmodified `5896f0b` source:

![Preferences before](images/local-models-before.png)

After, with both models explicitly downloaded into an isolated development cache. The two
methods still default to Classical. Downloads/removals use the engine's existing progress and
cancellation jobs; Apply/OK saves the method choices.

![Preferences after](images/local-models-preferences.png)

## Real model outputs and timings

The comparison uses Vermeer's public-domain *Girl with a Pearl Earring*. The 960 × 1137 source
was resized to 4500 × 5333 (23,998,500 pixels) to exercise the required 24 MP processing path;
upsampling does not add detail or make it an accuracy dataset. Each command ran twice in the
same session, with Undo restoring the input between runs. Timings include preparation, hash
verification, model loading on the first run, inference and document apply, excluding PNG export.

| Command / method | First run | Second run |
|---|---:|---:|
| Remove Background / Classical | 318 ms | 306 ms |
| Remove Background / BiRefNet HR Matting | 35,275 ms | 28,790 ms |
| Object Selection / Classical | 543 ms | 296 ms |
| Object Selection / SAM 2.1 Large | 7,112 ms | 5,555 ms |

For Object Selection, the same rectangle starts at 10% of the width and 5% of the height and
covers 85% × 95% of the canvas. Selection was converted to a layer mask only for export.
The checkerboard is a presentation background under the actual exported alpha, not painted
into the masks. Left to right: original, classical removal, BiRefNet removal, classical object
selection, SAM object selection.

![Actual command outputs](images/local-models-comparison.png)

The classical method works well on this example. BiRefNet supplies continuous edge alpha;
SAM leaves some unwanted holes. These results support keeping both choices available. The
exploratory multi-image evaluation below also includes failures and cases where Classical is better.

Repeated BiRefNet runs initially exhausted memory with ONNX Runtime's default CPU arena and
memory-pattern caching. Disabling both while retaining only the current model's weights let
all eight runs complete. Peak RSS was not measured; several GB of RAM are still required.

Reproduce using verified, explicitly installed models and a public-domain 24–36 MP image:

```sh
cargo run --release -p photocraft-engine --features local-ml --example local_models_bench -- \
  /absolute/model-cache /absolute/image.png /absolute/comparison-directory
```

## Fixed multi-image evaluation

The [33-page comparison PDF and public reproducibility supplement](https://github.com/mighty-programmer/photocraft/releases/tag/model-evaluation-2026-10-08)
contain all 22 distinct cases: 20 COCO val2017 images with independent binary instance references,
plus two user-supplied LEGO photos for visual inspection. There are 44 paired comparisons,
88 final method outcomes and 86 actual exported cutouts. BiRefNet's no-subject chair result and
Classical's empty zebra object selection are explicit failures, counted as zero rather than omitted.

| Feature | Classical mean target IoU | Learned mean target IoU | Learned wins / losses / near-ties |
|---|---:|---:|---:|
| Remove Background | 31.5% | 51.5% (BiRefNet HR Matting) | 15 / 2 / 3 |
| Box Object Selection | 52.5% | 79.1% (SAM 2.1 Large) | 16 / 3 / 1 |

Cases were selected using a fixed hash seed before inference, one from each of 20 categories.
Alpha is thresholded at 128/255, and a win/loss requires over one percentage point of IoU change.
Object boxes are derived from the annotation with 10% padding and reused for both methods.
The two unlabelled LEGO photos are excluded from the means. Each background-removal reference
contains one instance; keeping other plausible foreground subjects counts as a false positive.
COCO polygons cannot establish hair, soft-alpha or transparent-glass matting quality. This selected
pilot may overlap training data, and its annotation-derived boxes are easier than imperfect user boxes.

Median warm command times on the M3 Max / 36 GiB were 0.19 / 26.63 seconds for Classical /
BiRefNet removal and 0.20 / 5.59 seconds for Classical / SAM selection. These single-pass observations
exclude image import/export and initial/model-loading cases; they are not Windows/Linux timings.
Storage pressure interrupted two exports, which were rerun; the supplement preserves all 90 raw
attempts, 88 final outcomes and the recovery notes. Sampled resumed-run peak RSS was 13.35 GiB;
initial memory samples were not saved.

All methods ran in the same release engine at benchmark commit
`6e2b7bc9b20938787cf1f7af9718a832a2edf716`. The Classical algorithm sources match stock 0.3.0
`60224d3fb7d4006bcfcc97603c1611b9b756aebd`; the installed stock GUI was not timed separately.
Later UI/account changes were not re-benchmarked. The release tag identifies the tested engine,
and the supplement supplies the exact rectangles, annotation polygons/reference masks, source
hashes, command logs, CSV scores, model revisions and Rust harness. Its helper downloads and
hash-checks the 20 public inputs; model installation stays explicit. The report retains photo and
annotation attribution. Standalone user-photo originals, full-resolution cutouts and model weights
are not bundled in the public supplement or software repository.

## Checks

- ML unit tests execute both contracts through the actual CPU runtime using original tiny ONNX
  graphs. Streaming tests cover truncation, corruption, overlong data and cancellation; lifecycle
  tests cover scoped removal, busy state and dependency panic recovery.
- Engine tests cover RGB/Gray/CMYK/Lab at 8/16/32-bit, ICC input conversion, fractional alpha,
  layer-mask replacement, selection modes and rectangle mapping, one-step Undo/Redo, cancellation,
  missing-model errors, recorded method choices, batch/droplet capability inheritance and model
  management while a selection floats. Non-finite/out-of-range alpha is sanitized for model input
  while original 32-bit pixel values are preserved.
- Full engine, CLI and UI test suites pass locally. UI preference/localization coverage includes
  every supported complete-menu language. Default and `local-ml` build paths retain classical
  operation without installing models.
- The complete L0–L6 `xtask wasm` check and an additional optional-feature engine/UI wasm check
  pass. ONNX Runtime remains outside the wasm dependency graph. Layers and generated parity /
  scorecard checks pass; menu coverage remains 627/627.
- Clippy with all targets across the six touched packages and `local-ml` passes with warnings
  denied. The ignored `panic_hunt` test passes with the optional feature enabled.
- The opt-in full-model CPU smoke test verifies the real files, then runs both actual model
  pipelines. The repeated 24 MP release example above exports all four actual command results.
- `xtask perf --quick --bench perf_scenarios` passes for the existing selection/retouch scenarios;
  it does not establish a sub-100 ms budget for the optional models.

The dedicated `local-models.yml` workflow runs Linux, Windows and macOS CPU runtime tests and
native build checks. The [cross-platform run at `1904396`](https://github.com/mighty-programmer/photocraft/actions/runs/37849240044)
passes on all three platforms, including the final document-state and input regression tests.
The contribution is ready for review. Upstream PR CI needs maintainer approval for
this first contribution. Representative held-out/matting evaluation, packaged installer / sandbox
download checks, web inference, point prompts and GPU acceleration remain separate work.

Image sources and licences: [SOURCES.md](images/SOURCES.md),
[LICENSE-local-models.txt](images/LICENSE-local-models.txt).
