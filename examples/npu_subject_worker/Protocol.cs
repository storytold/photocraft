using System.Buffers.Binary;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace PhotoCraft.WindowsAI;

internal sealed record ImageMetadata(
    int x, int y, int width, int height, int channels, string color_mode,
    int source_depth, bool alpha, int[] icc_profile);

internal sealed record RequestHeader(
    int protocol, string? operation, string? resource, JsonElement @params,
    string? device, bool allow_fallback, ImageMetadata? image, ImageMetadata? selection);

internal sealed record MaskOutput(string kind, ImageMetadata image);
internal sealed record Execution(string[] devices, string backend, string? fallback_reason);
// Protocol 1 deliberately has exactly these five wire keys. The host rejects unknown
// fields, even when null; readiness details belong in message, not extra properties.
internal sealed record ResponseHeader(
    int protocol, string status, string message, MaskOutput? output = null,
    Execution? execution = null);

internal sealed record InputImage(RequestHeader Header, ushort[] Rgba64);
internal sealed class ProtocolException(string message) : Exception(message);

/// <summary>Bounded file framing shared with PhotoCraft's external adapter host.</summary>
internal static class Protocol
{
    internal const int MaxHeaderBytes = 1024 * 1024;
    internal const int MaxPixels = 36_000_000;
    internal const int MaxSide = 16384;
    internal const int MaxIccBytes = 4 * 1024 * 1024;
    internal const string DeviceId = "windows-ai.npu";
    internal const string Backend = "windows-ai";
    internal const string Operation = "select-subject";
    private static readonly byte[] RequestMagic = "PCADREQ1"u8.ToArray();
    private static readonly byte[] ResponseMagic = "PCADRES1"u8.ToArray();
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        MaxDepth = 32,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
    };
    // Required null-valued fields must still be serialized in responses.
    private static readonly JsonSerializerOptions ResponseOptions = new() { MaxDepth = 32 };

    internal static FileStream OpenRequest(string path) =>
        new(path, FileMode.Open, FileAccess.Read, FileShare.Read, 64 * 1024, FileOptions.SequentialScan);

    internal static RequestHeader ReadHeader(Stream input)
    {
        Span<byte> prefix = stackalloc byte[12];
        input.ReadExactly(prefix);
        if (!prefix[..8].SequenceEqual(RequestMagic))
            throw new ProtocolException("Invalid request magic; expected PCADREQ1.");
        uint size = BinaryPrimitives.ReadUInt32LittleEndian(prefix[8..]);
        if (size is 0 or > MaxHeaderBytes)
            throw new ProtocolException("Request JSON header must be 1..1048576 bytes.");
        byte[] headerBytes = new byte[(int)size];
        input.ReadExactly(headerBytes);
        using var document = JsonDocument.Parse(headerBytes, new JsonDocumentOptions { MaxDepth = 32 });
        var root = document.RootElement;
        ValidateUniqueKeys(root);
        if (root.ValueKind != JsonValueKind.Object) throw new ProtocolException("Request header must be an object.");
        RequireFields(root, "protocol");
        if (root.TryGetProperty("operation", out _))
        {
            RequireFields(root, "operation", "params", "allow_fallback", "image");
            ValidateImageFields(root.GetProperty("image"));
            if (root.TryGetProperty("selection", out var selection) && selection.ValueKind != JsonValueKind.Null)
                ValidateImageFields(selection);
        }
        else if (root.EnumerateObject().Count() != 1)
            throw new ProtocolException("Probe/prepare requests must contain only {\"protocol\":1}.");
        var header = JsonSerializer.Deserialize<RequestHeader>(headerBytes, JsonOptions)
            ?? throw new ProtocolException("Request header is null.");
        if (header.protocol != 1)
            throw new ProtocolException("Only adapter protocol 1 is supported.");
        return header;
    }

    internal static void ValidateControlRequest(RequestHeader header, Stream input)
    {
        if (header.image is not null || header.selection is not null || header.operation is not null ||
            header.resource is not null || header.device is not null || header.allow_fallback ||
            header.@params.ValueKind is not (JsonValueKind.Undefined or JsonValueKind.Null))
            throw new ProtocolException("Probe/prepare requests must contain only {\"protocol\":1}.");
        RequireEnd(input);
    }

    internal static InputImage ReadImage(RequestHeader header, Stream input)
    {
        if (header.operation != Operation)
            throw new ProtocolException("This adapter supports only select-subject.");
        if (header.resource is not null)
            throw new ProtocolException("Windows AI uses its system model; resource must be null.");
        if (header.device is not null && header.device != DeviceId)
            throw new ProtocolException("The only supported device is windows-ai.npu; no fallback exists.");
        var image = header.image ?? throw new ProtocolException("Run requires image metadata.");
        int pixels = ValidateGeometry(image);
        if (image.color_mode != "rgb" || image.channels != (image.alpha ? 4 : 3))
            throw new ProtocolException("Only rgb with 3 channels (or 4 with straight alpha last) is supported.");
        if (image.source_depth is not (8 or 16 or 32))
            throw new ProtocolException("source_depth must be 8, 16 or 32; samples are always f32le.");
        ValidateIcc(image.icc_profile);
        string? colorPolicy = ReadColorPolicy(header.@params);
        if (image.icc_profile.Length == 0 && colorPolicy != "assume-srgb")
            throw new ProtocolException("Untagged RGB needs explicit params.colorPolicy='assume-srgb'; no profile is silently assigned.");
        if (header.selection is { } selection)
        {
            ValidateGeometry(selection);
            if (selection.x != image.x || selection.y != image.y || selection.width != image.width ||
                selection.height != image.height || selection.channels != 1 || selection.alpha ||
                selection.color_mode is not ("gray" or "grayscale") || selection.source_depth != 32 ||
                selection.icc_profile is null || selection.icc_profile.Length != 0)
                throw new ProtocolException("Selection must be same-geometry, profile-free, depth-32, one-channel gray.");
        }
        long payloadBytes = checked((long)pixels * (image.channels + (header.selection is null ? 0 : 1)) * 4);
        if (input.CanSeek && input.Length - input.Position != payloadBytes)
            throw new ProtocolException("Image/selection payload length does not match the metadata.");

        // Keep input precision through Windows color conversion (16-bit/channel staging).
        // Native Windows AI ultimately accepts an SDR BGRA8 bitmap; no document pixels are changed.
        var rgba = new ushort[checked(pixels * 4)];
        using var reader = new BinaryReader(input, Encoding.UTF8, leaveOpen: true);
        for (int pixel = 0; pixel < pixels; pixel++)
        {
            int offset = pixel * 4;
            rgba[offset] = Quantize(reader.ReadSingle());
            rgba[offset + 1] = Quantize(reader.ReadSingle());
            rgba[offset + 2] = Quantize(reader.ReadSingle());
            rgba[offset + 3] = image.alpha ? Quantize(reader.ReadSingle()) : ushort.MaxValue;
        }
        if (header.selection is not null)
            for (int i = 0; i < pixels; i++)
                ValidateSdr(reader.ReadSingle());
        RequireEnd(input);
        return new InputImage(header, rgba);
    }

    private static int ValidateGeometry(ImageMetadata image)
    {
        if (image.width is <= 0 or > MaxSide || image.height is <= 0 or > MaxSide ||
            (long)image.width * image.height > MaxPixels)
            throw new ProtocolException("Image limit is 36 million pixels, with each side in 1..16384.");
        if ((long)image.x + image.width > int.MaxValue || (long)image.y + image.height > int.MaxValue)
            throw new ProtocolException("Document-space image endpoint overflows i32.");
        return checked(image.width * image.height);
    }

    private static void ValidateIcc(int[]? profile)
    {
        if (profile is null || profile.Length > MaxIccBytes)
            throw new ProtocolException("icc_profile must be a byte array (maximum 4 MiB; header cap still applies).");
        foreach (int b in profile)
            if (b is < 0 or > 255)
                throw new ProtocolException("icc_profile contains a value outside 0..255.");
        if (profile.Length == 0) return;
        // ICC header: declared profile size, magic 'acsp', input RGB. WIC validates tags/transform.
        if (profile.Length < 128 || profile[36] != 'a' || profile[37] != 'c' ||
            profile[38] != 's' || profile[39] != 'p' || profile[16] != 'R' ||
            profile[17] != 'G' || profile[18] != 'B' || profile[19] != ' ')
            throw new ProtocolException("Embedded ICC must be a valid RGB profile; other color spaces are unsupported.");
        uint declared = ((uint)profile[0] << 24) | ((uint)profile[1] << 16) | ((uint)profile[2] << 8) | (uint)profile[3];
        if (declared != profile.Length)
            throw new ProtocolException("Embedded ICC profile size does not match its header.");
        // Device-link, abstract and named-color profiles are not source RGB characterization profiles.
        string profileClass = string.Concat(profile.Skip(12).Take(4).Select(b => (char)b));
        if (profileClass is not ("scnr" or "mntr" or "prtr" or "spac"))
            throw new ProtocolException("ICC profile class is unsupported as a source RGB profile.");
    }

    private static string? ReadColorPolicy(JsonElement parameters)
    {
        if (parameters.ValueKind != JsonValueKind.Object)
            throw new ProtocolException("params must be an object.");
        string? policy = null;
        foreach (var property in parameters.EnumerateObject())
        {
            if (property.Name != "colorPolicy" || property.Value.ValueKind != JsonValueKind.String ||
                property.Value.GetString() != "assume-srgb")
                throw new ProtocolException("Only optional params.colorPolicy='assume-srgb' is supported.");
            policy = property.Value.GetString();
        }
        return policy;
    }

    internal static ushort Quantize(float sample)
    {
        ValidateSdr(sample);
        return (ushort)MathF.Round(sample * ushort.MaxValue);
    }

    private static void ValidateSdr(float sample)
    {
        if (!float.IsFinite(sample) || sample < 0 || sample > 1)
            throw new ProtocolException("Only finite SDR samples in [0,1] are supported; HDR needs an explicit host-side conversion.");
    }

    internal static void RequireEnd(Stream input)
    {
        if (input.ReadByte() != -1) throw new ProtocolException("Unexpected trailing request bytes.");
    }

    private static void RequireFields(JsonElement element, params string[] names)
    {
        if (element.ValueKind != JsonValueKind.Object) throw new ProtocolException("Expected JSON object metadata.");
        foreach (string name in names)
            if (!element.TryGetProperty(name, out _)) throw new ProtocolException($"Missing required field: {name}.");
    }

    private static void ValidateImageFields(JsonElement element) =>
        RequireFields(element, "x", "y", "width", "height", "channels", "color_mode", "source_depth", "alpha", "icc_profile");

    private static void ValidateUniqueKeys(JsonElement element)
    {
        if (element.ValueKind == JsonValueKind.Object)
        {
            var keys = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in element.EnumerateObject())
            {
                if (!keys.Add(property.Name)) throw new ProtocolException("Duplicate JSON property.");
                ValidateUniqueKeys(property.Value);
            }
        }
        else if (element.ValueKind == JsonValueKind.Array)
            foreach (var item in element.EnumerateArray()) ValidateUniqueKeys(item);
    }

    internal static ImageMetadata MaskMetadata(ImageMetadata source) =>
        new(source.x, source.y, source.width, source.height, 1, "gray", 32, false, []);

    internal static void WriteResponse(string path, ResponseHeader header, byte[]? mask = null)
    {
        byte[] json = JsonSerializer.SerializeToUtf8Bytes(header, ResponseOptions);
        if (json.Length > MaxHeaderBytes) throw new ProtocolException("Response header is too large.");
        if ((header.output is null) != (mask is null)) throw new ProtocolException("Inconsistent mask response.");
        if (mask is not null && header.output is { } output &&
            mask.Length != checked(output.image.width * output.image.height))
            throw new ProtocolException("Mask byte count is inconsistent.");
        string directory = Path.GetDirectoryName(path) ?? throw new ProtocolException("Response needs an absolute path.");
        string temporary = Path.Combine(directory, ".photocraft-response-" + Guid.NewGuid().ToString("N"));
        try
        {
            using (var stream = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            using (var writer = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: false))
            {
                writer.Write(ResponseMagic);
                writer.Write((uint)json.Length);
                writer.Write(json);
                if (mask is not null)
                    foreach (byte coverage in mask) writer.Write(coverage / 255f);
                writer.Flush();
                stream.Flush(flushToDisk: true);
            }
            // Never overwrite an existing file supplied by a caller.
            File.Move(temporary, path, overwrite: false);
        }
        finally
        {
            if (File.Exists(temporary)) File.Delete(temporary);
        }
    }
}

internal sealed record Arguments(string Action, string Request, string Response)
{
    internal static Arguments Parse(string[] args)
    {
        if (args.Length != 6 || args[0] != "--action" || args[2] != "--request" || args[4] != "--response" ||
            args[1] is not ("probe" or "prepare" or "run"))
            throw new ProtocolException("Usage: --action <probe|prepare|run> --request <absolute request.bin> --response <absolute response.bin>");
        string request = ValidatePath(args[3]);
        string response = ValidatePath(args[5]);
        if (request.Equals(response, StringComparison.OrdinalIgnoreCase))
            throw new ProtocolException("Request and response paths must differ.");
        if (!string.Equals(Path.GetDirectoryName(request), Path.GetDirectoryName(response), StringComparison.OrdinalIgnoreCase))
            throw new ProtocolException("Request and response must share a private job directory.");
        if (File.Exists(response)) throw new ProtocolException("Response file must not already exist.");
        return new(args[1], request, response);
    }

    private static string ValidatePath(string path)
    {
        if (!Path.IsPathFullyQualified(path) || path.Length < 4 || !char.IsAsciiLetter(path[0]) ||
            path[1] != ':' || path[2] != '\\' || path.IndexOf(':', 2) >= 0)
            throw new ProtocolException("Only absolute local drive paths are accepted (no UNC, device paths or alternate data streams).");
        return Path.GetFullPath(path);
    }
}
