using System.Buffers.Binary;
using System.Text;
using System.Text.Json;

namespace PhotoCraft.OnnxQnn;

internal sealed record ImageMetadata(int x, int y, int width, int height, int channels,
    string color_mode, int source_depth, bool alpha, int[] icc_profile);
internal sealed record RunHeader(int protocol, string operation, string? resource, JsonElement @params,
    string? device, bool allow_fallback, ImageMetadata image, ImageMetadata? selection);
internal sealed record InputImage(RunHeader Header, float[] Samples);
internal sealed record MaskOutput(string kind, ImageMetadata image);
internal sealed record Execution(string[] devices, string backend, string? fallback_reason);
// Exactly the Rust v1 response keys, including nulls. Do not add descriptor/readiness extensions.
internal sealed record ResponseHeader(int protocol, string status, string message,
    MaskOutput? output = null, Execution? execution = null);
internal sealed record Reply(ResponseHeader Header, float[]? Mask = null)
{
    internal static Reply Status(string status, string message) => new(new ResponseHeader(1, status, message));
}

internal static class Protocol
{
    internal const string Operation = "select-subject";
    internal const string Device = "qnn.htp";
    internal const string Backend = "qnn";
    internal const int MaxHeaderBytes = 1024 * 1024;
    internal const int MaxPixels = 16 * 1024 * 1024;
    internal const int MaxSide = 16384;
    private static readonly byte[] RequestMagic = "PCADREQ1"u8.ToArray();

    internal static JsonDocument ReadHeader(Stream input)
    {
        if (!input.CanSeek) throw new WorkerException("A seekable bounded request file is required.");
        if (input.Length > 12L + MaxHeaderBytes + (long)MaxPixels * 5 * 4)
            throw new WorkerException("Request exceeds worker byte limit.");
        Span<byte> prefix = stackalloc byte[12];
        input.ReadExactly(prefix);
        if (!prefix[..8].SequenceEqual(RequestMagic)) throw new WorkerException("Expected PCADREQ1 request magic.");
        uint length = BinaryPrimitives.ReadUInt32LittleEndian(prefix[8..]);
        if (length is 0 or > MaxHeaderBytes) throw new WorkerException("JSON header must be 1..1048576 bytes.");
        if (input.Length - input.Position < length) throw new WorkerException("Truncated JSON header.");
        var bytes = new byte[(int)length];
        input.ReadExactly(bytes);
        var document = Json.Parse(bytes);
        try
        {
            if (!document.RootElement.TryGetProperty("protocol", out var protocol) || protocol.GetInt32() != 1)
                throw new WorkerException("Only adapter protocol 1 is supported.");
            return document;
        }
        catch { document.Dispose(); throw; }
    }

    internal static void Control(JsonElement root, Stream input)
    {
        Json.Fields(root, "protocol");
        End(input);
    }

    internal static InputImage ReadImage(JsonElement root, Stream input)
    {
        Json.Fields(root, "protocol", "operation", "resource", "params", "device", "allow_fallback", "image", "selection");
        ImageFields(root.GetProperty("image"));
        if (root.GetProperty("selection").ValueKind != JsonValueKind.Null) ImageFields(root.GetProperty("selection"));
        var header = root.Deserialize<RunHeader>(Json.Options) ?? throw new WorkerException("Missing run header.");
        if (header.operation != Operation) throw new WorkerException("Only select-subject/mask is supported.");
        if (header.device is not null && header.device != Device) throw new WorkerException("Only qnn.htp is supported; no fallback exists.");
        if (header.resource is not null && !Json.Id(header.resource))
            throw new WorkerException("resource must be an opaque registered model ID, never a path or URL.");
        if (header.@params.ValueKind != JsonValueKind.Object) throw new WorkerException("params must be an object.");
        Json.Fields(header.@params, "colorPolicy");
        if (header.@params.GetProperty("colorPolicy").GetString() != "assume-srgb")
            throw new WorkerException("Explicit params.colorPolicy='assume-srgb' is required for untagged RGB.");
        var image = header.image ?? throw new WorkerException("Run requires image metadata.");
        int pixels = Geometry(image);
        if (image.color_mode != "rgb" || image.channels != (image.alpha ? 4 : 3))
            throw new WorkerException("Only RGB with 3 channels or 4 straight-alpha-last channels is supported.");
        if (image.source_depth is not (8 or 16 or 32)) throw new WorkerException("source_depth must be 8, 16 or 32; samples are f32le.");
        if (image.icc_profile is null || image.icc_profile.Length != 0)
            throw new WorkerException("ICC-tagged input is unsupported: convert explicitly in the color-managed host; no ICC is ignored or substituted.");
        if (header.selection is { } selection)
        {
            Geometry(selection);
            if (selection.x != image.x || selection.y != image.y || selection.width != image.width ||
                selection.height != image.height || selection.channels != 1 || selection.alpha ||
                selection.color_mode is not ("gray" or "grayscale") || selection.source_depth != 32 ||
                selection.icc_profile is null || selection.icc_profile.Length != 0)
                throw new WorkerException("Selection must be same-geometry, one-channel profile-free gray F32.");
        }
        int count = checked(pixels * image.channels);
        long bytes = checked(((long)count + (header.selection is null ? 0 : pixels)) * 4);
        if (input.Length - input.Position != bytes) throw new WorkerException("Sample payload length does not match metadata.");
        var samples = new float[count];
        using var reader = new BinaryReader(input, Encoding.UTF8, leaveOpen: true);
        for (int i = 0; i < count; i++) samples[i] = Sdr(reader.ReadSingle());
        // Optional selection is validated but not used as a seed or multiplied into the authoritative mask.
        if (header.selection is not null)
            for (int i = 0; i < pixels; i++) Sdr(reader.ReadSingle());
        End(input);
        return new(header, samples);
    }

    private static void ImageFields(JsonElement image) => Json.Fields(image, "x", "y", "width", "height", "channels", "color_mode", "source_depth", "alpha", "icc_profile");

    internal static int Geometry(ImageMetadata image)
    {
        if (image.width is <= 0 or > MaxSide || image.height is <= 0 or > MaxSide ||
            (long)image.width * image.height > MaxPixels ||
            (long)image.x + image.width > int.MaxValue || (long)image.y + image.height > int.MaxValue)
            throw new WorkerException("Invalid/oversized image geometry (maximum 16 Mi pixels, side 16384, i32 endpoints).");
        return checked(image.width * image.height);
    }

    internal static float Sdr(float sample)
    {
        if (!float.IsFinite(sample) || sample < 0 || sample > 1)
            throw new WorkerException("Only finite SDR [0,1] RGB/alpha is supported; HDR is rejected, not clamped.");
        return sample;
    }
    private static void End(Stream input)
    {
        if (input.ReadByte() != -1) throw new WorkerException("Unexpected trailing request bytes.");
    }

    internal static void Write(Stream stream, Reply reply)
    {
        var h = reply.Header;
        if (h.protocol != 1 || h.status is not ("ok" or "ready" or "needs_model" or "unavailable" or "error") ||
            h.message is null || Encoding.UTF8.GetByteCount(h.message) > 1024 || h.message.Any(char.IsControl))
            throw new WorkerException("Invalid response status/message.");
        if (h.status == "ok")
        {
            if (h.output is not { kind: "mask" } output || h.execution is not { } execution || reply.Mask is not { } mask ||
                execution.backend != Backend || execution.devices.Length != 1 || execution.devices[0] != Device || execution.fallback_reason is not null)
                throw new WorkerException("Invalid successful mask response.");
            var image = output.image;
            if (Geometry(image) != mask.Length || image.channels != 1 || image.alpha || image.color_mode != "gray" ||
                image.source_depth != 32 || image.icc_profile.Length != 0)
                throw new WorkerException("Invalid mask metadata/size.");
            foreach (float value in mask) Sdr(value);
        }
        else if (h.output is not null || h.execution is not null || reply.Mask is not null)
            throw new WorkerException("Status replies must have null output/execution and no payload.");
        byte[] json = JsonSerializer.SerializeToUtf8Bytes(h, Json.Options);
        if (json.Length > MaxHeaderBytes) throw new WorkerException("Oversized response header.");
        using var writer = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: true);
        writer.Write("PCADRES1"u8);
        writer.Write((uint)json.Length);
        writer.Write(json);
        if (reply.Mask is { } data) foreach (float value in data) writer.Write(value);
        writer.Flush();
    }

    internal static ImageMetadata MaskMetadata(ImageMetadata source) =>
        new(source.x, source.y, source.width, source.height, 1, "gray", 32, false, []);

    internal static void Publish(string path, Reply reply)
    {
        string directory = Path.GetDirectoryName(path) ?? throw new WorkerException("Response must have a parent directory.");
        string temporary = Path.Combine(directory, ".photocraft-qnn-" + Guid.NewGuid().ToString("N"));
        try
        {
            using (var file = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            { Write(file, reply); file.Flush(flushToDisk: true); }
            File.Move(temporary, path, overwrite: false);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}

internal sealed record Arguments(string Action, string Request, string Response)
{
    internal static Arguments Parse(string[] args)
    {
        if (args.Length != 6 || args[0] != "--action" || args[2] != "--request" || args[4] != "--response" ||
            args[1] is not ("probe" or "prepare" or "run")) throw new WorkerException("Expected exactly --action probe|prepare|run --request absolute --response absolute.");
        string request = LocalPath(args[3]);
        string response = LocalPath(args[5]);
        if (request.Equals(response, StringComparison.OrdinalIgnoreCase) ||
            !string.Equals(Path.GetDirectoryName(request), Path.GetDirectoryName(response), StringComparison.OrdinalIgnoreCase) || File.Exists(response))
            throw new WorkerException("Request/response must be distinct files in one private job directory; response must not exist.");
        return new(args[1], request, response);
    }
    private static string LocalPath(string path)
    {
        if (!Path.IsPathFullyQualified(path) || path.Length < 4 || !char.IsAsciiLetter(path[0]) || path[1] != ':' ||
            path[2] != '\\' || path.IndexOf(':', 2) >= 0)
            throw new WorkerException("Only absolute local drive paths are accepted; no UNC/device paths or alternate data streams.");
        return Path.GetFullPath(path);
    }
}
