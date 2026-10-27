---
title: Choose a resampling method
description: Pick how PhotoCraft resizes an image in Image Size and Export As, from Bicubic and Preserve Details to Lanczos, Mitchell, Spline36 and Nearest Neighbor.
order: 55
---

<!-- Draft until the new resampling methods ship: then drop the leading _ from the file name, list
it in guides/index.md, link it from step 4 of export-an-image.md ("**Resample** picks how the image
is resized; see [Choose a resampling method](choose-a-resampling-method.md)."), and set `verified`
to that release. -->

When you make an image bigger or smaller, the **Resample** method decides how new pixels are worked out. Pick it in **Image › Image Size…** (it changes the document) or in **File › Export › Export As…** (it changes only the exported copy). For photos, try **Lanczos 3** or **Spline36**; for pixel art, use **Nearest Neighbor**.

## In Image Size

1. Choose **Image › Image Size…**.
2. Make sure **Resample** is ticked, then type the new **Width** and **Height**.
3. Pick a method from the list next to **Resample**.
4. Click **OK**.

**Automatic** uses **Preserve Details** when the image grows and **Bicubic Sharper** when it shrinks. In an Indexed Color document only **Automatic** and **Nearest Neighbor** are offered, because the other methods make colours the colour table doesn't have.

## In Export As

1. Choose **File › Export › Export As…**.
2. Under **Image Size**, set **Scale** (or the size in pixels).
3. Pick a method under **Resample**.
4. Click **Export**.

**Bicubic Automatic** (the default) uses **Bicubic Smoother** when the image grows and **Bicubic Sharper** when it shrinks. The preview uses a fast method while you adjust it; the exported file uses the one you picked.

## Which method to choose

| Method | Good for | What to expect |
|---|---|---|
| **Preserve Details** | Enlarging photos | Lanczos 3 with light sharpening when enlarging; **Bicubic Sharper** when reducing. |
| **Bicubic Smoother** | Enlarging | Smooth, soft results. |
| **Bicubic Sharper** | Reducing | Crisp results. |
| **Bicubic** | Smooth gradients | A middle ground. |
| **Lanczos 3** | Shrinking or enlarging photos | Sharp and detailed. A faint halo can show along very hard edges. In **Image Size** it is listed as **Lanczos (sharp)**. |
| **Lanczos 4** | The most detail | The sharpest, with the strongest halos. |
| **Lanczos 2** | Sharp but gentle | A little softer than **Lanczos 3**, with fewer halos. |
| **Spline36** | Enlarging photos | Almost as sharp as **Lanczos 3**, with fewer halos. |
| **Spline16** | Enlarging, gently | Like **Lanczos 2**, with fewer halos. |
| **Mitchell-Netravali** | A natural look | Balanced sharpness, hardly any halos. |
| **Catmull-Rom** | Matching other apps | The "bicubic" most other apps use (PhotoCraft's own **Bicubic** follows Photoshop). |
| **Hermite** | Smooth results | Soft, never brighter or darker than the image around an edge. |
| **B-Spline** | The smoothest result | Soft; never shows halos. |
| **Gaussian** | Soft, blurred output | Soft; never shows halos. |
| **Bilinear** | Speed | Plain and a little soft. |
| **Nearest Neighbor** | Pixel art, screenshots, hard edges | Copies whole pixels: edges stay hard, photos look jagged. |

In **Image Size**, some names carry a hint in brackets, for example **Lanczos 4 (sharpest)**, **Spline36 (sharp, less ringing)** or **B-Spline (smoothest)**.

## Sharpness, ringing and overshoot

The sharper a method is, the more it tends to **ring**: thin light and dark outlines appear on both sides of a hard edge, such as text, a horizon or a dark branch against the sky. Where the outline is brighter or darker than anything near it, the method **overshoots**. In bright or dark areas that can clip to pure white or black.

| Rings and overshoots | Most | Some | A little | Never |
|---|---|---|---|---|
| Methods | **Lanczos 4**, **Bicubic Sharper** | **Lanczos 3**, **Preserve Details**, **Bicubic** | **Lanczos 2**, **Spline36**, **Spline16**, **Catmull-Rom**, **Bicubic Smoother**, **Mitchell-Netravali** (barely) | **Hermite**, **B-Spline**, **Gaussian**, **Bilinear**, **Nearest Neighbor** |

**Bicubic Automatic** behaves like **Bicubic Sharper** when it shrinks the image and like **Bicubic Smoother** when it enlarges it.

- **Photos** rarely show ringing at normal viewing sizes: the sharp methods are usually the best choice.
- **Text, logos, line art and screenshots** have hard edges, where outlines show most. Pick **Mitchell-Netravali**, **Hermite** or **B-Spline**, or **Nearest Neighbor** to keep pixel edges exact.
- If you see halos after a sharp method, step down a column: **Lanczos 3** to **Spline36** or **Lanczos 2**, then to **Mitchell-Netravali**.
- Methods that never ring are softer: they trade a little detail for clean edges.

## Things to know

- **Image Size** changes the document. Choose **Edit › Undo** to go back.
- Masks and selections are always resized with **Bilinear**, whatever method you pick, so their edges stay smooth.
- The method you pick in **Export As** is remembered for next time and saved with export presets.
