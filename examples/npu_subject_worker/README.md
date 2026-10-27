# PhotoCraft Windows AI foreground-extraction companion

This is an external ARM64 Windows companion for the reusable, host-neutral PhotoCraft
adapter protocol. It calls the system-managed
`Microsoft.Windows.AI.Imaging.ImageForegroundExtractor` API on compatible Windows 11
Copilot+ PCs. It is **not a QNN implementation** and does not load custom ONNX models,
Qualcomm AI Hub models or an ONNX Runtime execution provider. There is no Python/DirectML
worker in this example.

## Architecture and trust boundary

```text
Host supplies a trusted executable and a separate, validated adapter descriptor
  -> native Rust WorkerAdapter creates a private job directory
  -> request.bin (PCADREQ1 + bounded JSON + f32le samples)
  -> registered ARM64 Windows AI companion execution alias
  -> response.bin (PCADRES1 + bounded JSON + optional f32le mask)
  -> strict Rust protocol decoder and adapter-registry validation
  -> host decides how to apply the result
```

This is file-based IPC, **not stdin/stdout streaming**. The Rust transport starts the
executable directly, without a shell, with standard streams null and exactly these arguments:

```text
--action <probe|prepare|run> --request <absolute request.bin> --response <absolute response.bin>
```

Both files must use distinct absolute local-drive paths in the same private directory. A
response is atomically published and never overwrites an existing file. The executable is
trusted code with the user's permissions; process separation is reliability isolation, not
an OS sandbox. The adapter contract can be used by other hosts, but this example alone does
not establish PhotoCraft UI/selection-command wiring or independent physical NPU execution.

The host supplies capabilities separately from probe results. Every response header has
**exactly** `protocol`, `status`, `message`, `output` and `execution`; no `ready_state` or
`descriptor` fields are allowed, even when null. Control/error responses have null `output`
and `execution` and no sample payload. A successful run returns `status: "ok"`, a depth-32,
one-channel, profile-free gray mask with the input's document-space origin/geometry, and
little-endian `f32` coverage normalized from native Gray8 bytes. Device/backend execution
metadata (`windows-ai.npu` / `windows-ai`) is the companion's **assertion**, not independent
NPU attestation or a promise verified by CI. There is no CPU/GPU fallback implementation.

## Readiness, preparation and input limits

- `probe` calls `GetReadyState`; it does not create the extractor or request preparation.
  Responses map to `ready`, `needs_model` or `unavailable`, with OS state details in `message`.
- `prepare` is the explicit, user-authorized entry point for `EnsureReadyAsync`. System model
  preparation **may download/install components**; system-managed does not mean zero download.
  Failed preparation returns `error`. The host must obtain consent before invoking it.
- `run` requires the API to report Ready. The companion neither prepares models implicitly nor
  substitutes another backend. Malformed requests and operation failures produce framed `error`
  responses when a valid response path is available.
- This implementation accepts only `select-subject` and RGB input (3 channels, or 4 with straight
  alpha last), depth metadata 8/16/32, finite SDR samples in `[0,1]`. Untagged RGB requires explicit
  `params.colorPolicy: "assume-srgb"`; embedded source profiles must be RGB ICC profiles. Native
  Windows AI consumes an SDR BGRA8 bitmap after color conversion/16-bit staging, not full HDR or
  original document precision. Input document pixels are not rewritten by this process.
- Limits are a 1 MiB JSON header, sides of at most 16,384 pixels, at most 36 million pixels,
  checked coordinate endpoints, exact payload lengths, no duplicate JSON keys and no trailing
  bytes. The host's own adapter limits also apply and may be lower. Optional input selection is
  validated but is not an extractor prompt or automatically multiplied into the returned mask.

## Windows package versus a separate QNN adapter

The Windows AI system API used here needs a correctly declared **MSIX package identity** and
an effective `systemAIModels` capability on a compatible OS/hardware configuration. Launching
an unpackaged published EXE is not an equivalent test. The package includes the pinned
experimental Windows App SDK and its app-local activation registrations.

This requirement is specific to this system API; it is **not** a general requirement for the
host-neutral adapter protocol or for a separate ONNX Runtime QNN adapter. Such an adapter
would need its own QNN runtime/provider dependencies, compatible model, explicit device policy
and validation. This companion package does not provide any of those or prove QNN inference.

For a reviewed target PC, use the package's installation helper and exported certificate only
after reviewing the trust/installation operations. Development signing is not production
publisher trust, and building a package does not install it, prepare a model or run inference.

## Cloud-only build and protocol regression evidence

The [companion workflow](../../.github/workflows/companion-windows-ai.yml) has two independent jobs:

1. Build the [standalone protocol fixture project](ProtocolTests/PhotoCraft.WindowsAI.ProtocolTests.csproj),
   which links the real [Protocol.cs](Protocol.cs) and targets plain `net8.0`. It references **no
   Windows AI SDK, Windows target framework, package identity or hardware**. Its executable runs
   assertions (nonzero exit on failure), records named checks in JSON, and uses the actual
   `Protocol.WriteResponse` in a fresh fixture directory to generate `ready.bin`, `needs_model.bin`, `unavailable.bin`,
   `error.bin` and `ok-mask.bin`. Assertions check the exact allowed wire keys, null fields,
   normalized mask bytes, malformed/truncated/duplicate JSON, framing/geometry/sample ranges,
   selection validation and no-overwrite writer behavior.
2. Cross-publish and development-sign an ARM64 Windows AI MSIX with the existing packaging
   script; record the package hash and matching exported signing certificate. No package/model
   is installed and no hardware inference is run on the cloud runner.

The protocol job explicitly runs the [Rust interop test](../../crates/plugins/tests/worker_companion_contract.rs)
with stable Rust, the generated fixture directory and `--ignored`. It decodes all five C#
responses through the production `photocraft_plugins::worker::protocol::read_response`, checks
status/messages, mask metadata/samples and execution metadata, and confirms obsolete null
response fields remain rejected. Rust's production `write_request` then writes a control request
and 8/16/32-bit provenance, RGBA/selection and synthetic ICC-header fixtures. A separate C#
`--verify-rust` invocation reads them without regenerating response files, checks every f32le
sample/selection byte and uses the production reader to verify RGBA64 staging. The ICC fixture
is only a synthetic header for transport validation, not a WIC color-conversion oracle.
An explicitly run test with missing environment/fixtures **fails** rather than reporting a
successful skip; CI has a negative gate for missing inputs. Fixtures, C# JSON results, Rust
result JSON and both tools' logs are uploaded as protocol evidence; package signing/build
evidence is uploaded separately. No run is considered passing merely because the YAML exists.

For maintainers reproducing the **GitHub Actions commands** (do not treat these as evidence of
a local or hardware test):

```powershell
dotnet build ./examples/npu_subject_worker/ProtocolTests/PhotoCraft.WindowsAI.ProtocolTests.csproj --configuration Release
$env:PHOTOCRAFT_COMPANION_FIXTURES = Join-Path $PWD 'artifacts/companion-protocol/fixtures'
dotnet run --no-build --configuration Release --project ./examples/npu_subject_worker/ProtocolTests/PhotoCraft.WindowsAI.ProtocolTests.csproj -- --fixtures "$env:PHOTOCRAFT_COMPANION_FIXTURES" --results ./artifacts/companion-protocol/csharp-test-results.json
cargo +stable test --locked -p photocraft-plugins --test worker_companion_contract -- --ignored --nocapture
dotnet run --no-build --configuration Release --project ./examples/npu_subject_worker/ProtocolTests/PhotoCraft.WindowsAI.ProtocolTests.csproj -- --verify-rust "$env:PHOTOCRAFT_COMPANION_FIXTURES/from-rust" --results ./artifacts/companion-protocol/csharp-bidirectional-results.json
./examples/npu_subject_worker/Build-Package.ps1 -DevelopmentCertificate -OutputDirectory ./artifacts
```

These checks prove a wire-contract/build/signing result only after a successful workflow run.
Target-device installation, readiness transitions, model preparation, color conversion quality,
mask quality and physical execution still require separate, explicitly authorized validation.
