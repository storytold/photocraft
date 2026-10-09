# Scratch disks

On native builds, Edit > Preferences > Scratch Disks chooses a directory for cold pixel tiles.
The first enabled, non-empty directory is used. `(system temp)` selects the operating system's
temporary directory. Disable every entry to keep pixels in RAM. Web builds keep pixels in RAM.

Preferences > Performance chooses a process memory ceiling, either in MiB or as a percentage of
physical RAM. Fresh preferences use 90%; existing MiB preferences retain their previous value
until the user selects a percentage. The ceiling does not allocate memory in advance. A native
monitor measures process RSS and available physical memory and reduces the resident tile budget
under pressure, leaving room for non-tile memory and temporary operations. Windows paging remains
available; managed scratch is the main mechanism for keeping original pixels beyond RAM.
`PHOTOCRAFT_TILE_BUDGET_MB` overrides the tile budget for diagnostics. Active readers and writers
can temporarily prevent eviction. This is an admission and eviction policy, not an OS-enforced
process limit: third-party allocations and driver memory can still overshoot it.

The GPU memory setting is independent of RAM. Large canvases load visible 512-pixel pages with
zoom-specific levels and an eviction budget shared across open documents. Two workers prepare
pages, cancel obsolete requests and load scratch outside the UI thread. At 100% and closer, the
CPU fallback also displays source-resolution regions instead of a reduced full-document image.
Cold pages are disposable; original pixels and history remain in the tile store. GPU composition
on the UI thread is admitted only while its input tiles are resident.

Viewport caches discard cold zoom pages before admitting new visible pages and reserve capacity
for worker results until consumed. A view waiting for the two shared workers retries after
capacity becomes available. Canvas cache identity follows the document and viewport, so closing
or reordering tabs does not duplicate a surviving document's cache or reduce its budget.
Streaming diagnostics include visible/ready page counts, queued-result capacity and cache limits.

Native GPU composition uses a 32 MiB upload threshold and drains the final batch before
returning from a render that transferred pixels. This keeps consecutive viewport requests
from accumulating mapped staging buffers, whose available memory can be much smaller than
total VRAM. Rendering cached inputs remains asynchronous.

Current documents, masks, channels and undo snapshots share the same tile storage. Cold tiles
are compressed with LZ4, CRC-checked and written to a temporary scratch file. Reading restores
them on demand; editing keeps copy-on-write semantics. With scratch enabled, History States
still limits undo depth but the byte limit no longer discards older states merely for their pixels.

A scratch write failure retains the resident pixels and stops eviction. A failed read retains
the original extent and blocks edit commits and saves until those pixels are readable again;
display placeholders are never stored as replacement tiles. Errors appear in the status bar.
Failed reads also prevent derived pixel caches and native-format hashes from being committed;
retrying after storage recovery uses the original pixels.
`ui.inspect` exposes `scratch` (resident and scratch bytes, evictions, reloads and errors),
`memory` (policy, effective budget, RSS, available RAM and reservations), and `streaming`
(visible pages, pending work, CPU/GPU cache estimates and errors). Virtual bytes measure process
address space, not Windows committed memory. GPU bytes are estimates with staging/in-flight
headroom, not a DXGI measurement of the driver's current budget.

Native layered PSD/PSB import indexes seekable channel ranges and decodes raw, RLE, ZIP and ZIP
prediction rows directly into tiled storage for RGB, Gray, CMYK and Lab at 8/16/32 bits. It skips
the merged image when the layers are sufficient, avoiding its full-channel allocation limit.
With scratch disabled, admission counts decoded layer and mask tiles against the available
RAM budget before opening channels and rechecks growth for each band. If scratch stops during
import, the import fails instead of continuing to accumulate pixels in RAM.
Flattened files, extra merged channels and unsupported modes use the guarded byte importer.
Preserved metadata is bounded; oversized opaque blocks produce an actionable error.

Native single-part flat scanline EXR import also reads one compression block directly into
tiles, retaining source dimensions and HDR values as F32 GrayA/RGBA (HALF samples promote
exactly to F32). Missing alpha is opaque. Rec. 709 linear RGB is tagged with the existing
linear-sRGB profile. Cancellation, scratch failure and the RAM-only admission guard apply
before publishing a document. It allocates neither the whole compressed file nor full channels.
The bounded reader admits at most 8 MiB/4096 metadata attributes, 64 channels, 262144 pixels
per dimension and 64 MiB per uncompressed block. Tiled/deep/multipart files, subsampled
channels and explicit chromaticities retain the guarded legacy decoder; they are not covered
by this streaming route. EXR export still uses the whole-buffer path.

Native PSB saves write original-depth layer channels and a band-composited merged image into a
seekable temporary file, then publish it atomically. Cancellation or disk failure preserves the
previous destination. The streaming writer uses raw channels, so its file can be substantially
larger than a compressed source. Background saves capture a snapshot; later edits remain dirty,
and Save-before-close waits for publication. Whole-buffer formats and global filters that exceed
their admitted working memory fail gracefully rather than changing resolution.

Lazy `.pcraft` packs, streaming every codec and arbitrarily large smart-object blobs are still
open. Scratch files are temporary storage, not durable crash recovery; autosave is separate.

Adapted from robertocantore126/photocraft commit
`8c0d36bc1b6bfe79437202c36018885f21aa691e` (MIT OR Apache-2.0), with the current upstream
PSD importer retained, guarded read failures, and bounded color conversion batches.
