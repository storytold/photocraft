# Remote CLI command files

`photocraft-cli run --control 7878 --control-token-file control.token build.jsonl`
runs commands sequentially against a running `photocraft --control 7878` or
`photocraft-cli serve --port 7878`. Both sides need matching control credentials.
`--token-file` aliases `--control-token-file`; the existing
`PHOTOCRAFT_CONTROL_TOKEN` / `PHOTOCRAFT_CONTROL_TOKEN_FILE` inputs also work.
Connections stay on loopback. Remote run cannot be combined with local run flags.

Each UTF-8 JSONL line is an object with a nonempty `command`, optional object
`params` (defaults to `{}`), and optional `saveAs`. Other fields, blank lines,
empty files and duplicate capture names are errors. For example:

```jsonl
{"command":"file.new","params":{"width":64,"height":64}}
{"command":"layer.new.layer","params":{"name":"Ink"},"saveAs":"ink"}
{"command":"layer.renameLayer","params":{"layer":"$ink/layer","name":"Scripted ink"}}
```

`saveAs` captures the whole JSON command result. Names contain 1–64 ASCII letters,
digits or underscores. A string value `$name` inserts that captured JSON value;
`$name/field` selects a field with a JSON pointer (arrays use `/0`, escaped `~`
uses `~0`, escaped `/` uses `~1`). Substitution preserves numbers, booleans,
objects, arrays and null; it works recursively inside params, never object keys.
`$$name` becomes the literal `$name`. References occupy the entire string;
there is no interpolation, shell execution or expression evaluation.

All line shapes, reference syntax and capture ordering are checked before any
connection or command. A pointer's existence can only be checked once its result
has arrived. A failed reference, command or connection stops the script with its
line number. Earlier commands remain applied; this is not a transaction.
A lost reply may mean an edit completed: inspect state before rerunning. The
runner never retries a dispatched command. Server error text is kept out of local
errors because it can echo request data; inspect server diagnostics when refused.

Results are printed as JSONL objects `{ "command": ..., "result": ... }`.
Limits are 8 MiB per script, 4096 lines, 256 KiB per line, 64 levels of params,
256 captures, and 8 MiB of captured results or substituted values per command.
The server's request/response budgets also apply. File access remains subject to
the control server's automation roots and command policy; see
[the control protocol](control-protocol.md).
