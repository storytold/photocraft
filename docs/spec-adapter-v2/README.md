# PhotoCraft adapter v2 — UNREVIEWED DRAFT / NOT IMPLEMENTED

- Status: **UNREVIEWED DRAFT / NOT IMPLEMENTED — NOT FROZEN**.
- Draft revision: `2.0.0-draft-review-1`, 2026-10-22. Breaking draft changes are expected.
- This directory proposes a language-neutral contract. It does **not** implement a v2 codec, negotiation state machine, resource store, model loader, independent client/worker or host bridge.
- The earlier “Frozen Baseline” and “Golden Vectors” descriptions were incorrect. The fixtures are synthetic JSON documents, **not binary goldens**. No SINet artifact, model hash/shape/license, QNN session or NPU success is established here. A passing JSON/schema CI job cannot establish those facts.
- Review and CI are still required. No local build, test, validation script or dependency installation was run for this revision.

This is shared effect-service design, not a Rust ABI, generic tensor RPC, Photoshop plug-in loader or permission to use proprietary models. PhotoCraft remains native Rust. Runtime-specific deployment and legitimate host bridges remain separate work.

## 1. Three independent versions and compatibility

| Plane | Proposed field / scope | Rule |
| --- | --- | --- |
| Worker package manifest | `manifest_version: 2`; installation, executable inventory, architecture/trust | Not the wire version; v2 installation is not implemented. Do not feed it to the current v1 installer. |
| Wire | `protocol: 2`; framing, bounded sections, handshake | v1 and v2 need independent codecs and an explicitly selected endpoint. No magic sniff-and-reinterpret fallback. |
| Operation | `operation` + exact `operation_version`; ports, parameters, color and geometry | No “latest” selection or silently substituting another model/profile/version. This draft uses bounded three-part ASCII versions with an optional prerelease label (no build metadata); a complete SemVer range/resolution API is not defined. |

Resource ID, resource version and manifest digest identify an immutable model record separately from the executable package. Package, resource and operation versions do not imply compatibility with one another.

### Compatibility matrix (requirements, NOT verified implementations)

| Host / worker or condition | Required behavior | Evidence now |
| --- | --- | --- |
| Existing v1 host / existing v1 worker | Existing v1 codec and declared v1 features only | Outside this draft's validation scope; no new conclusion |
| v1 host / v2-only worker | Reject before dispatch; require a compatible endpoint | Not implemented/tested |
| v2 host / v1-only worker | Explain mismatch; any user-selected v1 path uses a fresh process and v1 codec | Not implemented/tested |
| v2 host / dual v1+v2 worker | Use an explicitly installed/authorized endpoint and negotiated v2; never send v2 to its v1 parser | Not implemented/tested |
| v2 host / v2 worker, no common version | `error/unsupported_version`; do not execute | Synthetic JSON checks only, pending CI |
| Unknown required feature | `error/unsupported_feature`; do not execute | State-machine/cross-language tests missing |
| Unknown optional feature | Do not enable it; absence from agreed set means disabled | State-machine tests missing |
| Unsupported operation version / format / runtime / geometry / port | Explainable rejection before run; no dropped ports or guessed conversions | Not implemented/tested |
| Matching v2 drafts | Header/schema and semantic checks are necessary, not sufficient | No binary, independent implementation or hardware conformance |

## 2. Proposed bounded negotiation

Only a Worker already installed and explicitly authorized for native execution may start. A manifest or resource record never authorizes launching code, fetching a model, resolving a remote schema, or loading an arbitrary DLL. External processes are **not an OS sandbox**.

1. Start a fresh, explicit v2 endpoint; one outstanding request at a time. Send one `type: hello` before controls/runs. Its bounded offer contains `wire_versions`, `supported_features`, `required_features` and `request_id`.
2. This bootstrap envelope is **v2-only**, not a universal discovery protocol. Its offer must contain `2`; only `2` can be selected. A v1 endpoint is selected out of band, never negotiated through v2 framing.
3. Required features must be a subset of the offer. The worker rejects unknown mandatory features, otherwise returns `status: negotiated`, selected version and enabled intersection. Enabled features must have been offered and supported by the worker; all required features must be enabled. These are cross-message/state constraints, not solely JSON Schema constraints.
4. The minimum proposed feature set is `image-mask-v2`, `same-geometry`, `resource-control`; all three must be agreed before control/run. Unknown optional identifiers remain disabled. This draft defines no extension payload mechanism.
5. The host validates the response's `request_id` and `type`, then retains the agreed set for **this process only**. A new process requires a new handshake. Control/run before negotiation, repeated hello, missing base features, unsolicited/duplicate response, or incompatible response closes the endpoint without execution/materialization.
6. Rejection uses the matching v2 `hello` error envelope when safely decodable. Bad framing/UTF-8/JSON can instead close the stream. No retry as v1, binary execution during “discovery”, or silent downgrade.

See [synthetic hello request](fixtures/hello-request.json) and [synthetic hello response](fixtures/hello-response.json). There is no negotiation implementation or launchable example in this change.

## 3. Proposed framing and bounds (codec NOT IMPLEMENTED)

```text
magic[8] | header_length:u32le | UTF-8 JSON header[header_length]
         | section_count:u32le | table[section_count * 16] | payload_area
```

- Request magic: ASCII `PCADREQ2`; response: `PCADRES2`. These are proposals, not registered or deployed wire formats.
- Header: 2..1,048,576 **bytes**; semantic/schema requirements make `{}` invalid. No BOM, invalid UTF-8, duplicate keys at any nesting, NaN/Infinity, non-finite numeric conversions or unknown fields. JSON maximum nesting depth 16 (root depth 1); at most 100,000 value nodes, 512 entries per array/object, 128 UTF-8 bytes per key and 4,096 Unicode code points per string. A streaming parser must enforce budgets before building large trees. Character `maxLength` never substitutes for the byte cap.
- Every header includes `sections`, even when empty. Its array order is the zero-based section index. The binary count/table must match that array **exactly**; JSON validation alone cannot check the real table or bytes.
- Table entry: `kind:u32le`, `reserved:u32le`, `offset:u32le`, `length:u32le`. Kind `1 = icc`, `2 = f32`. Reserved is zero. Other kinds, including the old undefined tensor kind `3`, are rejected.
- Maximum 64 sections. `hello`, control and non-`ok` responses must have **zero** sections and zero payload. No hidden error payload, compression, external pointer, shared-memory handle or URI payload.
- Offset is relative to payload-area start, not frame/header start. First offset is zero; each next offset equals `align_up(previous_end, 4)`. Padding is 0..3 **zero bytes**. No gaps, overlap, reordering, aliases, empty sections or unreferenced sections. Each section is referenced exactly once by one port field. Sample sections and offsets are multiples of 4. ICC is at most 4,194,304 bytes per section, 16,777,216 in aggregate.
- Each section and complete payload area, **including padding**, is at most 268,435,456 bytes. Complete frame bound is `16 + 1,048,576 + 64*16 + 268,435,456 = 269,485,072` bytes; this is a ceiling, never an allocation request. Total f32 sample count is at most 67,108,864. Worker-package inventory and model-package limits are separate (§6).
- Each raster dimension is 1..16,384; signed origins and rectangle endpoints must fit i32. Require `length == checked(width * height * channels * 4)` before allocation/read. Validate section-index range, kind, byte count, checked offset+length, aggregate sums, aligned offsets and frame budget before slicing. Cap all allocations using a smaller local memory budget when necessary; reject rather than truncate.
- Read exactly the declared frame. This draft's transports are lockstep; no pipelining, concatenated unsolicited responses or extra bytes in a single-frame fixture. A future stream codec must distinguish subsequent authorized frames from forbidden trailing data. Truncation, EOF inside a frame and unexpected extra frames are errors.

[Shared schema definitions](schemas/common.schema.json) impose scalar/collection limits. The CI document checker also checks declared offsets, references and sample products. **It neither encodes nor decodes frames**, inspects padding/sample bytes/ICC, or proves overflow safety in a runtime codec. Independent binary vectors and hostile-input decoder tests are open requirements.

## 4. Control and response state

Controls require an operation ID/version and explicit device policy. `probe`, `prepare`, `evict` require immutable `resource: {id, version, manifest_digest}`. `listResources` may omit resource to enumerate the bounded operation/device scope, or supply it to filter. No implicit default-model or all-model readiness.

| Action | Proposed semantics | Successful response |
| --- | --- | --- |
| `probe` | Read-only bounded availability inspection; no downloads, sessions or inference. May report cached verification only if tied to immutable identity. Do not rehash huge models on UI refresh. | `ready` + explicit readiness: `declared`, `files_present` or `files_verified`; never `session_created` |
| `prepare` | Explicit verification and optional runtime-session creation; no network acquisition | `ready` + `files_verified` or `session_created`; neither means a successful graph run |
| `listResources` | Bounded installed/local allowlisted inventory only; max 256 records, fail if too large (pagination is undefined) | `ready` + `resources`, including each identity, operation version and distinct readiness |
| `evict` | Explicit `sessions` or `derived_cache` only | `ready`, no outputs or inference report; source resources/OS models/Worker never deleted |
| `run` | Bind immutable resource, exact operation version, validated params/ports, negotiated policy and budgets | `ok` + nonempty outputs + execution report + sample sections |

“ready” means the **control action completed at the stated level**, not “all models work”. Files present, digest verified, session created, and graph inference succeeded are distinct claims. No status in these examples is evidence of a real runtime.

Status/error pairs: `needs_model` → `resource_missing`; `needs_permission` → `permission_required`; `unavailable` → `device_unavailable`; `cancelled` → `cancelled`; `timeout` → `deadline_exceeded`; `error` → a specific registry code. The schema contains the bounded code registry. Cancellation/progress messages, deadlines, session handles and cancellation cleanup are not defined or advertised; statuses reserve failure reporting only, not an implemented cancel API. No automatic retry. `retryable` is advisory, not consent.

Success and failure are disjoint: no `outputs`, `execution`, readiness/list results or binary payload on failures; no `error` on successful messages. The schema rejects control/run/hello field mixing. Pair validation additionally checks correlation, legal readiness and matching error code.

## 5. Raster ports, color and device intent

### Minimal raster subset

This revision proposes named **image/mask** ports and `same_geometry` outputs only. The manifest requires one primary, required output and a required `source` image. Optional/extra named inputs are accepted only when declared by the operation; `selection` is a same-geometry mask if declared. Unknown ports are rejected, never dropped. All returned outputs match source origin/dimensions. General tensors, prompt streams, depth units/invalid samples, rational scaling, explicit transforms, output multiplicity/writeback transactions and richer host materialization remain **undefined/deferred**, not enabled capabilities.

- Interleaved pixel-major little-endian IEEE754 f32; component order follows the declared model (`rgb = R,G,B`, `gray = Y`, `cmyk = C,M,Y,K`, `lab = L*,a*,b*`), alpha last. Samples must be finite. NaN/Inf is rejected, never normalized away.
- `source_depth` is provenance, not storage width. RGB/gray/CMYK integer-depth samples normalize to [0,1]; finite 32-bit samples may preserve HDR/out-of-range color. Lab colorimetry is **not** blindly normalized: use L* [0,100], a*/b* in CIE units; exact integer-source mapping/white-point profiles still need reviewed interoperable vectors. Until those exist a runnable profile must reject Lab rather than guess. No clipping HDR, reinterpreting arbitrary Lab as RGB or assuming all 8-bit color is sRGB.
- Image channels equal color components plus optional alpha: RGB/Lab 3/4, gray 1/2, CMYK 4/5. Alpha is straight and finite [0,1]; premultiplied transport is not supported in this subset. Model preprocessing must explicitly preserve/reject or define compositing through a reviewed profile, not silently ignore alpha.
- Images declare `color_encoding`. `icc` requires an ICC section; other encodings prohibit it. `srgb`/`linear-srgb` apply to RGB only. `untagged` is not implied sRGB: run requires an explicit operation/model conversion policy or rejection. Header policy must not hide inside opaque `params.colorPolicy`. Color-management implementation, ICC integrity checks and cross-depth oracles remain open.
- Masks are coverage, not color: gray, one channel, provenance 32, no alpha/ICC/color encoding, all samples in [0,1]. The input image's provenance remains on `source`; masks do not inherit it.

### Device policy and reports

`device_policy` is mandatory even on controls, with a nonempty backend allowlist and explicit `allow_graph_fallback`. Backends are open identifiers, not a list tied to QNN, ONNX Runtime, Windows AI or any three runtimes.

- `auto`: no target fields; select only within the allowlist. `allow_graph_fallback: false` does **not** imply NPU. An allowed CPU backend can satisfy `auto`.
- `exact`: require `target_device`, prohibit `target_kind`, require graph fallback false.
- `require_kind`: require `target_kind`, prohibit `target_device`, require graph fallback false. All graph devices must match that kind.
- No silent backend substitution or graph partition onto disallowed hardware. CPU pre/postprocessing is reported separately from graph devices and is not itself graph fallback. For QNN specifically a future implementation must use and validate its real strict EP policy; a schema flag is not proof.

An `ok` run report requires the echoed policy and resource identity, actual graph-device labels/kinds, backend/runtime versions, configuration digest, partition/fallback flags and reason, provenance level and bounded timing. Pair validation rejects policy/resource mismatch and illegal backend/device/fallback claims. Timing and worker/runtime profiles are **self-reports, never hardware attestation**. The `synthetic_fixture` evidence tag is test-only and must never be accepted by a production materializer.

## 6. Model resource and executable package separation

[Model schema](schemas/model-resource.schema.json) uses open `format`, runtime/backend/architecture IDs, not an enum limiting use to ONNX/TFLite/OpenVINO. An identifier describes a format; it does not install a loader or grant compatibility. Unknown format, constraints, dtype/layout, binding/transform profile or runtime version must fail closed.

Runnable resource records require immutable ID/version, manifest digest, reviewed source URI/revision/exporter version, reviewed license expression and adjacent license-file inventory, all constituent file paths/sizes/SHA-256s, exact operation version, named input/output binding shapes/dtypes/layouts, explicit preprocess/postprocess profile IDs/versions, color/alpha policy and nonempty runtime/backend/OS/architecture/memory constraints. External weights, tokenizer, configuration and auxiliary data must all be inventoried; no implicit sidecar or library search. SHA-256 is lowercase hex. These declarations require independent artifact/license verification; schema acceptance is not that verification.

**Proposed digest rule:** `manifest_digest` is SHA-256 of RFC 8785 canonical UTF-8 JSON of the resource object with the **top-level `manifest_digest` member omitted**. No other member is removed. Duplicate keys/non-finite values are invalid before canonicalization. File digests cover exact bytes, not filenames or canonicalized files. Digest identity is not a signature/trust decision. RFC 8785 implementation and cross-language digest vectors are still missing; this checker does **not** verify a runnable manifest digest.

- Resource record ≤1 MiB, ≤128 files; each model file ≤16 GiB; model aggregate ≤32 GiB, subject to **separate, smaller local installation and execution budgets**. Do not raise Worker executable limits to accommodate weights. Package manifest ≤1 MiB, ≤512 inventory files, each ≤256 MiB and aggregate ≤512 MiB.
- Relative inventory/entrypoint/license paths use portable `/`-separated ASCII components; no roots, drives, backslashes, colon/ADS, traversal, empty components, dot components, trailing dots or platform reserved device names. Case-insensitive/Unicode-normalized collisions, symlinks, hardlink/reparse escapes and undeclared files must be rejected by a future installer. The schema's lexical subset plus CI checks catch common names/collisions; there is **no secure installer here**.
- `deployment: placeholder` is deliberately non-installable/non-runnable: `example.synthetic.*` ID, resource digest null, files empty; package entrypoint null and files empty. A loader must reject this before session creation regardless of schema validity. Placeholders use `LicenseRef-UNVERIFIED-NOT-FOR-USE`, never a guessed upstream license.
- The all-zero digest in **synthetic wire headers** is a lexical sentinel, not the digest of this placeholder or any model/config. It is prohibited for runnable resources. A placeholder with null digest cannot satisfy those requests: they illustrate isolated header structure, not an executable end-to-end pair.
- Only explicitly installed, authorized local resources are proposed. No downloader, mutable path-as-model, executable-in-resource, runtime package acquisition, cache implementation or user-data deletion is implemented.

[Package manifest schema](schemas/manifest.schema.json) separately declares operation port/parameter schemas, geometry, source color/depth/alpha/range and budgets. All input raster ports in this minimum subset share source geometry; independent-size references require a future negotiated profile. Embedded parameter schemas use Draft 2020-12; host validates structure/budgets and normalized params, then worker validates semantic meaning. Schemas/profile references must be allowlisted local data, never a network fetch or executable payload. Operation/profile specifications and language-neutral normalization rules need further review.

## 7. Fixtures, CI command and exact verification scope

[Fixture index](fixtures/index.json) enumerates every positive fixture and its schema. Legacy file names are kept only for diff readability: [run header](fixtures/run-subject-request.json) now describes `synthetic-mask`, not a real subject model. [OK header](fixtures/ok-mask-response.json) fabricates a **test-only CPU label with zero synthetic timing**, clearly tagged `synthetic_fixture`; no inference occurred and no payload bytes accompany it. [Model example](fixtures/model-resource-example.json) and [package example](fixtures/package-manifest-example.json) are non-runnable placeholders. Example URNs are labels, not download links.

**CI only**, in GitHub Actions with Python 3.12, repository root as working directory:

```sh
python docs/spec-adapter-v2/validate.py --static
python -m pip install --disable-pip-version-check -r docs/spec-adapter-v2/requirements-ci.txt
python docs/spec-adapter-v2/validate.py --schema
```

[Checker](validate.py) refuses to run unless `GITHUB_ACTIONS=true`; there is no local override. `--static` uses only the Python standard library and performs bounded/strict JSON loading, local reference/inventory checks, declared section/port/resource/package semantic checks and request/response fixture-pair checks. It does **not** implement JSON Schema validation. `--schema` repeats static checks, requires the [exact CI dependency pins](requirements-ci.txt), validates all schemas against Draft 2020-12, all indexed fixtures and embedded parameter schemas, and runs negative mutations for structural/conditional and semantic rejection. Missing dependencies, missing/extra fixtures, unresolved/nonlocal references or unsupported schemas fail; nothing is skipped and there is no runtime-code/dependency installation in the app.

The checker has not been executed locally. No GitHub Actions success is claimed by this document. Workflow wiring lies outside this directory and must be independently authorized/reviewed; CI runs only the revision actually uploaded by the owner. This task neither commits nor pushes.

**In scope of a green document CI:** schema syntax/metaschema, selected synthetic JSON fixture structures, declared arithmetic/reference budgets, inventory rules, chosen negative regressions and pair correlations. **Not in scope:** real binary lengths/table/padding/trailing bytes/float contents, any production parser/codec or v1 interoperability, canonical digest correctness, file availability/hash/license, runtime install/session creation, actual QNN graph placement/NPU use, host writes/undo, plugin compatibility or security sandboxing.

## 8. Still required before review/freeze (all open)

- [ ] Maintainer RFC review of framing/bootstrap/versioning, rejection/error mapping, feature registry and all draft breaking changes; independent review of the tightened schemas and checker; actual GitHub Actions results.
- [ ] Implement independent v1/v2 codecs and state machine with preallocation-safe limits; no codec/resource-store implementation is included here.
- [ ] Reviewed binary request/response goldens with payloads and exact hashes: float endianness, ICC/padding, negative origins, 8/16/32 provenance, finite HDR/masks, duplicate/unknown JSON, all corrupt/truncated/overflow/overlap/alias/trailing-byte cases; production-decoder and cross-language conformance runner.
- [ ] Standalone host-neutral client and Worker examples that actually interoperate, including negotiation incompatibility matrix; JSON examples are not those implementations.
- [ ] Immutable resource store and installer security/trust/permission design, full file inventory, separate budgets, evict isolation and canonical RFC 8785 manifest/config digest definitions and cross-language vectors.
- [ ] Verified **real** model artifact/export/license/source/binding/constraints and preprocessing profiles. Placeholder shapes are synthetic, not a SINet/QNN profile or a usable model package.
- [ ] Parameter/profile dialect and normalization/security review; Lab integer mapping/white point, ICC/HDR/alpha cross-depth oracles and memory budgets.
- [ ] Depth/tensor/scaled/explicit-transform metadata, required-feature negotiation and host materialization semantics before advertising them. Progress/cancel/session/tiling APIs, timeout cleanup and multiple-output transactional writeback likewise remain undefined.
- [ ] Executable worker deployment (including QNN native/DSP payloads if pursued), strict real backend fallback enforcement, runtime API/build verification and legal redistribution review.
- [ ] PhotoCraft host integration, undo/cancel/permissions/diagnostics and failure tests, then specified model/version/Snapdragon hardware accuracy/profiling/placement evidence. Ordinary CI and synthetic reports do not prove NPU execution.
- [ ] Any external-host/Photoshop bridge is separate. No `.8bf`, Adobe Neural Filters or proprietary resource compatibility is established.

See the authoritative [P2 plan](../flash-agent-adapter-qnn-plan.zh-CN.md#6-p2冻结最小通用-v2-规范再开放给外部实现者) for the intended milestone. This directory has **not** reached M2 or any freeze/conformance gate.
