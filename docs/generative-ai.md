# Generative AI

PhotoCraft can fill a selection from a prompt (Edit › Generative Fill…) through an image service
the user picks and pays for with their own API key. It is off by default: until a provider is set
up, the command is hidden, nothing is contacted, and no key is read. This page follows the shape
the maintainers asked for in issue #41.

## Using it

1. Edit › Preferences › AI Integrations… › Generative AI Settings… (or the ▾ next to the selection
   tools' Generative Fill… button). Pick a provider and model, paste an API key, and read what is
   sent before pressing OK.
2. Make a selection, then Edit › Generative Fill… (or the options bar button). Describe what to
   put there, or leave the prompt empty to continue the surrounding image through it (anything
   that runs into the selection carries on and partly covered objects are completed).
3. Three variations come back, as in Photoshop. They land in a new group named after the prompt
   and masked by the selection, one layer per variation (Variation 1, 2, 3), with the first
   showing. Nothing under it changes; one undo removes it. The requests run side by side as one
   background job that can be cancelled; if some fail and others succeed, the successes are kept.
4. With the group (or one of its variations) active, Properties › Variations shows a thumbnail
   per variation: click one to show it alone (`layer.generative.selectVariation`, one undo step).
   Generate More sends the same image, mask and prompt again and adds three more
   (`layer.generative.more`).

Photoshop keeps its variations inside one generative layer. PhotoCraft uses a group of ordinary
layers instead, so a PSD round trip needs no format changes and other apps see plain layers. What
each group was generated from is kept in the session only: after reopening a file the group is an
ordinary group (the variations are all still there; show one by hand), and Generate More is off.

Providers today: **Google Gemini** (the Nano Banana image models, default
`gemini-nano-banana-2.1`). Keys come from [Google AI Studio](https://aistudio.google.com/apikey).

## What is sent

Only when Generative Fill or Generate More runs, and only to the configured provider, once per
variation (so three requests by default, each billed to the user's key):

- the selection's bounds plus a margin of context (half the selection's longer side, 64 to
  1024 px), grown toward the provider's output shape and kept inside the canvas, rendered
  from all visible layers in sRGB and scaled to at most 2048 px on the long side, with the
  selection itself cleared to flat grey. The provider never sees what is being replaced, as
  Photoshop's fill doesn't, so it can't paint it back: an empty prompt removes what was selected
  and continues the surroundings, including anything that crosses the selection's edge;
- a black-and-white PNG mask of the selection at the same size (white = fill);
- the prompt.

Gemini takes no mask parameter, so the mask is a second image and the instruction says what it
means. PhotoCraft keeps only the masked pixels of the result, so anything the model changes
outside the selection is discarded. The model redraws the whole image and its colours drift a
little, differently each time, which would leave a seam at the selection's edge; the area outside
the selection should come back unchanged, so comparing it with the original measures the drift.
A per-channel gain and offset fitted there, then the remaining difference near the edge spread
smoothly inward, bring the fill back in line (`match_surroundings`). A result that doesn't
resemble the original around the selection (over 25% mean difference) is left as it came. Gemini returns JPEG (its only output format) and its results carry Google's SynthID watermark.

## Keys and privacy

- Keys live in the OS credential store: the macOS Keychain, Windows Credential Manager. On Linux
  and the BSDs a key stays in process memory and is asked for again after a restart; it is never
  written to a file.
- Preferences hold only the provider id and model (`dialogs["ui.generativeAiSettings"]`).
- The key field is not a dialog field, so `ui.inspect` and the control channel never see it. An
  `ApiKey` never prints (`Debug` is redacted), errors never carry a response body, and a
  service's own error text is passed on only after any copy of the key is scrubbed from it.
- The key goes in a request header (`x-goog-api-key`), never in a URL. The transport is HTTPS
  only and follows no redirects.

## Automation

`edit.generativeFill` is a normal engine command (one undo step, a cancellable job, `prompt` and
`variations` (1 to 4, default 3) params) so scripts and recorded actions can run it locally, but
automation (the control channel and MCP) may not: `authorize_engine_command` refuses it and
`layer.generative.more`, because they upload document pixels. `layer.generative.selectVariation`
sends nothing and is allowed. An
explicit opt-in for automation can come later.

## Code

| Where | What |
|---|---|
| `crates/genai` | `ImageProvider` (generate, edit with mask, variations; capabilities; output shape), `HttpTransport` (the network seam), `ApiKey`, and the Gemini backend. No network code: it builds for wasm and its tests use a fake transport. |
| `crates/engine/src/genai_cmds.rs` | `edit.generativeFill`, `layer.generative.selectVariation`, `layer.generative.more`: context area, pixel interchange (after #730's approach), the job, the masked group of variations. `Session::set_image_provider` installs a provider. |
| `crates/ui-egui/src/genai_ui.rs` | Generative AI Settings, the prompt dialog, the options bar button, Properties › Variations, `restore` at launch. |
| `apps/photocraft/src/genai.rs` | The desktop `GenAiServices`: credential store (`keyring`) and HTTPS transport (`ureq` + rustls). |

### Adding a provider

Implement `ImageProvider` in `crates/genai` over an `HttpTransport` (any local endpoint, ComfyUI,
OpenAI, a generic HTTP contract), add it to `build` in `apps/photocraft/src/genai.rs` and to
`providers()` in `genai_ui.rs`, and test it against a fake transport the way `gemini.rs` does.
No backend is privileged; the engine only sees the trait.
