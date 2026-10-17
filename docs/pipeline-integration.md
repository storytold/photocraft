# Pipeline integration

How an asset pipeline tool drives PhotoCraft: a studio's publish and load manager (AYON and
similar), a DAM connector, or your own scripts. Such a tool runs as its own process, launches
PhotoCraft, and talks to it over the [control protocol](control-protocol.md). This page maps what
a host integration needs onto the calls that provide it; the protocol page has the details of
each.

PhotoCraft has no in-app extension panels or menu contributions, so the tool shows its own
windows (workfiles, loader, publisher) beside the app.

## Launching

```sh
PHOTOCRAFT_CONTROL_PORT=7878 \
PHOTOCRAFT_CONTROL_TOKEN_FILE=/private/path/photocraft-control.token \
PHOTOCRAFT_AUTOMATION_READ_ROOT=/projects/show \
PHOTOCRAFT_AUTOMATION_WRITE_ROOT=/projects/show \
photocraft /projects/show/sh010/work/sh010_paint_v003.psd
```

- Files named on the command line open at start, so "open the last workfile" needs no call.
- Each of these is also a command-line option (`--control`, `--control-token-file`,
  `--automation-read-root`, `--automation-write-root`); with the variables a launcher only has
  to extend the environment. Pick the port yourself, or pass `0` and read the bound port from
  the launch log on standard error.
- The token file is created (mode `0600` on Unix) when it does not exist; read the token from it.
- There is **one read root and one write root**. Workfiles, the published files you load and the
  folder you export into must all be beneath them, so point them at a common parent (the
  project root). Paths in requests are relative to the root or absolute beneath it.

## Talking to the app

One JSON object per line over loopback TCP; the first request authenticates. A minimal client:

```python
import json, socket

class PhotoCraft:
    def __init__(self, port, token):
        self.sock = socket.create_connection(("127.0.0.1", port))
        self.lines = self.sock.makefile("r", encoding="utf-8")
        self.last = 0
        self.call("auth", token=token)

    def call(self, method, **params):
        self.last += 1
        request = {"id": self.last, "method": method, "params": params}
        self.sock.sendall((json.dumps(request) + "\n").encode())
        while True:
            reply = json.loads(self.lines.readline())
            if reply.get("id") == self.last:
                break
        if not reply["ok"]:
            raise RuntimeError(reply["error"])
        return reply["result"]

    def run(self, command, **params):
        return self.call("engine.execute", command=command, params=params)
```

The app closes a connection that has been idle for 30 seconds: reconnect and authenticate again
before a call after a pause. A reply can take up to 60 seconds; for a large save pass
`wait=False` and poll `jobs.list`.

## What a host integration needs

| Need | Call |
|---|---|
| Current file, its name, unsaved changes | `ui.inspect` → `session.documents[session.active]`: `path`, `name`, `dirty`, `revision` |
| Open a workfile | `app.open {path}` |
| Save, save as, save the next version | `app.save {path?}`: a PSD, PSB or `.pcraft` path becomes the document's file |
| Load a published file into the document | `app.place {path}` → `{layer}`, an embedded Smart Object |
| Switch a loaded file to another version | `app.replaceContents {path, layer}`: same layer, transform, filters and masks |
| Read the layer tree | `document.inspect` → `layers` (groups nest under `children`) |
| Make a group for something to publish | `layer.new.group {name}`, `layer.groupLayers`, `layer.setProps` |
| Publish the whole image or a review | `app.save {path}` with a flat extension (`.png`, `.jpg`, `.tif`, …) writes a copy |
| Publish one layer or group | `app.exportLayer {path, layer, trim: false}`; add `overwrite: true` to replace an earlier export |
| Close | `file.close`, `app.quit` |

Engine commands (`document.inspect`, `layer.*`, `file.close`) go through `engine.execute`; the
`app.*` and `ui.*` rows are methods of their own.

```python
pc = PhotoCraft(7878, open(token_file).read().strip())
layer = pc.call("app.place", path="sh010/publish/bg/v012/bg.png")["layer"]
pc.call("app.replaceContents", path="sh010/publish/bg/v013/bg.png", layer=layer)
pc.call("app.exportLayer", path="sh010/work/staging/bg.png", layer=layer, trim=False, overwrite=True)
pc.call("app.save")
```

`app.exportLayer` writes only the layer's rendered pixels, never the document's channels, paths
or metadata, and never writes over the file of an open document, so a staging path can't
replace the workfile by mistake.

### Layer ids

The `id` that `document.inspect` reports addresses the layer in commands and belongs to the
session: a PSD's layers get new ones every time the file opens. Look a layer up again by name
or position after a reopen.

## No events

The protocol is request and reply; the app never calls out. To notice that the user switched,
saved or edited a document, poll `ui.inspect` and compare `session.active` and each document's
`path`, `dirty` and `revision`. Script events (File › Scripts › Script Events Manager) do not
fire for automation opens and saves.

## Headless

`photocraft-cli serve` answers the same envelope without a window, for a render farm or a batch
publish: `doc.open`, `doc.save`, `doc.place`, `doc.replaceContents`, `doc.exportLayer` and
`doc.exportLayers` are the scoped file methods there, and `engine.execute` runs the same
commands. See [Headless server](control-protocol.md#headless-server).
