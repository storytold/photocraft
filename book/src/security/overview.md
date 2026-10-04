# Security overview

PhotoCraft processes complex attacker-controlled files and exposes automation that can act with the user's filesystem permissions. Security is therefore part of parser, engine, I/O, automation, release, and dependency design.

## Current posture

| Area | Implemented protections | Known limitations / future work |
|---|---|---|
| PSD/PSB | Header, channel, decoded-byte, nesting, and pattern limits; malformed/property tests; fuzz targets | Broader hostile corpus, aggregate work budgets, embedded-object coverage |
| Raster codecs | Default dimension/pixel/allocation limits; decoder-specific budgets; malformed/property tests; fuzz targets | Continuous fuzzing and more format-specific semantic cases |
| `.pcraft` | Manifest/blob/total-byte limits; bounded decompression; CRC/hash and version checks; fuzz target | Tighter complexity/entry limits and directory-bundle path-race analysis |
| Engine commands | Error-return contract, focused bad-parameter tests, adversarial `panic_hunt` test | Continue eliminating input-derived panic and exhaustion paths |
| Control TCP | Loopback binding; 256-bit bearer-token authentication; 1 MiB request lines; 16 active connections; I/O timeouts | No capabilities, filesystem policy, encryption, client identity, JSON-depth/operation budgets, or audit trail |
| MCP | Stdio transport; typed tool schemas; headless/bridge separation; authenticated bridge; 256-step batch limit | No authorization scopes or filesystem policy |
| Dependencies | Cargo lockfile and normal CI build/test/clippy | No `cargo-audit` or `cargo-deny` job/configuration confirmed in current tree |
| Releases | Checksums and platform signing/notarization paths when secrets are configured | Signing can be absent; no SBOM generation confirmed in current tree |

The table is a snapshot of the current repository, not a guarantee about deployed artifacts. Verify source and release evidence for high-risk use.

## Security priorities

1. Add capability-scoped automation sessions on top of the authenticated transport.
2. Restrict automation file access through explicit workspace handles.
3. Add JSON-complexity, response-size, render, document-memory, and command-duration budgets.
4. Expand PSD/PSB, codec, and `.pcraft` fuzzing with regression corpora.
5. Add dependency advisories/license policy checks and release SBOMs.

Start with the [Threat model](threat-model.md), then use the focused pages for file handling, parsers, fuzzing, and automation.
