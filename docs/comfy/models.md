# Model catalogue

Researched 2026-10-08. Model facts move fast; every row carries its source. Licence summaries are
not legal advice: read the licence file on the model card before shipping a build that offers the
model by default.

Target hardware for the numbers below: one NVIDIA RTX 5090 (32 GB VRAM, Blackwell), Windows 10.
ComfyUI offloads text encoders and VAEs between runs, so "fits" means the diffusion transformer
plus working memory at 2K resolution.

Licence classes used in this document:

- **Permissive**: Apache-2.0 / MIT. Commercial use allowed. Safe as a default in an open-source app.
- **Community**: open weights with conditions (revenue caps, moderation duties, naming rules).
  Usable, but the app must tell the user the conditions.
- **Research-only**: non-commercial. Can be offered for personal use only, never as the default
  for a feature the user might use commercially.

## 1. Primary models

### Krea 2 Turbo (text-to-image, style reference)

| | |
|---|---|
| Weights | `krea/Krea-2-Turbo` (gated: accept the licence on Hugging Face); ComfyUI repack `Comfy-Org/Krea-2` |
| Released | 2026-06-22 (v1.0) |
| Architecture | Diffusion Transformer, 12 B parameters (HF metadata says 13 B); trained from scratch |
| Conditioning | Text encoder **Qwen3-VL-4B** (`qwen3vl_4b_fp8_scaled.safetensors`), VAE **`qwen_image_vae.safetensors`** (the Qwen-Image VAE) |
| Inference | 8 steps, CFG 0.0, shift/mu 1.15, 1K–2K output (set 2.0 megapixels for 2K) |
| ComfyUI | Native since **0.26.0 (2026-06-23)**. Official workflows: "Text to Image (Krea-2 Turbo)" and "Image Style Reference (Krea-2 Turbo)" subgraphs; style reference uses `krea2_turbo_int8_convrot.safetensors` + the `krea2_style_reference.safetensors` LoRA; style LoRAs such as `krea2_softwatercolor.safetensors` |
| Files (ComfyUI) | `diffusion_models/krea2_turbo_fp8_scaled.safetensors` (FP8, recommended), BF16 / NVFP4 / MXFP8 variants for high-end cards; GGUF community quants need a custom node because stock GGUF loaders do not yet know the `krea2` architecture tag |
| VRAM | FP8 transformer ≈ 12 GB; BF16 ≈ 24.8 GiB (community figures). The 5090 runs BF16 or NVFP4 comfortably |
| Licence | **Community: Krea 2 Community License.** Gated; deployers "are required to implement content filtering measures or equivalent review processes"; Krea claims no copyright over outputs. Third-party summaries report commercial use allowed below **US$1M annual revenue** (one source adds "under 50 seats"), derivatives must keep the licence and start their name with "Krea". Full text: krea.ai/krea-2-licensing. **Verify before relying on the thresholds.** |
| Role here | Default **Generate Image** model and the style-reference path. Best open text-to-image quality at 2K in 8 steps. |
| Style LoRA here | `loras/krea2_style_reference.safetensors` from [ostris/krea2_turbo_style_reference](https://huggingface.co/ostris/krea2_turbo_style_reference) (not gated; 457 111 760 bytes; SHA-256 `f50df5a9…13daa82b8`, equal to the repo's LFS hash), downloaded 2026-10-28 on the owner's go for the native style-reference port. Not shipped (BYOM). |

Also downloaded 2026-10-28 on the owner's go, for the native 2511 port's Angles benchmark: fal's
Multiple Angles LoRA, `loras/qwen-image-edit-2511-multiple-angles-lora.safetensors` from
[fal/Qwen-Image-Edit-2511-Multiple-Angles-LoRA](https://huggingface.co/fal/Qwen-Image-Edit-2511-Multiple-Angles-LoRA)
(not gated; Apache-2.0 per the model card; 295 140 688 bytes; SHA-256 `42426ded…155bb6765`, equal
to the repo's LFS hash). Not shipped (BYOM).

Sources: [Krea 2 open-source page](https://www.krea.ai/krea-2-open-source) · [krea/Krea-2-Turbo model card](https://huggingface.co/krea/Krea-2-Turbo) · [ComfyUI Krea-2 workflow docs](https://docs.comfy.org/tutorials/image/krea/krea-2) · [krea-ai/krea-2 inference code](https://github.com/krea-ai/krea-2) · [Krea 2 local guide (third party)](https://localaimaster.com/blog/krea-2-local-guide) · [Krea 2 open weights write-up](https://www.computeleap.com/blog/krea-2-open-weights-image-model-frontier-2026/).

### Krea 2 Raw (base model for fine-tuning)

Undistilled pretrained checkpoint (`krea/Krea-2-Raw`, gated), 52 steps, "diverse, malleable, built
for fine-tuning and LoRA training"; Krea does not recommend it for plain generation. Same licence
and conditioning as Turbo. Relevant to us only as the base of editing LoRAs (next entry) and for
users who train their own styles.

### "Krea 2 Edit" (community LoRA, not an official Krea release)

**There is no official Krea 2 editing checkpoint.** What the community calls "Krea 2 Edit" is
`conradlocke/krea2-identity-edit`, an unofficial LoRA fine-tune of Krea 2 Raw (v1.2 recommended
file `krea2_identity_edit_v1_2.safetensors`): instruction-based, identity-preserving editing with
head / face / person swap, inpainting, outpainting, virtual try-on and character reference sheets.
It needs its own ComfyUI node pack because it uses dual conditioning (in-context VAE tokens plus an
image-grounded Qwen3-VL encoding) that stock nodes do not provide. Licence: Krea 2 Community
License (inherited). Status for us: **experimental lane**, behind the Apache-licensed editors
below, re-evaluated when Krea ships an official editor.

Sources: [conradlocke/krea2-identity-edit](https://huggingface.co/conradlocke/krea2-identity-edit) · [community demo](https://krea2edit.github.io/) · [Civitai Krea 2 ecosystem](https://civitai.com/ecosystems/krea2).

### Krea 2 AnyPaint and Registered Outpaint (community LoRAs, inpainting and outpainting)

`yijunwang2/krea2-anypaint` (`krea2_anypaint_rank32.safetensors`, 229 MB, rank 32, trained
against Krea 2 Raw, run on Turbo) gives Krea 2 arbitrary-mask inpainting, outpainting and
combined edits in one pass; `krea2_outpaint_rank32.safetensors` ("Registered Outpaint") extends
one axis per pass. Both need a **registered reference**: the masked canvas as one ≤ 384-px
reference whose tokens are pinned onto the canvas grid in the attention layers (ComfyUI: the
AusBoss `Krea 2 Encode` + `Krea 2 Outpaint Model Patch` nodes, MIT; the model card's own
`pipeline.py`), which stock nodes do not do. Recommended: 8 steps, guidance 0, LoRA 1.0, VLM
reference on, KV cache on, a 32-px redraw band. **Licence: Krea 2 Community License** for the
weights (inherited), Apache-2.0 for the pipeline code. Status for us: **installed on the
owner's go 2026-10-26** (`models/loras/krea2_anypaint_rank32.safetensors`, 218 MB, sha256
`3a7d09f6…412f7a`, equal to the repo's `SHA256SUMS`), **BYOM, never a default**; the recipe is
in `plan-native-engine.md` §9 for the native engine, and the app's fill-from-reference on 2511
(Apache-2.0) covers the use case today
(`docs/comfy/research/notes/ausboss-krea2-clothing-workflow.md`, 2026-10-21).

The author's other three Krea 2 adapters, fetched the same day on the owner's go (rank 32,
Krea 2 Community License, each repo's `SHA256SUMS` matched; all three are "functional
adapters" that need their own pipeline code, not a plain LoRA load):

| File | What it does (the card's words, briefly) | sha256 |
|---|---|---|
| `krea2_outpaint_rank32.safetensors` (218 MB) | Registered Outpaint: extends one axis per pass; superseded by AnyPaint for arbitrary masks | `1de7c106…3a7e76` |
| `krea2_anygles_rank32.safetensors` (219 MB) | Anygles: a new camera view of one person (orbit, elevation, distance) from an aligned 3-D human normal and a short instruction | `248a2a65…631ce9` |
| `krea2_reid_rank32.safetensors` (218 MB) | ReID Reference: a character reference that keeps identity, with reference K/V caching | `a80349fa…d967944` |

Sources: [yijunwang2/krea2-anypaint](https://huggingface.co/yijunwang2/krea2-anypaint) · [ausboss/ComfyUI-AusBoss](https://github.com/ausboss/ComfyUI-AusBoss) (node help under `js/docs/`).

### Removal-trained adapters and other fill models (searched 2026-10-21)

What a "stronger fill model" for the removal tools could be, after the stock set showed
Qwen-Image-Edit-2511, Qwen-Image-2.1 and the whole-crop edit instruction all leaving the same
objects (`benchmarks.md` §5i). None installed yet; the first is the candidate.

| Model | What it is | Licence | Notes |
|---|---|---|---|
| [prithivMLmods/QIE-2511-Object-Remover-v2](https://huggingface.co/prithivMLmods/QIE-2511-Object-Remover-v2) | a LoRA on Qwen-Image-Edit-2511 trained for object removal (January 2026; 225 MB; also the [v1](https://huggingface.co/prithivMLmods/Qwen-Image-Edit-2511-Object-Remover)) | Apache-2.0 (adapter) | the object is painted red in the input and the prompt is "Remove the red highlighted object from the scene"; a model trained to delete what is highlighted, against an edit model asked not to copy what it sees. **Installed 2026-10-22** as `qwen-edit-2511/remove-lightning-8` (the Lightning graph with the adapter loaded after the Lightning LoRA, stacked fine): the first variation of every removal under `auto`, the selection grown 6 px and painted red, the smoothed plain fill the second, the judge picking (`benchmarks.md` §5i). Removes thin and overlapping things the plain fill cannot (a tattoo, a fresco's earring, a figure's umbrella); leaves its red when handed a hole far bigger than the object, which the 6-px growth and `paint_left` keep out. |
| [lrzjason/QwenEdit2509-ObjectRemovalAlpha](https://huggingface.co/lrzjason/QwenEdit2509-ObjectRemovalAlpha) | the same idea on 2509 (585 MB) | Apache-2.0 | the card admits less strength on some removals |
| [FLUX.2 \[klein\] 4B](https://huggingface.co/black-forest-labs/FLUX.2-klein-4B) + [fal's object-remove LoRA](https://huggingface.co/fal/flux-2-klein-4B-object-remove-lora) | a 4 B generation and editing model (January 2026), ~13 GB VRAM, about 2 s at 4 steps; the LoRA on the same red-highlight convention ("Remove the highlighted object from the scene", scale 1.1, the `_comfy_converted` file for ComfyUI) | Apache-2.0 (both); the 9B klein is FLUX non-commercial | a new family for the templates and later the native engine; the strongest at text and logo removal in a [2026 local comparison](https://note.com/ai_0049/n/n8051776106dc?hl=en) |
| FLUX.1 Fill dev | the dedicated inpainting model | FLUX non-commercial | never a default |
| LaMa | a small fast inpainter | Apache-2.0 | fails semantically on large objects (a forum's "defurnishing"); a pre-fill candidate for the native engine |
| OmniEraser, OmniPaint, RORem | academic removal models | unclear / SDXL-based | not candidates |

### Qwen-Image-2.1 (generation + editing, RGBA output)

| | |
|---|---|
| Weights | `Qwen/Qwen-Image-2.1` (BF16 safetensors); ComfyUI repack `Comfy-Org/Qwen-Image-2.1`: `diffusion_models/qwen_image_2.1_int8_convrot.safetensors` (or `_bf16`), `text_encoders/qwen3vl_8b_int8_convrot.safetensors` (or `_bf16`), **its own VAE** `vae/qwen_image_2.1_vae_bf16.safetensors`, optional prompt-enhancer encoders `qwen3.5_9b_qwen_image_2.1_pe_t2i` / `_pe_i2i` |
| Released | 2026-09-20 |
| Architecture | 7 B visual generator, 32 single-stream DiT layers, mixed-granularity attention, prefix KV-cache reuse; text encoder Qwen3-VL 8B |
| Capabilities | One model for text-to-image **and** editing; up to **10 reference images**; local edits from circles, painted annotations or masks; identity preservation; **native RGBA / transparent output** (subject extraction without a matting model); native 2K (e.g. 2048², 2752×1536) |
| Inference | 40 steps in the Diffusers examples; ComfyUI templates ship their own settings |
| ComfyUI | Native since **0.37.0 (2026-09-20)** with official text-to-image, image-edit and background-removal templates (`image_qwen_image_2_1_t2i.json`, `image_qwen_image_2_1_image_edit.json`, `image_qwen_image_2_1_background_removal.json`). Nodes: `TextEncodeQwenImage21` (prompt, negative_prompt, resolution 0 = keep the input size, a grouped `images` input whose **API-format keys are `images.image_1`…`images.image_16`**; outputs positive, negative and an **empty** latent sized to image_1: the references enter the conditioning as latents, so 2.1 regenerates the whole picture in context and latent masking does not inpaint), `QwenImage21Cache` on the model, `CLIPLoader` type `qwen_image`, KSampler euler/simple **25 steps, CFG 1**. PhotoCraft templates: `qwen-2.1/fill` (the selection mask goes in as `images.image_2` with a local-edit instruction wrapped around the prompt; the layer mask confines the visible change) and `qwen-2.1/image`. |
| VRAM | int8 transformer plus an 8 B encoder: comfortable on 32 GB; community GGUF quants exist (`AlperKTS/Qwen-Image-2.1-GGUF`) |
| Licence | **Research-only: Qwen Research License Agreement** (research or evaluation purposes; commercial use needs a separate agreement from Alibaba). This is a change from every earlier Qwen-Image release, which was Apache-2.0. Output ownership is not spelled out in the licence; Alibaba's official account has said images belong to the user. |
| Role here | The model the user asked for ("qwen 2.1 edit"). Offered for **personal / research use**, with the licence shown in the picker. Its RGBA output is the best local **Remove Background** available: shipped as `qwen-2.1/matte` (ComfyUI's official background-removal recipe: the edit graph, the picture as image_1 at its own size, "Remove the background, and output a PNG image"; the 2.1 VAE decodes a real alpha channel, verified live 2026-10-09) and as the `transparent` option of Generate Image (`qwen-2.1/image`'s `promptFormatTransparent`). |

Sources: [Qwen/Qwen-Image-2.1 model card](https://huggingface.co/Qwen/Qwen-Image-2.1) · [QwenLM/Qwen-Image-2.1](https://github.com/QwenLM/Qwen-Image-2.1) · [ComfyUI v0.37.0 notes (wiki)](https://comfyui-wiki.com/en/news/2026-09-20-comfyui-v0-37-0) · [GIGAZINE hands-on](https://gigazine.net/gsc_news/en/20260924-qwen-image-2-1/) · [licence analysis (datanorth)](https://datanorth.ai/news/qwen-releases-qwen-image-2-1) · [licence analysis (latenode)](https://latenode.com/ai-trends/qwen-image-2-1-commercial-licence).

### Qwen-Image-2.1 LoRAs installed here (2026-10-26, the owner's go; BYOM, nothing ships)

Read for the recipes in
[`research/notes/ausboss-catalogue-2026-10.md`](research/notes/ausboss-catalogue-2026-10.md);
every file in ComfyUI's `models/loras`, fetched from Hugging Face, sha256 recorded here.

| File | Author, source | Licence | What it does (the author's held-out numbers) | sha256 |
|---|---|---|---|---|
| `qwen-image-2.1-consistency.safetensors` (152 MB, rank 32) | AusBoss, `ausboss/Qwen-Image-2.1-Consistency-LoRA` | Qwen Research (derivative) | 2.1 edits stay on the original's frame: restyle drift median 24.3 → 1.6 px; pixels changed outside a local edit 12.8 → 7.5 %; strength 1 (0.5 keeps more skin detail); not for pose changes; off for big removals | `4f44ada1…0bd63` |
| `qwen-image-2.1-outpaint-v2.safetensors`, `-outpaint.safetensors` (v1) (152 MB each) | AusBoss, `ausboss/Qwen-Image-2.1-Outpaint-LoRA` | Qwen Research | pad flat `#808080`, no noise mask: the picture stays in place 32 of 32 (plain 2.1 moved 13 of 16); v2 for 1–2 MP extensions, v1 for big zoom-outs; trigger instruction first in the prompt | v2 `3611a5cc…eca28`, v1 `201ab351…7eb177` |
| `qwen-image-2.1-restore.safetensors` (152 MB) | AusBoss, `ausboss/Qwen-Image-2.1-Restore-LoRA` | Qwen Research | the redraw-upscale stays on the picture (0.3 px, face match 0.79 → 0.88) and invents less grit; 0.5 for blurry inputs; with the restore prompt at about 2 MP | `65de4f6c…c08c` |
| `qwen-image-2.1-outfit-swap.safetensors` (152 MB) | AusBoss, `ausboss/Qwen-Image-2.1-Outfit-Swap-Consistency-LoRA` | Qwen Research | two-picture outfit swap keeps face, pose, background (shift 3.6 → 0.1 px, face pixels changed 14.1 → 1.7 %) | `4ddfeac5…ae1f0` |
| `Qwen-Image-2.1-viggle-turbo-v0.3-6step-lora-r128.safetensors` (648 MB, rank 128) | Viggle, `Viggle/Qwen-Image-2.1-viggle-turbo` | Qwen Research | 6-step sampling on the base 2.1 (about half the time, a little softer); not on top of the official Turbo checkpoint | `0c985917…c94b3` |
| `bfs_body_swap_v1.0_qwen_2.1.safetensors` (200 MB), `bfs_head_v1.1_qwen_2.1.safetensors` (248 MB) | Alissonerdx, `Alissonerdx/BFS-Best-Face-Swap` | **MIT** | put a character from a second picture into a scene, whole body or head only (AusBoss's Person Swap workflow runs on them); the author asks for pictures of people who agreed | `7dc0a53a…b664`, `d1d748d5…a88e` |

The workflow JSONs that came with them (the author's recipes, for reading) are in
`C:\Users\5090\Documents\_eval\ausboss-downloads\`, outside the tree.

### Qwen-Image-Layered (image to layers, permissive)

| | |
|---|---|
| Weights | `Qwen/Qwen-Image-Layered`; ComfyUI repack `Comfy-Org/Qwen-Image-Layered_ComfyUI`: `diffusion_models/qwen_image_layered_fp8mixed.safetensors` (19.1 GB) or `_bf16` (38 GB), `vae/qwen_image_layered_vae.safetensors` (0.24 GB, decodes RGBA); text encoder `qwen_2.5_vl_7b_fp8_scaled.safetensors` (shared with 2511) |
| Released | 2025-12 |
| Capabilities | Decomposes a picture into N RGBA layers (background first) that composite back to it; a text-to-layers mode; a control variant (`qwen_image_layered_control`) |
| ComfyUI | Native in **0.39** with official templates `image_qwen_image_layered.json` (Image to Layers, Text to Layers) and `image_qwen_image_layered_control.json`, and the blueprint "Image to Layers (Qwen-Image-Layered)". Nodes: `EmptyQwenImageLayeredLatentImage` (width, height, layers), `ReferenceLatent` on both conditionings, `ModelSamplingAuraFlow` shift 1, KSampler euler/simple **20 steps CFG 2.5**, `LatentCut` (dim t, index 1) + `LatentCutToBatch` to turn the layer slices into a batch, the layered VAE decoding RGBA. The official graph scales the input to **640 px** on its longer side. PhotoCraft template: `qwen-layered/split` (`generate.splitLayers`). |
| VRAM | the fp8 repack behaves like 2511 fp8 on the 32 GB card |
| Licence | **Apache-2.0** (model card and repack both tagged `apache-2.0`, checked 2026-10-09 through the Hugging Face API) |
| Role here | **Split into Layers** (Edit › Split into Layers (Generative)…): a permissive default; the app ships no weights |

Sources: [Qwen/Qwen-Image-Layered](https://huggingface.co/Qwen/Qwen-Image-Layered) · [Comfy-Org/Qwen-Image-Layered_ComfyUI](https://huggingface.co/Comfy-Org/Qwen-Image-Layered_ComfyUI) · ComfyUI 0.39's bundled templates (`comfyui_workflow_templates_json`).

### Qwen-Image-Edit-2511 (instruction editing, permissive)

| | |
|---|---|
| Weights | `Qwen/Qwen-Image-Edit-2511`; ComfyUI files `qwen_image_edit_2511_fp8mixed.safetensors` (24 GB cards) and `qwen_image_edit_2511_bf16.safetensors` |
| Released | 2025-12-23 (the Edit line ships dated snapshots: 2509 on 2025-09-22, then 2511) |
| Capabilities | Single- and multi-image instruction editing, strong text rendering, pose/identity consistency; Lightning LoRAs give 4–8-step edits |
| ComfyUI | Native since December 2025 (`TextEncodeQwenImageEditPlus` family of nodes; official templates on docs.comfy.org). The official template (2026-10) chains UNETLoader → ModelSamplingAuraFlow 3.1 → CFGNorm 1.0 → (Lightning LoRA) → KSampler euler/simple, 40 steps CFG 4 (Comfy's note: 20 steps CFG 4 is fine) or 4 steps CFG 1 with the LoRA, and puts `FluxKontextMultiReferenceLatentMethod index_timestep_zero` on both conditionings |
| Speed tiers | **Lightning LoRAs by lightx2v, Apache-2.0** (checked 2026-10-09): `Qwen-Image-Edit-2511-Lightning-4steps-V1.0-bf16.safetensors` and `…-8steps-V1.0-bf16.safetensors`, 850 MB each, in `ComfyUI/models/loras`. PhotoCraft templates: `qwen-edit-2511/fill` (40 steps), `fill-lightning-8` (the `auto` tier when its LoRA is installed), `fill-lightning-4`, and `fill-guided` (mask as a second reference image: best placement, 1.5–3× slower). The same repository's `qwen_image_edit_2511_fp8_e4m3fn_scaled.safetensors` (19 GB) gave noise with our graph under `--fast fp8_matrix_mult` on 2026-10-09: not used. Numbers in [`benchmarks.md`](benchmarks.md) |
| Camera angles | **Multiple Angles LoRA by fal, Apache-2.0** (model card read 2026-10-12): `qwen-image-edit-2511-multiple-angles-lora.safetensors` in `ComfyUI/models/loras`, trained on 3000+ Gaussian-splatting render pairs, 96 cameras from the prompt `<sks> [azimuth] [elevation] [distance]` (8 directions from "front view" to "front-left quarter view", 4 heights from "low-angle shot" to "high-angle shot", "close-up" / "medium shot" / "wide shot"), strength 0.8–1.0. The repository's own `comfyui-workflow-multiple-angles.json` chains it before the 4-step Lightning LoRA, then ModelSamplingAuraFlow 3.1 and CFGNorm, 4 steps CFG 1, every node stock ComfyUI (checked against our server's `/object_info` 2026-10-12). PhotoCraft template `qwen-edit-2511/angles`, command `generate.angles`. Not run live here: the LoRA is not installed on the dev server and the app ships no weights; its estimate (5.5 s + 3.5 s/MP) is the 4-step tier's, unmeasured |
| Licence | **Permissive: Apache-2.0** (model, Lightning and Multiple Angles LoRAs) |
| Role here | **Default editor** for Generative Fill, Expand, Remove and Harmonize in any build that must stay commercially usable. |

Sources: [fal/Qwen-Image-Edit-2511-Multiple-Angles-LoRA](https://huggingface.co/fal/Qwen-Image-Edit-2511-Multiple-Angles-LoRA) (Apache-2.0, with its ComfyUI workflow) · [Qwen/Qwen-Image-Edit-2511](https://huggingface.co/Qwen/Qwen-Image-Edit-2511) · [ComfyUI blog: Qwen Image Edit 2511](https://blog.comfy.org/p/qwen-image-edit-2511-and-qwen-image) · [ComfyUI docs: Qwen-Image-Edit-2511 workflow](https://docs.comfy.org/tutorials/image/qwen/qwen-image-edit-2511) · [official template JSON](https://github.com/Comfy-Org/workflow_templates/blob/main/templates/image_qwen_image_edit_2511.json) · [lightx2v/Qwen-Image-Edit-2511-Lightning](https://huggingface.co/lightx2v/Qwen-Image-Edit-2511-Lightning) (Apache-2.0) · [local guide (third party)](https://localaimaster.com/blog/qwen-image-edit-local-guide).

Qwen-Image-2.1's few-step option, `Viggle/Qwen-Image-2.1-viggle-turbo` (6 steps, v0.3, 2026-09), is
under the same **Qwen Research License** as the base model and needs the author's custom nodes (a
sigma schedule and an unmerged LoRA loader) rather than a stock `LoraLoaderModelOnly`; it is not
shipped as a template. `QwenImage21Cache` (in our 2.1 templates) is the other 2.1 speed lever.

Note: there is **no** "Qwen-Image-Edit-2512". `Qwen-Image-2512` (2025-12-31, Apache-2.0) is the
text-to-image base of that generation; its editing counterpart is Edit-2511.

### Tiny decoders for live previews (Sharp; downloaded 2026-10-29, the owner's go on the preview plan)

| | |
|---|---|
| Files | `vae_approx/taew2_1.safetensors` (22,642,902 bytes, SHA-256 `04766eac0221b5390b985ae3fdcca652cbb4b1e8b82b28ea7ff89dfad1b1a93f`, f16; git blob `ec1a2be4…` = the author's GitHub `madebyollin/taehv` `safetensors/taew2_1.safetensors`; lightx2v's mirror of the same name is a different encoding with another hash) and `vae_approx/taeqi2_1.safetensors` (30,591,264 bytes, SHA-256 `6578b31cc5e336233a2c842eba61b56425bbcac97c7088705f7c7db93291887e`, equal to the Hugging Face LFS id of `madebyollin/taeqi2_1`) |
| What | TAEHV for the Wan 2.1 latent family (Krea 2, Qwen-Image-Edit-2511, the Layered VAE; a video decoder run on one latent frame) and TAESD's 16× RGBA variant for Qwen-Image-2.1's 64-channel VAE; both take the sampler's normalised latents. Native ports in genai-native `qir::preview` |
| Licence | **MIT** (taehv repository LICENSE; taeqi2_1 model card `license: mit`), checked 2026-10-29 |
| Where here | ComfyUI's `models/vae_approx/` (where ComfyUI keeps its own tiny decoders; ComfyUI looks for other names, so its previews are unchanged), linked into `_eval/native-models/vae_approx` |
| Role | **Preview while generating: Sharp**; without them Sharp shows Fast. Fast needs no file (ComfyUI's `latent_rgb_factors` for `Wan21` and `QwenImage21`, in the code) |

Sources: [madebyollin/taehv](https://github.com/madebyollin/taehv) · [madebyollin/taeqi2_1](https://huggingface.co/madebyollin/taeqi2_1) · ComfyUI 0.43 `comfy/latent_formats.py`, `comfy/taesd/`.

### Z-Image Turbo (fast permissive text-to-image)

6 B distilled model from Alibaba Tongyi Lab, released 2025-11-27, about 8 sampling steps,
**Apache-2.0** (base Z-Image released 2026-01-27, also Apache-2.0). ComfyUI BF16 weights from
Comfy-Org. VRAM: BF16 ≈ 16 GB, FP8 ≈ 8 GB, GGUF ≈ 6 GB. Role: the permissive, fast default when
Krea 2's community licence is not acceptable, and the model for live/low-latency previews.

Sources: [Z-Image Turbo ComfyUI guide (thundercompute)](https://www.thundercompute.com/blog/z-image-turbo-comfyui) · [open-model survey (bentoml)](https://www.bentoml.com/blog/a-guide-to-open-source-image-generation-models).

### MOSS-Image (permissive text-to-image with strong text rendering, native)

OpenMOSS (MOSI.AI), technical report arXiv 2610.23374 (October 2026), **Apache-2.0** for the
weights and the inference code (Hugging Face model cards of `OpenMOSS-Team/MOSS-Image-Base` and
`-SFT`, read 2026-10-29). A ~5 B single-stream DiT (42 blocks, width 3072, shared AdaLN, a
two-block text refiner, SiTU feed-forward), Qwen3.5-9B's language model as the text encoder
(Apache-2.0; its last hidden state), the FLUX.2 small decoder (`flux2-vae_small`). Base and SFT
(instruction-tuned, cleaner text grouping) share the architecture. Official sampling: 50 Euler
steps, CFG 3, shift 5 snapped to the 1000-step training grid, the negative branch with block 22
skipped, empty negative prompt; sides multiples of 16, about 1 to 2 MP recommended. Takes natural
language, tag lists and a JSON layout (bounding boxes for text and objects, experimental).

Files (Comfy-Org/MOSS-Image, Apache-2.0 repack, 2026-10): `diffusion_models/moss_image_{sft,base}_
{int8_convrot,bf16}` (5.4 / 10.2 GB), `text_encoders/qwen3.5_9b_{int8_convrot,bf16,w4a8_gptq}`
(9.8 / 19.3 / 7.3 GB), `vae/flux2-vae_small` (250 MB) and `flux2-vae` (336 MB), and `loras/` with
an edit LoRA and a colour/depth control LoRA (not wired natively yet). Qwen3.5's `tokenizer.json`
comes from `OpenMOSS-Team/MOSS-Image-SFT/tokenizer/` and goes in `text_encoders/qwen35_tokenizer/`.
On this PC (2026-10-29): SFT int8 DiT, the int8 encoder, both VAEs and the tokenizer in
`D:\PhotoCraftModels\moss-image`, junctioned into the ComfyUI `models` folder as `moss-image`
subfolders (C: had 30 GB free). ComfyUI supports it from v0.43.0.

Live previews (2026-10-30): Fast uses ComfyUI's `Flux2` latent→RGB factors on the unpacked 32
channels; Sharp uses madebyollin's **TAEF2** (MIT, https://huggingface.co/madebyollin/taef2,
`taef2.safetensors` 10.7 MB, SHA-256 `701D31C0ECBEF3B609FE236E2E3A65FFB17F3A94237ACDD771E8EA3194DF557A`)
in the Models folder's `vae_approx/`; 35.1 dB against the full FLUX.2 decode on a final latent.

Native (genai-native `qir::moss`, templates `moss-image/image` and `moss-image/image-base`):
against the official pipeline on the same weights, encoder cosine 0.9996, DiT velocity cosine
0.9999, VAE 60.7 dB (bf16 decode 60.0 dB), a 512² 20-step picture 25.3 dB (the same picture).
Speed at 1024², 50 steps, CFG 3, warm, same GPU state: native 11.5–12.0 s of sampling, ComfyUI
0.43 11.1 s (11.6 s in all); the first native port took 35.2 s (`devlog.md`). NVFP4 GEMMs at
MOSS's shapes: ~1.4× the int8 ones with their activation quant, 13 % layer error against int8's
1.2 % (the rebuild thread's bench, 2026-10-29).

## 2. Other models worth knowing

| Model | What | Licence | Notes |
|---|---|---|---|
| FLUX.2 [dev] | 32 B generation + editing (Black Forest Labs, Nov 2025) | FLUX non-commercial | Comfy-Org FP8 builds; [pro]/[flex] are API-only. Licence reportedly mandates safety filtering. |
| FLUX.2 [klein] 4B | small generation model (Jan 2026) | Apache-2.0 | 8–13 GB VRAM (sources disagree). The 9 B klein is non-commercial. |
| FLUX.1 Kontext [dev] | the 2025 instruction editor | FLUX non-commercial | Adobe offers Kontext *pro* as a partner model; dev is the local equivalent. |
| FLUX.1 Krea [dev] | 2025 BFL × Krea aesthetic fine-tune | FLUX non-commercial | superseded by Krea 2 for our purposes. |
| Qwen-Image-2512 | 2025-12-31 text-to-image base | Apache-2.0 | good LoRA-training base; permissive T2I alternative. |
| Qwen-Image-Edit-2509 | the previous editor | Apache-2.0 | keep for compatibility with existing LoRAs. |
| BiRefNet | matting / background removal | MIT | ComfyUI custom nodes; fast; no diffusion needed. |
| RMBG-2.0 (BRIA) | background removal | non-commercial | do not make default. |
| **SAM 3.1** (Meta, 2026-03-27) | open-vocabulary segmentation: text prompts ("red car", `eye:2, window panels:4`, ≤ 32 tokens, comma-separated terms), point/box prompts, video tracking (Object Multiplex) | **SAM License** (custom, see below) | **Native in ComfyUI** (PR #13408 by kijai, a dependency-free re-implementation); templates "SAM3: Image Segmentation" / "SAM3: Video Segmentation" under Utility; one checkpoint `checkpoints/sam3.1_multiplex_fp16.safetensors` from `Comfy-Org/sam3.1` (≈ 1.75 GB); the official `facebook/sam3` weights are gated behind accepting the licence. **This is the model for Select by text, ML Select Subject and object masks that feed Generative Fill.** |
| SAM 2 | promptable segmentation (point/box) | Apache-2.0 | superseded by SAM 3.1 for us; keep as the permissive fallback. |

**SAM License in one paragraph** (read 2026-10-08 from `facebookresearch/sam3/LICENSE`; not legal
advice): a non-exclusive, worldwide, royalty-free licence to use, reproduce, distribute and modify
the "SAM Materials" (code, weights, docs) for research **and** commercial use, with no user or
revenue thresholds. Conditions: pass the licence on with any redistribution, acknowledge SAM in
publications, comply with trade controls; no reverse engineering; **no ITAR, military/warfare,
nuclear, espionage or weapons end uses**; suing Meta over the materials terminates the licence;
Meta may change the terms with immediate effect. Outputs (masks) are not restricted. For an
open-source editor this is workable: the app ships no weights, points the user at the gated
download, and shows the licence summary once. The native engine runs the same checkpoint
(`sam3.1_multiplex_fp16.safetensors`) on candle as it is; the ONNX graphs exported from it
(2026-10-19) and their export script were retired with the ONNX Runtime route on 2026-10-29. Sources: [Meta SAM 3 / 3.1 repo](https://github.com/facebookresearch/sam3) · [SAM License](https://github.com/facebookresearch/sam3/blob/main/LICENSE) · [ComfyUI SAM 3.1 guide (official)](https://docs.comfy.org/tutorials/utility/video-segment-sam3) · [ComfyUI PR #13408](https://github.com/Comfy-Org/ComfyUI/pull/13408) · [SAM 3.1 release note (third party)](https://the-agent-report.com/2026/05/meta-sam-3-1-video-detection-multiplexing-may21/) · [Ultralytics SAM 3 page](https://docs.ultralytics.com/models/sam-3).
| SeedVR2, 4x-UltraSharp, SUPIR | upscalers | **verify** (SeedVR2 is reported Apache-2.0; 4x-UltraSharp CC BY-NC-SA; SUPIR non-commercial) | Generative Upscale lane; licences must be checked per model before listing. |

Sources: [FLUX.2 / Z-Image on AWS (hands-on)](https://builder.aws.com/content/363GAtT4hAB7stB8mbva5H1h1Ab/running-flux-2-and-z-image-diffusion-models-on-aws-a-hands-on-implementation-guide) · [best ComfyUI models 2026](https://iimagined.ai/blog/best-comfyui-models-2026) · [open-model comparison](https://www.pixazo.ai/blog/top-open-source-image-generation-models).

## 3. Feature → model mapping

**2026-10-27, the owner's choice:** with research-only models allowed and the native worker
running Qwen-Image-2.1, `auto` runs 2.1 for Generative Fill (`qwen-2.1/fill`), the removals
(Content-Aware Fill, Remove, Spot Healing: `qwen-2.1/edit` on the crop), Generative Expand
(`qwen-2.1/expand`) and Generative Edit (`qwen-2.1/edit`), and Qwen-Image-Edit-2511 is not
used there (`generate_cmds::native21_auto`; numbers in `benchmarks.md`). The "Default
(permissive build)" column below is still what a build without the opt-in runs; 2511 stays the
template for a fill from a reference, Generative Angles and an image from references, which 2.1
does not take yet.

| Feature (command) | Default (permissive build) | Best quality (user opts in) | Workflow shape |
|---|---|---|---|
| Generative Fill (`generate.fill`) | Qwen-Image-Edit-2511, `auto` = the Lightning 8-step tier when its LoRA is installed, else 40 steps | Qwen-Image-2.1 | composite crop (selection bounds + context margin, sent at ≤ 1 MP) + mask → edit model with inpaint conditioning → resampled back (Lanczos) under the selection's full-resolution mask |
| Content-Aware Fill, Remove tool, Spot Healing Brush, generative (2026-10-21: `edit.contentAwareFill {generative}`, `paint.remove {generative}`, `paint.spotHealing {type: generative}`) | the `auto` fill template (2511 Lightning / base by Quality; 2.1 fill natively under auto) | — | the pixels kept under the noise mask, `REMOVE_PROMPT` ("remove the object in the selected area and fill its place with the surrounding scene continued seamlessly…"), the object under a stroke found by SAM 3.1 (by point, then by the name the Qwen3-VL rewriter gives it: "remove the gold necklace…") and taken whole, the hole grown by 15 % of its side, two seeds judged side by side by the rewriter, a masked layer; the classical fills without a backend |
| Generative Fill from a reference (`generate.fill` + `references[]`, 2026-10-21) | Qwen-Image-Edit-2511 `fill-ref-lightning-8` (`fill-ref` under Quality best): one or two references on image 2/3 | — | the same crop with the selection flattened to grey (`erase`), optionally widened (`grow`); the prompt names the hole and "exactly as the references show it"; the enhancer sees the picture and the reference side by side |
| Generative Expand (`generate.expand`) | Qwen-Image-Edit-2511 | Qwen-Image-2.1 | pad composite to the new canvas, mask = padding (feathered inward) → same as Fill |
| Generate Image (`generate.image`) | Z-Image Turbo | Krea 2 Turbo | prompt (+ optional reference / style LoRA) → new layer or new document |
| Instruction edit (`generate.edit`) | **shipped 2026-10-09**: Qwen-Image-Edit-2511, `auto` = the Lightning 8-step edit tier when its LoRA is installed, else 40 steps (`qwen-edit-2511/edit-lightning-8`, `edit`) | Qwen-Image-2.1 (multi-ref), later | the composite + instruction → the official 2511 edit graph with no noise mask → a new layer, masked to the selection when there is one |
| Select by text (`select.byText`) | SAM 3.1 | SAM 3.1 | prompt → instance masks → a selection (union, or one instance by index); the selection then drives Fill, Remove, masks, anything |
| Select Subject / Sky / Hair (ML) (`select.subjectML`) | SAM 3.1 ("person", "sky"…) or BiRefNet for soft hair mattes | SAM 3.1 + BiRefNet refine | replaces the classical `select.subject` path when the server is up |
| Remove Background (`generate.removeBackground`) | **SAM 3.1** (`sam3.1/segment`, SAM License, no opt-in): the prompt or "the main subject" → a hard-edged mask | **Qwen-Image-2.1's RGBA output** (`qwen-2.1/matte`, research licence, opt-in): a soft matte; `auto` takes it when allowed and installed | layer (or composite) → the model's alpha channel (2.1) or the union of the detector's instance masks (SAM), resampled → layer mask or selection (never destroys pixels); the classical Quick Action `layer.removeBackground` needs no server |
| Transparent Generate Image (`generate.image` + `transparent`) | — (Krea 2 and Z-Image cannot) | Qwen-Image-2.1 RGBA (`promptFormatTransparent`) | prompt + "isolated on a transparent background, output a PNG image" → a layer whose pixels carry the model's alpha |
| Split into Layers (`generate.splitLayers`) | Qwen-Image-Layered (`qwen-layered/split`, Apache-2.0) | — | the composite at ≤ 640 px → N RGBA layers, background first, each a new layer above the active one, resampled to the canvas |
| Prompt enhancer (`generate.enhancePrompt`; `enhance` on fill, edit, image) | **shipped 2026-10-09**: Qwen3-VL-4B (`qwen3vl_4b_fp8_scaled`, the Krea 2 text encoder, Apache-2.0) run as a vision-language model through ComfyUI's `TextGenerate` (`enhance/edit`, `enhance/image`) | the 8B Qwen3-VL would read pictures better; the 2511 encoder (Qwen2.5-VL) cannot generate in ComfyUI 0.39 | the composite (+ a fill's selection place) and the typed prompt → one sentence that leads with the verb, names what is in the picture, says what the result looks like and what stays; an image idea → a 60–120-word paragraph; 1–2 s on the 5090 |
| Harmonize (`generate.edit` preset) | Qwen-Image-Edit-2511 | Qwen-Image-2.1 | pasted layer + composite context + "match lighting and colour" → new layer |
| Generative Upscale (`generate.upscale`) | ESRGAN-class (licence-checked) | SeedVR2 | layer → upscale-model node → new layer / Image Size |
| Generate Similar | same model as the source layer | — | stored prompt + new seed |

## 4. VRAM budget on the RTX 5090 (32 GB)

| Setup | Approximate resident VRAM | Fits? |
|---|---|---|
| Krea 2 Turbo FP8 + Qwen3-VL-4B FP8 + VAE | ≈ 12 + 4–5 + <1 GB | yes, with headroom for 2K |
| Krea 2 Turbo BF16 | ≈ 25 GB transformer | yes, encoder offloaded to RAM between runs |
| Qwen-Image-2.1 int8 + Qwen3-VL-8B FP8 + VAE | ≈ 8–9 + 8–9 + <1 GB | yes |
| Qwen-Image-Edit-2511 fp8mixed | sized for 24 GB cards | yes |
| Z-Image Turbo BF16 | ≈ 16 GB | yes |

These are community figures (see sources above), not measurements on this machine. Phase 1 of the
roadmap includes measuring them with `GET /system_stats` before and after a run and recording the
numbers in [`devlog.md`](devlog.md).

Measured on this machine (ComfyUI 0.39's "staged" sizes from its log, `/system_stats` free VRAM,
2026-10-09; the card reports 31.8 GB):

| Set, as resident on the server | Staged | Fits? |
|---|---|---|
| Qwen-Image-Edit-2511 fp8mixed + its Qwen2.5-VL-7B fp8 encoder + VAE | 19.6 + 7.9 + 0.2 GB | yes: 16 s Lightning edits, steady |
| … plus the prompt enhancer's Qwen3-VL-4B fp8 (the Krea 2 encoder) | + 5.0 GB ≈ 32.7 GB | not quite: 1.9 GB free, 2511 edits at 19–25 s while everything stays resident; the NVFP4 2511 encoder (5.7 GB file) brings the set back under the card |
| Qwen-Image-2.1 int8 + Qwen3-VL-8B int8 + its VAE + SAM 3.1 fp16 | 6.8 + 8.7 + 0.6 + 1.6 GB file sizes ≈ 18 GB | yes, all together, with room for the enhancer: 2.1 and SAM can run turn by turn without a purge |
| Krea 2 Turbo fp8 + Qwen3-VL-4B fp8 + VAE | 12.2 + 5.0 + 0.2 GB | yes; the enhancer shares this encoder, so it costs nothing extra |
| Krea 2 Turbo **NVFP4** + Qwen3-VL-4B fp8 + VAE | 7.2 + 5.0 + 0.2 GB | yes: 18.5 GB free after a run against 13.4 GB with fp8, and 18 % faster (`benchmarks.md` §5f) |

### NVFP4 (Blackwell) alternatives

Comfy-Org's official NVFP4 repacks, checked 2026-10-09 (the app ships none of them; a user
downloads them into the same folders as the defaults):

| File | Folder | Size | Stands in for | Source |
|---|---|---|---|---|
| `qwen_2.5_vl_7b_nvfp4.safetensors` | `text_encoders` | 5.7 GB | `qwen_2.5_vl_7b_fp8_scaled` (8.7 GB) in every 2511 template | Comfy-Org `Qwen-Image_ComfyUI` (Hugging Face), `split_files/text_encoders` |
| `krea2_turbo_nvfp4.safetensors` | `diffusion_models` | 7.2 GB | `krea2_turbo_fp8_scaled` (12.2 GB) in `krea2-turbo/image` | Comfy-Org `Krea-2` (Hugging Face), `split_files/diffusion_models` |

Not available officially: an NVFP4 2511 diffusion model (a community repack exists; not wired)
and NVFP4 for Qwen-Image-2.1 (its official encoders are int8 and w4a8). The templates name the
alternative in the slot's `nvfp4` field; Preferences › AI Integrations › Model precision decides
(`auto` = Blackwell GPUs only, by the device name ComfyUI reports). Same output, less memory;
only Krea 2 gets faster.

### Reference images (ShotoPhopEX's reference deck, checked 2026-10-11)

Which templates take reference images (`meta.references`; `generate.models` reports them), each
verified against the development PC's ComfyUI 0.39 `/object_info` on 2026-10-11:

| Template | Licence | References | How they enter the graph | Run live |
|---|---|---|---|---|
| `qwen-edit-2511/edit-lightning-8`, `qwen-edit-2511/edit` | Apache-2.0 | 0–2 (image 2, image 3) | `image2` / `image3` of both `TextEncodeQwenImageEditPlus` encoders, straight from `LoadImage`, as ComfyUI's official 2511 graph wires its extra images (optional inputs; an unused slot is pruned) | yes: a two-reference edit (a fox and a compass into the lighthouse picture), 39.5 s with the model load |
| `qwen-edit-2511/image-ref` (new) | Apache-2.0 | 1–3 | the references on `image1`…`image3`, an `EmptySD3LatentImage` of the asked size: the official 2511 nodes in the pattern of the official Krea 2 style-reference graph | yes: 12.1 s for a 3:2 image at 1K from one layer |
| `krea2-turbo/image-style` (new) | Krea 2 Community | 1–2 | the official "Image Style Reference (Krea-2 Turbo)" subgraph: Ostris's `krea2_style_reference.safetensors` LoRA (Hugging Face `ostris/krea2_turbo_style_reference`, licence metadata `krea-2-community-license`, trained for one or two references) on the Turbo model, references through `TextEncodeQwenImageEditPlus` | no: the LoRA is not installed on the development PC (BYOB) |
| `qwen-2.1/edit` (new) | Qwen Research (opt-in) | 0–9 (image 2…image 10) | `TextEncodeQwenImage21`'s grouped `images.image_N`, the official 2.1 edit template's limit of ten images | not yet |

The engine's `references` param takes `{"layer": id}`, `{"path": "…"}` (refused to automation)
or `{"job": id, "index": i}` (a stored generation); each is scaled to at most 1 MP before upload.
`generate.image` with references and no template named switches from a default that takes none
(Krea 2) to `qwen-edit-2511/image-ref`.

## 5. What the picker must show

For each installed model the UI shows: name, task (generate / edit / matte / upscale), licence
class with a one-line summary and a link, resolution range, steps, and whether it is loaded on the
server right now (`/system_stats`, `/object_info`). Research-only models are never pre-selected.
Community-licence models show their conditions once per session. The catalogue is data
(`docs/comfy/models.toml` in Phase 4), not code.

## Sources (feature list to match)

- Adobe help: [Generative Fill on desktop](https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html) (Adobe models Firefly Fill & Expand, Firefly Image 5, Firefly Image 1; partner models FLUX.2 pro, FLUX Kontext pro, Gemini 3.1 / Nano Banana 2, Gemini 3 / Nano Banana Pro).
- [Adobe Design: partner models in Photoshop](https://medium.com/@Adobe_Design/behind-the-design-partner-models-in-adobe-photoshop-91b54e1aaca9).
- [What's new in Photoshop 2026 (PhotoshopCAFE)](https://photoshopcafe.com/whats-new-in-photoshop-2026-full-release-overview/) · [Generative Upscale 2026](https://www.photoshopessentials.com/photo-editing/how-to-use-generative-upscale-in-photoshop-2026/) · [Harmonize / Generative Upscale announcement](https://alternativeto.net/news/2025/7/photoshop-expands-ai-tools-with-harmonize-generative-upscale-and-project-collaboration) · [AI features field report](https://weandthecolor.com/ai-features-in-adobe-photoshop-that-actually-changed-how-i-work-a-designers-field-report/208072).
