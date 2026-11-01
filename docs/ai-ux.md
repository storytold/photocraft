# AI edit notices and folding API settings

This UI follow-up builds on [Codex editing](ai-codex.md) and
[explicit API providers](ai-api.md). It adds no provider, account flow, automatic
retry or paid-service fallback.

## Configuration disclosure

The API settings header contains address/protocol, key input, storage preference
and save/remove buttons. A fresh dialog defaults closed for a configured provider
and open when a key has not been configured. Model, prompt and readiness/error
feedback remain visible. Folding does not save or discard an unsaved key; an
explicit successful save is still required before editing.

Each provider's non-secret disclosure state is a dialog field
`__aiApiSettingsOpen:gemini`, `__aiApiSettingsOpen:seedream` or
`__aiApiSettingsOpen:flux`. It is readable/drivable through the existing dialog
control interface. Keys stay outside serialized/inspected UI state.

## Content-review notices

Known safe provider codes explain whether the instruction, reference image or
output failed review, or whether review itself was unavailable. Codes with explicit
policy, privacy or impersonation categories get corresponding advice; an unspecified
rejection never guesses which stage or exact policy was involved. In particular,
a named character's rejection does not establish a copyright-specific cause.

Both foreground AI failures and background job errors use the same localized notice.
Arbitrary response messages, prompt text, key values and unknown code suffixes are
not included. Other failures retain the existing error handling. Failed edits do
not add a layer or change existing document history. A retry requires user action.

The classification currently uses the bounded/allowlisted diagnostic vocabulary
from the API batch, including known Seedream codes. It does not claim to recognize
every Gemini or FLUX rejection payload. Notices store English source templates and
safe code arguments so an already-visible notice follows changes among all 27
shipped languages. New translations still need native-speaker review.

## Verification

On Windows/Rust 1.95 (2026-11-01), the complete extracted stack passed **7,712 tests**
across the provider, engine and UI crates, with 46 default ignores. Documentation
compilation passed for all three crates (no runnable doctests). The explicitly
ignored full-registry `panic_hunt` was then run separately and passed on adversarial
parameters. This includes 2,529 engine unit tests, 4,615 UI unit tests, 34 provider
tests, 10 API form tests, eight Codex form tests and 146 graphical integration tests.

Formatting, layer checks (32 crates), wasm checks (25 crates plus HEIF) and generated
menu parity (631/631) passed. The API batch's native desktop check and provider
all-target strict Clippy passed. Whole-stack strict Clippy is blocked by unchanged
upstream IO lints (`drop_non_drop`, `nonminimal_bool`); a `--no-deps` diagnostic also
reports the existing stroke/menu lints. No allowances or unrelated fixes are added.

The following 900×760 offscreen captures were visually inspected:

- [Chinese configured service, folded](ai-ux-images/seedream-zh-hans-False.png)
- [Chinese configured service, expanded](ai-ux-images/seedream-zh-hans-True.png)
- [German configured service, folded](ai-ux-images/gemini-de-False.png)
- [Chinese instruction-policy review response](ai-ux-images/review-instruction-zh.png)
- [English unspecified-stage rejection](ai-ux-images/review-unspecified-en.png)

Captures use a synthetic white canvas, dummy session keys and loopback-only fixtures.
Each review fixture received exactly one request and returned HTTP 400 with private
marker text that is absent from the displayed notice. The layer panel retains the
single original layer. No paid image generation, real account login or credential-store
write was performed. Real-service acceptance and translation review remain pending.
