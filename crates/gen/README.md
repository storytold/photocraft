# photocraft-gen

L4 provider interface for optional native image editing. No UI dependency, bundled runtime
or automatic provider fallback. The module compiles on wasm; the native Codex implementation
is excluded there.

- `ImageProvider::edit(EditRequest, cancelled)` returns an `EditResult` containing a saved image
  path. Providers own authentication/transport; the engine owns document import and history.
- `EditRequest` contains the private workspace, flattened PNG path and explicit instruction.
- `codex::CodexProvider` discovers a complete installed runtime, requires ChatGPT authentication,
  and waits for a completed image-generation event and successful turn.
- `codex::AccountService` supplies cancellable account checks/browser login on a worker.
- `api::ApiProvider` implements explicitly configured Gemini, Seedream and BFL FLUX requests.
- `api_settings::Settings` holds public profiles and endpoint-bound keys (session or OS vault).
- `api_error` provides a bounded, allowlisted diagnostic vocabulary for transports/frontends.

Transport failures return `Error`; cancellation and deadlines are explicit. Codex owns its
credential persistence and never falls back to API-key billing. API providers use only an
explicitly selected configuration; PhotoCraft does not manage OAuth tokens. See the
[Codex guide](../../docs/ai-codex.md) and [API guide](../../docs/ai-api.md).

`cargo test -p photocraft-gen` uses only synthetic fixtures. The examples are explicit manual
operations: `codex_edit` submits a real image request, while `codex_account` checks or signs in
through the installed runtime. They are not run by the automated test suite.
