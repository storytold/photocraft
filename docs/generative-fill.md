# Generative Fill and Generative Remove

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-11-01 · **Change:** minor (plaintext-HTTP note)

Right-click a selection with a selection tool: **Generative Fill…** opens a dialog with a prompt and
a model;
**Generative Remove** takes the selection out in one click and lets the model continue the
surroundings. Both are engine commands (`edit.generativeFill`, `edit.generativeRemove`;
`edit.generativeFill.models` lists the server's models), so agents, the CLI and the control
channel can drive them. Code: `crates/engine/src/cmd/generative_fill_cmds.rs`.

## The model runs on your own server

PhotoCraft contains no model. It sends the selection plus surrounding context and a mask to an
inpainting server you host, and writes the result into a new layer (or the active layer) as one
undo step. Two backends:

| Backend | Server | Notes |
|---|---|---|
| `comfyui` (default) | ComfyUI HTTP API (default port 8188) | Uploads the image and mask, queues an inpainting workflow, polls the queue, downloads the result. **FLUX.1 Fill** (GGUF or safetensors, in `models/unet`) gets its own workflow; any other model uses an SD/SDXL checkpoint workflow, which needs a real inpainting checkpoint for good results. |
| `a1111` | AUTOMATIC1111, Forge, SD.Next (`/sdapi/v1/img2img`, default port 7860) | Start the server with `--api`. |

**Nothing leaves your network.** The host must be plain `http://` on this machine or a private
network (loopback, RFC 1918, link-local, unique-local IPv6, Tailscale's 100.64.0.0/10); public
addresses and `https://` are refused. The ComfyUI server must be reachable from the machine
running PhotoCraft (start it with `--listen`).

**The connection is not encrypted.** Plain HTTP carries the selection's pixels, the mask and your
prompt in the clear. That is fine to a server on this computer; to another machine, use it only on a
network you trust (not shared or guest Wi-Fi), or reach the server through a tunnel you set up
yourself (an SSH port forward to `127.0.0.1`, or Tailscale, which encrypts its own traffic).
`https://` is refused on purpose: a home server has no certificate to check.

For FLUX.1 Fill, ComfyUI needs the `ComfyUI-GGUF` node (for GGUF files), `clip_l`, `t5xxl` and the
Flux VAE. A missing file is named in the error. The FLUX.1 \[dev] weights are under Black Forest
Labs' non-commercial license.

## Set it up first (opt-in)

Both commands stay greyed in the selection menu, marked "(not configured)", until a server is set
up: Edit › Preferences › Integrations, **Generative Fill host** (or `PHOTOCRAFT_GENFILL_URL`). A
server on the same computer needs its address too, e.g. `127.0.0.1:8188`. A command given an
explicit `host` parameter (an agent, the CLI) runs without it.

The dialog shows the prompt, model, edge softness and colour match, and output. Server override,
negative prompt, steps, guidance, strength, resolution and seed sit in a collapsed **Advanced**
section.

## Settings

Edit › Preferences › Integrations stores the backend, host and model (the model is a menu of what
the server has, with Refresh). A command parameter overrides the environment
(`PHOTOCRAFT_GENFILL_BACKEND`, `PHOTOCRAFT_GENFILL_URL`), which overrides the saved preference.
Parameters of the fill: `prompt`, `negativePrompt` (SD only), `steps`, `guidance` (0 = automatic:
7 for SD, 30 for Flux), `strength`, `resolution`, `seed`, `feather` (grow and soften the edge) and
`colorMatch` (pull the colour toward the surroundings), `output` (`new` or `current`).

## Not done

- The fill dialog's field labels are not translated (the menu items, messages and preference
  labels are).
- No live preview: a fill is one request to a server and can take minutes the first time a model
  loads.
- Tested end to end against a ComfyUI server with FLUX.1 Fill Q4_K_S, and in unit tests against
  fake ComfyUI and AUTOMATIC1111 servers; the A1111 backend has not been run against a real
  AUTOMATIC1111, Forge or SD.Next install.
