# photocraft-ai

The image-provider contract for PhotoCraft's AI commands, kept apart from the document model and
the UI.

- `ImageProvider`: capabilities plus `generate(request, control)`. The HTTP transport is injected
  (`native::Http`), so tests run against a local peer and never touch the network.
- Validated requests (`AiRequest`): operation (generate, edit, fill, remove), bounded sizes,
  prompt and input byte limits, mask rules.
- `native::openai::OpenAiImages`: an OpenAI-compatible Images adapter (generations and edits with
  a mask). Other backends are separate `ImageProvider` implementations.
- `secret::Secret`: a redacted credential type; keys live in memory for a session.
- `config`: endpoint and key-reference validation, and `cleartext_credentials_allowed`, which
  keeps a key off plain HTTP except to loopback, private-network and `.local` hosts.

Nothing here contacts a provider on its own or mutates a document. The engine prepares result
layers and the user applies one explicitly. See [AI image providers](../../docs/ai-usage.md).
