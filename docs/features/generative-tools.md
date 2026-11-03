# LumaFoundry generative tools

Implementation status, 2026-11-02. This is a draft port onto upstream 0.19.0.

The branch also contains generated-edge harmonization (enabled by default for fill, removal, and expansion), learned
subject masks (`select.subjectEnki`), a masked cutout duplicate (`layer.removeBackgroundEnki`),
and source-aware background replacement (`image.replaceBackground`). Background replacement
protects the subject selection and edits its complement. Edge matching defaults off for
background replacement because the protected pixels are the subject itself. Subject-mask inference requires the
separately configured Enki U2Net worker/model; edge refinement reuses the image-guided engine,
not a learned alpha-matting model. These latest additions still require complete acceptance;
earlier live evidence below belongs to the previous checkout, not this 0.19.0 port.

## Current implementation

The first native implementation is `image.generate`, available through Image › Generate Image and Properties Quick Actions
for pixel and Smart Object layers. It supports a new image layer or a generated background
beneath a masked subject. Linux connects directly to the installed Enki runtime, verifies the
peer user, negotiates protocol v3, bounds frames/PNG headers, and uses engine jobs for progress
and cancellation. Output conversion uses the CMS and retains the document profile and depth.

Prompt, negative prompt, dimensions, model, optional sampling overrides and seed are exposed.
An empty model requests Enki's recommendation. Generation creates one editable layer and one
undo step. Repeating with a negative seed retains alternatives as separate layers; an interactive
variation browser is still planned. Background generation requires a subject mask and full
canvas coverage, preserves active subject and selection, and does not claim automatic scene
or lighting matching. Native transports for other platforms remain open.

The next implementation adds **Image › Fill Selection**, **Remove Object** and **Expand Canvas**.
They send a colour-managed source/context PNG and a saved edit mask to Enki, then apply the
result as a new masked root layer. Outside-mask pixels and original layers remain unchanged.
Selection edits use bounded surrounding context, including in large documents, without shrinking
the selection. The selection itself is preserved, including existing feather weights.

Expansion preserves the original canvas region independently of the model output and changes
canvas size/layer positions in the same undo step. It currently supports static pixel layers
and groups without effects, artboards or dissolve blending. Other content is rejected rather
than silently rasterized. Each margin is at most 512; final dimensions must be 64–1024 and
multiples of 8, with no implicit rounding. Learned subject selection remains a separate task.

## Delivery order

| Stage | Tools | Completion gate |
|---|---|---|
| 1 | Generate Image, explicit model/seed controls | Native dialog and engine command, cancellable job, installed Enki runtime, bounded PNG decoding, new editable layer, one undo; deterministic protocol tests and live generation |
| 2 | Select Subject, Refine Edges, Remove Background | Learned segmentation and edge matting behind the existing editable masks; real-image fixtures, thin structures and transparency evaluated; existing classical tools remain available |
| 3 | Fill Selection, Remove Object, Generate Background | Enki image+mask editing capability; source/context supplied to a compatible editing model; output clipped to the saved mask; protected pixels unchanged; one undo and save/reopen |
| 4 | Expand Canvas | Same editing pipeline with an outside-canvas mask; source layers translated without rasterizing; requested dimensions exact; original region unchanged |
| 5 | Reference Images, Prompt Edit | Explicit composition/style/subject references and model capability checks; references never ignored silently; all edits use the mask pipeline |
| 6 | Variations | Repeat generation with reported seeds, retain alternatives as layers, explicit keep/delete; cancellation and document-switch tests |
| 7 | Enhance Resolution | Dedicated compatible upscale model; explicit output size/resource estimate, profile/depth preservation, real-image detail and artifact evaluation |
| 8 | Generate Vectors, Recolour Vectors | Editable paths and fills, validated bounded geometry, native-format round trip |
| 9 | Creative Boards | Separate document/workspace design for arranging references and outputs; explicit storage and ownership |
| 10 | Motion and Audio | Separate design for video generation, speech, sound effects and translation; media timeline, codecs, resource limits and export tests required |

## Shared implementation

Reuse engine commands, background jobs, existing progress/cancel UI, native dialogs and document history. Native runtime calls must never run on the UI thread. No runtime installation, model downloads or model switching at startup. Model choice is explicit or comes from Enki's recommendation API. The web build reports native-runtime unavailability.

Model input/output is a colour-managed boundary. Convert working copies through the CMS; retain the original document profile, depth, layers and masks. Reject unsupported dimensions rather than silently shrinking documents. PNG dimensions and frame lengths are bounded before allocation.

Masked edits use a snapshot of the document, selection and context at submission. Never accept text-only generation as a substitute for masked editing. A model result is a new masked layer; outside-mask pixels are not replaced. Expansion must preserve source pixels independently of model output. Completion cannot apply to a changed or closed document. Cancel/error leaves document history unchanged.

## Current backend boundary

Enki source now implements the additive v3 masked-edit revision 1 extension: `image_capabilities`,
`imagine` task `fill`, `params.mask_png`, source validation and exact restoration of fully
protected RGBA pixels. Its SDK exposes `ImageRequest::fill`. `image_edit_defaults` returns an
editing-model profile separately from the general generation recommendation. The current edit
recommendation uses the already-catalogued reference-conditioned FLUX.2 Klein model. No model
is installed or downloaded automatically.

These changes live in the adjacent Enki checkout as well as this application. Existing installed
runtimes without the extension are refused before source images are sent. Runtime source/builds
were tested on separate private sockets; the installed running daemon was not replaced or
restarted. Both updated builds must be deployed before using these tools with the normal endpoint.

The backend still lacks learned segmentation/matting and a dedicated upscale operation. External
reference controls, automatic background harmonization and a variation browser remain open.

## Image command acceptance

| Tool | Status |
|---|---|
| Generate Image | Implemented on Linux; real runtime and native offscreen UI verified; interactive desktop acceptance remains |
| Generate Background | Implemented as a new image beneath the subject; source-aware matching remains open |
| Variations | Separate generated layers and explicit seeds work; browsing/reusing settings remains open |
| Fill Selection / Remove Object | Implemented with source/context and a saved mask; live CPU/native offscreen checks pass on a generated fixture; broad model/real-photo quality acceptance remains |
| Expand Canvas | Implemented for static pixel/group documents within backend limits; exact original-region preservation verified; broader document kinds remain open |
| Learned subject masks / edge matting | Requires a segmentation/matting backend and evaluated model assets |
| References / Prompt Edit | Requires explicit model edit inputs and capability discovery |
| Enhance Resolution | Requires a dedicated upscale backend and evaluated model assets |
| Vectors / Boards / Motion / Audio | Planned only; outside this image-tools implementation |

## Validation and quality boundaries

Unit tests cover 8/16/32-bit RGB, grayscale, CMYK and Lab, protected pixels, original layers,
selection/feather weights, undo/redo, save/reopen, bounded context on a 2400×1800 document,
transparent source padding and old-runtime refusal. `dryRun: true` validates an operation
without inference or history changes; adversarial-command tests use it to avoid real model jobs.

Real CPU inference and native offscreen captures used an earlier generated landscape. Fill and
Remove each changed pixels inside the 64×64 selection and changed **zero** pixels outside it.
Expansion to 320×320 changed **zero** pixels of the original 256×256 region. The editing model
produced scene-aware output; the small generation-only test model produced poor artifacts and
is not an accepted editing-quality baseline. Hard selection boundaries can still show seams;
use/refine selection weights and review the new layer. These checks do not establish general
real-photo quality, interactive desktop/GPU acceptance, model rights or release readiness.

## Naming

Use LumaFoundry names in new product copy, implementation notes and tests. Keep existing machine identifiers and required format/legal provenance intact.
