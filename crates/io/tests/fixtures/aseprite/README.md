# Original Aseprite fixture

`layers.aseprite` is an original 304-byte synthetic file, authored for PhotoCraft #3811
from the public Aseprite file-format specification. No Aseprite application code or assets
were used. It is licensed under MIT OR Apache-2.0, like PhotoCraft; see the adjacent license.

The canvas is 2 × 2 RGBA8, one frame. Header flags are 3 (valid layer opacity and isolated
groups). Bottom-to-top layers: `Base`, `Group`, and `Top` inside `Group`. Base contains four
opaque red pixels. Top is at (1,0), size 1 × 2, containing blue alpha 128 and opaque green.
The two cels are raw. The expected composite, row-major RGBA8, is approximately:
`(255,0,0,255), (127,0,128,255), (255,0,0,255), (0,255,0,255)`.

The independent MIT OR Apache-2.0 `aseprite-io` 0.2.0 reader also accepts the file and agrees
on canvas dimensions, frame count, layer names and parent indices. The unit tests build
additional fixtures in memory, including compressed and grayscale data.

`gray.aseprite` is an original 3 × 2 grayscale sprite generated using the independent
MIT OR Apache-2.0 `aseprite-io` 0.2.0 writer (no third-party artwork). Its one compressed
3 × 1 cel at (-1,1) has samples `(32,0), (128,128), (240,255)` in Gray/Alpha order.
The layer is named `Gray from independent writer`. Its exact samples are tested through
import and a native-format round trip. That library is a test-generation tool only, not
an app or test-suite dependency.

`animation.aseprite` is an original indexed 3 × 2, three-frame fixture generated with
aseprite-io 0.2.0. Delays are 17, 83 and 29 ms. A compressed 2 × 2 cel at (-1,0) stores
indices [1,2,3,0]; two red entries have distinct names and indices. Frame 1 links that
image at (0,0); frame 2 has no cel. A ping-pong Walk tag repeats twice. Sprite, layer,
cel and tag text metadata are original synthetic text. The fixture and its PhotoCraft
native/Aseprite round trips are checked against hand-computable sample expectations.

`tiles.aseprite` is an original 16 × 2 RGBA fixture generated with aseprite-io 0.2.0.
It has one embedded 2 × 2 tileset (stable index 42, two tiles including the empty tile),
one tilemap and all eight X/Y/diagonal flip combinations. Its four tile colors are red,
green, blue and yellow; tileset text metadata is synthetic. Tests independently enumerate
the expected transformed pixels and retain the tile records through native/Aseprite saves.

`application-indexed.aseprite`, `application-tiles.aseprite` and `application-blends.aseprite`
were saved by Aseprite 1.3.18.6-arm64 (Steam, macOS) from original synthetic data. Their adjacent
PNGs were rendered by that application, with no third-party artwork or application assets.
They use the same adjacent MIT OR Apache-2.0 license. The indexed fixture derives from
`animation.aseprite` with sprite/layer data set through the published Lua API, and adds an
original nested typed property and a named slice with center/pivot/text. The tiles fixture
derives from `tiles.aseprite`, with complete tileset/tile user-data sequences and original
`Tile note` text on tile1. Aseprite normalizes its own saved palette names and linked-cel
representation; the independent fixture separately tests exact original names/links.

The blend fixture is 8×19 RGBA, with an opaque `Base` and nineteen row-sized layers, one
for each documented blend mode in format-ID order. Eight colors (black, white, 53/149/231,
213/79/17, gray128, green, magenta, 32/64/192) form the base columns; foreground columns
reverse those colors with alternating alpha255/128. Each foreground layer has opacity207
and its cel opacity193. Its PNG tests all19 modes including separate cel/layer opacity
against the application, allowing at most2/255 in premultiplied channels for byte rounding.
This is a finite synthetic oracle, not an exhaustive blend-compatibility corpus.

The integration test compares every frame of each imported sprite, its native save/reload,
and its Aseprite re-export/reimport against the application-rendered sheet (horizontal frame
order, no trimming). Animation/tilemaps allow at most1/255. Locally the exported files were
also reopened/saved in Aseprite, and its published scripting API asserted frame durations,
cel offsets, tags, object text, nested properties, slices, and retained tilemap/tile data.
To regenerate PNGs with a licensed local installation:

```sh
aseprite --batch application-indexed.aseprite --sheet application-indexed.png
aseprite --batch application-tiles.aseprite --sheet application-tiles.png
aseprite --batch application-blends.aseprite --save-as application-blends.png
```

Application testing found two independent-writer compatibility differences: sprite user data
must precede layer definitions, and a tileset user-data chunk must be followed by one chunk
per tile (empty entries included). Export normalizes these sequences; regressions assert
both. `asefile` cannot read every valid user-data ordering, so its reader is supplementary.
