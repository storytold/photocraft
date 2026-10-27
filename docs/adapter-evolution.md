# Compatibility adapter audit and evolution

> **2026-10-21 — implementation audit and design backlog, not a v2 support claim.**
> The existing API/worker/package remain v1. A concrete worker implementation and a
> passing host test do not by themselves establish model compatibility or NPU execution.

## Goal and reusable boundary

The reusable unit is a host-neutral **image-effect request**, not an Adobe model format and
not a vendor inference ABI. Preserve this separation:

```text
PhotoCraft host bridge ─┐
other editor bridge ───┼─ versioned request / result ─ trusted worker ─ model loader / runtime
Photoshop host bridge ─┘                                       ├ Windows system AI
                                                             ├ ONNX Runtime / QNN
                                                             └ independently implemented loader
```

No document/session/egui/runtime handle belongs in the neutral contract. Existing opaque
resource references must remain byte-for-byte unchanged; adapters interpret references.
A model loader and a Photoshop plug-in host bridge are distinct implementations.

## Current support and boundaries

| Area | Implemented foundation | Remaining boundary |
| --- | --- | --- |
| Execution | Operation/provider/device IDs, bounded object parameters, opaque resource, background jobs, cancellation and stale-result rejection | No arbitrary layer-edit or host callback protocol |
| Image data | Native color mode, ICC bytes, 8/16/32-bit source metadata and straight-alpha f32 samples | Worker v1 has a **combined 1 MiB JSON header**; ICC is a JSON byte array, so core's 4 MiB ICC allowance cannot always cross this transport |
| Inputs/results | One image + optional same-geometry selection; one image/mask/depth result | No reference images, text/tensor ports, multiple results, changed output geometry, model-specific depth units |
| Host writeback | Pixels, selection and layer mask with transactional undo | `depthLayer`, resizing/super-resolution writeback, generated new layers not implemented; subject path still quantizes to u8 for legacy refinement/selection combination |
| Deployment | Trust confirmation, disabled install, separate enable, file inventory and BLAKE3 integrity checks, bounded process IPC | Not an OS sandbox or publisher signature; model store is not separate from runtime files |
| Device control | Exact requested device checked; changed/mixed device set needs allowed fallback and explanation | No independent NPU attestation or structured graph-placement/runtime/driver report; unspecified device is not a require-NPU policy |
| Control | Explicit probe and consented prepare, without automatic probe/download during discovery or idle UI | V1 control frame is only `{\"protocol\":1}`: readiness/preparation cannot target a named resource |
| Model discovery | Adapter declares resource-format labels | No actual model catalog, per-model capability/availability/version/terms, install/evict/cache keys |
| Parameters | Provider-level opaque `params_schema` | Not validated JSON Schema; no per-operation schema/version or generated general adapter editor |

A second backend must not be picked solely because its adapter ID sorts first. Automatic
subject selection now filters mask operation, input color mode, requested device and cached
package readiness; it does not launch probes. An installed package must be explicitly checked
Ready before automatic routing. Embedded Rust adapters retain their own readiness policy.
Explicit provider requests stay strict: they do not silently select a different provider or
turn an unavailable required NPU run into a classical/CPU result. Cached readiness can become
stale; run-time validation remains authoritative. This is not a user-configurable preferred
provider system yet.

## Immediate correctness work

1. **Cross-language framing:** the original Windows AI companion serialized extra `ready_state`
   and `descriptor` fields although Rust v1 rejects unknown fields. Keep the decoder strict and
   correct the producer. CI must generate real C# frames and decode them with Rust; same-language
   roundtrips alone are insufficient. Add request-direction fixtures as well.
2. **QNN implementation:** ordinary Windows ARM64 worker, no mandatory MSIX. Real ORT QNN EP,
   `QnnHtp.dll`, disabled CPU graph fallback, complete native/DSP dependencies and license notices.
   Begin with a documented fixed-shape, float graph-I/O model profile and scalar segmentation
   masks, not arbitrary ONNX graphs. Profile metadata specifies exact bindings/pre/postprocessing
   and expected artifact digest. Missing compatible weights is an actionable state, not fake output.
3. **Packages:** allow up to 512 explicitly inventoried runtime files (complete .NET/QNN payload),
   still 256 MiB per file and 512 MiB total. Do not remove required runtime assets to fit a cap or
   expand the package cap into an unlimited model store. Installation copies only declared files;
   changing a managed profile/model requires explicit trusted reinstall in v1.
4. **Verification and performance backlog:** stream package hashes with cancellation/deadline,
   move all large preflight I/O off the caller thread, remove duplicate checks, and include it in
   end-to-end budgets. Current implementation still reads whole files and performs some verification
   synchronously. Model/session residency is absent (one worker invocation per process).
5. **Precision backlog:** retain F32 mask coverage in subject selection with float-aware combine;
   restrict any legacy u8 refinement conversion to an explicit option and document it.

QNN controls without an actual model/target NPU test establish only protocol/configuration
behavior. Test doubles must never be reported as QNN or NPU execution. Strict CPU-EP fallback
rejection concerns graph placement; image preprocessing/postprocessing still runs on CPU.
Windows system AI remains an independent MSIX companion; its packaging rule is not an NPU rule.

## Proposed v2 — design, not implemented

Do not silently widen v1 or reinterpret existing fields. Independently version:

- **Package manifest** (deployment and authority).
- **Wire transport** (framing, negotiation, length/shape/allocation bounds).
- **Effect API and operation semantics** (ports, geometry, schema, results).

A negotiated extension needs a tested downgrade/rejection matrix, mandatory-vs-optional feature
rules and language-neutral schemas/examples. Prefer introducing only ports used by real adapters.

### 1. Resource lifecycle

Keep executable runtime packages separate from a bounded immutable model store. A model resource
record needs format/version, model ID, digest, named files with hashes/sizes, loader/runtime
constraints, source/license/redistribution information and an offline install path. Preserve the
original files; never infer compatibility by file suffix. External ONNX tensors, tokenizers,
configuration and optional adapters such as LoRA require explicit multi-file records, not an
unbounded directory scan. `probe`, `prepare`, `listResources`, `evict` must target resource identity;
network permission/consent must be scoped to an explicit acquisition source and operation.

### 2. Operation and device capabilities

Per-operation semantic version and schema dialect, accepted depths/color/alpha/ranges,
input/output shape constraints and named optional inputs. Separate static capabilities from
dynamic current availability. Device policy should express auto, exact device or required-kind
and allowed backends. Execution provenance can report model digest, runtime/provider/driver
versions, requested policy, graph partitions and fallback reason. Clearly label it as
runtime-reported evidence, not trusted physical-device attestation.

### 3. Ports, binary payloads and geometry

Named image/mask/depth/reference/text ports, multiple results only when needed, and declared
same-size vs scaled/cropped output transforms. Add depth units/range/invalid values before
materializing depth layers. Move ICC/other binary blobs out of the JSON envelope into bounded
length-delimited sections. Add streaming/tiling and aggregate memory budgets before raising image
limits. Generated outputs/new layers require host command/history/selection semantics and tests,
not only a new enum tag.

### 4. Worker lifecycle and UI

Optional resident sessions for expensive model loads need explicit ownership, bounded cache,
reset/shutdown, cancellation and invalidation by artifact/runtime identity. Progress/phase events
and structured error codes require an actual transport, not fake percentages. User-facing
operation/model/device selectors and preferred-provider policy belong in command-driven UI;
new labels need real translations and offscreen visual checks. Maintain default automation
restrictions until an adapter-specific capability policy exists. Process-tree/resource limits
are additional security/reliability work; the current native worker is trusted, not sandboxed.

## What “Photoshop sharing” can mean

These must have separate compatibility matrices:

1. **Shared compute:** an authorized Photoshop UXP/native host bridge maps Photoshop pixels to
   this worker API and maps results back. The model/worker may be shared with PhotoCraft while
   each application has its own thin bridge. No Photoshop runtime is required by PhotoCraft.
2. **Run a third-party Photoshop-compatible native plug-in in PhotoCraft:** implement the actual
   supported SDK suites/callbacks, tiles/channel encodings (including Photoshop 16-bit ranges),
   dialogs/parameters, architecture and licensing. A `.8bf` suffix is not a loader. An x64 bridge
   can be out of process; ARM64 cannot arbitrarily load x64 native libraries.
3. **Adobe's built-in Neural Filters or opaque protected bundles:** no general independent loader
   is implemented or established. Particular resources can require proprietary runtimes,
   authorization, protection or cloud services. Neither QNN nor this interface establishes the
   right or technical ability to run them. Only advertise a resource after an authorized loader
   and deterministic per-resource tests exist. No renaming/decryption workaround is proposed.

The interface is therefore a useful extensible foundation, **not** a universal Photoshop neural
plug-in compatibility layer. Independently deployable ordinary models are the first concrete
backend milestone; do not make Adobe compatibility a prerequisite for landing that milestone.

## Evidence required before release claims

- Host contract/unit/authorization, strict Clippy, wasm, layering, panic-hunt and normal CI gates.
- Actual C# ↔ Rust serialized-frame fixtures (including malformed/unknown/trailing data).
- Model profile/path/digest/tensor shape/pre/postprocess tests without NPU, clearly labeled.
- Real worker process exit/timeout/cancellation/malformed response tests.
- On the target Snapdragon device: a licensed compatible artifact, strict QNN session success,
  provider placement/profiling evidence, output quality and end-to-end timing.
- For each claimed Photoshop bridge/resource: supported host/version/architecture, deployment
  terms, fixture comparison, selection/alpha/color/depth/undo behavior and clear unsupported cases.

Prior green CI runs predate the cross-language fix/QNN backend and are not evidence for these
new changes. Record new run URLs, commit and test scopes rather than reusing older green results.
