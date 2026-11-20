---
title: Export an image
description: Export a PNG, JPG, WebP, TIFF or TGA from PhotoCraft with a preview, a size estimate and a scale.
order: 50
verified: 0.8.2
---

Choose **File › Export › Export As…**, set the format, and click **Export**. The dialog shows a preview and an estimate of the file size before you save. Your layered document is not changed.

## Export with settings

1. Choose **File › Export › Export As…**.
2. Under **File Settings**, pick a **Format**: **PNG**, **JPG**, **WebP**, **TIFF** or **TGA**.
3. Set the options for that format:
   - **JPG** and lossy **WebP** have a **Quality** slider from 1 to 100.
   - **WebP** also has a **Lossless** checkbox.
   - **TGA** has **Resolution**: **Auto**, **24 bits/pixel** or **32 bits/pixel**.
   - **Transparency** keeps transparent areas. JPG has no transparency, so this checkbox is not shown for JPG and the image is flattened.
   - **Metadata** is **None** or **All**.
4. Under **Image Size**, set **Scale** in percent. The size in pixels shows below it.
5. Check the preview on the right. The line under it shows the format and the estimated file size.
6. Click **Export**, then choose a name and a place for the file.

PhotoCraft shows "Exported" with the file path and size in the status bar when it is done.

## Export in one step

**File › Export › Quick Export as PNG** exports without asking about settings. It uses the format, quality, metadata and location you set in **File › Export › Export Preferences…**.

## Export layers

Choose **Layer › Export As…** (or **Layer › Quick Export as PNG**) to export only the active layer.

With several layers or artboards selected in the **Layers** panel, both commands export each one as its own file. You choose a name and place once. The first file gets that name, and the others get the same name with a number and the layer name added.

## Keep your layers

To keep the editable PhotoCraft document, use **File › Save As…** with **PSD Document** or **PhotoCraft**. See [Open and save files](open-and-save-files.md).

To interchange raster layers from an RGB document, choose **File › Export › Layered OpenEXR…**. Pick a `.exr` destination and a lossless compression method. The default writes all raster layers and a **Beauty** part with the visible composite. Choose **Selected Layers Only** to export the current selection, or clear **Include Composite** to omit Beauty. Unsupported layer types are reported in the export warnings and can still appear in Beauty.

Open the EXR in PhotoCraft to restore the raster layers, their groups and blend settings. EXR stores linear RGB with premultiplied alpha, so color beneath fully transparent pixels is discarded. The exporter writes a tile at a time. Rendering Beauty can use additional memory for effects that span tiles.
