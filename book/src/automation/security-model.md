# Automation security model

Automation requests are untrusted input with potential effects beyond the active document. Opening, saving, rendering to a path, preferences changes, application control, pointer/type injection, and arbitrary engine commands have different risk levels but currently share broad transport access.

| Control | Current status |
|---|---|
| Loopback-only TCP binding | **Implemented** for desktop control and headless TCP serve |
| Stdio transport | **Implemented** for MCP and headless JSON lines |
| Command-level error handling | **Implemented**, with command-specific tests and `panic_hunt` coverage |
| Control-channel authentication | **Known limitation:** absent |
| MCP capability scopes | **Known limitation:** absent |
| Allowed read/write roots | **Known limitation:** absent |
| Symlink-safe capability filesystem | **Known limitation:** absent |
| Request-byte and JSON-depth limits | **Known limitation:** absent at the transport boundary |
| Batch-step limit | **Known limitation:** absent |
| Connection/worker limit | **Known limitation:** absent for TCP servers |
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

A token alone would identify possession of a secret but would not limit what that client can do. The proposed design combines a random per-launch credential with explicit capabilities, filesystem handles, resource budgets, and command policy. Until those controls land, process isolation and least-privileged execution are the practical containment mechanisms.
