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

Transport failures return `Error`; cancellation and deadlines are explicit. No API keys or OAuth
tokens are handled by the provider. Codex owns its credential persistence. Capabilities depend
on the installed runtime and account; see [the integration guide](../../docs/ai-codex.md).

`cargo test -p photocraft-gen` uses only synthetic fixtures. The examples are explicit manual
operations: `codex_edit` submits a real image request, while `codex_account` checks or signs in
through the installed runtime. They are not run by the automated test suite.
