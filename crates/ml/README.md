# photocraft-ml

L4 boundary for optional local image segmentation. The default crate contains a pinned model
catalogue, image/mask validation and an `InferenceBackend` trait. The `onnx` Cargo feature adds
the native CPU backend using the safe `ort` API. It never links ONNX Runtime into wasm.

Hosts inject a cache directory. Constructing a backend, querying its status, opening an image
or selecting a method never downloads anything. Only `download` contacts the pinned export
URLs. Downloads stream into a temporary directory, verify exact sizes and SHA-256 hashes, and
install by rename after every artifact succeeds. Failure/cancellation discards staging files.
Inference re-verifies artifacts before parsing; arbitrary model paths and URLs are not commands.

One model's weights stay resident at a time. CPU arena and memory-pattern caching are disabled
to release large temporary tensors between runs instead of retaining peak allocations.
Per-model locks protect download/removal/inference, and a
shared session lock prevents two large inferences from running together. CPU runs use up to eight
threads; `RunOptions::terminate` handles cancellation. Slow session construction finishes before
the next cancellation check. Stalled HTTP reads time out after 30 seconds, separately from the
one-hour total body budget. `ureq` is pinned because its idle-timeout adapter uses the unversioned
transport seam without changing the default TLS/proxy connector.

`RgbImage` inputs must already be sRGB; the engine converts the document profile through CMS.
Inputs are mapped to the training square with bilinear interpolation and ImageNet-normalized
NCHW. BiRefNet's reviewed export returns sigmoid alpha directly; SAM returns logits, with the
best predicted-IoU mask chosen and thresholding after interpolation. Box prompts share the
full-image normalized coordinates. `AlphaMask::coverage` supplies the existing selection/mask
coverage representation while source pixels retain their original depth and colour mode.

See [local models](../../docs/local-models.md) for UI/CLI usage, limitations, exact provenance,
licences and reproducible checks. Tiny original synthetic ONNX graphs in `src/test_models.rs`
test the runtime contracts offline; they contain no learned weights and make no accuracy claim.
