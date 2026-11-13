# Local ML models

PhotoCraft's optional ML features run local ONNX models through the `photocraft-ml` crate and `tract-onnx` 0.23.9 (MIT OR Apache-2.0). Models are downloaded into a user cache directory; they are not committed, bundled, or used by the WebAssembly build.

## Commands

- `select.subject` and `layer.removeBackground` accept `engine: auto|model|classical`. `auto` uses BiRefNet-lite when installed and otherwise uses the classical implementation. `classical` avoids model inference; `model` requires the model.
- `edit.removeSubject` accepts `fill: auto|lama|patch`. `auto` uses LaMa when installed and otherwise uses the patch based fill; `patch` avoids model inference; `lama` requires the model.

## Install

In the app, **Edit › Preferences › AI Models** lists the models with their size and licence, and downloads (with progress and cancel) or removes each one; nothing is fetched until you click Download, and no image ever leaves the computer. On the command line, `photocraft-cli models fetch` downloads the models into the resolved models directory (`photocraft-cli models list` shows what is installed). Both verify the size and SHA-256 before installing. To install manually, set `PHOTOCRAFT_MODELS_DIR` and fetch either or both files:

```sh
export PHOTOCRAFT_MODELS_DIR="$HOME/Library/Application Support/PhotoCraft/models"
mkdir -p "$PHOTOCRAFT_MODELS_DIR"
curl -L --fail -o "$PHOTOCRAFT_MODELS_DIR/birefnet_lite.onnx" https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/main/onnx/model.onnx
curl -L --fail -o "$PHOTOCRAFT_MODELS_DIR/lama_fp32.onnx" https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx
```

The app reads `PHOTOCRAFT_MODELS_DIR` when set. Otherwise it uses the per-user data directory: macOS `~/Library/Application Support/PhotoCraft/models`, Linux `$XDG_DATA_HOME/photocraft/models` (or `~/.local/share/photocraft/models`), and Windows `%APPDATA%\PhotoCraft\models`. The app checks the expected file size before loading a model; a missing or truncated file is treated as unavailable.

| File | Model and input | Published size | SHA-256 | License | Source file |
|---|---|---:|---|---|---|
| `birefnet_lite.onnx` | BiRefNet-lite, 1024 × 1024 | 224,005,088 bytes | `5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333` | MIT | [onnx-community/BiRefNet_lite-ONNX](https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/blob/main/onnx/model.onnx) |
| `lama_fp32.onnx` | LaMa, 512 × 512 | 208,044,816 bytes | `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6` | Apache-2.0 | [Carve/LaMa-ONNX](https://huggingface.co/Carve/LaMa-ONNX/blob/main/lama_fp32.onnx) |

## Runtime and licences

Inference runs on the CPU. A 1024-pixel inference can take tens of seconds; performance is being measured. Classical selection and patch-based filling remain available without model files.

The model weights keep their upstream licences: BiRefNet-lite is MIT and LaMa is Apache-2.0. The runtime dependency, `tract-onnx` 0.23.9, is MIT OR Apache-2.0. Model files are not included in PhotoCraft distributions; see [`ATTRIBUTION.md`](../ATTRIBUTION.md).
