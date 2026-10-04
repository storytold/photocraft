# MCP

`photocraft-automation` implements an MCP server over stdio with two backends:

- **Headless:** an in-process `Headless` session owns the engine and performs file I/O.
- **Bridge:** `BridgeClient` forwards tools to a desktop application started with `photocraft --control <port>`.

Start the headless server with:

```sh
photocraft-cli mcp
```

Bridge to a running desktop application with:

```sh
photocraft-cli mcp --bridge 127.0.0.1:7878
```

Tools cover session/document operations, command discovery and execution, batching, and—when bridged—UI inspection and control. Tool schemas improve correctness but are not authorization boundaries.

## Security notes

Stdio MCP does not open a network listener. Its client can still invoke file operations and engine commands using the MCP process's operating-system permissions. Bridge mode additionally inherits the unauthenticated control-channel limitation.

The current server does not issue scoped sessions or distinguish trusted tools from high-impact tools. A connected client is not restricted to document-only access. Do not expose the stdio transport through an untrusted broker, shell, or remote service without adding an external policy boundary.

Capability-scoped MCP access is [proposed](../security/automation-security.md), not implemented.
