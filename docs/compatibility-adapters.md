# Compatibility adapters: shared compute and unchanged plug-in resources

> **Status (2026-10-22):** experimental Rust host API v1, with a separate bounded native
> worker protocol v1 and explicit package management. This is separate from the sandboxed
> [WebAssembly ABI v1](plugins.md); it is **not** a stable native binary ABI, a Photoshop SDK
> implementation, or proof of GPU/NPU execution on a particular machine. The audit and proposed
> next-version work are tracked in [adapter evolution](adapter-evolution.md).

## Purpose

Let a community author connect an existing filter, model bundle or inference runtime without
rewriting that resource or teaching the document engine its vendor-specific format:

```text
PhotoCraft: pixels, ICC, selection, history and jobs
                       |
        host-neutral request / response contract
                       |
              trusted compatibility adapter
                       |
     original filter / model bundle / external worker
                       |
             CPU, GPU or NPU runtime
```

The engine does not interpret or rewrite the `resource` reference. Its meaning belongs to the
adapter: a model id, an unchanged bundle, or a filter handled by a separately implemented bridge.
An adapter must have an actual loader and any required deployment rights; passing a path is not
proof that the file can execute. Do not advertise a resource format merely because its extension
is recognized. Existing sandboxed `.wasm` plug-ins keep their existing loader and commands.

The contract is implemented in `photocraft_plugins::adapter`. Its data types do not contain a
`Session`, `Document`, `Surface`, GPU handle, ONNX graph or vendor-specific runtime type. Other
hosts can map their own image APIs to the same request and response types. The native
`photocraft_plugins::worker` module supplies a separate bounded file-framed transport and direct
child lifecycle; it is not a network/authenticated service. Samples are binary f32, not a large
JSON pixel array. A reusable worker does not implement another editor's host bridge automatically.

## What is, and is not, provided

| Area | This API | Not provided by this API |
|---|---|---|
| Discovery | Adapter, operation, accepted colour modes, resource-format claims and device descriptors | Hardware detection or validation of third-party device claims |
| Execution | Opaque resource and parameter forwarding; image/mask/depth result types | `.8bf`, UXP, CEP, Adobe protected-model or arbitrary ONNX loaders |
| Devices | Explicit device id, CPU/GPU/NPU classification, fallback policy, execution report | Runtime/driver or physical-device validation merely from a descriptor; runtimes remain separate worker packages |
| PhotoCraft | Session registration, explicit-consent native package install/enable UI, list/run commands, jobs and transactional writes | Automatic DLL loading or arbitrary shell execution; general model/operation preferences UI |
| Cross-host reuse | A host-neutral data boundary and no requirement to modify original resources | An existing plug-in binary working in every host/CPU/OS without an adapter |
| Safety | Input/result validation, limits, cancellation checkpoints, caught Rust unwinds and direct-child worker timeout/termination | An OS sandbox, full descendant-process containment, or recovery from an in-process abort |

No models, proprietary SDK files, plug-in binaries or new vendor runtime dependencies are included.

## Registering an adapter

Implement `photocraft_plugins::adapter::Adapter`:

```rust,ignore
pub trait Adapter: Send + Sync {
    fn descriptor(&self) -> Descriptor;
    fn run(&self, request: &Request, context: &dyn Context) -> Result<Response>;
}

pub trait Context {
    fn cancelled(&self) -> bool;
    fn progress(&self, fraction: f32);
}
```

A trusted embedding application registers its implementation with
`Session::register_adapter(Arc<dyn Adapter>)`. Registration validates and caches its descriptor;
a duplicate id is rejected. Registries belong to sessions, not global process state. A new session
starts with **no** compute adapters: an empty list is not an initialization failure or a hardware
fallback. `Session::unregister_adapter(id)` removes a registration without invalidating an already
running job's immutable adapter snapshot. Cancel that job through the job API; resources are
released after the last snapshot is dropped. Re-registration is explicit, never silent replacement.
The reference adapter example is deliberately CPU-only, not a neural filter.

The descriptor contains `api_version`, `id`, `name`, `version`, `operations`, `devices`,
`resource_formats` and an opaque `params_schema` object. Each operation declares its id,
`output_kind` (`image`, `mask`, `depth`) and accepted `color_modes`. Devices declare an id,
`kind` (`cpu`, `gpu`, `npu`), `backend`, display name and supported operation ids. Capabilities
are adapter claims, not a substitute for querying the adapter's actual runtime/device status.

The host does **not** discover native adapters by scanning the existing `.wasm` folder.
Native compatibility adapters run with user privileges and require explicit user trust:
1. Manifest inspection via `plugin.adapter.inspectPackage`.
2. Installation snapshot via `plugin.adapter.install` with explicit `trust: true`.
3. Separate toggle via `plugin.adapter.enable`.
4. Availability probe (`plugin.adapter.status`) without automatic downloads.
5. Explicit model download/preparation (`plugin.adapter.prepare`) with `consent: true`.

The desktop UI exposes this under **Filter › Plug-ins › Compatibility Adapters…**.

### Worker IPC protocol

Native out-of-process adapters use `photocraft_plugins::worker::WorkerAdapter`.
The host launches the worker with fixed CLI arguments in an isolated temporary directory:
`--action <probe|prepare|run> --request <request.bin> --response <response.bin>`

Framing format:
- Request magic: `PCADREQ1` (8 bytes), followed by LE u32 JSON header length (capped at 1 MiB), UTF-8 JSON metadata, and raw LE f32 samples.
- Response magic: `PCADRES1` (8 bytes), followed by LE u32 JSON header length, UTF-8 JSON status/metadata, and raw LE f32 samples.
- Actions:
  - `probe`: checks worker and model readiness state without starting inference or downloads.
  - `prepare`: explicitly prepares models with user consent. The Windows system-AI worker may
    trigger OS downloads; a local QNN worker need not have any network/download implementation.
  - `run`: performs inference and outputs the result buffer.

The control header is only `{"protocol":1}`. It cannot target a specific resource/device, so
probe/prepare applies to the worker's package/default model, **not every model** an adapter may
accept in a run request. Response headers contain exactly `protocol`, `status`, `message`,
`output` and `execution`; unknown fields are errors, even if null. Manifest descriptors are the
host's capability authority, not extra fields returned by the worker. The 1 MiB header cap is
combined metadata including parameters and ICC bytes; a valid core request can still exceed
this transport cap. No silent ICC truncation or profile substitution is permitted.

## Engine commands

### `plugin.adapter.packages`

Returns `{ "available": bool, "packages": [ ... ], "warnings": [ ... ] }` describing installed packages from local configuration storage.

### `plugin.adapter.inspectPackage`

```json
{ "path": "path/to/adapter.json" }
```

Inspects an adapter manifest before installation without executing it.

### `plugin.adapter.install`

```json
{ "path": "path/to/adapter.json", "trust": true }
```

Snapshots the manifest and its files into the managed adapter directory. The package remains disabled until explicitly enabled.

### `plugin.adapter.enable`

```json
{ "provider": "org.example.adapter", "enabled": true }
```

Enables or disables an installed adapter. Enabling registers it in the session's adapter registry.

### `plugin.adapter.list`

```json
{}
```

Returns `{ "apiVersion": 1, "installed": [ ...descriptors... ] }`. It does not initialize vendor
runtimes or probe files; it describes adapters already registered by trusted application code.

### `plugin.adapter.run`

```json
{
  "provider": "org.example.compatibility-adapter",
  "operation": "filter",
  "resource": "opaque-original-resource-reference",
  "params": { "amount": 0.5 },
  "device": "device-id-from-descriptor",
  "allowFallback": false,
  "output": "pixels",
  "layer": 7
}
```

- `provider`, `operation` and `output` are explicit; `layer` defaults to the active pixel layer.
- `params` defaults to `{}`. Its semantic validation belongs to the adapter; the host validates
  its object type and metadata budget. Vendor-specific names and values are not rewritten.
- `resource` is optional and opaque. The host does not open, normalize or convert it.
- `device` is optional; without it, the adapter chooses a declared device and reports that choice.
- `allowFallback` defaults to `false`. An explicit device request must not silently run elsewhere.
- Unknown keys and wrong value types fail before document modification.
- `output: "pixels"` requires an **uncomposited candidate image** and writes the targeted raster
  layer, blending by the original feathered selection exactly once and respecting Lock Transparency.
  `Request.selection` is analysis/seed context; adapters must not first blend candidate pixels with
  the original image by that same coverage. With unlocked alpha, feathered writeback mixes
  associated colour contributions and alpha, then converts back to straight colour; hidden
  colour in transparent pixels cannot darken the edge. Zero/full coverage preserves the exact
  original/candidate samples; Lock Transparency keeps the original alpha. A bridge to a filter
  that already performs coverage compositing must reconcile that behavior explicitly, not
  submit a doubly blended image.
- `output: "selection"` requires a mask result and replaces the selection as one undo step. The
  host does not multiply it by the old selection again; the old selection is already available
  to the adapter as an optional input.
- `output: "layerMask"` requires a mask result and writes a layer mask subject to target/lock
  restrictions. A locked Background is not silently unlocked or converted.
- A depth result is representable in the core API; `depthLayer` writeback is explicitly unsupported
  in this initial engine bridge. It is not silently stored as RGB, a selection or a mask.

### `select.subject` integration

When the user triggers **Select › Subject** (`select.subject`), the host automatically inspects the session's
adapter registry:
- Automatic routing filters the declared mask operation, actual native input color mode, requested device,
  and an installed package's cached `ready` state. It never implicitly probes/prepares a worker. This v1
  cache covers a package/default model only, can become stale, and does not prove a selected model or NPU.
- A requested provider/device is strict: incompatible or unavailable choices fail rather than trying another
  provider. With no matching adapter and no explicit adapter options, the command retains classical saliency
  and GrabCut; `forceClassical: true` / `"method": "classical"` explicitly selects the classical path.
- **Precision limitation remains:** the subject bridge still converts the returned F32 mask to a u8 `Region`
  even with `enhanceEdge: false`. F32-aware combine/undo regressions are pending; the existing generic
  `plugin.adapter.run` selection destination retains F32. Do not describe these two paths as equivalent.
- Untagged images are not silently treated as sRGB. A worker requiring `params.colorPolicy` still needs an
  explicit command parameter; a general user color-policy/default-model preference UI is not implemented.
- Package hashing remains synchronous in some paths, and total cancellation/deadline coverage of that I/O
  is still incomplete. Routing improvements do not resolve those P4 tasks.

This first bridge accepts an explicit raster-layer pixel target and the canvas rectangle in
RGB, Gray, CMYK and Lab documents at 8/16/32-bit depth. Indexed, Bitmap, Duotone and Multichannel
documents are explicitly unsupported in this engine bridge: their expanded storage is not a
complete representation of their palettes/inks/semantics. The host-neutral core types can describe
other declared modes, but that does not imply complete PhotoCraft writeback support.

An active floating selection must first be explicitly dropped (`select.drop`) or returned (Undo).
Adapter discovery does not drop it, and an adapter run rejects it without a preliminary edit.
Smart objects, non-raster layers, off-canvas-only content, targeted alpha/colour channels and Quick
Mask are not interchangeable inputs. Off-canvas layer pixels are retained unchanged. Unsupported
targets fail with an error rather than modifying the wrong layer. Resizing the image/canvas and
arbitrary layer/document editing are not adapter effects.

`Session::execute` runs inline, while `Session::start` uses the existing job infrastructure on
native builds. A worker sees a snapshot; it never writes the live document. Progress maps to the
job, cancelled or failed results are discarded, and successful writeback is one history step.
The existing document job guards reject stale results. Cancellation is cooperative: a native
runtime or an external worker must actually implement its cancellation/timeout behavior.

## Image contract and bounds

`Image` contains document origin `x/y`, `width/height`, `channels`, `color_mode`, `source_depth`,
`alpha`, `icc_profile` bytes and interleaved row-major `Vec<f32>` samples. Colour-mode identifiers
include `rgb`, `gray`, `cmyk`, `lab`, `bitmap`, `indexed`, `duotone` and `multichannel`.

- Samples follow the host's normalized representation at every source depth (8, 16 or 32).
  A bridge must translate a vendor's integer encoding/range rather than treating these buffers as
  raw host-specific 16-bit samples. Float colour samples may be outside 0–1; do not clamp HDR
  colour to make a model accept it.
- Alpha is straight, not premultiplied. Its coverage is 0–1.
- Preserve the source colour model and ICC bytes. An empty profile means unspecified; it does
  **not** mean sRGB. A runtime needing RGB/sRGB must do an explicit colour-managed conversion and
  return the required host format. Never reinterpret CMYK/Lab channels as RGB.
- A selection/mask is one non-alpha gray channel with coverage in 0–1. A depth output is one gray
  floating-point channel with adapter-defined, documented units, not selection coverage.
- Image results preserve geometry, colour model, sample-depth metadata, channel layout and ICC.
  Mask/depth results may use their own gray/F32 representation but still preserve geometry.
- Width/height/channel products and coordinate ends are checked; buffer lengths must match;
  non-finite samples and malformed results are errors, never partially applied.
- Default limits bound images to 32 Mi pixels (33,554,432), 128 Mi samples (134,217,728) and ICC
  data to 4 MiB; metadata is also bounded. The engine also checks the combined input-plus-selection
  sample budget before allocating dense buffers. Callers may use smaller limits. Large-image tiling, zero-copy GPU
  interop and cached model residency remain future adapter-specific work. The implemented
  worker v1 uses file-framed process transport with additional combined metadata limits.

`Limits::check_params(&Value)` exposes the same iterative metadata preflight used by execution:
object root, depth counted from 1, and exact compact-JSON byte accounting, including escapes.
The engine checks adapter command parameters before admission clones them; rejected owned
adapter inputs are disposed iteratively, including authorization failures. A caller retaining
its own extremely deep `Value` after a borrowed query still owns its destruction.

Limits validate the contract; they are not an operating-system memory/CPU sandbox. A trusted
adapter controls allocations and native libraries inside its own implementation. Use an isolated
worker with explicit resource/authority limits for code not suitable for the host process.

## Execution and fallback reports

The response contains an `Output` (`kind` plus `image`) and an `Execution` report with actual
`devices`, `backend` and optional `fallback_reason`. Device ids must be declared and support the
operation. Reporting multiple devices makes a mixed execution visible instead of presenting a
partially CPU-executed graph as “all NPU”. Strict requests reject undeclared or unauthorized
fallback. CPU/GPU/NPU descriptors are not proof of acceleration; use runtime profiling to establish
actual model placement, output quality and end-to-end timing on the target hardware.

An adapter error, malformed image, contradictory execution report, cancellation or caught Rust
unwind leaves the document unchanged. Process aborts, unsafe third-party FFI and uncooperative
hangs cannot be made safe by `catch_unwind`; they need an isolated implementation.

## Automation and trust

`plugin.adapter.list` is discoverable through the engine/automation interface. The default
capability-scoped automation policy **rejects `plugin.adapter.run`**, including indirect action
steps: arbitrary adapter parameters or an innocent model id can resolve files, native code or
network operations outside the automation roots. Checking only for a key named `path` is not a
security boundary. A future integration must provide an adapter-specific capability policy before
enabling such execution. Local trusted callers can use the engine normally.

There is intentionally no JSON/MCP “load this DLL” or “execute this program” command. Registering
an adapter is a trust decision, not something a document or imported plug-in manifest can grant.

## Photoshop, Affinity and ARM boundaries

Affinity Photo 2 documents support for a **subset of Photoshop-compatible 64-bit plug-ins**, with
per-plug-in support status. That is not support for UXP/CEP, all Photoshop suites, or every CPU
architecture. Its help also states that non-pixel layers cannot invoke these plug-ins. See
[Affinity Photo 2 plug-ins](https://affinity.help/photo2/en-US.lproj/pages/Filters/plugins.html).
Do not silently extrapolate that matrix to another Affinity generation.

The ordinary Photoshop filter contract is chiefly raster input/output and selection coverage.
Some Photoshop SDK features can handle an existing layer mask, but creating native selections,
masks and layers is a host-specific capability. An adapter for a host lacking that capability
should return/export mask pixels or report unsupported, not pretend the native edit happened.
Photoshop's [UXP Imaging API](https://developer.adobe.com/photoshop/uxp/2022/ps-reference/media/imaging)
provides a separate host-specific path for selections and layer masks; it is not an Affinity API.

Native plug-in binaries are architecture-specific. Windows x64 application emulation does not
allow a plain ARM64 process to load an arbitrary x64 DLL; see Microsoft's
[ARM64EC ABI rules](https://learn.microsoft.com/en-us/windows/arm/arm64ec). Do not describe current
Photoshop Windows ARM as universally lacking plug-ins: Adobe's
[UXP migration announcement](https://blog.developer.adobe.com/en/publish/2026/09/investing-in-the-future-of-creative-cloud-extensibility-uxp-comes-to-our-flagship-applications)
explicitly includes Windows on ARM. Pure UXP, Hybrid native add-ons and `.8bf` are different cases.
An out-of-process compatibility bridge can separate architectures, but it must be implemented and
tested; the data contract alone does not create that bridge.

Keep any proprietary SDK integration separate and review its actual terms, supported hosts,
redistribution and open-source compatibility. The
[Adobe Photoshop C++ SDK licence](https://github.com/AdobeDocs/photoshop-cpp-sdk/blob/main/LICENSE.md)
is not PhotoCraft's MIT/Apache licence. No SDK header or example is copied into this implementation.

## Community work that can build on this boundary

1. Extend the existing bounded native worker/package path with version negotiation, resource-specific
   controls and transport fixtures; authentication for a future service transport and adapter-scoped
   automation remain separate reviewed work.
2. A tested traditional filter compatibility adapter, keeping original plug-in binaries unchanged
   and addressing SDK terms, CPU architecture, supported suites, dialogs and per-filter quirks.
3. Independently deployable model/runtime adapters (ONNX/QNN/DirectML/Metal/etc.), with correct
   colour handling, model packages, licences and reported CPU/GPU/NPU placement.
4. A loader for a particular existing offline bundle only when its format, execution mechanism and
   independent use are understood; renaming `.data` to `.onnx` is not such a loader.
5. Thin host bridges that map the same typed result to each host's actual pixel/selection/mask APIs.

Each adapter should publish a compatibility matrix and small deterministic tests; a resource
format string, a GPU name or a mock NPU descriptor must never stand in for a verified loader.
