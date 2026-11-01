# Sprite Sheet Exporter & Animation Timeline

## Overview
The Sprite Sheet Exporter packs canvas grid slices or visible layers into a unified PNG sprite sheet accompanied by an Aseprite/Godot-compatible JSON metadata file.

## Features
- **Sources:** Slice active canvas by grid dimensions or harvest visible raster layers.
- **Interactive Preview:** Clickable layout cells to pick and order frames.
- **Animation Timeline Strip:** Horizontal reorderable strip with per-frame tick duration (`durationTicks`).
- **Real-Time Playback:** Looping preview at configurable FPS (1–60).
- **Metadata Export:** Produces standard Aseprite JSON containing frame rects, millisecond durations, and frame tags.
- **Animated GIF Export:** Optionally writes a looping `.gif` beside the PNG using the selected timeline order and hold durations.

## Usage Guide
1. Open any document or sprite project in PhotoCraft.
2. Navigate to **File** → **Export** → **Export Sprite Sheet…**.
3. Configure frame dimensions (e.g. 32×32) and padding.
4. Click cells in the **Layout Outline** to add them to the animation loop.
5. In the **Animation Timeline**, adjust the tick multiplier (e.g. `2t`, `4t`) to extend hold times on key poses.
6. Check **Play** to verify the motion.
7. Click **Browse Output…**, choose your file name (e.g., `hero.png`), and click **Export Sheet**.
8. Enable **Export animated GIF alongside PNG** when a looping GIF preview is also needed; PhotoCraft writes `hero.gif` beside the PNG and JSON files.
