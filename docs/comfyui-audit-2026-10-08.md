# ComfyUI capability audit for PhotoCraft

Historical audit: 2026-10-08, against ComfyUI 0.38.0 at commit
`5c460d8172fe30761ff67c0df3d5643bb74e0d70`, using synthetic images and owned
request IDs. Personal server addresses, hostnames and deployment details are
omitted from the published report. No application code, package installation or
model download was part of that initial audit.

## Available capabilities

The tested server had an RTX 3060 with 12 GiB VRAM and used low-VRAM execution.
Its `/object_info` response listed 1,113 node classes; this is not a count of usable
AI features, because some nodes require additional weights or cloud credentials.

Installed models included Qwen Image 2.1 INT8 ConvRot, Qwen3-VL 8B INT8 ConvRot,
the corresponding BF16 VAE, BiRefNet and MoGe-2. Approximate sizes of the first
three were 6.76, 8.71 and 0.63 GiB. Additional prompt-enhancement encoders existed
but were not used in the initial probe. No usable ControlNet, SD/SDXL inpainting
checkpoint, SAM2 or GroundingDINO workflow was established.

## Executed synthetic tests

Times are single wall-clock observations including loading, transfer and polling,
not performance guarantees. Qwen tests used 512-pixel output, eight sampling steps,
CFG 1, Euler/Simple and seed 42123.

| Test | Result | Time |
| --- | --- | --- |
| Upload → LoadImage → SaveImage → download | RGB pixels identical | 0.36 s |
| BiRefNet foreground mask | 512×512 subject mask; handle opening remained background | 3.24 s |
| Qwen text-to-image | Generated a red mug | 110.86 s |
| Qwen reference edit | Changed the mug to blue while retaining recognizable composition | 62.47 s |
| Latent-mask edit plus final masked composition | 79,488 selected pixels changed; outside pixels identical | 5.02 s |
| Expand right edge by 128 pixels | 640×512 output; original 512×512 region identical | 64.58 s |
| MoGe depth visualization | Visible depth ordering | 6.60 s |
| Cancel an owned queued request | Owned request removed; unrelated running job preserved | — |

Images were reviewed visually and pixel preservation was compared directly.
ComfyUI history reported successful execution and WebSocket messages included
request-specific progress. Active-job interruption was not triggered in the
initial test; server code exposed `interrupt_if_running(prompt_id)` to avoid
interrupting a different request.

## Integration limits

Qwen's image-conditioning encoder accepts reference images, not a native edit
mask. Latent masking influences sampling; final native masked composition is what
guarantees unchanged pixels outside the selection. Soft edges, hair, transparency,
text, complex removal and realistic photos need separate quality fixtures.
Eight steps proved transport/workflow execution, not final image quality.

The node schema allowed multiple references, but only one was tested. The audited
encoder's latent output supplied the correct 64-channel Qwen representation.
Compatibility of unrelated latent generators was not established.

BiRefNet provides a dominant foreground mask, not arbitrary semantic selections
such as a named person, hair, clothing or a clicked object. MoGe's validity mask
is not a semantic sky selection; the saved PNG was a depth visualization, not a
validated metric float-depth export.

The initial audit did not call an OpenAI endpoint or cloud node. Later native and
browser tests used local HTTP prompt peers; these do not establish the user's
actual provider credentials or vision capability.

## Later verified additions

Official Real-ESRGAN x4plus weights and their BSD-3-Clause license were installed
later. The model size is 67,040,989 bytes and SHA-256 is
`4fa0d38905f75ac06eb49a7951b426670021be3018265fd191d2125df9d682f1`.
Direct native 4K generation passed; native 8K exceeded available VRAM. Separate
native-4K-to-8K Upscale passed. See [AI usage](ai-usage.md) for the current workflow
contract and [workflow fixtures](ai-workflows/README.md) for placeholder graphs.
