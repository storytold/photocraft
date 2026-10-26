# Photoshop workflow comparison

The supplied workflow description adds an important requirement: Photoshop's
value comes from combining selections, layers, masks, adjustments, filters and
blend operations, rather than counting isolated menu items. The description is a
partial requirements sample; its estimated coverage is not a measured parity
score.

| Workflow | AI contribution | Independent editor requirement |
| --- | --- | --- |
| Subject selection and fine cutout | BiRefNet proposes a foreground mask | Editable mask painting, edge refinement, alpha fidelity |
| Background replacement | Generate a background behind the preserved foreground | Layers, native masks, color matching, Undo and save/reopen |
| Selection-based edit or removal | Generate replacement pixels in context | Exact preservation outside the native selection |
| Canvas expansion | Generate the newly added border | Preserve original pixels and support all four edges |
| Smart filtering | Optional AI output as a separate layer | Editable filter parameters, smart objects and filter masks |
| Split Blend If sliders | AI may create material to composite | Correct luminance/channel blending and editable UI controls |
| Channel calculations and luminance masks | AI mask can provide an initial selection | Local mask arithmetic, channels and adjustment layers |
| Frequency separation | Optional retouching suggestions | Mathematical high/low-frequency reconstruction and depth fidelity |
| Typography, RAW, paths, warp, print color and PSD fidelity | Limited task-specific assistance | Dedicated non-AI parity work and measured file/behavior tests |

## Workflow acceptance

1. Select subject, create an editable mask, replace the background, refine the
   mask, match color, Undo/Redo, then save and reopen.
2. Select an object, prepare a prompt, generate a local edit, apply it, edit the
   mask, and verify identical pixels outside the selection.
3. Expand all four edges, preserve the original region, and retain mask and
   visible output across a native file roundtrip.
4. Build a classic retouching stack from commands and verify reconstruction,
   bit depth, clipping and Undo separately from generative image quality.

AI integration can support the first three workflows, but it does not by itself
complete classic Photoshop parity. Menu coverage, behavior coverage, file fidelity
and performance must continue to be reported separately. Current assessments
belong in the fork's roadmap, scorecard and parity documents; historical numbers
from the older implementation base should not replace newer measurements.
