# Optional local selection models

PhotoCraft keeps its classical Object Selection, Select Subject and Remove Background methods
as the defaults. A build with `local-ml` offers two separately downloadable alternatives for
cases where a larger learned model is useful. Images are processed on the device; downloading
weights is the only network operation. This first backend uses CPU inference on native desktops,
including Windows and Linux, without Python, CUDA or a GPU requirement. The web build retains
the classical methods.

| Model | Use | Download | Training input | Licence |
|---|---|---:|---|---|
| BiRefNet HR Matting | Select Subject / Remove Background, preserving soft alpha | 932,150,975 bytes (889 MiB) | 2048 × 2048 RGB | MIT |
| SAM 2.1 Large | Object Selection with a dragged rectangle | 911,109,265 bytes (869 MiB) | 1024 × 1024 RGB | Apache-2.0 |

These FP32 models can take tens of seconds and use several GB of RAM. They are optional methods,
not a guarantee of better results on every image. SAM is an object segmentation model, not a
hair/transparency matting model. BiRefNet may pick several foreground objects. Difficult edges
can still need Select and Mask or manual mask edits. HDR values are converted and clipped for
the model input; original pixels remain unchanged. Canvases are currently limited to 64 MP for
model commands, and output coverage uses PhotoCraft's existing 8-bit selection/mask format.

## Build and use

```sh
cargo run --release -p photocraft --features local-ml -- photo.png
```

1. Open Edit › Preferences › Integrations.
2. Download either model. Progress and Cancel use the existing status bar. Each download is
   independent and does not select a method automatically.
3. Choose a downloaded model for **Object selection** or **Subject / background removal**, then
   Apply or OK. These preferences are independent; Classical remains available.
4. Drag an Object Selection rectangle, use Select › Subject, or use the existing **Remove
   Background** Quick Action in the layer Properties panel.

Remove Background replaces the layer's mask, converts a Background to a normal layer when
needed, and clears the selection in one undo step. Source pixels are preserved. Its `refine`
option affects the classical method only: applying classical refinement to BiRefNet's alpha
would change the matte. Existing layer/parent locks still apply. Selection commands retain
replace/add/subtract/intersect and active-layer/composite sampling.

Missing or corrupt weights produce an actionable error; inference never downloads or silently
changes methods. **Remove download** removes only the selected model and returns its preference
to Classical. A cancelled job never applies its document result. Cancellation during model
loading is checked after session creation; network cleanup may wait for a stalled read timeout.

The desktop cache is `Models/` under the existing per-user configuration directory (including
`PHOTOCRAFT_CONFIG_DIR` or portable `PhotoCraftData`). It is separate from image files and
preferences. Each immutable model revision gets its own directory with `LICENSE.txt` and
`SOURCE.txt`; inference verifies the hashes again, including SAM's external weights.

## Commands and CLI

| Command | Parameters |
|---|---|
| `models.list` | `{}`; returns availability, installed/busy state, sizes, licences and provenance |
| `models.download` | `{"id":"birefnet-hr-matting"}` or `{"id":"sam2.1-large"}` |
| `models.remove` | Same ids; no document required |
| `select.subject`, `layer.removeBackground` | `"model":"classical"` or `"birefnet-hr-matting"`; omitted uses Preferences |
| `select.object` | `"rect":[x,y,width,height]`, plus `"model":"classical"` or `"sam2.1-large"` |

Downloads/removals do not enter recorded actions or document history. Commands accept catalogue
ids, never arbitrary URLs or model paths. Recorded selection/removal actions store the chosen
method, so changing Preferences does not change playback. Batch and droplets reuse a configured
backend across inputs. Automation hosts must explicitly inject a backend
capability; an untrusted headless MCP session does not gain cache/network authority by enabling
the feature. The trusted CLI accepts an explicit `PHOTOCRAFT_MODEL_DIR`:

```sh
# Build once. No model weights are downloaded at build time.
cargo build --release -p photocraft-cli --features local-ml
export PHOTOCRAFT_MODEL_DIR="/absolute/path/to/model-cache"

# Explicit installation, without an input photo (the empty document is only CLI run context).
target/release/photocraft-cli run --new '{"width":1,"height":1}' \
  --cmd models.download --params '{"id":"birefnet-hr-matting"}'

target/release/photocraft-cli run photo.png \
  --cmd layer.removeBackground --params '{"model":"birefnet-hr-matting"}' \
  --out cutout.png

target/release/photocraft-cli run photo.png \
  --cmd select.object --params '{"rect":[40,20,500,700],"model":"sam2.1-large"}' \
  --out selected.pcraft
```

PowerShell: set `$env:PHOTOCRAFT_MODEL_DIR = 'C:\\path\\to\\model-cache'` and use the `.exe` binary.
Distributors opt in with `local-ml`; existing/default installers do not acquire the feature from
downloading these weights. `ort = 2.0.0-rc.13` downloads its pinned ONNX Runtime 1.28 redistribution
at **build** time. Ship that runtime's MIT licence and third-party notices with enabled builds;
the copies in `crates/ml/licenses/` document this obligation. Release packaging is a distributor
decision, and this change does not enable the feature in official release workflows.

## Provenance and review

All URLs, exact byte counts and SHA-256 hashes are in
[`crates/ml/src/catalog.rs`](../crates/ml/src/catalog.rs), derived from the pinned exports' Hugging
Face LFS metadata. A model update requires a code change and review; there is no mutable remote
manifest. Original checkpoint licences apply to their converted weights:

| Model | Original checkpoint / code | Reviewed ONNX export and immutable revision |
|---|---|---|
| BiRefNet | [ZhengPeng7/BiRefNet_HR-matting](https://huggingface.co/ZhengPeng7/BiRefNet_HR-matting), [BiRefNet](https://github.com/ZhengPeng7/BiRefNet) | [PinkPixel/birefnet-hr-matting-onnx](https://huggingface.co/PinkPixel/birefnet-hr-matting-onnx/tree/792518b9d4dfe9793891712677de25e554f948c6) |
| SAM 2.1 Large | [facebook/sam2.1-hiera-large](https://huggingface.co/facebook/sam2.1-hiera-large), [facebookresearch/sam2](https://github.com/facebookresearch/sam2) | [onnx-community/sam2.1-hiera-large-ONNX](https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX/tree/3c23431f721e69cae82dbfd0c28fd692cc714021) |

The BiRefNet export has `image` input and `alpha` output with sigmoid already included. SAM uses
three image embeddings, an ignored padding point with int64 label `-1`, and normalized box
corners scaled to 1024. It returns three logits masks and predicted IoU scores. These contracts
are tested through the real runtime with small original synthetic graphs, and the downloaded
graphs are inspected/run separately. Community exports should remain part of upstream review:
file hashes establish identity, not an independent audit of model training or conversion.

```sh
cargo test -p photocraft-ml --features onnx
cargo test -p photocraft-engine -p photocraft-cli --features local-ml
cargo clippy -p photocraft-ml -p photocraft-engine --features photocraft-ml/onnx,local-ml --all-targets -- -D warnings
```

`.github/workflows/local-models.yml` exercises CPU runtime contracts, commands and native build
paths on Windows, Linux and macOS without downloading learned weights. The opt-in full-model
smoke test is separate; the download flag is explicit and transfers about 1.8 GB:

```sh
PHOTOCRAFT_MODEL_TEST_DIR=/absolute/path/to/test-cache \
PHOTOCRAFT_MODEL_TEST_DOWNLOAD=1 \
  cargo test -p photocraft-ml --features onnx real_models_cpu_smoke -- --ignored --nocapture
```

Omit `PHOTOCRAFT_MODEL_TEST_DOWNLOAD` to test existing verified files offline. A quantitative
accuracy benchmark with labelled hair, fine structures, multiple objects and transparent
materials remains necessary before making a general improvement claim. This initial contribution
keeps the smart-features scorecard partial for that reason.

See [validation and comparison](local-models-validation.md) for the measured 24 MP timings,
before/after Preferences screenshots, actual exported masks and outstanding platform checks.
