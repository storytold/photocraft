# Generative architecture

Updated 2026-10-29: ShotoPhopEX uses only the native worker. ComfyUI’s client, HTTP and
WebSocket transport, sidecar, graph templates and backend preferences have been removed.

The egui shell, CLI, control channel and MCP dispatch engine commands. Generative commands
build a backend-independent request from the composite, selection and prompt, then
`crates/genai/src/native.rs` sends a gn-engine `RunRequest` and raw picture, mask and reference
blobs to `photocraft --engine`. The worker comes from the pinned, separate `genai-native`
repository; this change does not modify it.

The worker manages model loading and residency. `generate.engine.status`, `start`, `restart`,
`purge` and `stop` address only that worker. Purge unloads models without starting a worker.
Generation runs as a cancellable background job and applies its result as one undo step;
source document pixels are preserved, and fill/expand results use layer masks.

Set **Models folder** (`integrations.modelsPath`) explicitly. It reads existing weights in
ComfyUI’s `models` layout (`diffusion_models/`, `text_encoders/`, `vae/`, `loras/`, `checkpoints/`)
or the worker’s family folders. ComfyUI need not be installed. `runtimePath` names native
runtime libraries, with the packaged runtime fallback retained. Old preference files containing
retired ComfyUI keys load normally; those keys are discarded.

`crates/genai/workflows/*.json` are metadata-only templates: IDs, families, tasks, licence
classes, prompts, defaults, model slots, request parameters, references, sizes and picker data.
Every retained ID is served by the pinned gn-diffusion or gn-perception template index.
The web build still compiles the metadata and commands; local generation remains unavailable.

Tests use a CPU-only in-process implementation of `gn_engine::server::Engine`, served over the
real worker protocol. It records requests and raw blobs and supplies deterministic images,
matte rectangles, segmentation masks, chat, errors, progress and cancellation.

`docs/comfy/bench/scripts/*` remains external comparison tooling. Any ComfyUI there runs
independently of ShotoPhopEX. Scripts using removed app preferences require the last
ComfyUI-capable app commit `cdcedbd8`, or direct ComfyUI HTTP calls.
