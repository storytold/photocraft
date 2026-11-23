# Secure file handling

PhotoCraft reads and writes documents in the desktop app, CLI, headless RPC server, and MCP server.
Remote automation paths are untrusted names resolved beneath explicitly granted roots.

## Implemented

- Format loaders return structured errors for I/O and malformed data.
- `.pcraft` ZIP loading validates archive offsets, entry sizes, CRCs, content hashes, and configured decompressed-size budgets.
- Raster and PSD loaders apply parser/decode limits before major pixel allocation.
- Native `.pcraft` saves use temporary-file/rename patterns in relevant store paths.
- `AuthorizedWorkspace` holds independent read and write directory capabilities for remote
  automation.
- Absolute paths beneath the canonical or configured spelling of the applicable granted root
  are translated to relative paths and resolved through the held directory capability. Absolute
  paths outside that root are rejected.
- Relative requests reject empty or `.` components and alternate separators. Absolute requests
  are split into native path components before checking the root-relative remainder, so repeated
  separators and interior `.` components are normalized. Both forms reject parent traversal in
  the remainder, disallowed component names and link escapes before file effects.
- Ordinary single-file writes support non-existent output files when their parent already exists
  beneath the write root; they do not create caller-selected directory trees.
- Filesystem-bearing engine commands that have not migrated to capability I/O fail closed for
  automation. Trusted local CLI subcommands and user-driven desktop file pickers keep their
  intended ambient authority.

## Known limitations

- Directory `.pcraft` bundles have not been documented as resistant to link replacement or time-of-check/time-of-use races.
- Root grants are process launch configuration, not per-document grants.
- General method capabilities and structured filesystem audit events are not implemented.
- Existing hard links inside a granted root are trusted root contents; the path policy cannot
  infer how they were created.

## Capability-based design

Prefer an opened directory capability or workspace handle over repeated string sanitization:

```text
AuthorizedWorkspace
  - open_document(relative_path)
  - save_document(relative_path)
  - export_render(relative_path)
```

The handle is created from an explicit launch-time grant, preserves separate read/write
permissions, checks absolute paths against the granted root and rejects escaping paths. File
contents are read and written through the held directory capability. On Windows UNC write roots,
directory creation and atomic-save renames use guarded ambient paths beneath the canonical root,
with each ancestor verified through its parent handle and held open. See
[Windows UNC write roots](../automation/security-model.md#windows-unc-write-roots) for the
provider and WSL limitations.
Ordinary single-file writes require an existing in-root parent. Capability-scoped multi-file
exports create their required export directory and missing ancestors beneath the write root
before writing files.

## Design requirements

- Canonicalization alone is insufficient: links can change after validation.
- Policy should operate on handles and relative components where platform APIs allow it.
- Reads and writes should have distinct capabilities.
- Replace, create-new, truncate, and directory creation semantics must be explicit.
- Logs should avoid file contents and should minimize disclosure of full private paths.
- Tests should cover `..`, absolute paths, mixed separators, case behavior, symlinks/junctions, non-existent targets, and replacement races on supported platforms.

OS-account, VM, container, or sandbox isolation remains appropriate defense in depth for
untrusted automation and for controls not yet covered by general method capabilities.
