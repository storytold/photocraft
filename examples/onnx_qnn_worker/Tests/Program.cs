using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using PhotoCraft.OnnxQnn;

namespace PhotoCraft.OnnxQnn.Tests;

internal static partial class Program
{
    private sealed record TestResult(string name, bool passed, string? error);
    private static readonly List<TestResult> Results = [];
    private static int Main(string[] args)
    {
        if (args.Length != 2 || args[0] != "--results" || string.IsNullOrWhiteSpace(args[1]))
        { Console.Error.WriteLine("Required: --results <evidence.json>; no missing-evidence skip."); return 2; }
        Console.WriteLine("QNN pure tests: synthetic protocol/config/envelope/imaging and FAKE scheduling only; NO ORT/native/model/hardware inference.");
        TestProtocol(); TestEnvelope(); TestProfilesAndWorker(); TestImaging();
        int failed = Results.Count(r => !r.passed);
        string path = Path.GetFullPath(args[1]);
        Directory.CreateDirectory(Path.GetDirectoryName(path) ?? ".");
        File.WriteAllText(path, JsonSerializer.Serialize(new
        {
            scope = "T1 pure C# Core tests, no ORT package/native library/model/hardware; generated protobuf is envelope-only, NOT an inference fixture",
            excludes = new[] { "Rust codec conformance (T2)", "native DLL loading", "NPU graph placement", "model accuracy/performance", "redistribution approval" },
            passed = Results.Count - failed, failed, tests = Results
        }, new JsonSerializerOptions { WriteIndented = true }));
        Console.WriteLine($"{Results.Count - failed}/{Results.Count} passed; {failed} failed.");
        return failed == 0 ? 0 : 1;
    }
    private static void Run(string name, Action action)
    {
        try { action(); Results.Add(new(name, true, null)); Console.WriteLine("PASS " + name); }
        catch (Exception error) { Results.Add(new(name, false, error.ToString())); Console.Error.WriteLine($"FAIL {name}: {error.Message}"); }
    }
    private static void Check(bool condition, string message = "Assertion failed")
    { if (!condition) throw new InvalidOperationException(message); }
    private static void Near(float expected, float actual) => Check(MathF.Abs(expected - actual) < 0.00001f, $"Expected {expected}, actual {actual}");
    private static void Reject<T>(Action action) where T : Exception
    {
        try { action(); } catch (T) { return; }
        throw new InvalidOperationException($"Expected {typeof(T).Name}");
    }
    private static void Temp(Action<string> action)
    {
        string root = Path.Combine(Path.GetTempPath(), "photocraft-qnn-test-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try { action(root); }
        finally { Directory.Delete(root, recursive: true); }
    }
    private static ModelProfile Profile() => new("test.scalar", Protocol.Operation, "model.onnx", new string('a', 64),
        new("input", "nchw", [1, 3, 2, 2], [0f, 0f, 0f], [1f, 1f, 1f], "nearest", "assume-srgb", "ignore-straight"),
        new("output", "nchw", [1, 1, 2, 2], "probability", "nearest"));
    private static ImageMetadata Metadata(int depth = 16, bool alpha = false) => new(-7, 11, 2, 2, alpha ? 4 : 3, "rgb", depth, alpha, []);
    private static InputImage Image(int depth = 16, bool alpha = false)
    {
        var meta = Metadata(depth, alpha);
        var samples = Enumerable.Range(0, 4).SelectMany(_ => alpha ? new[] { 0f, 0.5f, 1f, 0.25f } : [0f, 0.5f, 1f]).ToArray();
        return new InputImage(new RunHeader(1, Protocol.Operation, null, JsonSerializer.SerializeToElement(new { colorPolicy = "assume-srgb" }), Protocol.Device, false, meta, null), samples);
    }
    private static ModelProfile InstallSynthetic(string root)
    {
        byte[] bytes = Model(); // Envelope-only, invalid for inference; used solely with the explicitly fake backend.
        File.WriteAllBytes(Path.Combine(root, "model.onnx"), bytes);
        var profile = Profile() with { sha256 = Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant() };
        Registry(root, new(1, profile.id, [profile]));
        return profile;
    }
    private static void Registry(string root, WorkerConfig config) =>
        File.WriteAllBytes(Path.Combine(root, Profiles.ConfigName), JsonSerializer.SerializeToUtf8Bytes(config, Json.Options));
}
