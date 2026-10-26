# PhotoCraft AI integration plan

This is the implementation contract for native and browser AI workflows. The
existing implementation was validated on PhotoCraft 0.6.0. Integration with the
fork's newer 0.8.2 base passes the browser workflow and IO corpus gates; refreshed
installer packaging is in progress. Test numbers from the earlier base are
historical evidence, not proof that the newer base has passed the same checks.

## Implemented baseline

| Workflow | Provider | Native document behavior |
| --- | --- | --- |
| Generate Image | ComfyUI or OpenAI Images | Preview, then a new layer/canvas on explicit apply |
| AI Edit | ComfyUI or OpenAI Images | Captured visible image, editable result layer |
| Generative Fill / Remove | ComfyUI or OpenAI Images | Native selection limits the final composition |
| Replace Background | ComfyUI | Foreground mask plus generated background |
| Remove Background | ComfyUI BiRefNet | Original pixels with a separate editable mask |
| Generative Expand | ComfyUI | Preserve the original region; mask only new canvas areas |
| AI Upscale | ComfyUI Real-ESRGAN | Start from original pixels; preserve alpha/profile/depth |
| Automatic prompt preparation | OpenAI-compatible Responses | Text instruction only; strict structured prompt |

Direct native-size generation is the default for new ComfyUI images. Reduced
working copies and optional detail reconstruction remain explicit alternatives.
4K and 8K presets describe requested output dimensions, not a guarantee that a
particular GPU/model can execute them. Native 4K passed on the tested 12 GiB GPU;
native 8K exceeded its VRAM. Separate native-4K-to-8K Upscale passed.

## Newly requested work

1. **Interpret Image / Prompt from Reference.** Send an explicitly selected,
   bounded working copy to a vision-capable OpenAI-compatible model. Return an
   observable image description and editable reconstruction prompt. Review the
   prompt before opening Generate Image. This is separate from ordinary text-only
   prompt preparation; normal generation must not start sending source images.
2. **ComfyUI dependencies in AI Settings.** Report required nodes and the five
   model files used by the fixed workflows. Offer a scoped download of missing
   weights with progress, cancellation, sizes and license links. A Rust companion
   service writes into the configured server model directory. Existing files,
   unrelated custom nodes and active ComfyUI jobs remain protected. Missing core
   nodes require an explicit ComfyUI upgrade; the downloader does not silently
   replace the server installation or restart it.
3. **Publish in the user's fork.** Commit and push the reviewed changes with
   English messages. Keep personal API keys, addresses, hostnames, login files,
   local reports and installer artifacts out of Git. Use neutral public examples.
4. **English defaults and documentation.** English is the default UI and project
   documentation language. Preserve explicitly selected session languages.

## Architecture

`photocraft-ai` contains provider-neutral requests, workflow validation, bounded
transport images, native HTTP adapters and the browser gateway protocol.
`photocraft-engine` captures documents, runs registered commands, validates
results and prepares native layers. `photocraft-ui-egui` renders command dialogs,
progress and previews. No JavaScript UI framework or desktop webview is used.

Native provider calls run on background workers. The WebAssembly application
uses an authenticated same-origin Rust gateway, which executes the same native
engine commands. A bounded native `.pcraft` carrier transports prepared layers,
profiles, bit depth and masks back to the browser. Browser document incarnation,
revision validation, explicit application and Undo stay local.

The gateway owns one AI worker, limits document uploads and retained results,
and scopes job status/cancellation to the browser session. Lost submit responses
are reported as unknown outcomes and are not retried automatically. Cancellation
never clears the global ComfyUI queue or interrupts a different client's job.

## Credentials and configuration

Direct API keys are entered in password fields under AI Integrations. In the
desktop app they live in memory for the session and are never written to the
preferences file (nor to its copies and backups); a key that must survive a
restart is named by environment variable (`*ApiKeyEnv`), and only that name is
stored. In the web gateway keys are saved in the server's private configuration
file (mode 0600, not encrypted). Keys are excluded from public preference/control
snapshots, job journals, native image documents and browser localStorage.
Automation (control channel, MCP) cannot change `integrations.*` through
`prefs.set`: endpoints and key variables decide where a credential is sent.

The web host optionally accepts `OPENAI_API_KEY` and per-service Docker key
variables. Menu-saved service keys take precedence. Environment credentials are
not copied into persisted configuration or sent to the browser. Docker also sets
an English default language and a configurable prompt-model ID. Explicit browser
session language overrides and per-workflow model choices have precedence.

## Required model profile

- Qwen Image 2.1 INT8 ConvRot diffusion model.
- Qwen3-VL 8B INT8 ConvRot text encoder.
- Qwen Image 2.1 VAE.
- BiRefNet foreground model.
- Real-ESRGAN x4plus for standalone Upscale and optional detail reconstruction.

Model metadata includes immutable download revisions, expected file sizes and
SHA-256 hashes. Weights are downloaded on the configured server and are not
bundled in PhotoCraft installers or committed to this repository. Model licenses
are separate from PhotoCraft's code license.

## Acceptance gates

- Roundtrip prepared native layers and masks at RGB/Gray, U8/U16, with profile,
  selection, explicit apply, Undo/Redo and save/reopen fidelity.
- Real browser generation, result preview/application, stale-result recovery,
  cancellation and progress. Confirm no provider keys in browser storage or
  diagnostic/control responses.
- Local HTTP peers verify strict Responses/Images request schemas, opaque model
  IDs, refusal/truncation handling and errors. Verify that saved keys take
  precedence over environment defaults.
- Image interpretation explicitly uploads only its bounded working image and
  produces no document edit or automatic image generation.
- Dependency downloads accept only catalog identifiers, validate size/hash,
  write atomically, preserve existing files and clean only their own temporary
  data on failure/cancellation.
- Run relevant crate tests, strict clippy, layering, WebAssembly, command panic
  fuzzing, parity generation and visual UI checks. Run installed Linux and Wine
  package smoke tests; native Windows GPU testing remains a separate gate.

## Evidence and remaining limits

The earlier baseline passed installed Debian configuration persistence (0600),
GUI menu/context checks, Wine installation/load/registry/file roundtrip/removal,
real native 4K generation, separate 8K Upscale, result recovery and Undo/Redo.
The browser passed real 3840×2160 generation with a synthetic local prompt peer,
opaque routed model IDs, exact native output dimensions and explicit apply.
Docker key authentication and session-language precedence passed against local
HTTP peers without a paid API call.

These tests do not establish the user's actual prompt endpoint, credentials or
vision-model capability. Real-photo quality, exact reference recreation and
native Windows driver/GPU behavior require separate acceptance. On 0.8.2, the
browser image-analysis/review/edit/generate/apply/Undo/Redo sequence passed with a
synthetic Responses/Images peer. The live dependency inventory reported all five
models present; public diagnostics and browser storage omitted provider keys.
The new dialogs were visually inspected. The IO corpus suite passed after the
upstream merge. Existing model files are size-checked, not claimed hash-verified.

See [AI usage](ai-usage.md), [ComfyUI audit](comfyui-audit-2026-10-08.md) and
[Photoshop workflow comparison](photoshop-workflow-abgleich-2026-10-08.md).
