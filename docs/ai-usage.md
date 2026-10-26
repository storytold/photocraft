# PhotoCraft AI workflows

The current AI integration targets PhotoCraft **0.8.2**, shared by the native
Rust application and Rust/WASM browser build. Earlier 0.6.0 development installers
have separate verification reports; they do not contain the new image-analysis
and dependency-management features.

OpenAI Responses can prepare the image prompt automatically
before ComfyUI executes the fixed workflow. Direct OpenAI Images is an alternative
for generation/edit/fill/remove requests.

## Configure

Open **Edit › Preferences › AI Integrations…** and set the base URLs:

- ComfyUI: your server, for example `http://127.0.0.1:8188`.
- OpenAI prompt and images: `https://api.openai.com/v1` by default.
- API keys: enter the actual key in the password field for the corresponding service.
  **Apply** keeps it for the current session only; **Clear API key** removes it. An
  unauthenticated ComfyUI or self-hosted prompt endpoint can leave both key and
  optional variable blank.
- Environment variable: to keep a key between sessions, name the variable that holds it.
  Only the variable name is saved. A key entered in the session takes precedence.

Desktop keys are never written to the preference file; keys found in a file saved by an
earlier build are ignored and removed on the next save. Public preference/control
responses and job journals redact the key fields; image documents and result metadata
contain no keys. Agents driving the app cannot change `integrations.*` preferences.
In the browser, keys are saved through the authenticated Rust server into
`/data/ai-config.json`, persisted by the Docker configuration volume. Browser
localStorage contains only redacted key fields. A blank browser field keeps the
server key; **Clear API key** removes it. Endpoints saved from the browser must be in
the operator's allowlists (`PHOTOCRAFT_ALLOWED_COMFYUI_ENDPOINTS`,
`PHOTOCRAFT_ALLOWED_PROMPT_ENDPOINTS`, `PHOTOCRAFT_ALLOWED_IMAGE_ENDPOINTS`,
`PHOTOCRAFT_ALLOWED_DEPENDENCY_ENDPOINTS`; the configured endpoint is always allowed,
the dependency list is empty by default), and key-variable names cannot be changed
from the browser. Server configuration saving finishes
before the next image job starts. Reverse-proxy URL prefixes are retained.

The built-in ComfyUI profile requires these installed weights:
`qwen_image_2.1_int8_convrot.safetensors`, `qwen3vl_8b_int8_convrot.safetensors`,
`qwen_image_2.1_vae_bf16.safetensors` and, for foreground masks,
`birefnet.safetensors`. Required nodes/model dropdowns are checked before uploads
and before the OpenAI prompt call. These assets are not included in installers.

## Edit

Use **Edit › Generate Image… / Edit Image with AI… / Generative Fill… /
Generative Remove…**, **Image › Generative Expand…**, or
**Layer › Remove Background with AI…**. **Edit › Replace Background with AI…**
chains BiRefNet segmentation and masked background generation in one job. Fill/removal also appear in the selection
context menu. Select an area before fill/removal. Optional context margin adds
surrounding pixels while preserving the original selection mask.

Enable **Prepare the prompt automatically with OpenAI** for the two-stage
workflow. Model identifiers are sent unchanged to the configured prompt
endpoint; self-hosted tags/routes such as `qwen3.6:35b@GPU-HOST` and
`organization/model:tag` are accepted. Use the exact ID exposed by your server.
The endpoint must support Responses and the structured JSON prompt schema; an
OpenAI-compatible Chat Completions endpoint alone is not sufficient for this profile. The text model is editable; the default is `gpt-4.1-mini`. Only text,
operation, workflow identity and target dimensions reach the Responses API.
Direct OpenAI Images sends the source image/edit mask to its configured endpoint;
the image model is editable (default `gpt-image-2.5-sunburst`). OpenAI usage is
billed by the provider. Image quality is editable; low quality remains the initial
default. With GPT Image 2/2.5, the direct Images profile supports custom portrait
and landscape dimensions up to 3840 pixels per edge and 8,294,400 pixels total.
Dimensions must be multiples of 16 and the aspect ratio between 1:3 and 3:1;
resolutions above 2560 × 1440 are experimental according to OpenAI. Direct
generation rejects targets exceeding the chosen model limits. Explicit reduced
generation maps destinations to a supported API working size.
The legacy GPT Image 1/1.5 profile remains square-only. Compatible proxies may
have different model limits. Seed/negative prompt are ComfyUI settings.

## Create a prompt from an image

Open the reference image, then choose **Edit › AI › Create Prompt from Image…**.
Select a vision-capable prompt model and optionally describe which visual details
matter. **Analyze image** sends a color-managed composite with a maximum
1024-pixel long edge to the configured OpenAI-compatible Responses endpoint.
It leaves the original document unchanged. Ordinary automatic prompt preparation
remains text-only.

Review the returned description and edit the recreation/negative prompts.
**Use prompt for generation** opens the normal Generate Image form with automatic
prompt preparation disabled. Choose the desired output resolution and generate
explicitly. A textual description cannot guarantee identical source pixels.
The endpoint must support Responses image inputs and structured JSON output;
a text-only model or Chat Completions-only server is insufficient.

## ComfyUI dependencies

In **AI Integrations…**, configure the optional **ComfyUI · Dependency service**
base URL and its separate token, then **Apply**. Choose **Manage ComfyUI
dependencies…** (also **Edit › AI › Check ComfyUI Dependencies…**).
Check required nodes and installed model files, select missing weights, review
license links, accept the selected licenses and choose **Download selected models**.
Downloads run on the ComfyUI host with normal progress and cancellation controls.

The companion service verifies new downloads against fixed sizes and SHA-256
hashes and preserves existing files. Existing files are checked by size only;
a size mismatch is reported for manual review. It does not install arbitrary
custom nodes or restart ComfyUI. See the [service deployment guide](../packaging/comfy-deps/README.md).

English is the default UI language. Existing translations are retained; newly
added AI strings use English where a translation is pending.

## Resolution and detail

**Generate Image** offers current-document, UHD 4K (3840 × 2160), UHD 8K (7680 × 4320)
and custom output sizes. Use custom dimensions for portrait orientation or other
formats. Choosing a different size changes the canvas only when explicitly
applying the reviewed result; Undo restores the original canvas and layers.

**Direct generation** is the default. ComfyUI receives the requested dimensions
(up to 8192 pixels per edge, subject to the result memory budget). The fixed Qwen
workflow generates a latent at that size, rounded up to its 32-pixel grid, and uses
a tiled VAE decoder. It does not run an upscaler. Small VAE grid differences are
mapped to the exact requested canvas. **Advanced › Sampling steps** defaults to
25 for new images and 8 for image edits; valid values are 1–100.

Qwen Image 2.1 was trained at native 2K resolution. Direct 4K is beyond that
training resolution and can lose prompt adherence. On GPU-HOST, a direct
3840 × 2160 request produced 3840 × 2176 provider pixels in 543.64 seconds with
25 steps; the inspected image was substantially clearer than the 8-step probe.
A direct 7680 × 4320 attempt failed in KSampler with GPU memory growth on the
assigned RTX 3060 (12 GiB). An 8K request is supported by the client, but success
is hardware/model dependent. There is no automatic reduction or upscale fallback.
See the [official Qwen guide](https://docs.comfy.org/tutorials/image/qwen/qwen-image-2-1).

**Reduced generation** is an explicit alternative for limited hardware. ComfyUI
working dimensions are at most 1024 pixels per edge and the resulting pixels are
resampled to the destination. The result lists both working and output dimensions.

**Image › AI Upscale…** enlarges an existing visible image independently of
prompt generation. Choose 2×, 4× or custom dimensions preserving the aspect ratio.
The source is read at its original resolution, processed in overlapping
Real-ESRGAN patches, and converted back to the document profile and bit depth.
Original transparency is preserved separately. It produces a preview; applying
adds a new layer and changes the canvas in one Undo step. The original layers
remain available and can be hidden when applying the result. No OpenAI text or
image request is involved. Model jobs and cancellation belong only to this client.

Editing/fill/background replacement and expansion still use reduced provider
working copies. Their optional **Restore high-resolution detail with ComfyUI
(Real-ESRGAN)** stage reconstructs generated detail before applying the exact native
mask. Foreground cutouts retain native source pixels and resize only the AI mask.
Expansion retains the original pixels under an exact native edge mask. Display
previews remain at most 1024 pixels on their long edge.

The optional profile requires `RealESRGAN_x4plus.pth` in
`ComfyUI/models/upscale_models`, plus UpscaleModelLoader and ImageUpscaleWithModel.
Weights are not included in PhotoCraft packages. On the tested GPU-HOST server,
the official 67,040,989-byte weight was installed with its BSD-3-Clause license;
SHA-256: `4fa0d38905f75ac06eb49a7951b426670021be3018265fd191d2125df9d682f1`.
[Official model release](https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.1.0),
[model license](https://github.com/xinntao/Real-ESRGAN/blob/a4abfb2979a7bbff3f69f58f58ae324608821e27/LICENSE),
[ComfyUI upscale nodes](https://comfyanonymous.github.io/ComfyUI_examples/upscale_models/).

**Create Preview** starts one background job. Editing remains available, but an
content edit makes that preview stale. Saving under another name and opening or
closing layer groups do not invalidate an unchanged image. The status bar exposes progress/cancellation.
Cancellation targets only the returned ComfyUI job ID; it never clears the queue
or globally interrupts another client. A lost submit response is reported as an
unknown outcome and is not retried automatically.

**Window › List AI Results** opens previews of the actual masked composite,
prepared on the worker. Review the image/mask and prepared
prompt, then **Apply as New Layer**, **Open as New Document** or **Discard**.
A stale result can be opened in a new document from its captured source; this
leaves the edited original untouched. The result goes to its captured
document, with a separate editable mask and one Undo/Redo transaction. Foreground
and Upscale
results offer **Hide original layers**; the originals stay in
the document and Undo restores their visibility. Expansion preserves original
pixels beneath a mask containing only the new canvas area. All four edge amounts
are pixels; the default adds 128 pixels on the right.

## Current limits

- Native Windows/Linux and browser workflows use the same commands. Browser AI requires the authenticated gateway.
- RGB/Gray documents at 8/16 bits, ICC-converted to/from SDR sRGB transport.
- Direct ComfyUI generation: 32–8192 pixels per side, up to 64 Mi pixels in
  transport; editing/segmentation/expansion: 32–1024 pixels per side. Direct
  OpenAI working copies follow the documented model-size rules. Native destination size uses checked coordinates
  and a memory budget rather than the provider's 1024-pixel bound.
- One running AI workflow per session, one result per request; 1 GiB combined native
  result/artifact cache, at most 16 retained results. Scratch buffers and captured COW source data are additional.
- Qwen VAE rounding of at most 32 pixels maps back to the exact ROI; larger or
  unexpected outputs are rejected before document mutation.
- Direct OpenAI Images does not offer segmentation or expansion in this profile.
  Account/model access remains provider-controlled; arbitrarily wide panoramas
  need ComfyUI rather than the OpenAI 3:1 profile.
- No arbitrary custom graph execution, multi-reference editing, semantic object/
  person-part/sky selection or harmonization yet. Real-ESRGAN restores detail;
  diffusion-based high-resolution refinement remains separate future work.

## Automation and testing

The same commands are available through CLI/control/MCP. For example:

```json
{"command":"edit.generativeFill","params":{"prompt":"a blue ceramic mug","preparePrompt":true,"textModel":"gpt-4.1-mini","seed":42123,"contextMargin":32}}
```

Headless `Session::execute` finishes the pipeline inline. Desktop
`Session::start` runs it on a worker and returns a normal job ID; poll `jobs.list`
and `ai.result.list`, then explicitly invoke `ai.result.apply`.

The opt-in synthetic probe accepts an explicit endpoint:

```sh
cargo run -p photocraft-ai --example live_comfy -- http://SERVER:8188 generate /tmp/generated.png
# automatic also calls OpenAI using OPENAI_API_KEY:
cargo run -p photocraft-ai --example live_comfy -- http://SERVER:8188 automatic /tmp/generated.png
```

Transport tests use local HTTP peers, covering actual request serialization,
response bounds, authentication errors and the OpenAI PNG/structured-text paths.
Production code never substitutes the test provider for an unavailable service.

API formats: [OpenAI structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs),
[OpenAI image generation](https://developers.openai.com/api/docs/guides/image-generation),
[ComfyUI server routes](https://docs.comfy.org/development/comfyui-server/comms_routes).

The native synthetic live probe verifies direct generation (default), canvas
application, Undo/Redo and exact native pixel/mask persistence after reopening:

```sh
cargo run -p photocraft-engine --example live_ai_highres -- http://SERVER:8188 3840 2160 /tmp/direct-four-k.pcraft
# Legacy reduced generation plus detail restoration, explicitly requested:
cargo run -p photocraft-engine --example live_ai_highres -- http://SERVER:8188 7680 4320 /tmp/reconstructed-eight-k.pcraft detail
```

For the earlier reduced-generation plus Real-ESRGAN pipeline on GPU-HOST, UHD 4K completed in 76.55 seconds and UHD 8K in 162.44 seconds
(including application and the native save/reopen verification). These are measured
end-to-end samples, not generation-only performance guarantees.

## Web deployment

The WebAssembly build uses the same menus, dialogs and eight AI command implementations
as the desktop build. The Rust gateway executes provider workflows and returns native
prepared layers/masks to the browser; apply, Undo/Redo and document revision validation
remain local. The browser never contacts ComfyUI or OpenAI directly.

Build `apps/photocraft-web` with Trunk, build `photocraft-web-gateway` in release mode,
then run `packaging/web/build-container.sh` and
`docker compose --env-file secrets/web.env -f packaging/web/compose.yml up -d`. The private web-login file is
`secrets/web.env`; provider keys are configured in the KI menu, not this file.
The server config volume is `secrets/web-config`. Back it up privately.
The default host binding is `127.0.0.1:18888`. Use a TLS reverse proxy when
accessing the service across an untrusted network.

The server runs one AI worker, caps document uploads at 256 MiB and bounds retained
results. Job cancellation and cleanup target the requesting browser's job IDs.
Server GPU/model capacity still determines which native generation sizes succeed.
`?control` enables the opt-in, current-tab Rust/WASM automation export for testing;
there is no remote browser control listener.

### Docker defaults and browser overrides

`secrets/web.env` supports `OPENAI_API_KEY` as the shared OpenAI prompt/image key.
Optional `PHOTOCRAFT_OPENAI_PROMPT_API_KEY` and `PHOTOCRAFT_OPENAI_IMAGE_API_KEY`
replace the shared environment key for one service. A key saved through AI Settings
has higher priority than these environment keys. Environment keys are never copied
into the persisted server configuration or browser responses.

`PHOTOCRAFT_WEB_LANGUAGE=en` sets the English default UI language. A language selected by the
user overrides it in the current browser tab/session and survives reloading that
tab; a new browser session uses the server default again. Browser localStorage from
an older session does not replace this server default automatically.
`PHOTOCRAFT_OPENAI_PROMPT_MODEL=qwen3.6:35b@GPU-HOST` sets the prompt-model default for
new AI dialogs and requests without an explicit model. The dialog model field
remains editable for each workflow. This model requires the user's compatible
prompt endpoint; its ID alone does not discover or configure that server.

After changing Docker environment settings, recreate the container with
`docker compose --env-file secrets/web.env -f packaging/web/compose.yml up -d --force-recreate`.
`packaging/web/web.env.example` documents the settings without real credentials.
