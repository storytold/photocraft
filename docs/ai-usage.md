# AI image providers

PhotoCraft can send an image request to an OpenAI-compatible **Images API** (OpenAI, or a
self-hosted server that speaks the same protocol) and bring the result back as a new layer.
It is **off by default**: nothing is contacted at start-up, no AI command is enabled and no menu
item runs until you configure a provider and agree to send to it.

## Turn it on

1. **Edit › Preferences › AI Integrations…**
2. Set the **API base URL** (default `https://api.openai.com/v1`).
3. Provide a key, or leave both key fields empty for an unauthenticated self-hosted server:
   - type it into **API key**. It is kept in memory for the current session only and is never
     written to the preferences file, a journal or a log; or
   - name an **environment variable** that holds it. Only the variable name is saved.
4. Tick **Turn on AI and send my prompts and images to this provider** and press Apply.

The dialog states what is sent and to whom. The agreement is tied to the endpoint: changing the
base URL turns AI off until you agree to the new one. A key is only ever sent over HTTPS, or over
HTTP to a loopback, private-network or `.local` address; redirects are not followed.

## What is sent

| Command | Sent to the configured endpoint |
|---|---|
| **Edit › Generate Image…** | the prompt, model, quality and requested size |
| **Edit › Edit Image with AI…**, **Generative Fill…**, **Generative Remove…** | the prompt, a working copy of the image or selection (longest side at most 3840 px, converted to sRGB) and the selection mask |

Your original pixels stay local and are only replaced when you apply a result.

## Results

Each command runs as a cancellable background job and ends in a **preview** (Window › AI Results).
Nothing changes in the document until you choose **Apply as New Layer**, which is one undo step.
**Open as New Document** keeps a result whose source document has since changed. Results stay in a
bounded in-memory cache.

## Automation

The AI commands are not available to automation (MCP, CLI, the control channel, action playback)
unless the operator starts PhotoCraft with `PHOTOCRAFT_ALLOW_AUTOMATION_AI_UPLOADS=1`. Automation
cannot change `integrations.*` preferences, so it cannot point the app at another endpoint or
name a different environment variable. See [control protocol](control-protocol.md#ai-commands).

## Limits

- Image sizes must fit the selected model (up to 4K for GPT Image 2/2.5); the Images API accepts
  aspect ratios between 1:3 and 3:1.
- RGB and grayscale documents at 8 and 16 bits are supported; other colour modes are refused.
- Not available in the browser build.
- API usage is billed by your provider.

## For developers

`photocraft-ai` (L4) holds the provider contract (`ImageProvider`), request validation and the
OpenAI-compatible adapter behind an injected HTTP client; `engine::ai_cmds` captures document
snapshots and applies results; `engine::ai_runtime_cmds` registers the commands. Further backends
implement `ImageProvider` and are separate modules; no backend is privileged.
