# Photoshop action files

## Targeted layer links

| Photoshop event | Descriptor | PhotoCraft command |
| --- | --- | --- |
| `linkSelectedLayers` | `null`: layer target (`Lyr `/`Ordn.Trgt`) | `layer.linkLayers {"linked":true}` |
| `unlinkSelectedLayers` | `null`: layer target (`Lyr `/`Ordn.Trgt`) | `layer.linkLayers {"linked":false}` |

These events act on the current layer selection. Link keeps an already linked
selection linked; Unlink keeps an already unlinked selection unlinked. Unlink
also clears a link group left with only one member. Undo restores the edit.

Omitting `linked` retains the native UI toggle. That ambiguous toggle cannot be
saved as either Photoshop event. Explicit layer lists and extra parameters also
cannot be saved as these events. Unsupported targets or extra descriptor keys
are kept as Photoshop steps for lossless saving, rather than partially replayed.
