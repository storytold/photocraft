# PhotoCraft ONNX QNN Worker — validation-only ARM64 draft

**Not hardware-validated. Not a usable model bundle. Not cleared for runtime redistribution.** This independent external C# tool leaves PhotoCraft's native Rust engine unchanged. Its long-term inclusion/maintenance under the repository's Rust-only policy remains a maintainer decision. No MSIX, Windows AI system-model package, certificate installation, webview or main-process CLR dependency is introduced.

This revision was reviewed/edited statically only: **no local cargo/dotnet build, test or publish was run**. GitHub Actions must establish whether it compiles and whether the proposed payload meets the actual caps. No green result, NPU performance/accuracy or target-device compatibility is claimed in advance.

## What remains real, and what is not claimed

[The production backend](Runtime/QnnBackend.cs) calls locked ONNX Runtime QNN/Managed **1.24.4** APIs, selects `AppendExecutionProvider("QNN", ...)`, binds the absolute app-local HTP DLL and sets:

```text
session.disable_cpu_ep_fallback = 1
offload_graph_io_quantization = 0
```

It never retries session creation or inference with CPU, GPU, DirectML or a fake backend. Unsupported graph partitioning, model bindings, drivers or devices must fail. CPU image preprocessing/postprocessing still occurs; strict model-graph configuration is not physical-NPU attestation. App-local QnnCpu/Gpu files are preserved for complete source-payload accounting but never selected.

The tests' fake backend is compiled only into the separate pure-test executable. Its synthetic protobuf envelope is **not** an ONNX inference model. No fixture, fake mask, claimed device string or cloud ARM64 runner proves HTP execution. There is no SINet support claim or invented model shape/hash: this first version accepts only **one float32 RGB input and one scalar-per-pixel float32 output**, probability or sigmoid. Two-class logits, softmax/foreground-class selection and typical SINet exports are not supported. Human-portrait segmentation and general-object subject selection are different tasks; no quality claim exists for either.

## Validation and legal release gate

[Build-Package.ps1](Build-Package.ps1) is a CI-only validator despite its historical name. It:

1. Restores and publishes a full, self-contained `win-arm64` directory with matching RID, PlatformTarget and SelfContained properties. No trimming, single-file packing or ReadyToRun. Restore audits transitive packages and treats audit warnings as errors.
2. Verifies NuGet archive signatures, the reviewed QNN archive digest and actual ARM64/ARM64X native PE machines. Preserves all 15 actual pinned native assets, not just whichever files NuGet happens to copy. The apphost itself must be ARM64.
3. Requires HTP Prepare/System, **V73 and V81** Stub/Skel/CAT assets and ORT core/shared/QNN DLLs. Copies every source native asset verbatim and compares source/destination SHA-256. Missing files are errors, never warnings. No hand-built CPU-only ORT replacement is allowed.
4. Copies actual QNN/Managed LICENSE, Privacy, Qualcomm PDF and ThirdPartyNotices, plus restored transitive/.NET runtime notices. Copies these docs **before** constructing the host file list. The QNN top-level MIT description is not a licence for every Qualcomm binary.
5. Enforces host limits: **512 files**, **256 MiB per file**, **512 MiB total listed files**, **1 MiB manifest**, regular safe relative paths, no case-insensitive duplicates, links or reserved paths. Does not raise caps or strip framework/legal files to fit. A full publish exceeding caps must fail for review.
6. Writes current host-compatible `adapter.json`: exactly `package_version`, nested `descriptor`, `launch: {kind: executable, path: ...}`, `files: string[]`, `timeout_ms`. Docs/notices and the empty registry are in `files`; the manifest itself is separately snapshotted by the host. Inventory SHA-256 is not the host's BLAKE3 install record.
7. **Never creates a ZIP.** [Legal review is BLOCKED](LEGAL-REVIEW.md), with no workflow input, environment override or script switch to pretend otherwise. A future binary release needs a separate reviewed change.

The [QNN workflow](../../.github/workflows/companion-qnn.yml) uploads **only explicitly allowlisted JSON/log evidence**, never the published runtime folder or restored packages. Actual licence PDFs are retained on the ephemeral runner and inventoried, not uploaded as a runtime distribution. [Upstream observations and exact actual asset names](UPSTREAM-VERIFICATION.md) explain the pins and review basis. Neither this review nor NuGet audit is a general security/legal certification.

## No model is shipped, downloaded or fabricated

The generated registry is deliberately inert:

```json
{"schemaVersion":1,"defaultModel":null,"profiles":[]}
```

Probe, prepare and a valid run return `needs_model` without loading ORT when the registry, default profile or selected registered model is missing. Model absence is checked before the platform/native payload check, so no-model protocol smoke is hardware-neutral. If a verified configured model exists on a nonnative Windows ARM64 process, the worker returns `unavailable`; a configured ARM64 model with broken runtime/config/digest returns an error instead of fallback.

Only the fixed registry beside the EXE can bind opaque IDs to files; requests cannot select paths, DLLs, URLs, custom ops or runtime options. The strict registry shape is:

- Root: `schemaVersion` (1), `defaultModel` (registered ID or null), `profiles` (0..32).
- Each profile: `id`, `operation` (`select-subject`), relative `model` ending in `.onnx`, actual 64-hex `sha256`, `input`, `output`.
- Input: explicit `name`, `layout` (`nchw`/`nhwc`), fixed batch-one 4D `shape` with **3** channels, three finite `mean` and nonzero finite `scale` values, `resize` (`nearest`/`bilinear`), `colorPolicy` (`assume-srgb`), `alphaPolicy` (`ignore-straight`). Normalization is `(RGB - mean) * scale`; RGB channel order, no crop/letterbox/antialias.
- Output: explicit `name`, fixed batch-one 4D single-channel `shape`, layout, `postprocess` (`probability` or stable `sigmoid`), explicit resize method. Runtime names/dtype/shapes must match exactly. No first-output guessing; no auxiliary/dynamic/multiclass bindings.

No runnable profile example is supplied because no export/digest/shape/licence/QDQ calibration/HTP compatibility has been verified. Supply only a separately reviewed specific export, including the real upstream source/licence, digest, preprocessing/geometry and model-quality evidence. A zero/fabricated digest or random downloaded model is not an example of support.

## Explicit v1 transition boundaries

This is **not** the proposed v2 resource store. No model acquisition, network download, immutable resource manifests, per-model probe/prepare, eviction, caching, arbitrary tensors, multiple outputs, custom operators, EPContext, protected Adobe assets or Photoshop binary-plugin loader is implemented.

- v1 probe/prepare can inspect only the package's **default profile**. Probe verifies local bytes/envelope/payload but does not create a QNN session. Its `ready` status means that narrow check only, not “NPU ready.” Prepare creates/checks the default session without inference. Each run verifies and creates a new session for its selected opaque registry ID. No session cache/persistent preparation is claimed.
- The local registry/model paths are a deliberately limited allowlist, not a general resource-package system. After host installation its files are integrity-recorded: editing the installed registry/adding weights is not a supported provisioning flow. A reviewed complete package must be rebuilt/reinstalled explicitly; do not change host caps or sneak large weights into this worker as a substitute for a future independent store.
- Request framing is strict v1 file IPC (`PCADREQ1`/`PCADRES1`), same-geometry mask output only. RGB/RGBA SDR finite [0,1] f32 samples preserve 8/16/32-bit provenance. Require explicit `params.colorPolicy = assume-srgb` for **untagged** RGB. Any ICC tag/HDR/unsupported mode is rejected; no silent colour conversion/clamping. Straight alpha is explicitly ignored for analysis. Optional selection is validated but not used as a seed or multiplied into the authoritative mask.
- [The envelope scanner](Core/OnnxEnvelope.cs) is a **scope restriction, not a complete protobuf/ONNX parser or sandbox**. Public proto field/wire rules, recursion/work caps, strict UTF-8 security names and malformed-input tests protect its own boundaries. External tensors are rejected through dense/sparse/attribute/nested-graph paths; custom domains/opsets, functions/training/EPContext, unknown structural fields, wrong wires and duplicate singular fields are refused. IR >10 is deliberately unsupported pending review. Q/DQ presence is not quantization correctness or NPU compatibility; ORT still validates model semantics.
- Model/config paths reject traversal, UNC/ADS/absolute/hidden/device names and reparse-point ancestors. SHA-256 validation holds a deny-write/delete read handle through session disposal on Windows, but ORT reopens a pathname: ancestor-renaming/hardlink/racing filesystem/privileged replacement remain TOCTOU limitations. Never run untrusted models. Native process separation is **not an OS sandbox**; worker/runtime retain the user's authority.
- There is no cooperative cancellation/progress implementation inside a synchronous ORT call. The host's process deadline/termination is the v1 boundary; comprehensive cancellation/process-tree tests and target-device timeout behaviour remain future work.
- Self-contained .NET does not include Qualcomm drivers or guarantee CRT/OS dependencies. Real Snapdragon device/OS/driver compatibility must be checked separately. V73/V81 package presence does not imply every Snapdragon generation is supported.

## Remote validation scope (not yet claimed passing)

| Layer | Proposed job/evidence | What it does not establish |
|---|---|---|
| T1 | Windows x64 pure Core tests: framing/strict JSON/limits/f32, profile/digest/path rules, envelope malformed/nested routes, normalization/interpolation/sigmoid, fake scheduling/no retry/disposal; pure PowerShell cap/manifest/script-syntax assertions | Native loading, ORT execution, model validity/quality, NPU |
| T2 | Not run by this workflow; Rust↔C# contract evidence belongs to the separate main/contract CI | No invented interoperability claim from C# self-roundtrips |
| T3/T4 | `windows-11-arm`: full self-contained ARM64 publish/native-DSP/legal/byte inventory, real apphost no-model probe/prepare/run/error frames and bounded process exits | Cloud ARM64 is not Qualcomm HTP; no real graph/session/inference or full cancellation proof |
| T5 | **Not performed**: authorised Snapdragon acceptance with a specific licensed/hashed model, strict graph placement, numerical/quality/performance tolerances and OS/driver record | No arbitrary-model or Photoshop Neural Filter compatibility |

The ARM64 job requires the public `windows-11-arm` runner to be available. If unavailable, it must remain visibly pending/failing until an appropriate trusted ARM64 runner is chosen, not silently convert into hardware evidence or run arbitrary PR code on a user's NPU PC. All builds/tests/publishing in this plan are remote; do not use a local build to bypass review.
