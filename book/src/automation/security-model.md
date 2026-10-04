# Automation security model

Automation requests are untrusted input with potential effects beyond the active document. Opening, saving, rendering to a path, preferences changes, application control, pointer/type injection, and arbitrary engine commands have different risk levels but currently share broad transport access.

| Control | Current status |
|---|---|
| Loopback-only TCP binding | **Implemented** for desktop control and headless TCP serve |
| Stdio transport | **Implemented** for MCP and headless JSON lines |
| Command-level error handling | **Implemented**, with command-specific tests and `panic_hunt` coverage |
| Control-channel authentication | **Implemented:** 256-bit bearer token required before TCP method dispatch |
| MCP capability scopes | **Known limitation:** absent |
| Allowed read/write roots | **Known limitation:** absent |
| Symlink-safe capability filesystem | **Known limitation:** absent |
| Request-byte and JSON-depth limits | **Partial:** 1 MiB request-line limit; no explicit JSON-depth policy |
| Batch-step limit | **Implemented:** 256 steps for headless and MCP batches |
| Connection/worker limit | **Implemented:** 16 active TCP connections; one worker thread per accepted active connection |
| Security audit events | **Proposed** |

## Proposed gateway

```text
MCP or control client
          |
 authenticated session
          v
 security gateway
   - capabilities
   - path handles/roots
   - request budgets
   - command policy
   - audit events
          |
          v
 command engine / UI shell
```

The current token proves possession of a secret but does not limit what that client can do. The remaining design combines the authenticated transport with explicit capabilities, filesystem handles, resource budgets, and command policy. Until those controls land, private token handling, process isolation, and least-privileged execution are practical containment mechanisms.
