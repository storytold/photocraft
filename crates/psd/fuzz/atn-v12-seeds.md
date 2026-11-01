The small synthetic sets in `corpus/atn/v12-*.atn` were assembled from the measured grammar
in #10603, without using PhotoCraft's parser or writer. No proprietary files or
assets are included. They also serve as inputs for `tests/it/atn_v12.rs`.

- `v12-stop.atn`: Stop with Unicode text and a Boolean.
- `v12-values.atn`: named plug-in event, every legacy value/reference form,
  bare `warp`/`list`/`time` codes, an object array with three objects and two
  columns, unpaired UTF-16, opaque path/alias/data bytes, and a descriptorless step.
- `v12-commands.atn`: Gaussian Blur, Levels, Select All and Make Layer.

`Idnt` follows the measured reference's inferred index-like layout; no real
version 12 identifier reference was available. Saving these sets produces
version 16. Keep them outside the version 16 byte-identical corpus oracle.
