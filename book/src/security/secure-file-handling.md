# Secure file handling

PhotoCraft reads and writes documents in the desktop app, CLI, headless RPC server, and MCP server. Current automation accepts caller-supplied strings and converts them directly to `PathBuf`; render-to-file uses `std::fs::write`, and shared save code may create parent directories.

## Implemented

- Format loaders return structured errors for I/O and malformed data.
- `.pcraft` ZIP loading validates archive offsets, entry sizes, CRCs, content hashes, and configured decompressed-size budgets.
- Raster and PSD loaders apply parser/decode limits before major pixel allocation.
- Native `.pcraft` saves use temporary-file/rename patterns in relevant store paths.

These are file-content protections, not path authorization.

## Known limitations

- Automation has no configured read or write root.
- Absolute paths and parent traversal are not rejected by a central policy.
- Canonicalization and symlink/junction handling are not enforced as an authorization boundary.
- Directory `.pcraft` bundles have not been documented as resistant to link replacement or time-of-check/time-of-use races.
- The same automation session can combine document access and general filesystem effects.

## Proposed capability-based design

Prefer an opened directory capability or workspace handle over repeated string sanitization:

```text
AuthorizedWorkspace
  - open_document(relative_path)
  - save_document(relative_path)
  - export_render(relative_path)
```

The handle should be created from an explicit user grant, preserve separate read/write permissions, reject absolute and escaping paths, define link behavior, and perform operations relative to the held directory capability. New files require a parent-directory handle because a non-existent target cannot be canonicalized safely.

## Design requirements

- Canonicalization alone is insufficient: links can change after validation.
- Policy should operate on handles and relative components where platform APIs allow it.
- Reads and writes should have distinct capabilities.
- Replace, create-new, truncate, and directory creation semantics must be explicit.
- Logs should avoid file contents and should minimize disclosure of full private paths.
- Tests should cover `..`, absolute paths, mixed separators, case behavior, symlinks/junctions, non-existent targets, and replacement races on supported platforms.

Until this design is implemented, run untrusted automation inside an OS account, VM, container, or sandbox whose filesystem access already matches the intended workspace.
