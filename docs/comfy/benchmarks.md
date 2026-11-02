# Generative benchmarks (comfy-photocraft)

Measured 2026-10-09 with `bench/bench-fill.ps1` (local) on the development PC:
RTX 5090 (32 GB, driver 617.42), ComfyUI 0.39.0 portable, Python 3.13.14, torch 2.14.0+cu130.
Same image (the 1024² lighthouse, `q21-image.png`), a 384×320 selection with a 25 % margin (a
576×512 request), the same prompt. `ms` is `generate.fill`'s own wall time; the breakdown comes
from the command's `timings`: `encode` (PNG), `upload`, `queue` (queued → the server starts),
`sampling` (the server's execution, model loads included), `download` (fetch + decode). All
numbers are milliseconds. The raw CSVs and the output PNGs sit in
`C:\Users\5090\ComfyUI\photocraft-tests\bench\` (not in the repository).

## 0. Native vs ComfyUI, every feature at 1 MP (2026-10-28)

The owner's goal of 2026-10-28: every native feature at least as fast as its ComfyUI route at
the same settings and size, parity kept. `bench/scripts/speed-table.ps1` (the owner's local script, not in the repository):
an 864×1152 portrait (the owner's usual 1 MP 3:4 size), the same template, seed and settings on
both routes, wall seconds through the app's CLI. "Cold" is a case's first run (its model loads
after whatever ran before), "warm" the second, with a new seed (ComfyUI answers a repeated
request from its cache in 0.2–0.6 s, which the first attempt measured by mistake). Native: the
app's worker flags (no `--text-quant`, SAM 3.1 f16), genai-native `f66aeb6`; ComfyUI:
`start-comfyui-fast.ps1` (`--fast fp8_matrix_mult`). One route on the GPU at a time; two Codex
jobs were running CPU-only (8 threads each) during the native half.

| Feature (864×1152) | native cold | native warm | ComfyUI cold | ComfyUI warm | warm ratio | template (native / ComfyUI) |
|---|---|---|---|---|---|---|
| image (Krea 2) | 14.3 | 3.8 | 24.3 | 4.0 | 0.96× | krea2-turbo/image both |
| image (2.1) | 16.3 | 3.9 | 8.3 | 4.2 | 0.92× | qwen-2.1/image both |
| image from a ref | 6.0 | 10.3 | 7.4 | 5.6 | **1.83×** | qwen-2.1/image-ref both |
| fill | 8.2 | 8.2 | 4.9 | 3.3 | **2.51×** | qwen-2.1/fill both |
| fill from a ref | 9.6 | 9.6 | 120.5 | 114.0 | different model | 2.1 edit / 2511 fill-ref |
| edit | 10.1 | 10.1 | 8.7 | 5.6 | **1.82×** | qwen-2.1/edit both |
| expand | 10.0 | 10.0 | fails | fails | — | ComfyUI's 2.1 expand route: "needs an output size" (a bug of that route) |
| remove (stroke) | 9.3 | 4.3 | 14.3 | 12.6 | different model | 2.1 edit / 2511 fill-lightning-8 |
| remove background | 5.7 | 10.1 | 8.6 | 5.5 | **1.85×** | qwen-2.1/matte both |
| angles | 10.1 | 10.1 | 10.9 | 4.9 | different model | 2.1 edit / 2511 angles |
| select by text | 1.1 | 0.3 | 1.8 | 0.6 | 0.47× | SAM 3.1 both |
| upscale 2× | 80.3 | 79.7 | 42.8 | 40.8 | **1.96×** | qwen-2.1/edit tiles both |
| split (3 layers) | 130.1 | 125.0 | 129.0 | 124.8 | 1.00× | qwen-layered/split both (after `f66aeb6`: 175 → 130 s) |

Reading: text to image, selection and the split are level or faster natively; **every
Qwen-Image-2.1 route that conditions on a picture is 1.8–2.5× slower natively** (fill, edit,
image from a reference, matte, upscale's tiles): the reference path (the vision tower, the VAE
encode, the reference tokens) is the gap to close. Native image-from-a-ref and matte were faster
cold than warm (6.0 vs 10.3 s, 5.7 vs 10.1 s): something is recomputed on the second request.
The three "different model" rows run 2511 on ComfyUI because `paint.remove`, `generate.angles`
and a fill with references have no template parameter and ComfyUI's `auto` picks 2511 there.

**Remove Background with an empty prompt** (2026-10-29, through the app, the gen-tshirt
portrait, five unseeded runs on the native worker): 5 of 5 kept the man (511.8–515.5 k pixels,
the same bounds), where before ShotoPhopEX `49ecf6d3` (the subject named first) 3 of 5 kept
nothing.

**After genai-native `03fb72e`** (Codex: the text encoder stays resident instead of being
reloaded on every picture request, an exact conditioning cache when only the seed changes, the
vocabulary on the CPU, bounded VAE scratch; outputs within 0.06/255 of before;
`crates/qir/tools/qwen21_ref_performance.md`), native warm through the app, same cases:

| Feature (864×1152) | native warm before | native warm after | ComfyUI warm | ratio after |
|---|---|---|---|---|
| image (2.1), control | 3.9 | 3.9 | 4.2 | 0.93× |
| fill | 8.2 | **3.2** | 3.3 | 0.97× |
| image from a ref | 10.3 | **5.2** | 5.6 | 0.93× |
| edit | 10.1 | **5.1** | 5.6 | 0.91× |
| remove background | 10.1 | **5.1** | 5.5 | 0.93× |
| upscale 2× | 79.7 | **54.1** | 40.8 | **1.33×** (open) |

(The cold column of this rerun is not comparable: each case started its own worker.)

**Upscale** (2026-10-28 evening): ShotoPhopEX `3b6f3ca5` made the app side faster with
byte-identical output (54.1 → 47.5 s; `research/notes/upscale-native-speed.md`), and genai-native
`3da83bd` merges a LoRA into the resident INT8 weights once per binding as ComfyUI does (the
Restore LoRA had cost 14.5 s of side-path GEMMs per six tiles) and runs the DiT prefix through
the flash kernel; tiles within 0.0036/255 of before, Krea 2 with a LoRA byte-identical
(`crates/qir/tools/lora_runtime_performance.md`). Native upscale 2× warm through the app:
**40.6 s** against ComfyUI's 39.0 s with the same Restore LoRA (1.04×; the six model calls are
faster natively, the remainder is the app's per-tile work).

**Split into Layers with SageAttention2, 2026-10-29** (genai-native `perf/layered-attention`;
the 3-layer split of the 864×1152 portrait, seed 7, through the app's CLI, one fresh worker per
variant, RTX 5090). Quality is judged by compositing the three layers back onto the input picture,
the split's own job: the background layer's invented detail moves with any numeric change (int8
and fp8 matmuls moved it as much as Sage2), so a per-layer comparison with the bf16 run measures
divergence, not damage. Flash at two seeds sets the band.

| Variant | Warm | Composite vs input |
|---|---|---|
| bf16 flash attention (before) | 137.2 s | 36.91 dB (seed 8: 139.1 s, 38.73 dB) |
| **SageAttention2, blocks 0/52/58 on flash (now the default)** | **92.6 s (−33 %)** | **36.91 dB** |
| fp8 tensor-core GEMM, activations scaled (rejected) | 133.0 s | 34.34 dB |
| Sage2 + fp8 GEMM (rejected) | 106.2 s | 32.91 dB |
| int8 ConvRot from the fp8 file (rejected: slow path) | 222.6 s | 36.04 dB |

Sage2 takes V in f16 and Layered's values reach |23,680|: each head's V is scaled into f16's range
and its output back (exact). Blocks 0, 52 and 58 keep the flash kernel: their queries/keys reach
|796|, |45.5| and |80.5| where the rest stay at or under |21|, and on block 0 Sage2's INT8 was
12–89 % off flash per token range (blocks 1 and 59: 0.4–3.1 %). ComfyUI 0.43's split: 124.8 s
(§0, before this change); native now 92.6 s, **0.74×**.

**Upscale without text-encoder evictions, 2026-10-29** (genai-native `perf/vae-decode`; the
app's CLI, the 864×1152 portrait, Restore 0.5, seed 20, the bf16 text encoder as the app runs
it, one fresh worker per build, the desktop holding ~3.4 GB of the card). With the desktop's
share grown since the morning, each of Upscale's six tiles had evicted the 2.1 text encoder for
its sampling (3.93 GB usable against 4.35 GB) and reloaded it for the next (5.1 s), which
doubled the run. Now the f32 vision tower (~2.4 GB) gives way first and stays transient once it
had to; the sampling keeps 256 MiB of driver headroom (a run whose tower stayed resident paged
on Windows: 144.6 s cold).

| Upscale 2× | before | after | output |
|---|---|---|---|
| warm | 56.3 s (12 text-encoder evictions) | **30.6 s** (none) | byte-identical |
| cold | 60.3 s | **31.7 s** | byte-identical |
| Edit warm (control) | 4.22 s | 4.17 s | byte-identical |

ComfyUI 0.43's Upscale: 44.1 s, so native now **0.69×**. The Q8_0 text encoder (`--text-quant`)
also removed the evictions (29.1 s) but moved Edit's output to 38.45 dB for no Edit gain, so it
stays the low-VRAM option.

**Qwen-Image-Edit-2511 with SageAttention2, 2026-10-29** (genai-native `e9a4ea1`, the same flash blocks as Layered after its own survey: 0 at |728|, 58 at |79|, 52 at |53|): Lightning-8 edit **8.09 to 7.25 s** warm (cold 43.3 to 33.0 s), the 40-step CFG 4 edit **94.4 to 86.8 s**; same seed against flash 26.8 / 35.8 dB (the snowflakes placed differently, composition, scarf and face alike) where a second seed is 14.9 / 14.5 dB.

**Upscale's tiles in one batched request, 2026-10-29** (genai-native `perf/upscale-batch`
`dce1ce3`, ShotoPhopEX `perf/upscale-batch`: `params.batch` carries each tile's seed, and the
worker encodes all six tiles with the transient vision tower loaded once instead of once per tile,
then samples each tile as its own request would). The workers ran in ABCCBA order with a 45 s cooldown
before each worker, edit and upscale cold plus 2 warm each, 864x1152 x2, Restore 0.5. Per-tile
(the old CLI) **32.88 / 32.11 s** warm against batched **30.85 / 30.77 s** (about 5 %); cold
33.8 against 31.8 s. Vision reloads fell from 24 to 4 per worker, and per-tile denoise is the
same 3.71 s; Edit is unchanged (4.84 s). Every output is **byte-identical** to the per-tile path,
cold and warm. A worker that refuses the batch is asked tile by tile. The same ShotoPhopEX branch
also fixes Upscale to run as a background job: a nested `image.imageSize` used to switch the job
off, so Upscale ran inline on the UI thread.

**A first-block step cache for 2511, opt-in, 2026-10-30** (genai-native `perf/step-cache`
`0f7bc5f`, measure-only: `QIR_STEP_CACHE=<threshold>`, off by default, and the default path is
unchanged). Each step runs the first block. When that block's residual moved less than the
threshold since the last full step (mean |Δ| / mean |previous|), the other 59 blocks' residual
from that step is reused. There is one cache per CFG branch. The 40-step CFG 4 edit (INT8,
864x1152, seed 16, one worker per threshold):

| threshold | warm | steps cached of 40 | PSNR vs off | looked at |
|---|---|---|---|---|
| off | 39.4 s | 0 | — | — |
| 0.05 | **24.8 s** | 18 | 32.5 dB | barely different: the snow slightly softer |
| 0.1 | 16.9 s | 27 | 30.0 dB | fewer snowflakes, softer knit |
| 0.2 | 12.3 s | 32 | 26.1 dB | most of the snow gone, detail lost |

The pictures are in `photocraft-tests\bench\sp2-step-cache` (`sheet.png`). MOSS isn't on
`LayeredDit` and would need its own hook; not done.
**Also checked this pass and not landed:**
- **Krea 2's full-precision layers' bf16 weights cached:** byte-identical, but warm 3.677 to
  3.651 s for 9.6 GB more VRAM.
- **A per-shape cuBLASLt autotune at Krea's bf16 shapes:** candle's matmul already reaches
  230–232 TFLOPS; the best of the top-8 heuristic algos is 1–3 % faster, under 1 % a picture.
- **The 2.1 prefix pass's causal mask built on the device:** byte-identical, no change; the
  warm prefix is already ~0.2 s.
- **Split's and 2511's q, k and v sharing one rotated INT8 activation** (they quantized the same
  input three times): byte-identical, but warm Split 60.5 to 67.1 s (ABBA, consistent) and 2511
  Lightning-8 unchanged (4.53 / 4.52 s), so dropped. ConvRot's quantize is 3–26 % of an INT8
  linear (M 8272: 0.07 ms against a 0.29 ms GEMM at K 3072, 0.30 against 0.89 at K 12288).
  Fusing it into the producing kernels would save only the separate pass, about 3–4 % end to end
  by estimate, for new kernels in the shared ConvRot code; not done.

**Speed pass 3, 2026-10-30/31** (genai-native main `e53bab0`; RTX 5090, 864x1152 unless noted,
fresh workers, warm means):

| change | before | after | picture |
|---|---|---|---|
| INT8 disk cache (2511 / Split DiT, `QIR_INT8_CACHE`) | 2511 L8 cold 35.4 s; Split DiT load 7.35 s | 17.8 s; 6.03 s | byte-identical |
| Fast draft on Split (step cache 0.10) | 59.2 s | 40.2 s | composite 37.18 dB (37.25 off) |
| Layered MLP bias + GELU fused into the down quantize | Split 59.2 s | 57.5 s | byte-identical |
| Wan VAE decoder convolutions as implicit GEMM | decode Krea 0.356, 2511 0.432, Split 0.94 s | 0.232, 0.307, 0.57 s | ≥ 56 dB |
| Krea 2 full-precision layers on INT8 + rank-64 correction | 3.7 s a 1024² picture, DiT load 6.25 s | 2.9 s, 5.39 s | t2i suite vs ComfyUI 19/28 both (27.52 vs 27.57 dB) |
| Block-sparse attention on Split (opt-in `QIR_LAYERED_SPARSE`, off) | 57.5 s | τ 0.95/0.9/0.8: 54.2/52.3/50.4 s | composite 28–31 dB, layers reorganized: not used |

Tried and not landed: batching the per-linear INT8 conversion sync (with the cache it only runs on
a first load), pinned double-buffered uploads (upload 4.2 to 2.7 s for 27 GiB, wall clock
unchanged: the load waits on the 19 GB file), a pinned RAM copy of the last model (the OS file
cache already holds it on a 64 GB machine). Split's per-step budget after the INT8 pass: attention
~47 % (SageAttention2 at ~425 TFLOPS), INT8 linears ~38 % (~600 TOPS, the best tile at M ≈ 39k),
the rest ~15 %.

**Qwen-Image-Edit-2511 on INT8 linears, 2026-10-30** (speed pass 2, genai-native `perf/2511-int8`
`6a35b91`, on the Split INT8 change). 2511's resident DiT, with its LoRAs already merged into the
weights, runs ConvRot INT8 linears by default. The load that binds and materializes the LoRAs
stays fp8; the reload with the merged weights converts. `QIR_2511_INT8=0` keeps fp8. Measured in
ABBA order with cooldowns, seed 16, 864x1152, "Make it winter" on the portrait:

| 2511 edit, warm | fp8 | INT8 | same seed vs fp8 | a second seed (17) vs 16, fp8 |
|---|---|---|---|---|
| Lightning-8 | 7.16 / 7.15 / 7.15 / 7.15 s | **4.41 / 4.41 / 4.42 / 4.40 s** | 22.1 dB | 14.4 dB |
| 40-step, CFG 4 | 65.9 / 65.9 s | **38.2 / 38.0 s** | 36.3 dB | 16.3 dB |

Cold, Lightning-8 went from 31.9–34.5 to 30.4–30.8 s and the 40-step from 84.2–84.4 to 58.1–58.3 s.
Looked at side by side: on Lightning-8, scarf, pattern, pose and light match, and the snowflakes
and the knit's stripes shift; the 40-step is near-identical. **The CFG pair in one batched
forward was tried and not kept**: on the 40-step edit (INT8, ABBA) batched took 39.5 s against
37.9 s for two forwards, and it moved the picture (35.6 dB). Each forward already has ~8k tokens.

**Split into Layers on INT8 linears, 2026-10-29 night** (speed pass 2, genai-native
`perf/split-int8` `16adfc4`, on top of the resident DiT). Split's DiT now runs ConvRot INT8
linears by default: each fp8 weight is dequantized once at load and requantized.
`QIR_LAYERED_INT8=0` keeps fp8. 2511, which binds LoRAs, stays on fp8. The earlier INT8 attempt
paged the card at load, and the cause was a missing synchronize, not the INT8 weights: the
conversion ran asynchronously and hundreds of linears' freed bf16 temporaries were still pending,
with free VRAM falling ~2.3 GB per 60 linears. Synchronized per linear, the DiT loads with ~3 GB
free and step 0 has 3.1 GB to spare. Measured in ABBA order with cooldowns, both runs resident,
seed 7, 3 layers at ~1 MP: warm **86.3 to 61.9 s** (85.9 / 86.5 / 86.1 / 86.6 against 60.5 /
61.1 / 59.9 / 66.0), denoise 84.8 to 58.4 s, cold 91.4 to 67.9–80.4 s. Quality: the layers
composited back onto the input give **37.25 dB** against fp8's 36.91, coverage 0.999. Against
fp8, the picture's own layer is 44.5 dB and the empty third layer 47.1 dB. The first layer is
invented background on this picture under both, and it moves with any numeric change (2.65 dB),
as noted for SageAttention2. The INT8 resident set is still ~7.6 GB above fp8's (free after load
2.9–3.4 GB against 10.95), probably allocator granularity; not chased further.

**Split into Layers with the DiT resident, 2026-10-29 night** (speed pass 2, genai-native
`perf/split-resident` `9e10f13`). The Layered DiT (~20 GB) and VAE stay on the card between
splits, and the prompt's text conditioning is kept. The app always sends its default split
prompt, so a warm split loads nothing. A new prompt drops the DiT for the text encoder, since the
two don't fit 32 GB together. One binary, `QIR_LAYERED_RESIDENT=0` against the default, ABBA
order, 45 s cooldowns, seed 7, 3 layers at ~1 MP, a cold split and two warm per worker: warm
**91.9 to 86.0 s** (91.6 / 92.2 / 91.7 / 92.1 against 85.7 / 86.2 / 85.7 / 86.2). The phases
gone are the text encode (2.6 s, its load included), the DiT load (3.2 s) and the VAE loads
(0.3 s). The 84.4 s denoise is unchanged. Every layer is **byte-identical** to the per-split
loads.

**Upscale on fewer, larger tiles by default, 2026-10-29** (ShotoPhopEX `perf/upscale-tiles`, the
owner's call after the seam crops). The default tiling is now the fewest equal tiles of at most
1.15 MP each, never a strip of three or more; `grid` names one and `tileMegapixels` asks for the
old square tiles. 864x1152, 1232x816 and 1152x864 enlarged 2x take 2x2 where square
megapixel tiles took six. One worker, seed 20, Restore 0.5, batched, each picture both ways and
then again in the other order: portrait **30.3 to 24.2 s** (the first 6-tile run took 36.2 s),
cat **30.0 to 24.3 s**, bird **29.7 to 24.8 s**, so 16 to 20 % faster. Strips of three at ~1.5 MP
took 82 to 154 s in the prototype run. The crops are in `photocraft-tests\bench\upscale-tiles`.

**The 5090 GEMM plan, rerun under the fixed boundary (a84ab70), 2026-10-29:** byte-identical to
sm89 every time, but not a consistent win. In ABCCBA order the first sm120 worker gave Edit 4.81 s
and Upscale 31.26 s, a tie and a loss against sm89's 4.83 / 30.81; the last gave 4.54 / 29.05,
starting from a different idle clock (487 MHz against ~1,100). Two earlier back-to-back slots
disagreed with each other as well. It stays opt-in (`QIR_GEMM_PLAN=sm120`), with sm89 as the
default.

**Checked and not landed, 2026-10-29 afternoon.** The card runs **power-capped** under load (575 W default limit; SW Power Cap in 98 of 105 busy samples at 200 ms, mean 523 W, SM 2,653 MHz of 3,090, 61 C): compare only within one session. **CUDA graphs**: GPU utilisation averages 96.8 % while a 2.1 step runs, so launch and sync gaps are ~3 %. **The 5090 INT8 GEMM plan** (genai-native `perf/gemm-plan-5090`, `QIR_GEMM_PLAN=sm120`, opt-in): 3–11 % per GEMM in `tests/convrot_shapes.rs`, but end to end 2.1 text to image 3.56 -> 3.50 s and Edit 4.22 -> 4.39 s (byte-identical), within noise. **INT8 ConvRot for Layered** (parked): ConvRotLinear is 2.2–2.6x the bf16 matmul at M 8272 and 40000, but the split already peaks at 28.1 GB with bf16 linears; with INT8 it reached 31.97 GB and Windows paged it (both splits past the 600 s timeout).

**Upscale rerun, 2026-10-29** (`speed-table.ps1 -Only "upscale 2x" -UpscaleLora restore`, the
cdcedbd8 CLI so both routes run, genai-native `8d4e86e`, ComfyUI 0.43, the app sending the
Restore LoRA at 0.5 on both): native cold 51.7 s, **warm 30.8 s**; ComfyUI cold 63.3 s, warm
44.1 s (**0.70×**, the gap closed). Per warm tile: native request 4.9–5.3 s (25-step denoise
3.4–3.8 s; the merged LoRA costs nothing per step), ComfyUI 7.0–7.4 s; the app's own work
(enlarge, crop, align, colour, blend, composite) about 0.4 s per run on both routes. In the cold
run the native worker evicts its text encoder for the first sampling and reloads it on the
second tile (5.2 s); the warm run keeps it resident. The earlier 40.6 vs 39.0 was a single
run before `8d4e86e`; this one is too, so it is one sample, but the per-tile split is
consistent across all six tiles.

**Upscale follow-up, 2026-10-28:** [phase report](research/notes/upscale-native-speed.md).
Small CPU optimizations preserve byte-identical outputs on both routes (seeds 20/21). New
unprofiled native warm: 47.5 s with default Restore 0.5, versus 30.8 s with that adapter disabled
(diagnostic only). Matched Restore ComfyUI: 39.0 s. The speed target remains open; the native
worker's runtime adapter path dominates. These new observations do not replace the historical
measurements above; source state, profiling overhead and full phase data are in the report.

## 0b. Qwen-Image-Edit-2511 natively vs ComfyUI (2026-10-28)

genai-native `d40c536` (the 2511 port: a selectable alternative, not a default;
`crates/qir/tools/qwen_edit_2511_pipeline.md`). Same inputs, seed and template on both routes,
~1 MP, warm (a new seed per request), one route on the GPU at a time:

| Template | steps / CFG | native vs ComfyUI PSNR | native warm | ComfyUI warm |
|---|---|---|---|---|
| edit-lightning-8 | 8 / 1 | 43.3 dB | 13.5 s | 8.4 s |
| fill-lightning-8 | 8 / 1 | 49.7 dB | 13.5 s | 8.8 s |
| remove-lightning-8 | 8 / 1 | 50.0 dB | 11.4 s | 8.5 s |
| expand-lightning-8 | 8 / 1 | 33.3 dB | 13.5 s | 8.5 s |
| fill-ref-lightning-8 | 8 / 1 | 48.6 dB | 26.0 s | 21.1 s |
| image-ref | 8 / 1 | 35.5 dB | 25.1 s | 21.1 s |
| angles | 4 / 1 | 43.2 dB | 7.4 s | 4.5 s |
| edit (base) | 40 / 4 | 36.6 dB | 90.5 s | 81.9 s |

The pictures match. The cold image-ref gap (27 dB) is ComfyUI's, not native's (2026-10-29):
natively, a fresh worker's first image-ref and a second one at the same seed (42) are
byte-identical, while ComfyUI's first dynamic call after a load runs on its transient bf16 patch
before its resident fp8 copy; native always gives the resident answer, so nothing is changed.
**After genai-native `8d4e86e`** (the 20 GB DiT kept resident between 2511
requests instead of reloaded; outputs byte-identical) native is faster on every template: edit
8.3 vs 8.4 s, fill 8.3 vs 8.8, remove 8.3 vs 8.5, expand 8.3 vs 8.5, fill-ref 20.3 vs 21.1,
image-ref 20.4 vs 21.1, angles 4.4 vs 4.5, the 40-step edit 78.2 vs 81.9
(`crates/qir/tools/edit2511_performance.md`).

**2511 vs 2.1 on the swap set** (2026-10-28; `swap-run.ps1` routes `fill2511n` / `edit2511n`,
both native, the 15 cases of `swap-cases.json`, seeds 7 and 11, the object selected by name with
SAM 3.1 and replaced; judged on contact sheets):

| | Qwen-Image-2.1 | Qwen-Image-Edit-2511 (Lightning 8) |
|---|---|---|
| Generative Fill | about 13 of 15 swaps per seed; misses: the cup at seed 11, the subtle jacket | about 7 of 15: kept the bicycle, dog, hydrant, cup (both seeds), put the mug inside the glass, half-replaced the motorcycle; the outfits, sunglasses, umbrella, hat and bench right |
| Generative Edit | about 14 of 15 (the cup became a cup of red wine) | about 14 of 15, the same cup miss; stray text printed on the stone bench, a blend line at the raincoat's hem; the leather jacket more convincing |
| Warm time, 864×1152 | fill 3.2 s, edit 5.1 s | fill 8.3 s, edit 8.3 s |

Reading: for fill, 2.1 is clearly better and faster; for edit, the two are level on these swaps
and 2.1 is faster. The owner's choice of 2.1 as the default stands; 2511 stays selectable. The first image-ref
after a load is 27 dB (ComfyUI's first dynamic call uses the bf16 patch before its resident
copy). The 2511-vs-2.1 comparison on the earlier sets follows once 2511 is at speed.

## 0c. Live previews while generating: Off / Fast / Sharp (2026-10-29)

genai-native `feat/live-preview-2` (`df81a61`), its ignored test `gn-diffusion/tests/preview_live.rs`
on the 5090 with ComfyUI's models folder as the root: the engine in-process, one warm-up run,
then Off, Fast, Sharp and Off again at 864×1152, seed 7, each run's wall time and the card's
peak memory (nvidia-smi every 100 ms, the whole card: about 3 GB is other tenants). Fast is the
ComfyUI latent-to-RGB map at the latent grid (Layered: our least-squares fit); Sharp is taew2_1 /
taeqi2_1 at full size every 3rd step. **Every picture is byte-identical across the four runs of
its case** (FNV-1a of the result bytes; the test asserts it).

| case | steps | Off | Fast | Sharp | Off again | previews Fast / Sharp | peak card memory |
|---|---|---|---|---|---|---|---|
| Krea 2 Turbo, text to image | 8 | 3.37 s | 3.38 s | 3.57 s | 3.51 s | 7 (108×144) / 2 (864×1152) | 27.7 GB, the same in every mode |
| Qwen-Image-2.1, text to image | 25 | 3.70 s | 3.73 s | 3.90 s | 3.75 s | 24 (54×72) / 8 (864×1152) | 28.8 GB |
| Qwen-Image-Edit-2511 fill, Lightning 8 | 8 | 7.52 s | 7.50 s | 7.66 s | 7.44 s | 7 (108×144) / 2 (864×1152) | 31.1 GB |
| Split into Layers, 3 layers | 20 | 94.4 s | 96.2 s | 93.1 s (= Fast) | 93.0 s | 57 (19 × 3 layers, 108×144) | 27.8 GB |
| Upscale, one ~1 MP tile (2.1 edit + Restore LoRA 0.5) | 25 | 4.24 s | 4.25 s | 4.43 s | 4.24 s | 24 (54×72) / 8 (864×1152) | 18.1 GB |

Krea 2 and Layered are the second run (after the Layered fix); the others the first. Reading:
**Fast costs nothing measurable** (within the Off-to-Off spread, 0.03–0.15 s; Layered's runs
vary by 3 s on their own); **Sharp costs about 0.15–0.2 s per run** (60–90 ms a decode, two to
eight decodes) and no measurable peak memory. Each preview copies its pixels to the host and so
waits for the queued step (the worker's "pictures in N ms" log line counts that wait, not
added time). Layered has no tiny decoder (its RGBA VAE's latents run about ten times smaller than
Wan 2.1's, so taew2_1 and the Wan factors gave grey): its Sharp shows Fast. A Sharp run that finds
less free memory than the decode needs stays on Fast for the rest of the run (seen on the first
Upscale run with 31 GB in use). Not measured: MOSS-Image (on its own branch, not hooked yet).

## 1. Fill templates, default server flags, no cache (run 1)

Uploads had unique names, so every run paid the 7 B vision text encoder and the VAE encode;
"warm" = same pixels and seed again. The 2511 base was already resident from earlier work, so
its "cold" is not a disk load.

| Fill | cold | warm | encode | upload | queue | download |
|---|---|---|---|---|---|---|
| 2511 base, 40 steps CFG 4 | 166 703 | 162 129 | 2 | 5–6 | 0–1 | 294–372 |
| 2511 Lightning 8 steps CFG 1 | 37 406 (LoRA load) | 18 597 | 2 | 5–6 | 0–1 | 362–366 |
| 2511 Lightning 4 steps CFG 1 | 12 529 | 16 525 | 2 | 6–21 | 0–1 | 355–366 |
| Qwen-Image-2.1, 25 steps CFG 1 | 8 563 | 4 504 | 2 | 4–6 | 0–1 | 7–229 |

Reading: Qwen-Image-Edit-2511 is a 20 B model working at its 1 MP size whatever the request
(`FluxKontextImageScale` upsamples a 576×512 request to ~1 MP), and CFG 4 means two passes per
step: ~4 s per step, so 40 steps is 2½ minutes. The Lightning LoRA (CFG 1, one pass) makes a
step ~2 s and 8 steps a 16–19 s fill. The 4-step tier's "warm" being slower than its "cold" is
the text-encoder cost landing differently between runs: the encoder is several seconds of every
run in this table. Qwen-Image-2.1 (7 B, CFG 1, KV prefix cache) does the same fill in 4.5 s.
The client's own share is negligible: 2 ms encode, 5 ms upload, under 0.4 s download.

## 2. Fill templates, `--fast fp8_matrix_mult`, content-addressed uploads (run 3)

Uploads are now named by content hash, so ComfyUI's node cache keeps the loader, the text
encoder and the VAE encode for the same pixels. "cold" = a selection shifted by 16 px (new
pixels, encoder runs; the family's model already resident); "warm" = the same pixels with the
next seed (the variation / re-roll case: only sampling runs).

| Fill | cold | warm | sampling (warm) |
|---|---|---|---|
| 2511 base, 40 steps CFG 4 | 158 935 | 153 331 | 153 055 |
| 2511 Lightning 8 steps | 19 284 | 15 960 | 15 913 |
| 2511 Lightning 4 steps | 11 664 | 8 115 | 8 068 |
| Qwen-Image-2.1, 25 steps | 8 027 | 2 614 | 2 576 |

Reading: `fp8_matrix_mult` is worth about 5 % on the `fp8mixed` 2511 file (159 s vs 167 s). The
content-addressed cache is worth the encoder's 3–5 s on every re-roll: a 2.1 variation is 2.6 s,
a 2511 Lightning-4 variation 8 s, a Lightning-8 one 16 s.

Not adopted, measured:

- `--highvram` with 2511 fp8 (20 GB) + its 8.7 GB encoder resident left 2 GB of 32 free; the
  40-step run got slower (192 s) and the Lightning LoRA run thrashed on reload. The two runs
  that measured it also overlapped (two benchmarks on one GPU), so only the direction is
  reliable; the plain dynamic-VRAM default keeps the models resident anyway while they fit.
- lightx2v's `qwen_image_edit_2511_fp8_e4m3fn_scaled.safetensors` (the FP8-tensor-core
  layout) was 35 % faster per fill (10.6–12.6 s for Lightning 8) but produced **noise** in the
  masked area with this graph under `fp8_matrix_mult`; removed again. Worth a second look with
  the ComfyUI-specific `_lightning_comfyui_4steps` file or without the flag.
- `--fast fp16_accumulation`: only speeds up fp16 models; ours run bf16, fp8 or int8.

## 3. Quality: placement and seams (Lightning 8, same seeds)

Three cases on the lighthouse: the water/rock edge with seeds 7 and 11 ("a small red wooden
rowing boat floating on the water") and the sky ("a hot air balloon drifting in the evening
sky"). Judged by eye on the output PNGs.

| Variant | seed 7 boat | seed 11 boat | sky | cost |
|---|---|---|---|---|
| plain prompt, hard mask | on the rocks | cut off, visible rectangular seam | good | 16–18 s |
| mask as `image2` + instruction (`fill-guided`) | on the water | on the water | good, flatter palette | 28–51 s (another 1 MP of reference tokens) |
| "Add {prompt} to this image…" wrapper | on the rocks | on the water, no seam | good | same as plain |
| "Edit this image: {prompt}…" wrapper | on the water | on the water | rectangular seam | same as plain |
| **shipped**: "Add…" / imperative wrapper + outward-feathered request mask | on the rocks | on the water, no seam | good, no seam | same as plain |

What shipped: every 2511 fill template wraps the prompt ("Add {prompt} to this image, fitting
the scene's perspective, lighting and surroundings naturally. Change nothing else."; a prompt
that already starts with an imperative verb gets "{prompt}. Fit the result…" instead), and the
request mask is feathered outward so the latent noise mask blends the edge. `fill-guided`
remains in the picker for the cases where placement matters more than time. Qwen-Image-2.1 put
the boat on the water at every seed tried; it is the research-licensed option.

**Edges (later the same day).** With the layer mask equal to the selection, the model's blended
band was cut off at the selection's edge: a hard border wherever the re-rendered pixels differed
from the original in tone (the user's "the borders are harsh"). Measured on the right-side
expand's sky (mean luminance per column, y 200–420): the original stepped from 225 to 228 at
the old edge, with a dark line of 213 right on it. Three changes, in the order they were found:

1. The layer mask is by default the soft request band itself, its ramp **dithered**: per-pixel
   noise (a splitmix64 hash of pixel index and seed), amplitude `0.6 × (1 − |2m − 1|)`, zero
   inside and outside the band and strongest mid-ramp, clamped to 0..1. The transition reads as
   grain, like film, not as a gradient or a line, and 8-bit exports cannot band. Fills feather
   4 % of the longer side (6–48 px, was 2 %), a band of twice that; expands the same 4 %
   (8–80 px, was 8 %): at 8 % the model took the licence to move the horizon inside the band
   (a ghosted double horizon at one seed), at 2 % the old edge still read as a line.
   `"edge":"hard"` restores the exact selection (or exactly the added canvas) for compositing
   work that wants it.
2. The feather ramp now starts at **full** coverage on the edge and falls to nothing at twice
   the radius (ComfyUI's GrowMask + FeatherMask shape). The first version, two box blurs
   `max`ed with the selection, started at half coverage: as the noise mask it kept half of the
   original latent one pixel outside the edge, and as a layer mask it stepped from 1 to 0.5,
   so half the tone difference stayed as a line.
3. Expand's empty canvas is pre-filled by replicating the picture's edge pixels, not mid-grey.
   The model never sees it as content (its reference is the cropped picture and the noise mask
   is full over it), but the VAE's latents beside a wall of grey carried its tone into the edge
   band: that was the dark line of 213.

After the three: 229 → 226 across 160 px, no step and no dip in the profile, nothing visible in
a contrast-stretched crop; the hard variant keeps its step (as asked). Server cost: none, the
request's size and graph are unchanged (the hard/soft variants of one seed hit ComfyUI's cache
and return in 0.1 s); the dither is one pass over the mask. The grey pre-fill was A/B'd again
with the full-coverage ramp (`xab-f08-grey-*`, `xab-f04-grey-*`): still a 2–3 level step right
on the edge at both seeds, so the edge replication stays.

**Consistency of the new side (the thing no edge can hide).** With the user's prompt "more
open sea and evening sky", seeds 6 and 8 painted a crisper, darker sea with a sharp horizon
next to the hazy original: a different photograph, visible through any blend. Same widen,
Lightning 8 unless noted (`bench/xab2-*.png`, `xab3-*.png`):

| Prompt → what the model got | seed 6 | seed 8 | time |
|---|---|---|---|
| "more open sea and evening sky" (the description wrapper) | crisp sea, sharp horizon | sharp horizon, higher than the original's | 13–15 s |
| the same through the **40-step base** (`qwen-edit-2511/expand`, CFG 4) | a second lighthouse in the new area | hazy, consistent | 130 s |
| "Add more open sea…, keeping exactly the same soft haze… as the original photo" | hazy, consistent | a second lighthouse | 14–16 s |
| **empty prompt** ("extend the scene beyond its original edges, continuing it naturally") | hazy, consistent | consistent, a little sharper | 14–18 s |
| "Extend this image…, continuing the scene naturally with more open sea… Match the original's … haze… and do not repeat its objects" | a second lighthouse | consistent | 13 s |
| "Extend this image to show more open sea… continuing the original photo naturally with the same haze, light and colours" | a stub of railing copied | consistent | 13–15 s |

Lessons: the empty prompt is the most faithful continuation, and it is what the dialog and the
docs recommend for "more of the same"; a description gives the model licence to restyle; the
40-step base is no more consistent and ten times slower. The shipped wrappers are unchanged.

**But the table is indicative at best.** Re-running the empty prompt at seed 6 in a steady
server state gave a second lighthouse (`final-expand-empty-s6.png`) where the run above had
none, and the two runs were bit-identical with each other on repeat. Pixel diffs of identical
requests run at different times: 0.00 (seed 8 widen, two repeats of the empty seed 6), 0.58
(the seed-11 fill), 2.19 and 4.53 (seed-6 widens), 21.44 of 765 (the empty seed 6, a second
lighthouse against none). So ComfyUI is deterministic in a steady state and not across
model-load states: the partially offloaded model after a graph switch (next paragraph)
computes slightly differently, and the 8-step Lightning sampler amplifies that into a
different composition at a knife-edge seed. Conclusions about wordings need many seeds in one
server state (a prompt study is listed under §6); what stands is that the model does, at some
seeds, repeat the picture's main object in the new area, whatever the wording.

**Graph switches thrash the server.** Every first Lightning run after the 40-step base, or
after a purge followed by a fill, took 31, 56, 93 and 121 s (2.3–2.5 GB free, torch 63 MB:
ComfyUI loads the newly patched model partially and streams the rest every step), then
13–20 s for the runs after it. (The base's own 130–148 s is not thrash: 40 steps at CFG 4 are
two model passes a step, about 3.6 s each at 0.75 MP against Lightning's single pass at
CFG 1.) **Fixed the same day** by the engine's `run_switching` (§5c): it remembers the model
files of the last run per server and purges before a run that changes them.

## 4. Native-size sampling (shipped)

The official 2511 graph encodes the `FluxKontextImageScale` output, so every request is sampled
at ~1 MP whatever its size. The shipped templates now VAE-encode the uploaded image itself: the
text encoder still sees the 1 MP reference, the sampling latent has the request's own size
(the engine grows the request rectangle to the 16-px grid, caps it at 0.75 MP, see §5, and
floors it at 512 px on the longer side). Same three cases, Lightning 8, same seeds:

| Case | 1 MP latent | native latent | result |
|---|---|---|---|
| edge, seed 7 | 16.2 s | 11.2 s | boat on the water (was on the rocks) |
| edge, seed 11 | 16.5 s | 9.2 s | boat on the water, no seam |
| sky, seed 7 | 16.4 s | 11.0 s | balloon, no seam |
| 120×90 selection (192×160 rect, sent at 512×416) | — | 10.4 s steady, 16.3 s with a model reload | a crisp seagull on the rock |
| 56×70 selection (96×112 rect, sent at 432×512) | — | 10.4 s | a brass porthole on the door |

35–45 % faster, and better: with the latent at the request's size the mask lands exactly
where the selection is, and both boat cases came out on the water.

## 5. Server memory pressure

After several checkpoints had been loaded in one server session (the 2511 base, its LoRA
tiers, Qwen-Image-2.1, the scaled fp8 file), the small-selection fills took 36 and 58 s with
2.5 GB of VRAM free: ComfyUI was staging the 19.6 GB model on every run. `POST /free`
(unload models) brought 30 GB back and the next fills took 16 s (reload included) and 10 s.
That is **Edit › Purge › Generative Models** (`generate.free`) in PhotoCraft. The steady state
with 2511 fp8 is ~1.6–2.5 GB free on a 32 GB card (UNET 19.6 GB + text encoder 7.9 GB + VAE).
In that state a 1 MP latent plus the 1 MP reference did not fit the headroom: Generative
Expand's frame case took 45 s (the log shows the 19.6 GB model re-staged for the run) against
18 s a moment earlier. The request cap is therefore 0.75 MP (768×1024), which keeps the model
resident and costs little: the editing models' own working size is about that.

## 5b. Remove Background and transparent generation (Qwen-Image-2.1, int8)

| Case | time | result |
|---|---|---|
| `generate.removeBackground`, 1024² lighthouse, no prompt | 16.8 s | boat solid, lighthouse a ghost (the model's reading of "the subject") |
| …, prompt "the red boat" | 16.7 s | a perfect 147×46 px boat cutout |
| …, prompt "the lighthouse" | 16.8 s | lighthouse with its rocks; the white tower semi-transparent against the bright sky |
| …, `asSelection` of the same request | 0.14 s | ComfyUI's cache |
| `generate.image` + `transparent`, "a vintage brass compass, product photo", 1024² | 10.8 s | a clean cutout with a real soft alpha |
| the first run after switching from the 2511 models (a purge first) | +~8 s | the 2.1 model and encoder load (6.8 + 8.7 GB) |

The matte is 25 steps of the full model, so it costs a text-to-image. The permissive route
(`template: auto` without the research opt-in) is SAM 3.1 instead, hard-edged but fast:

| Case | time | result |
|---|---|---|
| Remove Background, prompt "the red boat", SAM 3.1 | 1.5 s (checkpoint load included) | the boat, 146×38 px |
| Remove Background, no prompt ("the main subject"), SAM 3.1 | 0.5 s | the lighthouse tower, 111×281 px |
| `select.byPoint` at (350, 665) | 0.3 s | the boat |

A soft matte from a dedicated matting model (BiRefNet) would sit between the two in quality;
it needs a custom node pack the server does not have, so it is not planned for the default.
The detector route softens its mask with the classical edge refinement instead (`refine`).

## 5d. Split into Layers (Qwen-Image-Layered, fp8)

| Case | time | result |
|---|---|---|
| the 1024² lighthouse into 3 layers, 20 steps CFG 2.5 at 640² | 72 s (the 19 GB model loaded in it) | layer 0 a near-white base with the boat's shadow, layer 1 sky and sea with the boat, layer 2 rocks and lighthouse with real soft alpha |

CFG 2.5 means two passes a step, like the 2511 base; the official 640 px working size keeps it
to about a minute. The layers come back resampled to the canvas.

**INT8 cache miss, in the app (2026-11-01, RTX 5090, 864x1152, 3 layers).** Before genai-native
#18 a miss (entry moved aside) loaded the converted DiT with 0 MiB driver-free; Windows paged
every step (about 45 s each) and the split ran past the editor's 600 s. With the converted model
reloaded from the entry it writes: 80.4 s on the miss (10.7 GB driver-free after the reload), 40.2 s
on the next run (a hit); a hit before the fix, in the same worker after a 2.1 edit: 65-73 s.

## 5c. Generative Edit and the automatic purge

`generate.edit` sends the whole 1024² lighthouse at 0.75 MP through the official 2511 edit
graph (no noise mask); the result is a layer, masked to the selection when there is one
(`bench/edit-*.png`):

| Case | time | result |
|---|---|---|
| "make the sky dark and stormy with heavy clouds", Lightning 8 | 16.0 s steady (22.6 s with the 2511 reload after a 2.1 run) | a storm with lightning, the lamp lit, the sea rough; the rocks and lighthouse kept |
| "turn the red boat blue" with a 200×120 selection on the boat | 16.1 s | only the boat changed on the canvas (the model re-rendered everything; the mask shows the boat) |
| "make it a sunny day with a blue sky" | 20.2 s after a model switch, 16 s steady | convincing |
| the same storm through the 40-step base (`qwen-edit-2511/edit`) | 145–148 s | two passes a step at CFG 4; the Lightning tier is the default for a reason |

**The purge.** Three edits in one process, Lightning → base → Lightning (`purge-live.ps1`):
16.0 s, 148.5 s (the base's own cost, purged first), **20.2 s** for the Lightning run after
the base. The same return to Lightning without a purge took 121 s earlier in the day. The
engine purges only when the model files change and never on a server's first run; separate
CLI invocations are separate processes, so the CLI only exercises it within one `--cmd` chain.

## 5e. Prompting: what the guides say, and the enhancer

The first user session (dev log, 2026-10-09 evening) turned a line drawing of a face into a plate
of food twice: "make this hyper realistic" gives Qwen-Image-Edit nothing to hold on to when the
picture is a doodle. What the model authors and ComfyUI recommend, checked 2026-10-09:

| Source | What it says | What we do |
|---|---|---|
| Qwen-Image-Edit model card and the `Qwen-Image` repo's `polish_edit_prompt` (Alibaba, 2025–2026) | run the instruction through a VLM that sees the picture and rewrites it: name the objects, one change per instruction, state what stays, quote literal text verbatim | `generate.enhancePrompt`: the same rewrite, on the Krea 2 text encoder (Qwen3-VL 4B) already on the server |
| ComfyUI's shipped Qwen-Image-2.1 "prompt enhancer" subgraph (`comfyui-workflow-templates` 0.11.76) | a 10 KB system prompt for an 8B VLM (`TextGenerate`, temperature 0.3, no thinking) | too long for the 4B model (it echoes or rambles); a 12-line prompt with the same rules works |
| ComfyUI's Krea 2 template | `TextGenerate` on the Krea 2 encoder expands a short idea into a paragraph before `CLIPTextEncode` | `enhance/image`: the same node, our short instruction, 60–120 words |
| Krea 2 prompting guide | detailed natural-language paragraphs, subject first, then materials, lighting, setting, style | the image system prompt orders the paragraph that way and keeps every subject, colour and medium the user gave |
| Qwen-Image-Edit-2511 settings in ComfyUI's templates | 40 steps / CFG 4 base, Lightning LoRA 4 steps / CFG 1 (shift 3), `ModelSamplingAuraFlow 3.1`, `CFGNorm`, `index_timestep_zero` | already what the templates ship (§2, §5c); the 8-step Lightning tier is the default |

**Settings check.** Our 2511 graphs match the official templates node for node (sampler euler /
simple, shift 3.1, CFG norm, the reference-latent method); the Lightning tier is the official
Lightning recipe with 8 steps instead of 4 (visibly cleaner edges at +4 s). Krea 2 Turbo: 8 steps,
CFG 1, `ConditioningZeroOut` negative, CLIPLoader type `krea2`, as shipped. Nothing was left on
the table there; the prompt was the lever.

**The enhancer, live** (`enhance-live.ps1`, the CLI against the server, 5090, ComfyUI 0.39):

| Case | time | the rewriter's sentence |
|---|---|---|
| the doodle, edit, "make this hyper realistic" | 0.9–1.3 s | Turn the doodle of a smiling face into a photograph of a real human face with the same expression and pose, replacing the black outline strokes with real skin, hair, and facial features; keep the white background unchanged. |
| the doodle, edit, "turn this drawing into a hyper realistic portrait that looks similar" | 0.9 s | Make the hand-drawn smiley face look like a hyper realistic portrait of a human face with the same expression and pose, replacing the black outline strokes with real skin, hair, and facial features, keeping the white background unchanged. |
| the doodle, edit, "make it a watercolor painting" | 1.1 s | Turn the hand-drawn smiley face into a watercolor painting of a smiling human face, replacing the black outline strokes with soft watercolor washes and brushstrokes, keeping the simple white background unchanged. |
| the lighthouse, edit, "make the sky stormy" | 1.0 s | Make the sky stormy with dark, swirling clouds and flashes of lightning, keeping the white lighthouse, red boat, and rocky shore unchanged. |
| the lighthouse, edit, "turn it blue" | 0.8 s | Turn the sky and sea blue, keeping the white lighthouse and red boat unchanged. |
| the lighthouse, edit, `put a sign on the lighthouse that says "OPEN"` | 1.1 s | Add a sign reading "OPEN" to the lighthouse's front door, keeping the white lighthouse, red boat, and rocky shoreline unchanged. |
| the lighthouse, fill with the upper part selected, "a hot air balloon" | 1.0 s | Add a hot air balloon floating above the lighthouse, blending with the soft pink sky, while keeping the white lighthouse, red boat, and rocky shoreline unchanged. |
| the same fill, "remove this" | 0.9 s | Remove the white lighthouse from the upper part of the image, keeping the pink sky and rocky shoreline intact. (the limit of the place hint: "this" was read as the lighthouse, not as the selected sky) |
| image idea, "a fox in snow" | 1.9 s | a 110-word paragraph: the fox alert in deep snow, fur, paws, soft diffused daylight, an open field, a photorealistic cool palette |
| image idea, already detailed (a fisherman mending a net, golden hour, 85mm) | 1.8 s | polished, keeping "85mm lens", "golden hour" and the shallow depth of field |

Same seed, same sentence every time (temperature 0.3); the seed changes little. The edit sentences
are what the Qwen guides ask for: the verb first, the subject named from the picture, what stays.

**The chat template.** The first live run answered nothing half the time: of 29 rewrites, 15 came
back empty and 9 began with a stray "assistant". ComfyUI's `TextGenerate` with `thinking: false`
appends an empty `<think>` block after the assistant turn (the Qwen3 convention), which the
non-thinking Qwen3-VL-4B-Instruct answers with an immediate end-of-turn or by restarting the
turn. `probe-enhance5.ps1`, 12 runs per variant: thinking=false 0 clean (7 empty, 5 echoes);
**thinking=true 12 of 12 clean**, with the rules in the user turn or as a real system turn
(`system_prompt`). The templates use thinking=true and the system turn.

**The doodle, through Generative Edit with `enhance: true`** (`bench/enhance-doodle-*.png`,
`doodle-sentence-*.png`): the first wording of the rules ("ends with what must stay unchanged")
made the rewriter write "keeping its simple outline and dot eyes unchanged", and 2511 drew a
loaf of bread with a smiley (as it had from the raw prompt). The sentence for "turn this drawing
into a … portrait" gave a human face with the doodle's lines still drawn over it. Three
hand-written sentences then showed what the model needs: "a photograph of a real human face …
replacing the black outline strokes with real skin, hair and facial features" (A) and "a hyper
realistic portrait photo of a real person … with no drawn lines left" (C) both gave a clean
photographic face in 19–20 s; "look like a hyper-realistic photograph of a real human face"
without the replacement clause (B) gave a face with the lines kept. The rules now say so for a
change of medium, and the rewriter's sentence for "make this hyper realistic" is the A form.

**Cost.** A rewrite is 0.8–1.3 s for a sentence, 1.8–2 s for a paragraph, after a 3 s first load
of the 4B encoder (5 GB staged). An enhanced Krea 2 image took 11.1 s against 6.2 s as typed
(the rewrite, a longer encode and the switch back from 2511). The encoder stays resident: with
2511 (19.6 GB), its own encoder (7.9 GB) and the 4B (5.0 GB) all loaded the card had 1.9 GB free
and 2511 edits ran at 19–25 s instead of 16 s (Krea 2 was resident too); with the enhancer and
2511 alone the steady state is 13.5–14.6 s (§5f).

Through the CLI, end to end (`enhance-doodle-final.ps1`, `bench/enhance-doodle-*-final.png`):
"make this hyper realistic" on the doodle now gives a photographic human face in 18.8 s (the
rewrite included), "turn this drawing into a hyper realistic portrait that looks similar" one in
17.2 s; both layers remember the typed prompt and the sentence the model got.

## 5f. NVFP4 against fp8 on the 5090

Comfy-Org publishes NVFP4 repacks of the 2511 text encoder (`qwen_2.5_vl_7b_nvfp4`, 5.7 GB
against 8.7 GB) and of Krea 2 Turbo (`krea2_turbo_nvfp4`, 7.2 GB against 12.2 GB); Blackwell
GPUs run the 4-bit weights through their own tensor cores (comfy-kitchen 0.2.37's
`scaled_mm_nvfp4`). Measured with the engine's own graphs posted to the server
(`nvfp4-bench.ps1`: a purge, then a cold run and two warm runs with their own seeds; ComfyUI
caches a repeated graph, which made a first attempt report 290 ms "runs"):

| Case (1024², 8 steps) | fp8 | NVFP4 |
|---|---|---|
| 2511 Lightning edit, the encoder swapped | cold 19.6 s, warm 14.1 / 14.0 s | cold 19.8 s, warm 14.1 / 14.2 s |
| Krea 2 Turbo image, the model swapped | cold 11.1 s, warm 7.1 / 7.0 s, 13.4 GB free afterwards | cold 9.2 s, **warm 5.7 / 6.0 s, 18.5 GB free** |
| 2511 edit with the enhancer's 4B encoder resident | 22.9 s with the load, then 13.5 / 13.6 s | 21.1 s, then 14.3 / 14.6 s |

The encoder runs once per prompt, so its precision does not move an edit's time; it saves
3 GB of weights (staged, in RAM and on disk). Krea 2 in NVFP4 is 18 % faster per image and
leaves 5 GB more free. Output: the lighthouse storm is the same picture with either encoder
(`bench/nvfp4-2511-te-*.png`), the fox equivalent (`bench/nvfp4-krea2-*.png`).

**Shipped as a policy, not a switch per file.** Template slots carry an `nvfp4` alternative
(the 2511 templates' encoder, Krea 2's model); Preferences › AI Integrations › **Model
precision** `auto` (the default) loads it when the server's GPU is a Blackwell part
(`is_blackwell` on `/system_stats`'s device name: GeForce RTX 50, RTX PRO Blackwell, B100–B300,
GB200/GB300, GB10) and the server lists the file; `default` never; `nvfp4` always (for
testing). The purge sees the switch (the model set is computed from the prepared request),
`generate.models` reports `device`, `blackwell` and each slot's `nvfp4` / `nvfp4Installed`, and
a caller's own `model` binding is never replaced. On Ampere and Ada there are no NVFP4 kernels
(ComfyUI would dequantize or fail), so `auto` leaves them on the default files. No official
NVFP4 repack of the 2511 diffusion model exists (a community one does); Qwen-Image-2.1 ships
int8 and w4a8 encoders instead, so neither has an alternative yet. Live through the CLI
(`nvfp4-live.ps1`): the server loaded `qwen_2.5_vl_7b_nvfp4` for an edit and
`krea2_turbo_nvfp4` for an image with nothing but the default preference.

## 5g. Reference images and the time estimate (ShotoPhopEX, 2026-10-12)

Warm runs through the release CLI, one process, ComfyUI on the 5090 (NVFP4 text encoder by
the `auto` policy): `generate.edit` on the 1024² lighthouse picture (sent at 880×880 = 0.77 MP),
Lightning 8, the same prompt, with 0, 1 and 2 file references (1024² each, scaled to ≤ 1 MP):

| References | Run 1 | Run 2 |
|---|---|---|
| none | 11.9 s | 11.7 s |
| one | 19.9 s | 20.0 s |
| two | 30.1 s | 30.3 s |

Each reference image adds about 9 s (another megapixel of tokens through the encoder and the
reference latents). The templates' `estimatedSeconds` were fitted to these runs and the Generate
panel's fills: the Lightning-8 family is `7.5 s + 5.5 s/MP + 9 s per extra reference` (a fill at
~0.2 MP: 8.7 s measured, 8.6 s estimated; the edits above: 11.7 / 20.7 / 29.7 s estimated). A
reference-driven text to image (`qwen-edit-2511/image-ref`) counts its first reference as the
picture: 12.1 s measured for a 3:2 1K image from one reference, 11.3 s estimated. The 40-step
tier's per-reference cost is scaled by its steps and CFG passes (90 s, not measured).

## 5h. Generative Fill from a reference (ShotoPhopEX, 2026-10-21)

`docs/comfy/bench/scripts/fill-ref-live.ps1`, ComfyUI on the 5090 (`--fast fp8_matrix_mult`),
the release CLI, one process per run (so each 2511 run below includes the model switch from
Krea 2 or the previous tier). Two Krea 2 pictures (a woman in a light grey cardigan; a red
cable-knit sweater on a person, 1024², 12.8 s and 6.0 s), the cardigan selected by text (SAM
3.1 on ComfyUI), then `generate.fill` with the sweater picture as a reference, `grow: 8`,
seed 7. The request: 928×816 (0.76 MP).

| Run | Template | Run time | What came back |
|---|---|---|---|
| typed "the red cable-knit sweater" | `fill-ref` (40 steps, CFG 4) | 150 s | the reference's cable knit, colour and ribbing on the woman, in the cardigan's silhouette (V neck: the selection decides the cut) |
| `enhance: true`, "the red sweater" | `fill-ref` | 150 s | the rewriter, shown the pair, wrote "red cable-knit sweater, crew neck, long sleeves, ribbed cuffs and hem, loose fit, textured pattern running down front and arms, worn over white top."; the picture differs from the typed run in 166 k pixels but reads the same |
| plain fill, "a red cable-knit wool sweater" | `fill` (40 steps) | 88 s | **the cardigan recoloured red**, its buttons and cut kept: the model copied what it saw under the mask, the failure the grey hole prevents |
| typed | `fill-ref-lightning-8` | 27.9 s | the same garment as the 40-step run (258 k pixels differ, mostly knit detail) |
| `enhance: true` | `fill-ref-lightning-8` | 29.2 s | the same |

Lessons: the grey hole is what makes a swap a swap; the selection's shape is the garment's
shape (select everything the new piece will cover, `grow` for the rest; the V stayed because
the white top inside it was not selected); the Lightning tier is the sensible default for a
swap (28 s against 150 s, the same garment); one reference on a fill costs about what it costs
on an edit (benchmarks §5g). Not yet judged over more seeds, and `erase: false` on the
reference template is untested live.

## 5j. The Remove tool in the app, per stage (ShotoPhopEX, 2026-11-01)

The owner's app and preferences (Quality fast, preview Sharp, Fast draft on, Auto: qwen-2.1/edit,
25 steps, two variations at up to 0.75 MP), generative `paint.remove` with seed 42 over the stock
photos' objects (`robust-strokes.ps1`'s serpentine), stage times from the job's progress messages
every 100 ms, RTX 5090.

| Case | warm | after a Krea 2 image, before #19 | after #19 | after #19 and #20 |
|---|---|---|---|---|
| cone (small object) | | | | 12.2 s |
| ring (small worn item) | 4.6 s | | | |
| hat (worn item) | 4.6 s | 24.7 s | 15.8–19.1 s | 12.4 s |
| watch (wearer: four names tried) | 5.7 s | 27.3 s | 17.8–17.9 s | 13.6 s |
| bicycle (large) | 11.7 s | | | |

Warm: naming (the 8B's chat) 0.3–0.7 s, SAM 0.7 s to load and 0.2 s a find, each 2.1 edit about
1 s (2.9 s for the bicycle's 0.75 MP), the judge 0.2 s. After an image the swap reloads the 2.1
text encoder (5.0–7.2 s), the DiT (3.1 s), VAE and vision tower (1.0 s), SAM (1.1 s) and the chat
head (10.2 s before genai-native #19, 1.5 s after: its Q8_0 now quantized on every core). The
results before and after #19 are byte-identical.

genai-native #20 then faults each load's pages in on every core before uploading from the mapping
(`tests/load_bench.rs` on the 16.3 GB Qwen3-VL 8B, in the file cache: candle's mapped VarBuilder
5.0 s at 3.4 GB/s, prefaulted on 8 threads 2.34 s). In the app the 2.1 text encoder went from
4.5–6.1 s to 2.6–2.8 s, the DiT from 3.0–3.4 s to 2.4 s, the vision tower from 0.65 s to 0.45 s;
Krea 2's reload logs 1.5 s for its encoder and 2.9 s for its DiT. Removals and seeded Krea images
byte-identical before and after.

genai-native #21 keeps SAM 3.1's read checkpoint (1.6 GB of host memory) when the perception
engine gives the device back: SAM's reload on each later Remove stroke went from 0.7–0.8 s to
0.3 s (reading the checkpoint was 0.8 s of it; the first load after a launch stays 1.1 s). #22
faults the Layered and 2511 checkpoints' pages on every core: Split at 640 px after a 2.1 edit,
warm, three runs each, the DiT ready at 8.8–10.4 s instead of 10.9–11.8 s and the Split done in
18.2–20.3 s instead of 20.2–21.3 s.

## 5i. Content-Aware Fill, Remove and Spot Healing through the model (ShotoPhopEX, 2026-10-21)

The lighthouse picture's red boat (`final-fill-s11.png`, 1024², the 200×120 rectangle at
300,600 that `edit-live.ps1` edits), ComfyUI on the 5090, the release CLI, one process per
run. First the classical fills against the model (`caf-generative-live.ps1`, Quality `best`:
the 40-step 2511 fill):

| Run | Time | What came back |
|---|---|---|
| Content-Aware Fill, classical (PatchMatch, auto sampling) | 0.4 s | boat gone; a copied rock blob in the water and a ghost of the horizon |
| Content-Aware Fill, generative, the hole flat grey | 56 s | **the grey rectangle, painted back as a panel** |
| the same, pixels kept, "fill the flat grey area" wording | 44 s | the boat, untouched (there was no grey to fill) |
| Remove tool, classical (non-local patches) | 0.4 s | boat gone; rock copies at the stroke's end |
| Remove tool, generative, the stroke's hole grey | 47 s | boat gone, the sea continued cleanly |

Then the instruction matrix on the Lightning tier (`caf-matrix-live.ps1`, Quality
`balanced`, seed 7, the same rectangle):

| Variant | Time | Result |
|---|---|---|
| A: pixels kept, "Remove the object in the selected area and fill its place with the surrounding scene continued seamlessly…" | 11.6 s | boat gone, the sea and the horizon continued |
| B: the mask-guided template (`fill-guided`), pixels kept | 21.5 s | the same |
| C: pixels kept, the object named ("Remove the red boat…") | 15.9 s | the same |
| D: the hole grey, "Fill the flat grey area…" | 5.9 s | the grey panel |
| E: the guided template with the hole grey | 21.6 s | the grey panel |

So the removal path keeps the pixels and sends A's generic wording (`REMOVE_PROMPT`): 2511
reads the picture under the mask as truth and must be told what to do with it, and a flat
hole is "content" to it (the grey-hole trick serves a swap from a reference, where the
reference says what goes there; AnyPaint's model card says the same of Krea 2 on dark or
saturated pictures). Naming the object, which would cost a VLM pass, buys nothing here. One
picture, one seed: judge on more before trusting the margins.

**The stroke itself** (later the same day, `spot-generative-live.ps1`: the strokes through
`paint.spotHealing {type: generative}`, so what the app sends is what is measured; the boat is
145×37 at 308,649):

| Stroke | First version (hole = the stroke) | Now (object whole, grown 45, hole all-or-nothing) |
|---|---|---|
| S1: hard brush 56, along the whole boat | boat gone, a dark smudge of its reflection left under the hull | clean |
| S2: soft brush (hardness 0.3), the same | the smudge | clean |
| S3: hard brush 36, the middle of the boat only | **the boat redrawn, ribbed** (the user's report) | clean, the boat gone whole |

What each step was worth, measured on the way (`caf-prefill-live.ps1`, since dropped;
`caf-shape-live.ps1`; `caf-grow-live.ps1`; seeds 7 and 8, Lightning):

| Variant | Result |
|---|---|
| the hole pre-filled classically (PatchMatch), REMOVE_PROMPT or a "make it seamless" wording | **worse**: the model keeps the patch fill's copied rocks as a smear; a small hole on the bow comes back as a rock-coloured bow. 2511 treats what is under the mask as truth, so a pre-fill must be right or absent |
| the detector's silhouette of the boat (`select.byPoint` on the stroke's spots) grown 15 | the boat stays, both seeds |
| its bounding box grown 15 | the boat stays, both seeds: not the shape |
| the matrix's 200×120 rectangle (35–50 px past the boat) | gone, both seeds |
| the silhouette grown 30 | gone at one seed of two |
| the silhouette grown 45 or 60 | gone, both seeds |
| a soft brush's partial coverage in the noise mask | the boat stays: partial mask values keep that much of the rim |

So a removal (1) asks SAM 3.1 for the object under the stroke's spots (one point prompt, as
the Quick Selection tool) and takes it whole when the stroke covers at least a quarter of it
and it is under 40 % of the canvas, (2) makes the hole all-or-nothing wherever the brush
touched, (3) grows it by 30 % of its larger side (32 to 192 px), (4) runs the fill with the
pixels kept. The detector adds 0.7 s warm and about 4 s on the first ask (model load); a
whole stroke is 20–22 s wall on the Lightning tier with the server switching between the two
models. Content-Aware Fill's generative switch takes the same growth (the user's selection
is usually tight) but never asks the detector.

**A necklace** (the same night; `necklace.png`, a portrait the app's own text-to-image made in
7.4 s: a 6-px gold chain over the neck, collarbones and the neckline of a dark top, with a
small pendant; Select by Text finds it as 5296 px in a 409×297 box). The silhouette from
Select by Text under `generate.fill`, Lightning, seeds 7 and 8 (`necklace-grow-live.ps1`,
`necklace-name-live.ps1`):

| Instruction | grown 32 | grown 64 | grown 96 | grown 123 (30 %) |
|---|---|---|---|---|
| "Remove the object in the selected area…" | stays, stays | **stays, clean** | trace, stays | **clean, stays** |
| the same, rewritten by the vision-language model with the picture in view ("Remove the gold necklace and replace it with the surrounding skin, collarbone, and dark grey fabric…") | | clean, clean | | clean, clean |
| named by hand ("Remove the gold chain necklace and its pendant completely, leaving bare skin…") | | clean, clean | | clean, clean |

Unnamed, the necklace is a coin flip however far the hole is grown: the model's prior that this
portrait wears a necklace beats "the object". Named, every run is clean, and on the boat the
named instruction also removes it with the hole grown only 15 or 30 (`caf-grow-named-live.ps1`,
both seeds), where the generic one needed 45. So a removal now runs through the rewriter by
default (`enhance`, +3 s wall), and the growth went down to 15 % of the larger side (16–96 px).

Through the tools (`necklace-strokes-live.ps1`; the rewriter's words in each case):

| Stroke | Result |
|---|---|
| the Remove tool along the whole chain ("Remove the gold necklace and replace it with the woman's skin, collarbone, and dark grey shirt neckline…") | **clean**: chain and pendant gone, skin and neckline natural |
| Spot Healing along the left half of the chain ("Remove the gold pendant and chain…") | the left half and the pendant gone, the right half stays with a loose end: the detector's point prompt returns only the brushed half of a thin chain, so "the object whole" was half a necklace |
| Spot Healing, one 40-px dab on the pendant ("Remove the gold disc pendant… keeping the necklace chain intact") | the pendant gone, the chain joined across the gap. The first run left a lighter, smoother patch of fabric: a 108-px request sent at 512 (4.7×) gives the model no context; a removal's request is now at least 384 px a side |

Each tool stroke is 23–25 s wall on Lightning: the detector, the rewriter and the fill, with
the server loading each in turn.

**Asked again by name** (`remove-note-live.ps1`, `necklace-strokes-live.ps1` rerun). The
rewriter's sentence names the object ("Remove the gold necklace and replace it…" →
`named_object` → "gold necklace"), and the detector is asked a second time by that name (the
text template, every instance that the stroke covers a quarter of):

| Stroke | Before | Now |
|---|---|---|
| Spot Healing along the left half of the chain | the left half and the pendant gone, the right half left with a loose end | **the whole necklace gone** (the name ask returns the whole chain, 5296 px; the hole grows 64) |
| one dab on the pendant | the pendant gone, the chain joined | the same ("gold disc pendant": the dab covers all of it, the chain is not asked for) |
| the Remove tool along the whole chain | clean | clean |
| the boat's three strokes | clean | clean; one soft-brush run of seven put a rock where the boat was (the rewriter's "ocean and rocky shore… rock textures"), the other six are sea |

A removal note of its own for the rewriter ("name that object precisely, then only what lies
directly behind it") was tried against the rock and dropped: it named "the gold circular
pendant" where the fill note named "the gold necklace", so the name ask found less and the
half-necklace stroke regressed to half; the boat was clean either way. On a fresh server the
three asks come to 14–17 s wall a stroke on Lightning (the first stroke after a start 40 s).

**The robustness set** (the same day; `robust-make.ps1` makes fourteen scenes with the app's
own text-to-image, 1024², seed 31, and locates each object by Select by Text;
`robust-strokes.ps1` runs two strokes a scene through the tools' own commands, Spot Healing
along the first half of the object's long axis and the Remove tool in a serpentine over its
whole box; `robust-sheets.ps1` makes the sheets; every run's rewrite, growth and time go to
`bench\rb-log.txt`). Six rounds of refinement, the fixes in the dev log; what each round's
sheets showed:

| Round | Clean of 28 | What failed |
|---|---|---|
| 1: as committed before the set | (half strokes only) none whole | the quarter-of-the-object rule rejected every half stroke over a thick object; the rewriter named the part under the stroke ("the window", "the dog's head", "the hand") |
| 2: a tenth, the stroke outlined in green for the rewriter | 24 | "the green circle", "the green-painted face"; the Remove tool's serpentine on the logo removed the shirt, on the glasses the face (the point prompt's segment) |
| 3: two views side by side, name first, point prompt the fallback | 22 | the watch's place filled with the table, both strokes; a panel over the eyes (Remove); a lighter patch of sky where the crow was; the mug's rim without its coffee |
| 4: the name alone in the generic wording, tone matched at the edge, enclosed gaps filled | 25 | the watch clean (seed luck), the glasses half a wall panel, the crow's place blue on a grey sky, the cup's coffee named and emptied |
| 5: two seeds, the better edge match shown | 26 | a second wire drawn under the first; "the painted-over area" as a name |
| 6: the rewriter judges the two side by side, selection-names rejected | 26 (final run) | a faint circular seam where the mug's half stroke met the pattern; the crow's claws left on the wire |

The final run, all fourteen scenes clean at both strokes but those two: watch, glasses,
earring, tattoo, logo, car, person, lamp post, sign, crow, mug, ring, poster, dog, the
rewriter naming every one right ("silver watch", "black-framed glasses", "silver hoop
earring", "black rose tattoo", "red circular logo", "blue car", "man", "black lamppost", "blue
sign", "black bird", "white coffee cup", "gold ring", "white poster", "golden retriever").
On the glasses seed by seed (`glasses-seeds-live.ps1`), seeds 1–3 were clean and seed 4 a
smeared blob over one eye; the edge score chose the blob (its colour and smoothness matched the
cheeks), the judge chose seed 3. A stroke is 20–27 s wall on the Lightning tier (the name, the
detector, two fills, the judge; three outliers of 40–143 s were the server reloading).

**The stock set** (the same day; the generated set is made by a model of the same family the
fill model was trained alongside, so it looks the way that model expects). `stock-fetch.ps1`
takes one photograph per subject from Wikimedia Commons by its search API, only files whose
own metadata gives the licence as CC0 or Public domain, at 1600 px on the longer side, and
lists each file's Commons page, author and licence in `bench\stock\st-sources.txt`;
`stock-objects.ps1` locates the object by Select by Text (the search hits were not all on
subject: the "watch" is a man in a mask on a 1950s ship, the "necklace" a tiara in a
stereograph, the "cat" the Liddell sisters on a sofa, the "hydrant" a church with a small car,
the "bottle" a glass filling its frame); the same `robust-strokes.ps1` and `robust-sheets.ps1`
with `-Dir`, `-Prefix st`, `-Objects st-objects.json`. Twenty-six scenes: a Roman fresco, a
stereograph, a daguerreotype in its case, a sepia print, a night street, a kite festival, a
flock on a wire, a sunset silhouette, a museum hall, a subway hand.

Lightning tier, two seeds, as shipped for the generated set:

| | Scenes |
|---|---|
| clean at both strokes | the mask (a face invented in its place), the ring, the spider-web tattoo, the latte cup on wood, the dog on the lawn, the lamp post, the bicycle on cobbles, the traffic cone, the top hat (hair invented), the trash can beside the bench, the cow in the grass, the tiny bird in the flock |
| one of two | the boat silhouette (half a boat from the half stroke, the whole stroke clean but for a dot), the umbrella (the head missing under the half stroke, whole clean), the bear kite (gone from the half stroke, back on the whole) |
| neither | the transparent-framed glasses on the old man (redrawn both times), the tiara (named "the black frame"), the heart sign at night (Select by Text found a 66-px "poster" elsewhere), the two people on the beach (ghosts), the Liddell print ("the selected object"), the Wall St sign (gone, under a grey blur), the motorcycle (a grey placard in its place) |
| not a test | the glass that fills its frame; the church's small car (not judged) |

So 27 of 45 judged strokes, against 26 of 28 on the generated set. Two kinds of failure:
the rewriter and detector losing the object on old framed prints (the frame named instead of
the tiara; "selected object"; no name for the fresco's earring), and the eight-step fill model
itself leaving or redrawing an object, or blurring its place, on grainy and busy real scenes.
Names that are a frame, a border or "the object" are rejected now, as "the selected area"
was.

The nine failing scenes' half strokes again on the **40-step tier** (`-Quality best`,
`-Suffix -best`, 110–164 s a stroke against 20–28): the same result on every one, the
glasses, the tiara, the heart sign, the beach ghosts, the Liddell print, the Wall St blur and
the motorcycle's placard all as before, the kite gone as before. The step count is not the
lever on real photographs; the fill model's capacity at its working size is. The next lever is
a different fill model: the native Krea 2 port with AnyPaint's inpainting recipe (plan §9).

The stock photographs' sources, authors and licences are kept in
`bench/stock-sources.txt` (local).

**What moved the stock set** (the same night, the nine failing scenes' half strokes under each
change, `-Suffix` runs of `robust-strokes.ps1`):

| Change | Result on the nine |
|---|---|
| the 40-step tier | the same nine outcomes at five times the time |
| Qwen-Image-2.1's fill (research licence) | the same, but the motorcycle removed cleanly |
| the whole-crop **edit** instruction, no noise mask (the user's suggestion: let an edit model edit) | the same nine outcomes |
| **the hole smoothed over** in the picture the model sees (`prefill`: a membrane from the hole's edge, `smooth_covered`) | the glasses gone, the tiara gone, the left walker gone, the motorcycle gone with the floor continued, the kite gone; the sign still a blur, the heart (a mislocated test) still there |

The three model-side changes did nothing because an edit model copies what its reference
shows, however it is asked; the smoothing takes the object out of the reference. (A flat grey
hole was painted back as a panel and a PatchMatch pre-fill's copied blobs were kept, both
measured earlier the same day: the model also copies what the hole is filled with, and a
smooth continuation is the one thing it has no reason to keep.) Both sets again with the
smoothing (`smooth-run.ps1`, the sheets `st-smooth-*.png`, `rb-smooth-*.png`):

| | Baseline | Smoothed |
|---|---|---|
| stock, half strokes (24 judged) | 13 | **16** |
| stock, whole strokes (24 judged) | 14 | **19** |
| generated, both strokes (28) | 26 | 26 |

New on the stock set: the transparent-framed glasses at both strokes, the tiara at both, the
mask with a consistent face, the Wall St sign on the whole stroke (the facade continued
instead of a blur), the left beach walker, the kite and the motorcycle on the whole stroke,
the bench itself. Still failing: a faint spider web left on the whole-stroke tattoo, a pale
blob under the bicycle, the fresco earring (repainted wider), the heart sign (the test's
66-px "poster"), and the half strokes that leave a boat's second figure, a headless figure
under the umbrella and the Liddell print untouched. So `prefill` is on by default for the
three removal commands.

**The fill's own settings** (the same night, `tune-run.ps1`; `generate.fill` and the removal
commands take `sampler`, `scheduler` and `maxPixels` now, the 2511 fill templates bind the
KSampler's sampler and scheduler). Eight whole-object strokes (the mask, the dog, the bicycle,
the top hat, the cow and the kite from the stock set; the watch and the mug from the generated
set) under each variant against the shipped default (Lightning 8, euler/simple, 0.75 MP, the
hole smoothed), the sheets `tune-<variant>-{st,rb}.png`:

| Variant | Time a stroke | What changed against the default |
|---|---|---|
| the request at 1 MP | 20–31 s (+15 %) | the bicycle's puddle smaller and the kite's line finer; the mask's face a different plausible face; nothing worse. The server stayed resident (no offloading spike: the 0.75 MP cap's reason no longer holds on the Lightning tier) |
| the request at 1.5 MP | 20–37 s (+35 %) | the bicycle clean, the hat's man sharper than his daguerreotype; **a seam rectangle in the dog's lawn** |
| `res_multistep` sampler | the same time | worse: a patch in the lawn, a patch of other grass for the cow, a mane of hair for the hat, **a blue kite invented** |
| `beta` scheduler | the same time | the bicycle clean, the dog and cow clean; a bow-tied gentleman invented for the hat, a beard for the mask: more "generated" |
| 40 steps at CFG 4, 1 MP | 96–188 s (×5) | worse: the mask's face blurred, the hat's face **erased to a blur**, a seam in the lawn; the bicycle clean |

Nothing in the sampler, scheduler or step count gives a consistent realism gain over
ComfyUI's official pair on this tier; the 40-step base tier is worse under the smoothed hole,
not better. The request size is the one lever with a small consistent gain at a small cost,
so the default cap is **1 MP** now (`MAX_FILL_REQUEST_PIXELS`), `maxPixels` for anything
else. The "realistic Qwen outputs" seen elsewhere are whole-picture generations and edits at
1 MP and up with long prompts; a removal is a crop with the object taken out, and its realism
is bounded by what the model can continue from the edge.

Since 2026-10-26 the removal commands therefore run at `balanced` even when the Quality
preference is `best` (`generate_cmds::removal_quality`): in the app, where Quality defaults
to `best`, a Spot Healing dab had been two 40-step fills, 98 s measured against 20–28 s
here, and read as the tool not working. `fast` still halves the candidates' tier.

**Where a Lightning fill's 25 s go** (2026-10-26, `same-graph-probe.ps1`, the app's
preferences, the lighthouse's boat at a 512×400 request, the card clean): the sampler's
eight steps take 6 s (1.3 it/s); the server's "Model Initializing" before the first step,
ComfyUI 0.39's dynamic loader re-staging the 19.6 GB 2511 with its LoRA patches, takes
**10–12 s on every prompt**, the same graph run twice in a row included (10.7 s the second
time); the text encoder, the VAE and the transfers take the rest. The second candidate of a
Best-of-two pair, with no VLM call in between, re-stages in 5.7 s (720 patches after 840). So
a dab is 40 % staging that no setting of ours removes. Launch flags measured against it
(`flag-probe.ps1`): `--disable-fast-disk` worse, `--highvram` stalled, `--disable-dynamic-vram`
unusable (`comfyui-setup.md`'s table). The staging is the loader's design; the way past it is
the native engine, which holds a model resident between runs.

**Could a dab skip the rewriter?** (the same day, `noname-run.ps1`: both sets, one candidate
a stroke, `enhance=false` against the default, the sheets `st-noname-*.png` and
`rb-noname-*.png`). The vision-language model is what evicts the 2511 between dabs, and a
one-candidate removal uses its answer only to name the object for the detector and the
prompt. **No.** Without the name the half strokes leave the object standing on most of both
sets (the glasses, the tattoo, the Wall St sign, the cup, the dog, the lamppost, the cat, the
bicycle, the boat, the cone, the cow, the hydrant, the motorcycle; the watch, the glasses, the
earring, the logo, the car, the crow, the mug, the poster, the retriever) because the point
prompt returns the brushed part only, and the whole strokes over-take where the point lands
on a bigger segment (the man behind the glasses replaced by a wall, the shirt under the logo
by concrete, the hat's whole daguerreotype erased, the mug a red blur, a lawn's worth of the
dog's park). The saving was 5 s a stroke, not the 10 s of staging: each bench stroke is its
own process and reloads the detector too. The naming stays; a one-candidate dab is 22–25 s
on this card with the tenants, the floor of the ComfyUI route. (The 2511's own encoder, which
is resident, cannot generate through `TextGenerate`: measured 2026-10-09, `enhance-edit.json`.)

**Qwen-Image-2.1 as the removal model** (2026-10-26, the owner's question "do we need 2511?";
the native worker, one candidate a stroke, against the 2511 one-candidate run `-one`; sheets
`st-n21-*.png`, `st-e21c-*.png` in `bench\stock`):

| Route | Strokes | What it did |
|---|---|---|
| `qwen-2.1/fill` (the mask shown as a second picture, `native21-run.ps1`, rewriter on ComfyUI's 4B) | 33 of 78 (the worker's CUDA context died at the 34th: an illegal address on a full card; recovery since added) | **worse than 2511 on every hard scene**: a grey panel over the glasses, the spider-web tattoo redrawn as a spider, a new cup of coffee, the retriever back, the beach walkers half there; fine on the ring, the sign, the birds, the lamppost. 2.1 copies what its pictures show, the mask included |
| `qwen-2.1/edit` + AusBoss's Consistency LoRA (the removal as an instruction on the crop, `edit21-run.ps1`, rewriter on the worker's 8B) | 20 | **unusable as run**: the smoothed hole (`prefill`) came back as the blur it was, and the 8B rewriter named the person instead of the thing ("the man's face", "the man wearing the cap and mask", "the person under the umbrella"), so faces and people went. Only the hat came out right |

Also: with the 2.1 set, the head, the LoRA and the card's other tenants, the worker ran at
0 GB free and sampling paged (6 s became 26–123 s).

**The 2.1 edit route done AusBoss's way** (the same evening; the ten hard stock scenes, half and
whole strokes, 20 a run; sheets `st-e21p-*`, `st-e21pc-*`, `st-e21n-*`, `st-p21-vs-auto3-*`,
`st-auto4-*`, `st-auto5-*` in `bench\stock`). The real picture in the model's view (`prefill`
off; now the rule for every 2.1 template), the crop at 0.75 MP, realigned. My count of clean
removals:

| Route | Rewriter | Candidates | Clean, of 20 | Time a stroke |
|---|---|---|---|---|
| 2511 fill (`-one`) | ComfyUI 4B | 1 | 12.5 | 22–35 s |
| 2.1 edit (`-e21p`) | ComfyUI 4B | 1 | 14 | 10–170 s (paging, before the pool trim) |
| 2.1 edit + Consistency LoRA (`-e21pc`) | ComfyUI 4B | 1 | 13.5 | 20–420 s (the same) |
| 2.1 edit (`-e21n`) | the worker's 8B, reworded | 1 | 13 | **9–30 s**, no ComfyUI |
| **2511 shipped** (`-auto3`: adapter + smoothed fill, judged) | ComfyUI 4B | 2 | **14** | 43–62 s |
| 2.1 edit (`-p21`) | the worker's 8B, judged by it | 2 | 12 | **10–38 s**, no ComfyUI |

2.1 wins the top hat and the umbrella (the person under it kept), loses the cup (a dark smudge
where it stood) and, with the 8B, the strokes where the 8B names the person instead of the
thing ("the man's face and glasses", "the man wearing the cap and mask"). The Consistency LoRA
helps small removals and brings big ones back (the motorcycle, the half-stroke bicycle), as its
author warns. So **2511's Best of two stays the removal default**; 2.1 as an edit is a close
second at twice the speed with no ComfyUI, and the 8B's naming is what separates them.

The rewriter's wording now says the object is the thing itself, not the person or animal that
wears, holds or carries it. On 2511 with the 4B (`-auto4`, `-auto5`): the whole-stroke glasses
are named "glasses" (was "white beard"), the boat and the umbrella come out cleaner, nothing
lost once "the thing the close-up is centred on" was dropped from the wording (it made the 4B
say "the black bicycle seat"). The 8B still names the wearer on two scenes.

**The rewriter's views per model** (the same night; `naming-probe.ps1` for the names alone,
three seeds, then `views-run.ps1` for the removals; sheets `st-e21v-*`, `st-p21v-*`,
`st-auto6-*` in `bench\stock`). The 8B named the wearer because for a small object the
surroundings and the close-up were one crop. The close-up's margin, a share of the grown
stroke's side:

| Margin | Worker 8B | ComfyUI 4B |
|---|---|---|
| 0.25 (was both) | the mask's whole stroke: "the man wearing the cap and mask", 3 seeds of 3 | right on every scene but the boat |
| 0.10 | the mask, the glasses, every scene but the boat's people | names parts: "the silver chain", "the black bicycle handlebar", 3 of 3 |
| 0.15, surroundings 2.5× the close-up | right but for the boat | "the white beard" for the glasses, 2 of 2 |
| 0.25, outside the stroke dimmed | "the selected area showing the man's face", "the floor" | (not run) |

So 0.1 when the worker's 8B answers, 0.25 for the 4B (`naming_views`), and the close-up keeps
its aspect beside its surroundings. On the removals, clean of 20:

| Route | Before | Now | Time a stroke |
|---|---|---|---|
| 2.1 edit, worker 8B, one candidate (`-e21n` → `-e21v`) | 13 | 14 | 6–29 s |
| 2.1 edit, worker 8B, two judged (`-p21` → `-p21v`) | 12 | **15** | 6–28 s |
| 2511 shipped, ComfyUI 4B (`-auto4` → `-auto6`) | 13 | 16 | 33–58 s |

Gained on 2.1: the glasses' whole stroke (was a blurred face), the mask's half stroke, the
boat's whole stroke with its hull. Still lost: the mask's whole stroke (the real crop's extra
face pixels flip the 8B to the man; the probe's near-identical crop says "mask" 16 of 16),
the cup (2.1 continues the table's shade into a dark patch), the half strokes on the boat and
the motorcycle. The dog's cast shadow stays on every route.

**2511 retired as the default where 2.1 runs natively** (2026-10-27, the owner: "it's an older
model, and it's way slower"). On the numbers above (2.1 as an edit 15 of 20 at 6–28 s, 2511 16
at 33–58 s through ComfyUI) `auto` now runs Qwen-Image-2.1 on the native worker for removals
(`qwen-2.1/edit`, the crop at 0.75 MP as measured), prompted fills (`qwen-2.1/fill`), expands
and edits, whenever research-only models are allowed and the worker runs the family
(`generate_cmds::native21_auto`); without the opt-in the 2511 orders stay the permissive
default. Prompted fills had not been compared before; the fill suite's 8 cases through the
app (`runs/2026-10-27-app-fill-qwen-21fill` against `references/2026-10-18-comfy-fill`, sheet
`fill-2511-vs-21.png` in the session scratchpad):

| Case | 2511 (ComfyUI) | 2.1 fill (native) |
|---|---|---|
| fox, knitted red hat, seeds 7 and 11 | a hat both times | a hat both times (a bobble beanie) |
| sky, a small white cloud | a cloud | a smaller, fainter cloud |
| compass, background continued | a purple fringe left | clean |
| stormy sea, a rowing boat | a boat on the rocks | **no boat** |
| buoy, seagull (transparent fixtures) | a noise square around the object | a clean square of water / sky around it |
| earring (the fixture is a dish, not a portrait) | nothing | nothing |

At 1.6–5.8 s a case on the worker. The removal weak spots stay as listed: an object over a face
(the mask's whole stroke) and the cup's dark patch.

**Swaps** (2026-10-27, the owner's "outfit swap or item swap"; `swap-cases.json`,
`swap-run.ps1`, `swap-sheets.ps1`; sheets `bench\swap\sheet-{item,outfit}-s{7,11}.png`). 15
cases at seeds 7 and 11: 11 item swaps on the stock set, 4 outfit swaps (three people made with
Krea 2 here, one stock). Each object selected by name with SAM 3.1 (`select.byText`, grown 12 px,
holes filled), the rewriter off; the fill routes get the description ("a glass of red wine"),
the edit route the instruction ("Replace the coffee cup with a glass of red wine."). My count
of clean swaps (a half for a swap with a visible fault):

| Route | Items (of 22) | Outfits (of 8) | Total (of 30) | Time a case (CLI, SAM included) |
|---|---|---|---|---|
| 2.1 fill (`qwen-2.1/fill`, native) | 19 | 4.5 | 23.5 | 5–19 s |
| **2.1 edit** (`qwen-2.1/edit`, native, masked to the selection) | **21** | 6 | **27** | 13–15 s |
| 2511 fill, Lightning 8 (ComfyUI, the old default) | 6.5 | **8** | 14.5 | 22–47 s |

- **2511 keeps the old object** on item swaps: the cup with a wine stain, the mug inside the
  glass, the dog, the bicycle, the hydrant, the motorcycle under a half scooter, the old bench
  behind the new one. It sees the object under the mask and continues it (the request is not
  greyed: `erase` is off by default; not tried here). On garments, where the old shape is the
  new one's, it was clean every time.
- **2.1 fill** misses the B/W leather jacket both times (the denim kept) and the second cup
  (the latte kept); a dark smear at the raincoat's hem on one seed.
- **2.1 edit** swaps every item but the watch (a bracelet on one seed); its faults are at the
  selection's edge: the raincoat runs past the jacket's mask and is cut off in a dark band at
  the hem (both seeds), and the suit's sleeve blurs where the mask ends (fill21 too). A wider
  selection for garments, or the result's edge softened over more pixels, is the next thing to
  try.
- **Krea 2 edit** (added the same night: `krea2-turbo/edit`, native, conradlocke's General Edit
  V2 LoRA at strength 1, `ref_boost` 4, 8 steps, genai-native `f56189b`): **22 of 22 items, 5
  of 8 outfits, 27 of 30**, at 15–23 s a case. The only route that swapped every item (the
  watch became a gold bracelet on both seeds, a proper mug with a handle, the postbox, the
  scooters); misses the B/W leather jacket on both seeds (a patterned shirt instead) and shares
  the 2.1 edit's raincoat-hem band (the app's selection mask cutting a longer garment; the same
  edit straight through the pipeline, unmasked, is clean). Slower than the 2.1 edit (the
  picture's 4 096 tokens ride along every step: the LoRA was trained without a reference KV
  cache, so they cannot be computed once).

**The edit band, then fills with the rewriter** (the same night). With `change_band` on edits
and whole-picture fills (`bbcd240f`) the Krea 2 edit's raincoat came out whole; the 2.1 edit's
still had its hem band until the band counted only strong changes (`EDIT_BAND_CHANGE` 0.25:
2.1 also re-renders the jeans faintly, which ran past the reach and refused the band); after
that the 2.1 edit's raincoat is whole (a long coat) and its suit sleeves clean, items unchanged.
The app's own flow with the rewriter on (`fillenh`, the 2.1 fill template and its fill
rewriter) swapped every case but the watch on both seeds, the B/W leather jacket included;
fill by instruction (`fillnamed`) left three unchanged because the 8B named a neighbour of the
selection ("Replace the dark car with a yellow hooded raincoat" for the denim jacket, "the
black trash can" for the bench, "the white lifebuoy" for the jacket), so `fillAsEdit` stays off
by default.

**Fill from a reference and image from references, natively** (Rust rebuild R2, 2026-10-27):
on the 2026-10-21 pictures (a woman in a grey cardigan, a red cable-knit sweater as the
reference), the 2.1 edit template by instruction swapped the cardigan for the sweater as the
reference shows it, typed ("Replace the selected part of image 1 with …") and with the rewriter
("Replace the grey cardigan with …"), level with 2511's two results but for a faint ghost of
the old neckline; 21–23 s through the CLI. `qwen-2.1/image-ref` made "a store mannequin
wearing the red cable-knit sweater from image 1" with the reference's knit, 11 s at 832×1216.

**Generative Angles natively** (Rust rebuild R2, 2026-10-27, whole-picture edits through the
CLI, seed 7, `bench\angles\`): four pictures (the denim and t-shirt people, the CC0 bench and dog)
on the native worker. Naming the view ("show the same scene as a right side view, eye-level
shot") made the 2.1 edit turn the subject in place; naming the camera's move ("Rotate the camera
90 degrees to the right around the subject", "Move the camera behind the subject", "Raise the
camera high above…", "Move the camera closer for a close-up") orbited the bench (its side, its
back with the bin now in front), raised the camera over the people and the dog and closed in on
all four, 5.5–6.7 s each warm. Krea 2's General Edit V2 on the same wording barely moved the
camera and invented content (people in a garage, a jacket on the dog), so it is not a route.
`generate.angles` under `auto` with the opt-in and the worker now sends the 2.1 edit that
instruction (`angle_instruction`). Not compared with 2511 + the Multiple Angles LoRA: the LoRA is
not installed on this PC.

| Route | Items | Outfits | Total | Time a case |
|---|---|---|---|---|
| Krea 2 edit (General Edit V2) | **22** | 5 | **27** | 15–23 s |
| 2.1 edit | 21 | 6 | **27** | 13–15 s |
| 2.1 fill | 19 | 4.5 | 23.5 | 5–19 s |
| 2511 fill, Lightning 8 | 6.5 | **8** | 14.5 | 22–47 s |

**The removal adapter** (2026-10-22; `lora-run.ps1`). prithivMLmods' Object Remover v2, an
Apache-2.0 LoRA on 2511 trained to delete what is painted red (`models.md`), as the
`qwen-edit-2511/remove-lightning-8` template: the Lightning graph with the adapter loaded
after the Lightning LoRA, the hole painted solid red, the adapter's trigger sentence "Remove
the red highlighted object from the scene." Both sets through it as the named template, the
hole grown as for any removal (15 %, 16–96 px), against the smoothed default:

| | Better than the smoothed fill | Worse |
|---|---|---|
| stock | the fresco's earring (a clean wall), the spider-web tattoo (gone, no trace), the bicycle (no puddle), the Liddell print (the sofa continued), the umbrella (the figure kept its head), the hat (a plausible head), the beach (both walkers gone on the whole stroke) | **red left in the hole** on the glass that fills its frame, the church's car (a red smear on the fence), the museum motorcycle (the whole hall tinted); the glasses and the tiara stayed on the half strokes |
| generated | nothing better (the smoothed fill was already clean) | **red left** on the lamp post's sky, the crow's sky, the gingham cloth, the dog's hedge: every hole grown to the 96-px cap |

The adapter removes what the plain fill cannot, and leaves its red where the painted area is
far bigger than the object: it was trained on object-shaped highlights, and the removal's
15 % growth (96 px on a big object) hands it a slab. So the shipped path (`auto`, two
variations) now runs **the adapter on the selection grown 6 px as the first variation and the
smoothed plain fill as the second**, and the judge picks; any red left is what the judge is
there to see. Measured on both sets below (`auto2-run.ps1`).

**The judged pair, first round** (`auto2-run.ps1`, the sheets `st-auto2-*`, `rb-auto2-*`):
the judge chose the adapter's result in 49 of 78 strokes. Against the smoothed-only run:

| | Won | Lost |
|---|---|---|
| stock, whole strokes | the tattoo (gone), the Liddell print, the bicycle, the umbrella, the bench, the tiara | the glasses kept, a boat with one figure kept, **the red-tinted glass chosen**, the motorcycle hall blurred |
| stock, half strokes | the hat, the hydrant | a white saucer left where the half-removed cup was |
| generated | nothing to win | **the crow kept** on the half stroke, a ghost of the poster's text |

A wash in count, and two kinds of loss the judge should have caught: it kept a crow on its
wire and a boat on its lake because both look natural, and it took a red-tinted glass. So
the judge is now told what was to be removed ("The crow was to be removed. In which one is
the crow gone and the scene untouched?") and that flat paint is a flaw, and an adapter result
with saturated red on more than 5 % of its hole is out before any judging (`paint_left`).

**The judged pair, second round** (`auto3-run.ps1`, the sheets `st-auto3-*`, `rb-auto3-*`;
the judge chose the adapter's result in 17 of 78 strokes, against 49 before):

| | Smoothed only | Judged pair, round 2 |
|---|---|---|
| stock, whole strokes (24 judged) | 19 | 17 |
| stock, half strokes (24 judged) | 16 | 18 |
| generated, both strokes (28) | 26 | **28** |
| all | 61 | 63 |

Won: the tattoo (gone without a trace), the fresco's earring (a clean wall), the Liddell print
(the sofa continued), the umbrella's figure with its head on the half stroke, the half-removed
cup clean, the hat's half stroke a sharp gentleman, the crow and the poster on the generated
set. Lost: the glasses kept at both strokes (the judge, told "the transparent-framed glasses
was to be removed", still took the adapter's result with them on: thin transparent frames in
a small crop), the hat's whole stroke a blurred face, a blur patch for the church's car, the
museum hall blurred. The two-seed noise between any two runs is about this size, so the pair
is a draw on real photographs in count and a win in kind: it removes thin and overlapping
things the plain fill cannot, and the red is gone. It ships as the default; the plain fill
alone is `variations: 1`.

**Third round, the judge's view enlarged** (2026-10-23, `judged-run.ps1 -Suffix -auto4`;
the judged crops enlarged to 320 px on the shorter side, the pair at most half a megapixel:
a pair at a megapixel had the GPU driver reset, see `dev-environment-windows.md`; 78 of 78
strokes, no restart, no driver event; the judge chose the adapter in 20):

| | Smoothed only | Round 2 | Round 3 |
|---|---|---|---|
| stock, whole strokes (24 judged) | 19 | 17 | **21** |
| stock, half strokes (24 judged) | 16 | 18 | 17 |
| generated, both strokes (28) | 26 | 28 | 26 |
| all | 61 | 63 | **64** |

Whole strokes won over the smoothed fill: the tattoo, the fresco's earring, the bicycle, the
umbrella's figure with its head, a sharper daguerreotype head. Lost: the old man's glasses
(gone, but the band across the eyes smeared), the heart sign (the test's mislocated
"poster"), the museum hall (blurred). Half strokes won: the glasses off the old man at last,
the umbrella; lost: the bench, the motorcycle and the boat's figure untouched (the stroke
covered under a tenth of them), the tiara and the fresco patch. The generated set gave back
two: a speck on the crow's wire, small marks on the gingham. Three rounds inside each
other's noise on the stock set in count; in kind the pair removes what the plain fill cannot.
It ships.

**The purge between the stroke's runs** (2026-10-23, `purge-run.ps1`; `integrations.purgeModels`
= `switch` | `heavy` | `never`, each policy on a freshly started server, six whole-object
strokes, the first warming the server):

| Policy | First stroke | Next five, mean | min | max |
|---|---|---|---|---|
| `switch` (purge whenever any model file differs; the rule since 2026-10-09) | 77 s | 60.1 s | 54.0 | 66.9 |
| `heavy` (purge only when the diffusion model or checkpoint differs) | 70 s | 55.0 s | 48.4 | 61.6 |
| `never` | 72 s | 53.9 s | 48.3 | 58.2 |

Measured under **contention**: two foreign GPU tenants from SSH sessions on the machine
(an `sd-server` since 22:26 the night before, 4.5 GB; a python tree since 00:35, 2.5 GB)
held 8.4 GB of the 32 GB, which is why a stroke is 48–77 s here against 20–28 s the day
before, and almost certainly why the driver reset four times that night (the first reset 48
minutes after `sd-server` appeared): 20 GB of 2511 plus its encoder no longer fit beside
them. The removal's own calls were a correlate, not the cause. `heavy` is the default: a
removal's adapter and plain fill share the 20 GB model and no longer purge between them,
8 % off a stroke here and more without contention (the purge is the reload of that model);
`never` buys 2 % more at the risk the purge was made for. Re-measure on a clean card.

**The overnight run of 2026-10-27** (`docs/comfy/goals/overnight-2026-10-27.md`; scripts
`q1-run.ps1` … `q11_denoise.py`; clean removals of 20 on the stock set's ten hard scenes, half
and whole strokes, my count; the robustness set's 28 as the no-regression check; the card's
session-0 tenants held 5.6 GB, so no timing here is a clean-card number):

| Route | Stock (of 20) | Robustness (of 28) | Notes |
|---|---|---|---|
| 2.1 edit on the worker, two judged, the night before (`-p21v`) | 15 | | the mask's whole stroke named the wearer |
| + the wearer's candidates and the change band (`-q2`) | **16** | 27 (`-q2b`) | the mask clean; the boat's half stroke went with both people |
| + `nameShadow` (`-q2s`) | 17 | 26 | the boat's whole stroke with its hull; the poster's tape stayed: no net gain, off |
| `shadow` off (`-q2off`) | | 27 | the band changes nothing on the robustness set |
| 2511 shipped, Best of two through ComfyUI, the same night (`-auto7`) | 12 | | a new man's face on the hat (both strokes) and on the mask; the boat left; 16 two nights ago |

The dog's long shadow stays on every route: the band takes only what the model removed inside
its request and is refused where it reaches the request's edge; `removeMargin` 0.5 gave the
model more context and it kept the shadow itself. The cup's dark patch stays on 2.1 (fainter
with `nameShadow`). The chat cache (genai-native `2f3cd2a`): the removal naming 0.8–2.1 s →
0.3–0.8 s a question, 17 of 20 sampled names word for word the same.

**LoRA parity through the app** (`q4_lora_parity.py`, the same seed, size and prompt on both
routes; ComfyUI applies the file with `LoraLoaderModelOnly` after the model loader), PSNR
native vs ComfyUI: edit "make it winter" 40.0 dB without a LoRA, 41.0 with Consistency, 41.1
with Restore; "turn the cup red" 44.1 / 44.7 / 43.7; the restore wording 38.2 / 38.5 / 40.3;
text to image 33.0 without, 22.6 with the BFS head LoRA (the same person and framing; the
window frame drifts). Each LoRA moves both routes by the same amount (17.8/18.1, 31.5/31.5,
27.5/27.7 dB against no LoRA).

**Generative Upscale** (`q6_upscale.py`): st-hat halved to 960×686 and enlarged back 2×, six
1-MP tiles on the worker in 89 s (about 5 s a tile is the text encoder reloading beside the
tenants); crisper than bicubic at 200 % on the frame, hair and face, no seams; PSNR against
the original 27.0 dB (bicubic 27.5: PSNR rewards the blur).

**A fill's denoise** (`q11_denoise.py`, st-hat's top hat asked to be red): the 2511 40-step
fill at 1 a new red hat, 0.9 red with the old hat's sheen, 0.8 the old hat with a red fleck;
the 8-step tier at 0.75 and 0.6 the old hat with a red fleck.

## 6. Where the time goes, and what is left

- A 2511 fill is sampling-bound: ~1.3 s per Lightning step at a 512-px request, ~2 s at 1 MP
  on the 5090; the encoder 3–5 s when the pixels change; everything on the PhotoCraft side
  under 0.5 s.
- Next levers, in order of expected gain: a working FP8-tensor-core checkpoint (the scaled
  file gave noise here), SageAttention once a wheel exists for this torch build, a 20-step
  middle tier for the base (Comfy's note accepts 20 steps; CFG 4 is still two passes a step),
  TeaCache/EasyCache-style step caching for the 40-step base, detecting the memory-pressure
  state from `timings` to suggest a purge, and a prompt study for Expand over more seeds (§3:
  two seeds showed the wording matters and cannot rank it). The automatic purge on a model-set
  change shipped (§5c).
- On a 24 GB Ampere card (RTX 3090) the same tiers apply with roughly 2–3× the step times and
  no FP8 gain; the text encoder will live in RAM; the Lightning 8-step tier is the one to install.

## 8. The native-engine baseline (plan-native-engine §5, 2026-10-18)

The reference every engine of the native-engine plan is compared against: ShotoPhopEX's
ComfyUI route as the product runs it (Quality `best`, the app-managed sidecar), through
`gn-bench run --engine comfy` in the `genai-native` repository (`crates/gn-harness/fixtures/`;
run folders under its ignored `references/2026-10-18-comfy-<suite>/`, 44 pictures, 52 MB).
ComfyUI 0.39.0, driver 617.42, RTX 5090 32 GB, one run per case, the server warm after the
first case.

| Suite | Cases | Template under `auto` / `best` | Engine time per case | Wall, whole suite |
|---|---:|---|---|---:|
| `t2i` | 28 | `krea2-turbo/image` (8 steps) | 4.0–4.4 s at 1024², 1344×768 and 768×1344 alike; 20.6 s for the first, cold | 142 s |
| `edit` | 8 | `qwen-edit-2511/edit` (40 steps) | 67.1–67.5 s; 119.7 s with one reference picture (two cases) | 652 s |
| `fill` | 8 | `qwen-edit-2511/fill` (40 steps) | 44.1–47.8 s; 59.5 s for the removal case (a 50 % rect) | 383 s |
| `sam` | 16 | `sam3.1/segment`, `-point`, `-box` | 0.2–0.7 s (text, point, negative point, box) | 8 s |

**sd.cpp, first rows (2026-10-19, prebuilt `sd-master-95b3090-bin-win-cuda12-x64`, arch 120,
flash attention on, `--rng cpu`, the fox prompt of `t2i-animal-s7` at 1024², 8 steps, CFG 1,
seed 7; `sd-cli` loads weights inside every call, which a resident worker would not):**

| Krea 2 Turbo through sd.cpp | Per step | Sampling (8) | Encode | Decode | `generate_image` | Weights in VRAM |
|---|---:|---:|---:|---:|---:|---:|
| GGUF Q8_0 DiT + Qwen3-VL-4B Q8_0 + Wan VAE (warm page cache) | 0.59 s (1.69 it/s) | 8.0 s | 1.1 s | 0.57 s (4.3 s the first time: graph build) | 9.7 s (+3.1 s DiT load inside the call) | 17.3 GB |
| The ComfyUI fp8 repack DiT + fp8 encoder (sd.cpp dequantises fp8; no tensor-core path) | ≈1.2 s | — | — | 0.7 s | 15.7–16.5 s | 16.8 GB |
| ComfyUI fp8, same prompt (row above, resident models) | ≈0.41 s | ≈3.3 s | ≈0.3 s | ≈0.3 s | 4.0–4.4 s | — |

Reading: sd.cpp's Q8 sampling step is **1.45×** ComfyUI's fp8 step on this card (the 1.3–2× band:
the Python-free lane stands, flagged for the kernel track — FP8/NVFP4 GEMMs in ggml's CUDA
backend are the lever); the whole-image gap (2.3×) is mostly per-call weight loading and a
slower text encoder, both of which the worker's residency removes. The picture is correct (the
fox, `_eval/sdcpp-out/krea2-gguf-q8-warm2.png`).

**The whole t2i suite through sd.cpp** (`gn-bench run --engine sdcpp`, same 28 cases, seeds and
sizes as the baseline; tables in `bench/sdcpp-t2i-summary-2026-10-19.md` and
`bench/sdcpp-t2i-compare-2026-10-19.md`): 28 of 28 ran, 311 s wall; per picture
`generate_image` 9.2–13.6 s (median 9.8), sampling 7.5–11.1 s (median 8.1), condition 1.1 s,
decode 0.58 s; **0.41× ComfyUI's wall per picture** (2.4×), the first picture apart (ComfyUI's
own cold start made that one 1.55× in sd.cpp's favour). **Parity at equal seed: 9 of 28** pass
the picture gate (PSNR ≥ 30 dB or SSIM ≥ 0.90; typography-s11 33.4 dB, animal-s7 30.9 dB,
product-s7 30.2 dB, landscape-s11 30.5 dB), 19 fall short at 14–29 dB. The misses are the same
scene and composition with detail drift (the street-wide pair: the same café window, umbrellas
and crossing), i.e. the noise path matches (`--rng cpu`) and the divergence is at the encoder
(the Q8 GGUF Qwen3-VL-4B and sd.cpp's own chat template against ComfyUI's fp8 repack and
template), amplified by 8 Turbo steps. Two consequences: the harness's seed-level parity gate
works across engines, and an engine that wants ComfyUI-identical output needs ComfyUI-identical
conditioning, which argues for our own encoder path in the native engine rather than borrowing
sd.cpp's.

**SAM 3.1 through ONNX Runtime (plan Phase 1, 2026-10-19; `gn-bench run --engine sam-ort`,
ORT 1.31.0 CUDA 13 provider, fp32 graphs exported from ComfyUI's own modules by
`genai-native/tools/export_sam31.py`; one process per case, so the wall includes 2 s of session
opening and the first-call autotune):** 16 of 16 at parity, bounds IoU 0.99–1.00
(`bench/sam-ort-comfy-compare-2026-10-19.md`); a first round through the community
`kishanstar2003` SAM 3 export reached 11 of 16 (text prompts only;
`bench/sam-ort-hf-compare-2026-10-19.md`). Engine times on the fox picture, warm:

| SAM 3.1 route | ComfyUI node (fp16, torch SDPA) | ORT fp32, first call | ORT fp32, warm | ORT fp16, warm | ORT fp16 + fused attention + device-resident features, warm (**the default set**) |
|---|---:|---:|---:|---:|---:|
| Picture embedding (vision trunk + both FPNs + CLS), once per picture | inside the node | 470–490 ms | 198 ms | 133 ms | **66 ms** (squash 8 + run 56) |
| Text (`select.byText`), one detection with 2 refinement passes, after the embedding | 154 ms whole node | 750 ms | 276 ms | 202 ms | **132 ms** (text 5 + decoder 36 + crop vision 66 + heads 2× 7) |
| Point (`select.byPoint`), 2 passes, after the embedding | 94 ms whole node | 140 ms | 21 ms | 22 ms | **10 ms** |
| Box, after the embedding | ≈ 90 ms | 137 ms | 21 ms | 22 ms | 10 ms |

Reading: the masks are ComfyUI's (fox: 344 133 vs 344 125 pixels by point, 344 151 vs 344 045 by
text, 142 239 vs 141 856 by box; every variant 16/16 at parity). The two steps that mattered
after fp16: **the feature maps stay on the device** (ORT IoBinding: the vision graph's 220 MB of
outputs no longer cross to the host and back for every prompt; "copy out" went from 25 ms to
0 and a heads call from 22 to 10 ms), and **the exporter's attention replaced by ORT's fused
`MultiHeadAttention`** (`export_sam31.py --mha`, a custom translation of
`scaled_dot_product_attention`; the trunk's run 71 → 56 ms). ORT's profile of the trunk shows
2 010 kernels per run and ≈ 37 ms of node time, so the remaining ≈ 20 ms is launch overhead; a
CUDA graph would remove it, but ORT 1.31's CUDA provider fails the capture on this model (the
path exists, off by default, `gn-sam --graph`). Against ComfyUI's node a fresh picture is
now **79 ms by point (the node: 94)** and 198 ms by text (the node: 154; the crop refinement
re-runs the trunk, as the node does). Tried and dropped: opset 23's `Attention` op (ORT 1.31's
CUDA provider ran it in 2.7 s); ORT's offline transformers optimizer (fused BiasGelu and
SkipLayerNorm only; RoPE and the window layout defeat its attention patterns).

**Qwen-Image-2.1 (plan Phase 2, 2026-10-19): the ComfyUI row and qwen-image-rs's first numbers
on this card** (`gn-bench run --engine comfy --template qwen-2.1/image` and `--engine qir`; the
`qwen-2.1/image` template: 25 steps, euler/simple, CFG 1, the INT8 ConvRot repack and the INT8
Qwen3-VL-8B; qwen-image-rs: the official bf16 snapshot, `--convrot` INT8 DiT, bf16 text encoder,
SageAttention2, one process per picture):

| Qwen-Image-2.1 at 1024², 25 steps | Per step | 25 steps | Text encoder | VAE decode | Per picture, resident | Per picture, cold process |
|---|---:|---:|---:|---:|---:|---:|
| ComfyUI (`best`, server warm) | ≈ 0.15 s | ≈ 3.7 s | inside | ≈ 0.2 s | 4.4–5.0 s engine time | 44 s the first (loads) |
| qwen-image-rs on the 5090 (4090-tuned tiles), first run | 0.14 s | 3.5 s | 17 s to load (bf16, 16 GB) | 2.1 s (tiled: 11 GB free) | — (no resident mode in its `generate`) | 45 s (16 GB encoder load + DiT quantise 20 s) |
| the same, page cache and INT8 cache warm (`genai-native/crates/qir`) | **0.116 s** | **2.9 s** | ≈ 5 s to load | 0.43 s | — | **11–13 s** (DiT cache 3 s + encoder 5 s + 2.9 + 0.4) |
| **ShotoPhopEX's native worker** (`gn-diffusion` over `qir::pipeline`, `integrations.engine = native`, 2026-10-20; the app's `ms`, channel included) | 0.116 s | 2.9 s | 0.25 s (bf16, resident) | 0.43 s | **3.65–3.73 s** | 11.7 s the first picture (loads), then resident |

The native worker's picture is **0.8–1.3 s faster than ComfyUI's** at the same steps and size once
resident, and the first picture costs 11.7 s against ComfyUI's 44 s cold. Measured with ComfyUI
stopped: with its models still resident the same run took 11.9 s per picture (the card's 32 GB
oversubscribed), so the two engines are not to be kept warm together.

Reading: the port's kernels are correct on sm_120 at the authors' 4090 speed or a little better
(0.116–0.14 s/step against their 0.148), so the Python-free lane is at ComfyUI's speed (≈ 0.15),
not ahead; the gap to close is
tuning (tile plans for 170 SMs / 96 MB L2, Blackwell block-scaled GEMMs) and residency. Pictures
match ComfyUI's at scene level only (0 of 24 at seed parity, PSNR 5–17 dB): the noise source and
the schedule differ, which `gn-models` fixes by construction (ComfyUI's CPU noise and `simple`
sigmas).

**With ComfyUI's conventions (2026-10-20, `qwen-image-rs generate --comfy`: PyTorch's CPU
generator reproduced bit for bit, the `simple` schedule over the model's sigma table, its bf16
timestep rounding; against `references/2026-10-19-comfy-t2i-qwen21`, all 28 cases including
the non-square four):**

| Qwen-Image-2.1, 25 steps, 28 cases | PSNR | SSIM | At the 30 dB / 0.90 gate | Per picture (one process each) |
|---|---:|---:|---:|---:|
| qwen-image-rs, candle's RNG + diffusers' schedule (2026-10-19) | 5–17 dB | 0.1–0.7 | 0 of 24 | 26–40 s |
| `--comfy` with the wrong sigma table (`ModelSamplingDiscreteFlow`'s SNR shift, 1000 entries; the morning's runs) | 9–24 dB | 0.49–0.93 | 6 of 28 | 11.4–11.6 s |
| `--comfy` with the right one (`ModelSamplingFlux`'s exponential shift, 10 000 entries; `genai-native` `992231a`; `runs/2026-10-20-qir-t2i-comfy-flux`) | **16.7–41.3 dB** (the fox 39.9) | **0.71–1.00** | **24 of 28** | 11.3 s |
| the same against ComfyUI run with its bf16 encoder and DiT (`references/2026-10-20-comfy-t2i-qwen21-bf16`) | 16.9–41.3 dB | 0.72–1.00 | 25 of 28 | |
| for scale: ComfyUI's INT8 route against its own bf16 route | 6.1–42.4 dB | 0.75–1.00 | 26 of 28 | |

The four misses are the same cases for everyone: the brutalist facade at both seeds (ComfyUI
against itself 24.8 / 25.8 dB), the vehicle at seed 11 (23 dB; ComfyUI against itself 26), and
the BAKERY sign at seed 7, where the app's INT8 encoder flips the background to transparent
(ComfyUI's two routes 6 dB apart; ours matches the bf16 route at 37.9 dB).

**How the gap was found (2026-10-20).** With the wrong table the routes agreed after one step
(sigmas `[1, 0]` on both sides: 42 dB) and parted from the second (31.5 dB at 2 steps, 21 dB
at 25). Ruled out one by one on the fox before the cause showed: the attention kernel (exact
bf16 and SageAttention2 both 21.0 dB), this port's DiT quantisation (the bf16 DiT with exact
attention: 20.9 dB), the DiT weights (ComfyUI's INT8 ConvRot repack loaded into the port, its
256-column groups permuted into the port's Hadamard basis: 21.1 dB), the VAE (ComfyUI's own
final latent, normalised, through the port's decoder: 49 dB against ComfyUI's picture), and a
single DiT forward on identical inputs (1 % apart, as ComfyUI's INT8 and bf16 DiTs are from
each other). ComfyUI against itself, bf16 DiT instead of the INT8 repack: 43.9 dB on the fox,
30.6 on the facade, 26 of 28 cases at the gate. What was left was the sampler's own state, and
stepping ComfyUI's real `KSampler` path next to a plain Euler loop over the same model showed
the second sigma at 0.9795 where the SNR shift gives 0.9430: the Qwen-Image-2.1 model is a
`FLUX`-type model in ComfyUI, whose `ModelSamplingFlux` builds `e^0.69 / (e^0.69 + 1/t − 1)` over
10 000 timesteps. The script that had printed the "ground truth" sigmas had instantiated the
other class. Lesson for every later model: take the sigma table from the loaded model object,
never from a formula read off a class.

**The text encoders, measured on the way** (the fox prompt, 28 tokens after the system turn,
against transformers' bf16 Qwen3-VL-8B from the official snapshot):

| Text encoder | Error vs the oracle | Cosine, first token / mean |
|---|---:|---:|
| ComfyUI's `qwen3vl_8b_int8_convrot` (what the ComfyUI route conditions on) | 8.8 % | 0.902 / 0.9956 |
| the port, bf16 attention scores (as imported) | 4.1 % | 0.988 / 0.9992 |
| the port, attention scores and softmax in f32 (`genai-native` `9617518`, kept) | **2.7 %** | 0.9992 / 0.9997 |

The INT8 encoder's error costs the ComfyUI route little at picture level (its INT8 and bf16
routes agree at 24–42 dB), and the native route conditions the DiT closer to the model than
the ComfyUI route does. A second reference set, ComfyUI with the bf16 encoder and the bf16 DiT
(`references/2026-10-20-comfy-t2i-qwen21-bf16`, the model as published, run through ComfyUI's
API with the template's graph), sits next to the INT8 one the app's route produces.

**Qwen-Image-2.1 edit and fill natively (plan Phase 3, 2026-10-20).** Both routes driven by
the app's own CLI so the request is built the same way (the composite, the feathered mask, the
880 px working size, the fill's prompt wrapper): `gn-bench run --engine comfy` against ComfyUI
for the references, then with `--pref integrations.engine=native` for the native worker
(`references/2026-10-20-comfy-{edit,fill}-qwen21`, `runs/2026-10-20-native-{edit,fill}-qwen21`).
One CLI process per case, so every native case loads the models (about 8 s of its time):

| Suite, 8 cases each, 25 steps | ComfyUI warm, engine time | Native, per process (loads included) | PSNR / SSIM | At the gate |
|---|---:|---:|---:|---:|
| edit (one reference = the picture) | 5.4–5.9 s | 12.6–13.5 s | 36–40 dB on six; 21.2 dB / 0.94 and 23.8 dB / 0.97 on two | **8 of 8** |
| edit with a second reference | 8.2–8.6 s | 30 and 68 s (paging: see below) | 36.6 dB; 23.8 dB / 0.97 | (counted above) |
| fill (the picture and the mask) | 1.9–5.6 s | 9.6–12.1 s | 17–56 dB, SSIM 0.91–1.00 | **8 of 8** |

Resident (`qwen-image-rs edit`, the fox at 1024², one reference): the references' encoders
and the prefix pass 1.5 s, **sampling 3.8 s** (47 s before the prefix cache), decode 0.7 s; the
prefix cache holds every block's keys and values of the text and reference rows (4 GB for one
1024² reference, 8 GB for two). **Through the app, resident** (one CLI process, the worker
keeps the models): the first edit 12.3 s (loads), then **4.2 s per 880² edit against ComfyUI's
5.4–5.9 s warm**, a 608×464 fill **1.8 s** (ComfyUI 1.9–3.5 s). With two references the bf16
text encoder (16 GB), the INT8 DiT, the VAE, the vision tower and that cache oversubscribe the
32 GB card and the driver pages, hence the 30–68 s; `Options::evict_text_encoder` (on by
default) drops the encoder after the prompt when `free_vram` says the cache will not fit, and
loads it again for the next prompt (7 s): the fox-in-the-boat edit then takes 9.0 s (ComfyUI
8.2–8.6 s).
A direct `--engine qir` run of the edit suite is not comparable with the app's references (2 of
8): the app resizes a 1024² picture to 880² before the model sees it.

Also native the same evening (one CLI process each, loads included): Remove Background with
`qwen-2.1/matte` on the fox at 1024², 10.5 s (ComfyUI 13.8 s), the two routes' alphas at soft
IoU 0.992; a transparent Generate Image at 1024², 12.3 s; Generative Expand with
`qwen-2.1/expand` (the picture placed inside the canvas as the reference, the whole picture
along), 11.7–12.0 s for 256–384 px of new canvas on the fox, seamless on the four cases looked
at (two symmetric, two one-sided).

**Krea 2 Turbo natively (plan Phase 4, 2026-10-20 night).** The port (`genai-native`
`cc0ee18`…): ConvRot INT8 block linears, SageAttention2, bf16 Wan 2.1 VAE, ComfyUI's noise and
`simple` schedule (shift 1.15, 8 steps). The references are the app's ComfyUI route with the
fp8 repacks (`references/2026-10-18-comfy-t2i`) and the same graph with the bf16 files the
converter made (`references/2026-10-20-comfy-t2i-krea2-bf16`, through ComfyUI's API), 28 cases:

| Krea 2 Turbo, 1024² (4 at other aspects), 8 steps | Engine time per picture | At the 30 dB / 0.90 gate vs the fp8 route | vs the bf16 route |
|---|---:|---:|---:|
| ComfyUI, fp8 repacks (the app's route) | 4.0–4.7 s warm | — | 20 of 28 |
| ComfyUI, bf16 files | 5.5–6.7 s | 20 of 28 | — |
| native, INT8 ConvRot + SageAttention2 (`runs/2026-10-20-qir-krea2-t2i`) | **3.5–4.9 s** (one process per picture: 11 s to load, then the picture) | 16 of 28 | 17 of 28 |
| native, bf16 DiT + SageAttention2 (`runs/2026-10-20-qir-krea2-t2i-bf16`; candle's bf16 GEMMs, the encoder reloaded per prompt) | 8.6–15.0 s | 19 of 28 | **21 of 28** |

Reading: an 8-step Turbo at CFG 1 is sensitive to its arithmetic. ComfyUI's two precisions
disagree on 8 cases (street ×3, food ×2, architecture-s7, interior-s7, vehicle-s7), the native
route on 11–12, the same scenes plus macro ×2 and interior-s11, and the misses are the same
scene with details shifted (a menu card, a tree, an umbrella), not another picture. The fox:
35.6 dB against the fp8 route, 36.2 against bf16; ComfyUI's two routes 39.3 dB apart on it.
The single DiT forward on identical inputs is 1.6 % from ComfyUI's fp8 route (cosine 0.99988),
the conditioning 1 %, the VAE decode 0.24 %. With the bf16 DiT the native route agrees with
ComfyUI's bf16 route as well as ComfyUI's fp8 route does (21 against 20 of 28), so the INT8
ConvRot linears cost about four cases on this model; the fp8 GEMM of the kernel track (§11)
is the lever that keeps both. **Through the app, resident** (`native-krea2-live.bat`,
`integrations.engine = native`): the first picture 15.7 s (loads and quantises), then **4.85 s
and 4.31 s per 1024² picture** against ComfyUI's 4.0–4.7 s warm; swapping the worker to
Qwen-Image-2.1 and back costs the loads (18 s, 23 s). Routing: Krea 2 stays on ComfyUI under
`auto` (neither faster nor at the INT8 precision ComfyUI's own two routes reach); `native`
serves it.

**Krea 2 on ComfyUI's own fp8 recipe (2026-10-27, `genai-native` `6354b13`).** ComfyUI's
`krea2_turbo_fp8_scaled` carries `_quantization_metadata`: five of a block's eight linears
(`wq wk wv`, the SwiGLU's `gate up`) run as fp8 matmuls under `--fast fp8_matrix_mult` (the
activation clamped and cast to e4m3 with scale 1, `torch._scaled_mm`, fp32 accumulation), the
other three (`attn.gate`, `wo`, `mlp.down`) are marked `full_precision_matrix_mult` and
dequantized to bf16. Its `qwen3vl_4b_fp8_scaled` encoder marks every layer fp8 (`comfy_quant`
records), but ComfyUI only forces fp8 matmuls there to generate text; encoding a prompt runs
them in full precision. `qir::fp8` reproduces this on cuBLASLt from the files as they are (the
DiT 13 GB resident, the encoder 4.4 GB). Each row is one process per picture
(`gn-bench --engine qir`), 28 cases, against the app's ComfyUI route; warm times from
`generate-krea2 --repeat`:

| Krea 2 Turbo, native, ComfyUI's fp8 files | Sampling, warm (8 steps, 1024²) | vs ComfyUI fp8 | vs ComfyUI bf16 |
|---|---:|---:|---:|
| fp8 DiT, bf16-converted encoder, SageAttention2 | 4.03 s | 16 | — |
| the same, exact attention (the plain f32 path) | not timed warm (9–15 s a picture, one process each) | 19 | — |
| fp8 DiT, the encoder's fp8 matmuls too, exact | — | 12 | — |
| fp8 DiT, fp8 encoder file in full precision, exact (plain f32) | not timed warm (≈ 9 s a picture, one process each) | **21** | — |
| the same, SageAttention2 with fp32 PV accumulation | ≈ 4 s | 17 | — |
| the same, our flash kernel (bf16 P·V), candle's small ops | 4.74 s | 17 | 18 |
| every DiT linear an fp8 matmul (the file's marks ignored), flash | 3.91 s | 6 | 6 |
| **flash (fp16 P·V) + Krea 2's small ops fused** (the shipped route) | **3.20 s** | **19** | **20** |
| … through ShotoPhopEX's CLI and worker, the assembled runtime folder only (`runs/2026-10-27-app-native-krea2-fused-t2i`) | 3.2 s; **4.0 s a picture wall** | **19** | — |

Reading: the precision ComfyUI's two routes reach with each other is 20 of 28; the variants
here span 16–21 and a difference of two cases is within what one rounding change moves on an
8-step Turbo (the misses are the same street, vehicle, food and architecture scenes every
time). What separates the rows clearly: the file's full-precision marks matter (6 of 28
without them) and the encoder must not take fp8 matmuls (12). The GEMMs alone
(`qwen-image-rs fp8-bench`, M = 4160): fp8 313–568 TFLOPS, bf16 186–220, the dequantize before
a bf16 matmul +6 %; summed over a step, the linears are ≈ 0.32 s, attention 0.1 s (our flash
kernel at ≈ 120 TFLOPS), and before fusion another ≈ 0.17 s went to candle's chains of small
ops (f32 copies for each norm, the modulation, RoPE and SwiGLU passes). Fused, a step is
0.40 s against ComfyUI's ≈ 0.41–0.45. **Routing: Krea 2 text to image goes native under
`auto`** (faster at parity); its style reference stays on ComfyUI (not ported).

Parity yardsticks and thresholds: `genai-native/docs/PARITY.md`. The SAM suite compares
instance bounds until an engine returns masks (Phase 1). The gates these numbers set for later
rows: sd.cpp within 1.3× of a row keeps that model on the Python-free lane without further work;
the native engine takes a model under `auto` only when it beats its row at parity.
