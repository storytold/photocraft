# Generative AI

PhotoCraft fills a selection from a prompt (Edit › Generative Fill…) with the model the person
chose in the Craft suite's shared AI layer,
[craft-ai-integrations](https://github.com/storytold/craft-ai-integrations). The layer owns
connections, keys, which model does which job, consent records and the job log, for every Craft
app: a model set up once works in all of them. PhotoCraft owns its commands, its pixels and its
windows. This page follows the shape the maintainers asked for in issue #41 and on #2878.

Every Craft app works with no AI set up. Until a model is chosen for the catalogue's
`generative-fill` tool, Generative Fill is greyed out, the options bar has no button for it, and
nothing is contacted. The selection tools' options bar shows a Generative Fill… button once it can
run; Preferences › Integrations › Always show the Generative Fill… button in the options bar
(off by default) shows it before that too, and then it opens the setup.

## Setting it up

The Craft launcher's Set up AI window is where a model is set up for the whole suite. Until it
ships, PhotoCraft has a small setup of its own built from the same shared pieces, in
Edit › Preferences › AI Integrations… › Generative AI:

- **Generative AI** says whether Generative Fill can run. When it can, it names the model and
  connection and says whether it runs on this computer or online. When it can't, it shows the
  shared layer's reason as it is ("no model is chosen", "the key is missing", "this app can't run
  the chosen model yet").
- **Model for Generative Fill** lists every connected model that can do the job (the
  `image.inpaint` capability with a source picture, a mask and a prompt). Picking one makes it the
  suite's choice for the tool, as the launcher's would (`ToolDesk::choose`); a model that can't do
  the job is refused and nothing changes.
- **Connections** lists the saved connections, with where they run and what is wrong with one
  (no key saved, the key can't be read, switched off), and removes one (the choices that used it
  go with it).
- **Connect a service**: OpenAI, Black Forest Labs or Google Gemini with an API key, or a ComfyUI
  server by its address. Connect first asks the service for its models with that key (nothing from
  any document is sent), and saves the connection only when that works. The key is handed to the
  shared layer once, which keeps it in the OS keychain; it is never in PhotoCraft's preferences,
  in a dialog's fields (`ui.inspect`) or in the control channel.

Which services can fill today: at the pinned revision, the layer lists for `image.inpaint`
OpenAI's image models (GPT Image, and DALL·E 2, which takes square pictures only), Black Forest
Labs' FLUX.1 Fill [pro] and Google Gemini's image models (with an AI Studio key for the Gemini
API, not Vertex). Gemini takes no mask: it is shown the selection as a second picture with an
instruction, so it may change the picture outside it; PhotoCraft keeps only the selected part of
what comes back. A ComfyUI connection offers whatever inpainting workflows were imported into the
layer's workflow library, and an OpenAI-compatible server the inpainting models declared for it in
the launcher. PhotoCraft asks OpenAI for quality "high"; left to choose, it returned soft fills.

The shared folder (connections, consent, history) is the layer's: `CRAFT_AI_DIR` moves it, which
is what tests do so they never touch a person's real settings.

## Using it

1. Make a selection, then Edit › Generative Fill… (or the selection tools' Generative Fill…
   button, shown only while the tool can run). Describe what to put there, or leave the prompt
   empty to continue the surrounding picture through the selection, which removes what was
   there. The dialog says which model is used and whether it runs online.
2. If the model runs off this computer and nothing on file covers it, PhotoCraft asks first (see
   [Consent](#consent)). Nothing is sent until the person agrees.
3. Three variations come back, as in Photoshop. They land in a new group named after the prompt
   and masked by the selection, one layer per variation (Variation 1, 2, 3), with the first
   showing. Nothing under it changes; one undo removes it. The three are separate requests, sent
   side by side; if some fail and others succeed, the successes are kept.
4. With the group (or one of its variations) active, Properties › Variations shows a thumbnail
   per variation: click one to show it alone (`layer.generative.selectVariation`, one undo step).
   Generate More sends the same picture, mask and prompt again and adds three more
   (`layer.generative.more`), also one undo step.

Fills don't wait for each other, and nothing waits for them. Press Generate and the selection is
let go: select the next thing and start another while the first generates. Each fill in progress
shows on the canvas (its area under a moving shimmer, with a tag: generating, waiting its turn,
waiting for your OK, or failed; the time taken; Cancel, Retry, Review…), in the Layers panel (a
locked row where its group will land) and in the status bar (how many are generating, waiting,
need your OK or failed; click it for the list). At most 5 fills talk to the service at once; the
rest wait their turn. A request the service turns down for its rate limit is started again after
a pause (what the service asked for, else 4, 8 and 16 s). The result lands where it was started,
on its own document, without becoming the active layer; it is dropped if the canvas changed size
meanwhile or the document was closed. Cancel stops a fill's jobs at the service where it can.

**Several objects at once.** Select them together and the dialog offers *Fill each of the n
separate areas on its own* (on by default, up to 16 areas): each area becomes its own fill and
layer, generated side by side, from one prompt. One consent answer covers all of them.

Photoshop keeps its variations inside one generative layer. PhotoCraft uses a group of ordinary
layers instead, so a PSD round trip needs no format changes and other apps see plain layers. What
each group was generated from is kept in the session only: after reopening a file the group is an
ordinary group, and Generate More is off for it.

## What is sent, and where

Only when a fill or Generate More runs, only to the connection of the chosen model, once per
variation (three requests by default; a metered service bills each one):

- **The picture around the selection**: the selection's bounds plus a margin of context (the
  selection's longer side on every side, 64 to 2048 px), grown toward the nearest common output shape
  (square, 3:2 or 2:3, so a model that returns one of those isn't stretched) and kept inside the
  canvas, rendered from all visible layers in sRGB and scaled to at most 2048 px on the long side.
  The selected area itself is replaced by a dimmed soft blur of the colours around it: the model
  never sees the replaced content, as Photoshop's fill doesn't, so it can't paint it back. Every
  pixel the selection touches, even faintly along a feathered edge, is replaced whole, and only
  unselected pixels feed the blur; the feather lives in the mask alone.
- **The selection's shape**: a PNG the size of that picture, transparent (and white) where the
  fill goes, opaque black where the picture is kept, partly transparent along a feathered edge.
  Transparent-means-edit is the convention mask-based image models read (OpenAI's image edits,
  ComfyUI's image loader).
- **The prompt**, or for an empty prompt a fixed instruction to continue the surrounding picture
  through the masked area.

Where it goes is the connection's address (`https://api.openai.com`, say), or nowhere off the
computer for a model that runs on it. The shared layer reads the key only when a job starts, from
the OS keychain, and sends it to that address and nowhere else. Its job log records which app,
which connection, what kind of data and how long, never the prompt's text.

The result is colour-matched to the photo around the selection (models relight what they
redraw: a tone curve fitted outside the selection is undone, and what is left at the edge is
carried smoothly inward), scaled back into the area, and composited only inside the selection
through the group's mask.

## Consent

Before the first request to a connection off this computer, PhotoCraft shows its own prompt:
where it goes (the destination and the connection and model), and what is sent, each item with
its kind and size. The person answers with one of the four choices every Craft app offers: only
this time, for an hour, for a day, or until they turn it off. Send starts the fills; Don't Send
discards them, and nothing of them was sent.

Only the longer answers are remembered, in PhotoCraft's own consent file kept by the shared
layer (`client.consent_file()`, per app, so a yes in another app never covers PhotoCraft). "Only
this time" leaves nothing on disk and covers that fill's requests only; the next fill asks again.
The layer enforces it too: it refuses to start a job its consent record doesn't cover. PhotoCraft
adds no second "are you sure".

Automation can't do any of this on the person's behalf: the commands that send pixels
(`edit.generativeFill`, `layer.generative.more`, `.retry` and `.agree`) are refused to the control
channel, MCP and actions, and confirming the prompt or the consent dialog through the control
channel is judged as those commands. Agents can look (`layer.generative.fills`), flip variations
and discard fills.

## When it can't run, and errors

The shared layer is asked whether the tool can run each time it is about to be used (menu
enablement, the dialog, the options bar), not once at start-up, since the person may change
their choice in another Craft app meanwhile (PhotoCraft reuses an answer for at most a second, so
drawing the menus doesn't read the settings every frame). When it can't, the menu item is greyed,
the reason is the layer's own sentence, and Preferences › AI Integrations is where to fix it.

A fill that fails keeps its tag and row until Retry (sent again as it was started, with the model
chosen now) or Discard. Messages are the layer's, which name the connection and never hold a
key: a refused key or a model this app can't run offers the setup; a busy service says "try again
shortly" (after PhotoCraft's own retries); quota, network and service errors show the message; a
cancelled fill shows nothing.

## Commands

| Id | Does |
|---|---|
| `edit.generativeFill` | `{prompt?, variations: 1..4 = 3, area?}` fills the selection (or one of its separate areas) |
| `layer.generative.selectVariation` | `{index, layer?}` shows one variation of a generative layer |
| `layer.generative.more` | `{count: 1..4 = 3, layer?}` more variations of a generative layer |
| `layer.generative.agree` | `{fill, term}` the person agreed to send a fill waiting for consent |
| `layer.generative.retry` | `{fill}` sends a failed fill again |
| `layer.generative.discard` | `{fill}` or `{fills: [...]}` stops or forgets fills |
| `layer.generative.fills` | lists fills in progress (`queued`, `running`, `consent`, `failed`) and whether the tool can run |
| `layer.generative.areas` | the selection's separate areas |

## For developers

- **The seam.** `photocraft_engine::genai` defines `GenerativeAi` (status, plan, setup, connect,
  disconnect, choose), `FillPlan` (disclosure, needs consent, agree, start) and `FillJob` (poll,
  cancel). The engine and the UI use only these, so they still build for wasm and never open a
  socket. The desktop app implements them in `apps/photocraft/src/genai.rs` with
  `AiClient::standard(AppId::new("photocraft"))`, the catalogue's `generative-fill` role
  (`craft_ai_features::builtin()`), `ConnectionManager` and `ToolDesk`. The craft-ai crates are
  pinned to one revision in the workspace `Cargo.toml` and used only by the desktop app. Until
  storytold/craft-ai-integrations takes FLUX.1 Fill, Gemini fill and `Plan::provider`, that
  revision is on texjer's fork; it moves back to a storytold revision when they merge.
- **Jobs.** A fill's work runs on the engine's job system without locking the document
  (`jobs::run_on`): prepare the picture and mask, plan, check consent, take a turn, start one job
  per variation, poll them together, cancel them if the engine job is cancelled, then place the
  results in one undo step.
- **Tests with no network.** Engine and UI tests use a stand-in `GenerativeAi`
  (`crates/engine/src/cmd/genai_cmds/tests.rs`, `crates/ui-egui/src/genai_ui/tests.rs`). The
  desktop link is tested against the real shared client in a temporary folder with an in-memory
  key store and a scripted adapter on a `ProviderFactory` (`apps/photocraft/src/genai/tests.rs`,
  the craft-ai-integrations adopting-scene pattern): Ready and Unavailable, consent refused and
  agreed (only remembered terms written), a whole fill as one undo step composited inside the
  mask, cancel, error mapping, setup, and a new connection's key staying out of the settings file.

What the earlier provider-based branch had that this one doesn't: its own Gemini backend, HTTPS
transport and keychain code (the shared layer does that now), and the Edit selection mode with
reference images. The `generative-fill` tool's inputs are a source, a mask and a prompt, with no
reference images, so those wait for a tool in the catalogue that takes them.
