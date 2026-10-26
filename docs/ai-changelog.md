# AI integration changelog

## Unreleased

- Add native ComfyUI and OpenAI-compatible image workflows with explicit previews,
  editable result layers/masks, Undo/Redo and stale-result recovery.
- Support direct requested-size generation, 4K/8K/custom output presets and a
  separate Real-ESRGAN AI Upscale workflow from original pixels.
- Prepare ComfyUI prompts through a structured OpenAI-compatible Responses call;
  preserve opaque self-hosted model IDs without interpreting them as paths.
- Add AI menus, context actions and endpoint/password configuration fields.
- Keep native API keys in memory for the session (only environment-variable names
  persist), save web keys in the private gateway file, and redact public
  preference/control responses and browser storage. Saving gateway configuration
  obeys the operator allowlists; automation cannot set `integrations.*`.
- Run the same AI commands in the Rust/WASM browser application through an
  authenticated Rust gateway deployed with Docker.
- Support Docker API-key, language and prompt-model defaults with explicit
  per-session language and per-workflow model overrides. English is the default.
- Add image interpretation with an editable recreation prompt and explicit handoff
  to the generation form.
- Add ComfyUI dependency checks and a scoped Rust model downloader with size/hash
  verification, cancellation, license review and no-overwrite installation.
- Integrate the fork’s PhotoCraft 0.8.2 base and omit private instance data
  from published source, examples and documentation.

Unsigned native installer validation includes Debian GUI/private-key persistence
and Wine installation/load/file roundtrip/removal. Native Windows GPU validation
and actual user-provider authentication/vision tests remain open.
