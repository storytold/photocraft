# photocraft-pdn

Bounded reader for native Paint.NET (`.pdn`) documents:

- Reads the PDN3 container: magic header, XML metadata, and indicator bytes.
- Parses the .NET Remoting Binary Format (MS-NRBF) serialized object graph without reflection or external runtime dependencies.
- Decompresses deferred pixel chunks (both GZIP compressed and uncompressed chunks).
- Decodes layer metadata: names, visibility, opacity, blend modes, dimensions and BGRA/RGBA pixels.
- Enforces strict safety limits on file size, header size, layer count, memory allocations, and recursion depth.
