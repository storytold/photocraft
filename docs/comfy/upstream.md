# Upstream, forking and naming

Comfy PhotoCraft is a fork of [storytold/photocraft](https://github.com/storytold/photocraft)
(MIT OR Apache-2.0). This page records the obligations and the working agreement with upstream so
nobody has to rediscover them.

## Remotes and syncing

```text
upstream  https://github.com/storytold/photocraft.git          (read-only; their main)
origin    https://github.com/CodyChuGit/comfy-photocraft.git   (the fork; our branches)
```

- `main` on the fork mirrors upstream `main`; never commit to it directly.
- Feature work happens on `comfy-photocraft` (the integration branch) and short-lived topic
  branches off it. Rebase or merge upstream `main` into `comfy-photocraft` at least weekly;
  upstream moves fast (642 commits by 2026-10-08, several PRs a day).
- Keep every `Cargo.toml` valid at all times and use a separate `CARGO_TARGET_DIR` per parallel
  agent (upstream `AGENTS.md` §6).

```powershell
git fetch upstream
git switch comfy-photocraft
git merge upstream/main      # or: git rebase upstream/main while the branch is unpublished
```

## What goes upstream, what stays here

Upstream explicitly defers generative AI (`docs/roadmap.md`: "generative AI backend (#41)" is
"later / needs decisions"). The fork should make that decision easy for them:

| Change | Where |
|---|---|
| Bug fixes, performance work, parity items found while building generative features | upstream PR first, then merge back |
| A `GenerativeBackend` trait, the `photocraft-genai` crate and the `generate.*` commands | fork; offered upstream as one coherent PR series once stable, referencing #41 |
| Workflow templates, model catalogue, ComfyUI client | fork (even upstream would ship them as data, not core) |
| Fork branding, docs in `docs/comfy/` | fork only |

Upstream's contribution rules apply to anything sent back (clean-room, never-crash, tests,
layering, `cargo xtask parity`, dev log). Their `contributors/people.toml` rule: add only your own
entry, never anyone else's.

## Brand licence obligations (read before publishing a build)

`docs/brand/LICENSE-brand.txt` says the ArtCraft name, wordmark and logos are trademarks, not
open source, and:

> If you distribute or publish a modified version of PhotoCraft, or any work derived from it, you
> must remove the ArtCraft Marks from it (or replace them with your own) and must not call it
> ArtCraft or present it as an ArtCraft product. You may say, in plain text, that your work is
> based on PhotoCraft by the ArtCraft team. A fork kept only to propose changes back to this
> repository may keep the marks unchanged.

Consequences for this fork:

1. While the branch is documentation-only and intended to be proposed upstream, the marks may stay.
2. **Before the first published build or public release of a modified version**, remove or replace:
   `docs/brand/*` (and the README's logo `<img>` tags), the About window's ArtCraft branding
   (search `crates/ui-egui/src` for `artcraft`), installer/package metadata in `packaging/`, and
   any `getartcraft.com` links presented as "our" site. Keep a plain-text credit: "based on
   PhotoCraft by the ArtCraft team".
3. Do not use "ArtCraft" in the fork's name, icon, domain or social handles.
4. Fonts, icons and images in `ATTRIBUTION.md` keep their own licences; adding assets means adding
   rows there in the same change (upstream rule 3).

`PhotoCraft` itself is the product name upstream enforces in user-facing text (`AGENTS.md`); the
brand licence covers the *ArtCraft* marks. "Comfy PhotoCraft" is a working title that says what
the fork is; a published product needs a name decision (and a trademark check) by the owner. Until
then the app title stays "PhotoCraft" and the fork is identified in About and the README text.

**ShotoPhopEX's private defaults (since 2026-10-28).** Before any build for others, set
integrations.allowResearchModels back to false and integrations.engine back to uto (or
comfy) in crates/engine/src/prefs.rs: Qwen-Image-2.1 is research-only (models.md), and
the permissive default for fill, edit and expand is still Qwen-Image-Edit-2511.

## Things upstream has that we do not

- `../craftrules` (standards shared across the Crafting Apps) is private. `AGENTS.md` refers to it
  for never-crash, fonts and release rules; the public documents in `docs/` and `book/` carry
  enough of those rules to work by. Ask upstream maintainers if a standard matters for a PR.
- `../craft-fonts` (Japanese UI/Type fonts) is public and optional; builds without it work.
- Signing material for Windows releases (upstream is still obtaining theirs).

## Licence of the fork

The fork stays **MIT OR Apache-2.0** for all code. The generative features depend on model
weights with their own licences (see [`models.md`](models.md)); the app ships no weights, only
pointers, and shows each model's licence class in the picker.
