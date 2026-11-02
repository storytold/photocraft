# User-configured image APIs (desktop prototype)

This follow-up to the Codex integration adds explicitly selected Gemini, Doubao/Seedream and
Black Forest Labs FLUX backends. It depends on the provider/import/history interface proposed
in [#10593](https://github.com/storytold/photocraft/pull/10593). Provider policy and the competing
interface in [#730](https://github.com/storytold/photocraft/pull/730) still need maintainer review.

## Use

Open a document, choose AI Edit and select the image service. Enter the provider's model ID
and API key, then **Save API settings**. Saving does not generate an image or verify model
access, credit balance or connectivity. Enter an editing instruction and confirm to send the
visible canvas to the explicitly selected service; its usage may be billed by that provider.
Success uses the existing single-layer, colour-conversion and undo/redo workflow.

API address and protocol are advanced settings. A custom service must implement the selected
image-editing protocol; this is not a universal adapter for arbitrary model endpoints.
Codex continues to use its installed runtime and ChatGPT sign-in. Service selection does not
automatically fall back, resubmit generation or switch models.

## Configuration and secrets

The native app loads public profiles from `ai-providers.json` in its normal configuration
directory. Only endpoint, protocol and model metadata are stored there. Remembered keys use
the OS credential store; session-only keys are discarded at exit. Keys are bound to the
configuration namespace, endpoint and protocol. Changing the destination requires its own key.
Malformed settings return an actionable error; they do not stop app startup.

Secret drafts are excluded from serialized UI/inspect state, actions, journals and command
results. Confirmation contains only provider and instruction. Blank key input retains the
existing key for the saved address; Remove API key deletes it. Boundary whitespace from
clipboard copies is trimmed, but internal whitespace/invalid characters are rejected.

Only HTTPS is accepted, except HTTP on loopback for local tests. Embedded URL credentials,
queries, fragments and redirects are rejected. The transport uses the configured process/OS
proxy. A successful Codex connection does not prove the separate API route is reachable.
Connection timeout is 30 seconds and the request deadline is ten minutes. A timeout does not
prove the upstream generation was cancelled or unbilled; no automatic retry is performed.

## Protocols and bounds

Protocol references checked for this extraction: [Gemini image generation/editing](https://ai.google.dev/gemini-api/docs/image-generation),
[Seedream image generation](https://docs.volcengine.com/docs/ark/image-generation-api?lang=zh),
and [BFL FLUX.2 image editing](https://docs.bfl.ai/flux_2/flux2_image_editing).
Documentation checks do not prove access or completion for any particular account.

- Gemini supports Interactions and generateContent inline image responses. Thinking images
  are excluded, and the service must return one completed PNG/JPEG rather than text alone.
- Seedream sends a reference PNG, instruction, same-aspect output size near 4 MP and
  `response_format=b64_json`. Synchronous requests omit `stream`, preserving compatibility
  with the configured flash/pro models. Model support remains the user's responsibility.
- FLUX uses BFL's submit/poll/download workflow. A generation is submitted once; polling is
  for that same task. Keys stay on permitted API hosts. Result download carries no key.
  Official BFL result domains are allowlisted; custom services' result URLs stay same-origin.

Input is the existing visible, colour-managed PNG without source document metadata or hidden
layers, capped at 30 MiB/16 MP. Output is one PNG/JPEG under 64 MiB/16 MP, fitted back to the
canvas only when the aspect ratio matches. Inline JSON responses are capped at 90 MiB;
FLUX metadata/downloads have separate bounds. Failed/cancelled requests do not add layers.
The wasm build retains the platform-neutral types but disables native editing/settings.

Seedream error bodies are bounded to 64 KiB/two seconds. Only known codes, fields and safe
classifications are retained; arbitrary messages, prompts, keys, URLs and unknown suffixes
are not echoed. This transport vocabulary includes content-review categories. Localized
review notices and folding the configuration controls are a separate UI follow-up.

## Commands and tests

- `ai.provider.settings`: `get`, `save`, `removeKey`, with an explicit `provider` of
  `gemini`, `seedream` or `flux`. Saving is not journalled or recorded as an action.
- `edit.ai.api {"provider":"seedream","prompt":"Make the circle blue"}`: explicit edit.

Tests use injected settings stores and loopback HTTP fixtures. They cover protocol payloads,
single submission, cancellation/deadlines, FLUX host/key restrictions, safe error handling,
configuration reload, endpoint-bound secrets, save feedback, all shipped UI languages,
new-layer import and undo/redo. The credential-store integration test is ignored by default.
Automated verification does not consume image-generation credit or call a real account.

Earlier manual testing of the complete fork confirmed Seedream generation. A named-character
request was rejected by content review; its precise policy reason was not established. A
later timeout stopped occurring when the user disabled their VPN. Neither observation proves
fresh acceptance of this extracted branch, or Gemini/FLUX model permissions and output quality.
Those real-service checks and native-speaker review of translations remain open.

## Extracted-branch verification (2026-11-01)

On Windows/Rust 1.95, the API batch passed 34 provider tests (one explicit vault
integration ignore), 21 engine AI tests, four shared UI AI tests, eight API form tests,
eight Codex form tests and 62 localization tests. Strict all-target Clippy passed for
`photocraft-gen`. Formatting, native desktop compilation, layer checks (32 crates),
wasm checks (25 crates plus the HEIF check) and generated menu parity (631/631) passed.
Whole-stack strict Clippy remains blocked by unchanged upstream IO lints; no lint
suppression is introduced here. Full-stack regression is also recorded with the UI follow-up.

These [Seedream Chinese](api-images/seedream-zh-hans-True.png),
[FLUX German](api-images/flux-de-True.png) and
[Gemini English](api-images/gemini-en-True.png) offscreen captures were inspected at
900×760 in light/dark themes. They use a white synthetic canvas, dummy session key
and unreachable loopback endpoint; no generation is confirmed or billed.
