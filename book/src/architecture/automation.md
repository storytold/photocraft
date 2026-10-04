# Automation architecture

PhotoCraft exposes the same engine through several transports:

```text
photocraft-cli commands/run/batch
             |
MCP stdio ---+--> photocraft-automation::Headless --> engine Session

MCP bridge --> BridgeClient --> 127.0.0.1 control server --> UI ControlRequest

JSON-lines stdio/TCP --> rpc::respond --> Headless::handle --> engine Session
```

Headless file operations are implemented in `crates/automation/src/files.rs`; RPC dispatch is in `crates/automation/src/rpc.rs`; MCP tools are registered in `crates/automation/src/server.rs`; and the live client is in `crates/automation/src/bridge.rs`. The desktop TCP listener is `apps/photocraft/src/control_server.rs`.

The control and headless TCP listeners are restricted to loopback addresses. Stdio MCP avoids a listening network socket, but the MCP process still acts with the filesystem permissions of the user who launched it.

Loopback is not authentication. Current transports do not implement authentication, capability scopes, allowed filesystem roots, request-byte limits, batch-step limits, or connection limits. See the [Automation security model](../automation/security-model.md) and [Automation security](../security/automation-security.md) before exposing or extending these surfaces.
