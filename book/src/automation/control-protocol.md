# Control protocol

The desktop control protocol is newline-delimited JSON over a TCP listener bound to `127.0.0.1`. Each request carries `id`, `method`, and optional `params`; each reply includes the matching `id` and either a result or error.

```json
{"id":1,"method":"ui.inspect","params":{}}
```

The current method catalog includes engine execution and discovery, application open/save operations, UI inspection and input, screenshots, and document/control helpers. The authoritative method and parameter tables are maintained in [`docs/control-protocol.md`](https://github.com/Gh0stlyKn1ght/photocraft/blob/main/docs/control-protocol.md).

Implementation is split across:

- `apps/photocraft/src/control_server.rs`: loopback TCP transport;
- `crates/ui-egui/src/control.rs`: live application handlers;
- `crates/automation/src/bridge.rs`: MCP-to-GUI client.

## Current security posture

**Implemented:** the desktop server binds to IPv4 loopback, the bridge accepts only loopback-style addresses, and request handlers use a 60-second response timeout.

**Known limitations:** there is no authentication handshake, encryption, client identity, per-method capability check, request-line byte limit, connection limit, or bounded worker pool. The listener creates one thread per accepted connection. Any local process able to connect can attempt exposed operations with PhotoCraft's user permissions.

Enable `--control` only for the duration of a trusted local automation session. See [Automation security](../security/automation-security.md) for the proposed gateway model.
