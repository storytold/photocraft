using System.Security.Cryptography;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace PhotoCraft.OnnxQnn;

internal sealed record WorkerConfig(int schemaVersion, string? defaultModel, ModelProfile[] profiles);
internal sealed record ModelProfile(string id, string operation, string model, string sha256,
    InputProfile input, OutputProfile output);
internal sealed record InputProfile(string name, string layout, int[] shape, float[] mean, float[] scale,
    string resize, string colorPolicy, string alphaPolicy);
internal sealed record OutputProfile(string name, string layout, int[] shape, string postprocess, string resize);
internal sealed record TensorBinding(string Name, string ElementType, int[] Shape);

/// <summary>Only a fixed local registry beside the EXE can bind a model; never request-supplied paths.</summary>
internal sealed class Profiles
{
    internal const int MaxConfigBytes = 256 * 1024;
    internal const long MaxModelBytes = 256L * 1024 * 1024;
    internal const int MaxTensorElements = 16 * 1024 * 1024;
    internal const string ConfigName = "qnn-models.json";
    internal string Root { get; }
    internal WorkerConfig Config { get; }

    private Profiles(string root, WorkerConfig config) { Root = root; Config = config; }
    internal static Profiles Load(string directory)
    {
        string root = Path.GetFullPath(directory);
        string path = Path.Combine(root, ConfigName);
        if (!File.Exists(path)) throw new NeedsModelException("No qnn-models.json registry beside the EXE; install an explicitly reviewed model profile. No download is performed.");
        LocalFiles.NoLinks(root, path);
        using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (stream.Length is <= 0 or > MaxConfigBytes) throw new WorkerException("Model registry must be 1..256 KiB.");
        var bytes = new byte[(int)stream.Length];
        stream.ReadExactly(bytes);
        using var document = Json.Parse(bytes);
        Json.Fields(document.RootElement, "schemaVersion", "defaultModel", "profiles");
        var list = document.RootElement.GetProperty("profiles");
        if (list.ValueKind != JsonValueKind.Array || list.GetArrayLength() > 32)
            throw new WorkerException("Registry requires at most 32 profile objects.");
        foreach (var entry in list.EnumerateArray())
        {
            Json.Fields(entry, "id", "operation", "model", "sha256", "input", "output");
            Json.Fields(entry.GetProperty("input"), "name", "layout", "shape", "mean", "scale", "resize", "colorPolicy", "alphaPolicy");
            Json.Fields(entry.GetProperty("output"), "name", "layout", "shape", "postprocess", "resize");
        }
        var config = document.RootElement.Deserialize<WorkerConfig>(Json.Options) ?? throw new WorkerException("Null model registry.");
        if (config.schemaVersion != 1 || config.profiles is null || config.profiles.Length > 32)
            throw new WorkerException("Registry requires schemaVersion 1 and at most 32 profiles.");
        var ids = new HashSet<string>(StringComparer.Ordinal);
        var paths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var profile in config.profiles)
        {
            Validate(profile);
            if (!ids.Add(profile.id) || !paths.Add(profile.model)) throw new WorkerException("Duplicate model ID or case-insensitive model path.");
        }
        if (config.defaultModel is not null && (!Json.Id(config.defaultModel) || !ids.Contains(config.defaultModel)))
            throw new WorkerException("defaultModel must reference a registered model ID.");
        return new(root, config);
    }

    internal ModelProfile Select(string? resource)
    {
        string? id = resource ?? Config.defaultModel;
        if (id is null) throw new NeedsModelException("No default model is configured. v1 probe/prepare have no resource; bind defaultModel or select an opaque registered model ID for run.");
        if (!Json.Id(id)) throw new WorkerException("Invalid model ID; paths are never accepted as resources.");
        return Config.profiles.FirstOrDefault(p => p.id == id) ??
            throw new NeedsModelException("Requested model ID is not registered locally; no acquisition or path interpretation is performed.");
    }

    internal VerifiedModel Verify(ModelProfile profile)
    {
        Validate(profile);
        string path = LocalFiles.Relative(Root, profile.model);
        if (!File.Exists(path)) throw new NeedsModelException("Configured model file is missing; supply licensed QNN-compatible weights and reinstall the package explicitly.");
        LocalFiles.NoLinks(Root, path);
        // Hold a deny-write/delete read handle from digest through session disposal, best effort on Windows.
        var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        try
        {
            if (file.Length is <= 0 or > MaxModelBytes) throw new WorkerException("Model must be 1..256 MiB.");
            string digest = Convert.ToHexString(SHA256.HashData(file));
            if (!digest.Equals(profile.sha256, StringComparison.OrdinalIgnoreCase)) throw new WorkerException("Model SHA256 does not match the allowlisted profile.");
            file.Position = 0;
            // Inspect the small protobuf surface before ORT sees it. External data/context/custom op
            // graphs would bypass the one-file digest binding and are outside this narrow worker.
            OnnxEnvelope.Validate(file);
            file.Position = 0;
            return new(profile, path, file);
        }
        catch { file.Dispose(); throw; }
    }

    internal static void Validate(ModelProfile? profile)
    {
        if (profile is null || !Json.Id(profile.id) || profile.operation != Protocol.Operation ||
            profile.sha256 is null || !Regex.IsMatch(profile.sha256, "\\A[0-9a-fA-F]{64}\\z", RegexOptions.CultureInvariant))
            throw new WorkerException("Profile needs an opaque ID, select-subject operation, and exact SHA256.");
        LocalFiles.Relative(Path.GetFullPath("."), profile.model);
        if (profile.model.Equals(ConfigName, StringComparison.OrdinalIgnoreCase)) throw new WorkerException("Model and registry paths must be distinct.");
        if (!profile.model.EndsWith(".onnx", StringComparison.OrdinalIgnoreCase)) throw new WorkerException("Only single-file ONNX models are supported.");
        var input = profile.input ?? throw new WorkerException("Missing input profile.");
        var output = profile.output ?? throw new WorkerException("Missing output profile.");
        Name(input.name); Name(output.name);
        var (width, height) = Shape(input.layout, input.shape, 3);
        _ = checked(width * height * 3);
        Shape(output.layout, output.shape, 1);
        if (input.mean is null || input.scale is null || input.mean.Length != 3 || input.scale.Length != 3 ||
            input.mean.Any(v => !float.IsFinite(v)) || input.scale.Any(v => !float.IsFinite(v) || v == 0))
            throw new WorkerException("Input normalization needs exactly three finite means and finite nonzero scales.");
        if (input.resize is not ("nearest" or "bilinear") || output.resize is not ("nearest" or "bilinear") ||
            input.colorPolicy != "assume-srgb" || input.alphaPolicy != "ignore-straight" ||
            output.postprocess is not ("probability" or "sigmoid"))
            throw new WorkerException("Unsupported preprocessing/postprocessing policy; all policies must be explicit.");
    }

    private static void Name(string? name)
    {
        if (string.IsNullOrWhiteSpace(name) || name.Length > 256 || name.Any(char.IsControl))
            throw new WorkerException("Tensor names must be nonblank, bounded and control-free.");
    }

    internal static (int Width, int Height) Shape(string layout, int[]? shape, int channels)
    {
        if (shape is null || shape.Length != 4 || shape[0] != 1 || layout is not ("nchw" or "nhwc"))
            throw new WorkerException("Only batch-one fixed 4D NCHW/NHWC tensors are supported.");
        int c = layout == "nchw" ? shape[1] : shape[3];
        int h = layout == "nchw" ? shape[2] : shape[1];
        int w = layout == "nchw" ? shape[3] : shape[2];
        if (c != channels || w is <= 0 or > 4096 || h is <= 0 or > 4096 || (long)w * h * c > MaxTensorElements)
            throw new WorkerException("Invalid tensor channels/dimensions or tensor allocation limit exceeded.");
        return (w, h);
    }

    internal static void Bindings(ModelProfile profile, TensorBinding[] inputs, TensorBinding[] outputs)
    {
        Validate(profile);
        if (inputs.Length != 1 || outputs.Length != 1 || !Matches(inputs[0], profile.input.name, profile.input.shape) ||
            !Matches(outputs[0], profile.output.name, profile.output.shape))
            throw new WorkerException("Model must have exactly one float32 input/output with configured names and fixed shapes; dynamic or auxiliary bindings are unsupported.");
    }
    private static bool Matches(TensorBinding binding, string name, int[] shape) =>
        binding.Name == name && binding.ElementType == "float32" && binding.Shape.SequenceEqual(shape);
}

internal sealed class VerifiedModel(ModelProfile profile, string path, FileStream file) : IDisposable
{
    internal ModelProfile Profile { get; } = profile;
    internal string Path { get; } = path;
    public void Dispose() => file.Dispose();
}

internal static class LocalFiles
{
    internal static string Relative(string root, string? relative)
    {
        if (string.IsNullOrEmpty(relative) || relative.Length > 512 || relative.Contains('\\') || relative.Contains(':') ||
            relative.StartsWith('/') || relative.EndsWith('/') || relative.Split('/').Any(c => c.Length is 0 or > 128 ||
                c.StartsWith('.') || c.EndsWith('.') || !c.All(v => char.IsAsciiLetterOrDigit(v) || v is '.' or '_' or '-')))
            throw new WorkerException("Model paths must be bounded relative paths with safe nonhidden components, using '/' separators; no traversal, URL, ADS or DLL paths.");
        foreach (string component in relative.Split('/'))
        {
            string stem = component.Split('.')[0].ToUpperInvariant();
            if (stem is "CON" or "PRN" or "AUX" or "NUL" || Regex.IsMatch(stem, "\\A(?:COM|LPT)[0-9]\\z"))
                throw new WorkerException("Reserved Windows path component.");
        }
        string canonicalRoot = Path.GetFullPath(root);
        string path = Path.GetFullPath(Path.Combine(canonicalRoot, relative.Replace('/', Path.DirectorySeparatorChar)));
        if (!path.StartsWith(Path.TrimEndingDirectorySeparator(canonicalRoot) + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
            throw new WorkerException("Model path escapes package directory.");
        return path;
    }

    internal static void NoLinks(string root, string path)
    {
        string fullRoot = Path.GetFullPath(root);
        string fullPath = Path.GetFullPath(path);
        if (!fullPath.StartsWith(Path.TrimEndingDirectorySeparator(fullRoot) + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
            throw new WorkerException("File is outside package directory.");
        // Check ancestors too: app directory itself may not be reached through a junction.
        for (string? current = fullPath; current is not null; current = Path.GetDirectoryName(current))
            if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new WorkerException("Reparse points/symlinks are not accepted for model/config paths.");
    }
}
