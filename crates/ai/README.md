# photocraft-ai

Provider-neutral AI contracts and bounded adapters for PhotoCraft. Native and
WebAssembly engine commands use these contracts without coupling providers to
the document model or UI.

- Validated requests, capabilities and fixed ComfyUI workflow packages.
- OpenAI-compatible Responses prompt preparation and Images generation/editing.
- Explicit vision interpretation contracts, separate from text-only prompts.
- Native HTTP clients, bounded image codecs and owned-job cancellation.
- Same-origin browser gateway protocol and async configuration persistence.
- Redacted credential types and private native configuration writes.
- Pinned metadata for the model files required by shipped workflows.

No provider call implicitly mutates a document. The engine prepares native layers
and the user applies a result explicitly. Direct and environment credentials are
runtime configuration; keys are never stored in native image documents.

See [AI usage](../../docs/ai-usage.md) and
[implementation plan](../../docs/ai-integration-plan.md).
