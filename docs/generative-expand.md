# Generative Expand

Image › Generative Expand… grows the canvas and fills the new area with an AI model, so a
photo can gain sky above, floor below or room at the sides without a visible seam. The model
runs in a separate helper program on the user's own computer; PhotoCraft talks to it over
local HTTP and never ships a model. The feature is **off by default** and shows no menu item
until the user turns it on and accepts the privacy notice (upstream issue #41).

## Using it

1. **Install the helper once.** [IOPaint](https://github.com/Sanster/IOPaint) (Apache-2.0) is
   a Python program that serves inpainting models. In a terminal:

   ```sh
   python3 -m venv iopaint-env
   iopaint-env/bin/pip install iopaint
   ```

2. **Start it** before editing (it keeps running until the terminal closes). The first start
   downloads the LaMa model (about 200 MB) into the model folder:

   ```sh
   iopaint-env/bin/iopaint start --model=lama --device=cpu --host 127.0.0.1 --port 8080 --model-dir ~/iopaint-models --no-inbrowser
   ```

   `--device=cpu` works on every machine; on a Mac with Apple silicon `--device=mps` is
   faster, and `--device=cuda` uses an NVIDIA GPU.

3. **Turn the feature on** in Edit › Preferences › AI Integrations…: tick *Enable Generative
   Expand*, keep the address `http://127.0.0.1:8080` (or type the one the helper printed),
   read the privacy notice under the settings and tick *I have read the privacy notice below
   and accept it*. The Image menu gains *Generative Expand…* as soon as all three are set; it
   disappears again when any of them is cleared.

4. **Run it.** Open a photo, choose Image › Generative Expand…, type how many pixels to add on
   each side (one side is enough) and press OK. The canvas grows transparently, the helper
   fills the new area, and the result lands on a new layer named "Generative Expand 1" above
   the active layer. The status bar shows progress with a Cancel button. On a CPU, count on
   seconds to minutes: adding 25% on one side of a 1280 px photo took about 8 s on an Apple M1,
   adding 320 px right and 120 px bottom to the same photo about 2.5 min, because the helper
   then works on almost the whole image. One undo step removes everything.

The new layer is ordinary pixels: mask it, repaint parts, blend it, or merge it down. A
32-pixel band inside the old edge is regenerated too and blended back with a soft falloff,
which hides the seam.

## What to expect from LaMa

LaMa (Apache-2.0, [advimman/lama](https://github.com/advimman/lama)) is a small
"erase and continue" model: it continues textures, gradients and simple structures (sky,
walls, floors, foliage, water) convincingly, and it needs no prompt and no GPU. It does not
invent new objects: a person cut by the frame will not grow a second arm, and fine repeated
detail (lettering, crater fields, crowds) fades to a smooth texture. For those cases, extend by
less, or run it in two steps and retouch in between. A provider that takes a prompt
(`accepts_prompt() == true`) gets a Prompt field in the dialog automatically.

## Privacy

Nothing is sent anywhere until the user runs the command. Each run sends a copy of the
**visible composite** of the document, plus a mask of the area to fill, to the address in
Preferences, and reads the answer back. With IOPaint on the same machine the pixels never leave
it. The notice saying so is shown under the settings and must be accepted before the menu item
exists; the dialog refuses to open without it. No credentials are involved (the helper has
none); a provider that needs a key would store it through the platform keychain, not in the
preferences file.

## Architecture

| Piece | Where | What it does |
|---|---|---|
| Provider interface | `crates/ml` (`photocraft-ml`, layer 4) | `GenerativeProvider` (info, `accepts_prompt`, `ping`, `generate` with a cancel callback), `RgbaImage`/`Mask` buffers, `expand_mask` and `expand_coverage` for the new-area mask and the blend band, PNG codec. No document, layer or UI knowledge. |
| Transport | `crates/ml/src/transport.rs` | `Transport` trait: `UreqTransport` (plain HTTP, native only, behind the `http` cargo feature) and `FakeTransport` for tests. The engine and the web build compile without an HTTP client. |
| IOPaint provider | `crates/ml/src/iopaint.rs` | `POST /api/v1/inpaint` with base64 PNGs; `GET /api/v1/server-config` as the ping. Asks for IOPaint's crop strategy so time scales with the filled area, not the canvas. |
| Engine command | `crates/engine/src/cmd/generative_cmds.rs` | `image.generativeExpand {left,right,top,bottom,margin,prompt?,seed?}`: a background job (cancellable, progress in the status bar) that composites the visible image, calls the provider, grows the canvas (the Canvas Size edit) and adds the result layer, as one history step. Also `generative.status` and `generative.ping`. The session holds `Option<Arc<dyn GenerativeProvider>>`; `None` means the feature is absent. |
| Preferences | `Integrations` in `crates/engine/src/prefs.rs` | `generativeEnabled`, `generativeEndpoint`, `generativePrivacyAccepted` on the existing AI Integrations page. |
| Shell | `crates/ui-egui/src/generative_ui.rs` | Once per frame, builds or drops the session's provider when those three settings change (through `Services::generative_factory`), owns the privacy notice, and the dialog in `menus/view_cmds/dialogs.rs` asks for the amounts. The Image menu item is placed after Canvas Size… and removed while no provider exists. |
| App | `apps/photocraft/src/services.rs` | The factory: `IoPaintProvider` over `UreqTransport` (10-minute request timeout). Only the desktop app links the HTTP client; the web build has no factory, so the feature is absent there. |

Adding another backend (a different local server, or a remote API) means one more
`GenerativeProvider` implementation in `crates/ml` and a factory choice in the app; the engine
command, the dialog and the preferences stay as they are.

## Driving it from the control channel or MCP

```json
{"command": "prefs.set", "params": {"values": {"integrations.generativeEnabled": true, "integrations.generativePrivacyAccepted": true}}}
{"command": "generative.status"}
{"command": "image.generativeExpand", "params": {"right": 320, "bottom": 120}}
```

`generative.status` answers `{configured, name?, destination?, acceptsPrompt?}`;
`generative.ping` is a job that reports whether the helper answers (no pixels sent). Like
every long command, `image.generativeExpand` runs as a background job: pass `"wait": false`
to get `{job, pending: true}` and poll `jobs.list`.

## Troubleshooting

- **No Image › Generative Expand… item:** one of the three settings is not set, or the
  address was refused (the status bar says why; only `http://` addresses work, the helper has
  no TLS).
- **"cannot reach the generative helper":** the helper is not running, or listens on another
  port. Run `generative.ping`, or open the address in a browser: IOPaint shows its page.
- **Slow:** large canvases, or two sides at once, take minutes on a CPU; the status bar's
  Cancel stops the job and leaves the document unchanged. Extend one side at a time, by less,
  or start the helper on a GPU. The helper needs about 1.5 GB of memory with LaMa loaded; on
  an 8 GB machine close other large programs first, or it can be killed mid-request (PhotoCraft
  then reports "cannot reach the generative helper" and the document is unchanged).
- **Blurry far edge:** expected from LaMa beyond a few hundred pixels; extend in steps.

## Tests

`cargo test -p photocraft-ml` (provider, mask, transport and codec tests; `--test
live_iopaint -- --ignored` talks to a running helper), `cargo test -p photocraft-engine --lib
generative_cmds` (the command against a fake provider: layers, offsets, blend band, undo,
cancellation, bad params) and `cargo test -p photocraft-ui-egui --lib generative_ui`
(the preferences-to-provider lifecycle and the menu item).
