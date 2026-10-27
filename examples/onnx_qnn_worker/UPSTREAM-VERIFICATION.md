# Locked upstream review (read-only, 2026-10-22)

This records static source/archive observations, **not compilation, security certification, legal approval or hardware validation**. No tools/packages were installed and no local build/test/publish was run for this review.

## Identity and API

- [QNN NuGet 1.24.4](https://www.nuget.org/packages/Microsoft.ML.OnnxRuntime.QNN/1.24.4) exists; the full `.nupkg` was read in memory from [NuGet's flat container](https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.qnn/1.24.4/microsoft.ml.onnxruntime.qnn.1.24.4.nupkg).
- Observed archive SHA-256: `e4d6eabb9e503d4f3c78494fc9400f02509b2ee315d9f707644a174ece8da17f`.
- Both QNN and Managed nuspecs identify immutable source commit `2d924974ef147392ced8409d36bd6d2e7fcc8a74`. QNN is native-only and depends on Managed >=1.24.4; this worker pins **both** to exact `[1.24.4]` and never references the CPU-only ORT package.
- [SessionOptions at that commit](https://github.com/microsoft/onnxruntime/blob/2d924974ef147392ced8409d36bd6d2e7fcc8a74/csharp/src/Microsoft.ML.OnnxRuntime/SessionOptions.shared.cs) contains `AppendExecutionProvider(string, Dictionary<string,string>)`, explicitly supporting provider name `QNN`, and `AddSessionConfigEntry`.
- [InferenceSession at that commit](https://github.com/microsoft/onnxruntime/blob/2d924974ef147392ced8409d36bd6d2e7fcc8a74/csharp/src/Microsoft.ML.OnnxRuntime/InferenceSession.shared.cs) contains `Run(IReadOnlyCollection<NamedOnnxValue>, IReadOnlyCollection<string>)`. [Tensor at that commit](https://github.com/microsoft/onnxruntime/blob/2d924974ef147392ced8409d36bd6d2e7fcc8a74/csharp/src/Microsoft.ML.OnnxRuntime/Tensors/Tensor.shared.cs) exposes `Length` and `ReadOnlySpan<int> Dimensions`.
- Managed has `lib/net8.0/Microsoft.ML.OnnxRuntime.dll`; its net8 dependencies include System.Memory >=4.5.5 and System.Numerics.Tensors >=9.0.0. Remote restore audits the actual resolved transitive graph, treating audit warnings as errors. Version existence/source review is **not** a statement that no vulnerabilities exist.
- NuGet archive signatures are verified remotely with `dotnet nuget verify --all`. CI records resolved package archive hashes, actual payload hashes, sizes and PE machine values; a changed pinned archive/native inventory is a failure, not a warning.

## Observed QNN ARM64 native payload

The package has **15** entries under `runtimes/win-arm64/native`. All its native DLLs have PE machine `0xAA64`. The draft preserves all 15, including the small link-time import library; it does not assume `QnnHtpStub.dll` is a real filename.

| Actual asset | Observed bytes |
|---|---:|
| QnnCpu.dll | 9,113,792 |
| QnnGpu.dll | 11,201,216 |
| QnnHtp.dll | 4,564,672 |
| QnnHtpPrepare.dll | 176,059,072 |
| QnnHtpV73Stub.dll | 1,021,120 |
| QnnHtpV81Stub.dll | 1,021,120 |
| QnnSystem.dll | 4,522,176 |
| libQnnHtpV73Skel.so | 10,249,464 |
| libQnnHtpV81Skel.so | 11,271,968 |
| libqnnhtpv73.cat | 12,216 |
| libqnnhtpv81.cat | 12,216 |
| onnxruntime.dll | 31,656,984 |
| onnxruntime.lib | 2,888 |
| onnxruntime_providers_qnn.dll | 5,099,592 |
| onnxruntime_providers_shared.dll | 41,504 |

QnnCpu/Gpu being retained for complete-source inventory does **not** make them allowed inference backends. Production selects only the absolute app-local HTP DLL and disables graph CPU fallback.

## Observed QNN top-level legal assets

| Asset | Bytes | SHA-256 |
|---|---:|---|
| LICENSE | 1,094 | `c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674` |
| Privacy.md | 2,490 | `bfb7712b075f3f07e3ebee5dc2a296d1de32dbf71ad28663f73d1538cb6c3b08` |
| Qualcomm_LICENSE.pdf | 147,577 | `ec1dccfdcba5c6e64126e84199b8362bf4999107bfa567ebe831dbb4c461692b` |
| ThirdPartyNotices.txt | 331,175 | `fb0af774b4d7cffc5b9d046f2aaeade2f37df2f80abf8033c95dfffcc77a8866` |

Managed separately includes `LICENSE.txt`, `Privacy.md` and `ThirdPartyNotices.txt`. The script copies these and the restored transitive/.NET runtime top-level legal assets verbatim into per-package notice directories before constructing the host file list. SHA-256 here is provenance evidence/model integrity; host installation records use **BLAKE3**, a separate mechanism.

The Qualcomm PDF's existence/hash has been checked, **not approved for redistribution**. See [the blocked legal gate](LEGAL-REVIEW.md).

## Model envelope review basis

The conservative scanner's field/wire table follows the [ONNX v1.20.1 public proto](https://github.com/onnx/onnx/blob/v1.20.1/onnx/onnx.proto), read from upstream. Tensor external-data routes through root/nested graph initializers, tensor attributes and sparse tensor values/indices are traversed. Wrong-wire encodings, duplicate singular fields, unknown container fields, custom domains/opsets, local functions, training data, EPContext and multi-device configurations are rejected. Model IR is deliberately limited to 1..10 pending further review.

Tensor raw content and non-file-bearing type/metadata messages are not fully parsed; protobuf/ONNX validity, tensor contents, operator semantics and graph placement remain ORT responsibilities. Presence of QuantizeLinear/DequantizeLinear is only a scope filter, not a guarantee of quantization correctness or HTP support. The scanner is not a sandbox and does not justify running untrusted models.
