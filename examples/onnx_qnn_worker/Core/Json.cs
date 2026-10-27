using System.Text.Json;
using System.Text.Json.Serialization;

namespace PhotoCraft.OnnxQnn;

internal sealed class WorkerException(string message) : Exception(message);
internal sealed class NeedsModelException(string message) : Exception(message);

internal static class Json
{
    internal static readonly JsonSerializerOptions Options = new()
    {
        MaxDepth = 32,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        PropertyNameCaseInsensitive = false,
    };

    internal static JsonDocument Parse(byte[] bytes)
    {
        var document = JsonDocument.Parse(bytes, new JsonDocumentOptions { MaxDepth = 32 });
        try { Unique(document.RootElement); return document; }
        catch { document.Dispose(); throw; }
    }

    private static void Unique(JsonElement value)
    {
        if (value.ValueKind == JsonValueKind.Object)
        {
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in value.EnumerateObject())
            {
                if (!names.Add(property.Name)) throw new WorkerException("Duplicate JSON property.");
                Unique(property.Value);
            }
        }
        else if (value.ValueKind == JsonValueKind.Array)
            foreach (var element in value.EnumerateArray()) Unique(element);
    }

    internal static void Fields(JsonElement value, params string[] names)
    {
        if (value.ValueKind != JsonValueKind.Object ||
            value.EnumerateObject().Count() != names.Length ||
            names.Any(name => !value.TryGetProperty(name, out _)))
            throw new WorkerException("JSON object has missing or unexpected fields.");
    }

    internal static bool Id(string? value) => value is { Length: > 0 and <= 128 } &&
        value.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '_' or '-');
}
