using System.Buffers.Binary;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using PhotoCraft.WindowsAI;

namespace PhotoCraft.WindowsAI.ProtocolTests;

// Dependency-free, hardware-neutral assertions plus fixtures for the real Rust decoder.
// A failing assertion produces a nonzero exit and is retained in the JSON test evidence.
internal static class Program
{
    private sealed record TestResult(string name, bool passed, string? error);
    private static readonly List<TestResult> Results = [];
    private static readonly byte[] Mask = [0, 1, 63, 127, 128, 255];
    private static readonly ImageMetadata Source = new(-7, 11, 3, 2, 4, "rgb", 16, true, []);
    private static readonly ImageMetadata Pixel = new(-7, 11, 1, 1, 3, "rgb", 32, false, []);

    private static int Main(string[] args)
    {
        string? directory = null;
        string? resultsPath = null;
        string? verifyRustDir = null;
        var seen = new HashSet<string>(StringComparer.Ordinal);
        for (int i = 0; i < args.Length; i += 2)
        {
            if (i + 1 >= args.Length || !seen.Add(args[i])) return Usage();
            switch (args[i])
            {
                case "--fixtures": directory = Path.GetFullPath(args[i + 1]); break;
                case "--results": resultsPath = Path.GetFullPath(args[i + 1]); break;
                case "--verify-rust": verifyRustDir = Path.GetFullPath(args[i + 1]); break;
                default: return Usage();
            }
        }
        // Verification is read-only: never regenerate the C# fixtures into an existing
        // directory. The production writer deliberately refuses to overwrite files.
        if (resultsPath is null || (directory is null) == (verifyRustDir is null)) return Usage();
        Directory.CreateDirectory(Path.GetDirectoryName(resultsPath)!);

        if (verifyRustDir is not null)
        {
            VerifyRustFixtures(verifyRustDir);
        }
        else if (directory is not null)
        {
            Directory.CreateDirectory(directory);
            foreach (string status in new[] { "ready", "needs_model", "unavailable", "error" })
            {
                Run($"response/{status}/exact-wire-contract", () =>
                {
                    string path = Path.Combine(directory, status + ".bin");
                    Protocol.WriteResponse(path, new ResponseHeader(1, status, "fixture: " + status));
                    AssertResponse(path, status, null);
                });
            }
            Run("response/ok-mask/exact-wire-contract-and-f32-payload", () =>
            {
                string path = Path.Combine(directory, "ok-mask.bin");
                Protocol.WriteResponse(path, new ResponseHeader(1, "ok", "fixture: ok",
                    new MaskOutput("mask", Protocol.MaskMetadata(Source)),
                    new Execution([Protocol.DeviceId], Protocol.Backend, null)), Mask);
                AssertResponse(path, "ok", Mask);
            });
            RequestRegressions();
            WriterRegressions(directory);
        }

        int failed = Results.Count(result => !result.passed);
        File.WriteAllText(resultsPath, JsonSerializer.Serialize(new
        {
            suite = "PhotoCraft companion protocol (no Windows AI API or hardware)",
            protocol = 1,
            hardware_inference = false,
            direction = verifyRustDir is null ? "C# response writer -> Rust" : "Rust request writer -> C#",
            passed = Results.Count - failed,
            failed,
            tests = Results,
        }, new JsonSerializerOptions { WriteIndented = true }) + Environment.NewLine);
        Console.WriteLine($"{Results.Count - failed}/{Results.Count} protocol checks passed; {failed} failed.");
        return failed == 0 ? 0 : 1;
    }

    private static int Usage()
    {
        Console.Error.WriteLine("Usage: (--fixtures <fresh output directory> | --verify-rust <existing input directory>) --results <test-results.json>");
        return 2;
    }

    private static void Run(string name, Action test)
    {
        try
        {
            test();
            Results.Add(new(name, true, null));
            Console.WriteLine("PASS " + name);
        }
        catch (Exception error)
        {
            Results.Add(new(name, false, error.ToString()));
            Console.Error.WriteLine($"FAIL {name}: {error.Message}");
        }
    }

    private static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
    }

    private static void Reject<T>(Action action) where T : Exception
    {
        try { action(); }
        catch (T) { return; }
        throw new InvalidOperationException($"Expected {typeof(T).Name}.");
    }

    private static void ExactKeys(JsonElement value, params string[] expected)
    {
        Check(value.ValueKind == JsonValueKind.Object, "Expected a JSON object.");
        string[] actual = value.EnumerateObject().Select(property => property.Name).ToArray();
        Check(actual.Length == expected.Length && new HashSet<string>(actual, StringComparer.Ordinal).SetEquals(expected),
            $"Wire keys differ: [{string.Join(", ", actual)}]; expected exactly [{string.Join(", ", expected)}].");
    }

    private static void AssertResponse(string path, string status, byte[]? expectedMask)
    {
        using var stream = File.OpenRead(path);
        using var reader = new BinaryReader(stream, Encoding.UTF8, leaveOpen: true);
        Check(reader.ReadBytes(8).SequenceEqual("PCADRES1"u8.ToArray()), "Wrong response magic.");
        uint size = reader.ReadUInt32();
        Check(size is > 0 and <= Protocol.MaxHeaderBytes, "Unbounded response header.");
        byte[] json = reader.ReadBytes((int)size);
        Check(json.Length == size, "Truncated response JSON.");
        using var document = JsonDocument.Parse(json);
        var header = document.RootElement;
        // This is the regression gate: null ready_state/descriptor are unknown keys too.
        ExactKeys(header, "protocol", "status", "message", "output", "execution");
        Check(header.GetProperty("protocol").GetInt32() == 1, "Wrong protocol version.");
        Check(header.GetProperty("status").GetString() == status, "Wrong status.");
        Check(header.GetProperty("message").GetString() == "fixture: " + status, "Wrong message.");
        if (expectedMask is null)
        {
            Check(header.GetProperty("output").ValueKind == JsonValueKind.Null, "Status output must be null.");
            Check(header.GetProperty("execution").ValueKind == JsonValueKind.Null, "Status execution must be null.");
        }
        else
        {
            var output = header.GetProperty("output");
            ExactKeys(output, "kind", "image");
            Check(output.GetProperty("kind").GetString() == "mask", "Wrong output kind.");
            var image = output.GetProperty("image");
            ExactKeys(image, "x", "y", "width", "height", "channels", "color_mode", "source_depth", "alpha", "icc_profile");
            Check(image.GetProperty("x").GetInt32() == -7 && image.GetProperty("y").GetInt32() == 11 &&
                image.GetProperty("width").GetInt32() == 3 && image.GetProperty("height").GetInt32() == 2,
                "Mask geometry/origin changed.");
            Check(image.GetProperty("channels").GetInt32() == 1 && image.GetProperty("color_mode").GetString() == "gray" &&
                image.GetProperty("source_depth").GetInt32() == 32 && !image.GetProperty("alpha").GetBoolean() &&
                image.GetProperty("icc_profile").GetArrayLength() == 0, "Mask metadata changed.");
            var execution = header.GetProperty("execution");
            ExactKeys(execution, "devices", "backend", "fallback_reason");
            Check(execution.GetProperty("devices").EnumerateArray().Select(device => device.GetString()).SequenceEqual(new[] { Protocol.DeviceId }) &&
                execution.GetProperty("backend").GetString() == Protocol.Backend &&
                execution.GetProperty("fallback_reason").ValueKind == JsonValueKind.Null, "Execution metadata changed.");
            foreach (byte sample in expectedMask)
                Check(reader.ReadSingle() == sample / 255f, "Gray8 coverage was not normalized to f32le.");
        }
        Check(stream.Position == stream.Length, "Unexpected response payload/trailing bytes.");
    }

    private static MemoryStream Frame(string json, params float[] samples)
    {
        var stream = new MemoryStream();
        using (var writer = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: true))
        {
            byte[] bytes = Encoding.UTF8.GetBytes(json);
            writer.Write("PCADREQ1"u8.ToArray());
            writer.Write((uint)bytes.Length);
            writer.Write(bytes);
            foreach (float sample in samples) writer.Write(sample);
        }
        stream.Position = 0;
        return stream;
    }

    private static RequestHeader Request(ImageMetadata? image = null, ImageMetadata? selection = null) =>
        new(1, Protocol.Operation, null, JsonSerializer.SerializeToElement(new { colorPolicy = "assume-srgb" }),
            Protocol.DeviceId, false, image ?? Pixel, selection);

    private static InputImage ReadImage(RequestHeader header, params float[] samples)
    {
        using var stream = Frame(JsonSerializer.Serialize(header), samples);
        return Protocol.ReadImage(Protocol.ReadHeader(stream), stream);
    }

    private static void ReadControl(string json)
    {
        using var stream = Frame(json);
        Protocol.ValidateControlRequest(Protocol.ReadHeader(stream), stream);
    }

    private static void RequestRegressions()
    {
        Run("request/control-valid", () => ReadControl("{\"protocol\":1}"));
        Run("request/control-null-is-not-missing", () => Reject<ProtocolException>(() => ReadControl("{\"protocol\":1,\"image\":null}")));
        Run("request/control-unknown-null-field", () => Reject<ProtocolException>(() => ReadControl("{\"protocol\":1,\"future\":null}")));
        Run("request/null-header", () => Reject<ProtocolException>(() => ReadControl("null")));
        foreach (string field in new[] { "protocol", "operation", "params", "allow_fallback", "image" })
            Run("request/missing-required/" + field, () => Reject<ProtocolException>(() =>
            {
                var node = JsonNode.Parse(JsonSerializer.Serialize(Request()))!.AsObject();
                node.Remove(field);
                using var stream = Frame(node.ToJsonString(), 0, 0.5f, 1);
                Protocol.ReadImage(Protocol.ReadHeader(stream), stream);
            }));
        foreach (string field in new[] { "x", "y", "width", "height", "channels", "color_mode", "source_depth", "alpha", "icc_profile" })
            Run("request/missing-image-field/" + field, () => Reject<ProtocolException>(() =>
            {
                var node = JsonNode.Parse(JsonSerializer.Serialize(Request()))!.AsObject();
                node["image"]!.AsObject().Remove(field);
                using var stream = Frame(node.ToJsonString(), 0, 0.5f, 1);
                Protocol.ReadImage(Protocol.ReadHeader(stream), stream);
            }));
        foreach (string location in new[] { "root", "image" })
            Run("request/unknown-null-field/" + location, () => Reject<JsonException>(() =>
            {
                var node = JsonNode.Parse(JsonSerializer.Serialize(Request()))!.AsObject();
                (location == "root" ? node : node["image"]!.AsObject())["future"] = null;
                using var stream = Frame(node.ToJsonString(), 0, 0.5f, 1);
                Protocol.ReadHeader(stream);
            }));
        Run("request/optional-fields-missing-or-null", () =>
        {
            var node = JsonNode.Parse(JsonSerializer.Serialize(Request()))!.AsObject();
            foreach (string field in new[] { "resource", "device", "selection" }) node.Remove(field);
            using var stream = Frame(node.ToJsonString(), 0, 0.5f, 1);
            var image = Protocol.ReadImage(Protocol.ReadHeader(stream), stream);
            Check(image.Header.resource is null && image.Header.device is null && image.Header.selection is null, "Missing optional fields must remain null.");
        });
        Run("request/rgb-valid-preserves-origin-and-quantization", () =>
        {
            var image = ReadImage(Request(), 0, 0.5f, 1);
            Check(image.Header.image is { x: -7, y: 11, width: 1, height: 1, channels: 3, source_depth: 32, alpha: false } &&
                image.Rgba64.SequenceEqual(new ushort[] { 0, 32768, 65535, 65535 }), "RGB staging or source metadata changed.");
        });
        Run("request/rgba-and-selection-valid", () =>
        {
            var image = ReadImage(Request(Pixel with { channels = 4, alpha = true }, Protocol.MaskMetadata(Pixel)), 0, 0.5f, 1, 0.25f, 0.5f);
            Check(image.Rgba64.SequenceEqual(new ushort[] { 0, 32768, 65535, 16384 }), "Straight alpha changed.");
        });
        Run("request/malformed-json", () => Reject<JsonException>(() => ReadControl("{\"protocol\":")));
        Run("request/wrong-protocol", () => Reject<ProtocolException>(() => ReadControl("{\"protocol\":2}")));
        Run("request/wrong-json-type", () => Reject<JsonException>(() => ReadControl("{\"protocol\":\"1\"}")));
        Run("request/duplicate-root", () => Reject<ProtocolException>(() => ReadControl("{\"protocol\":1,\"protocol\":1}")));
        Run("request/duplicate-nested-params", () => Reject<ProtocolException>(() =>
        {
            using var stream = Frame(JsonSerializer.Serialize(Request()).Replace("\"colorPolicy\":\"assume-srgb\"", "\"colorPolicy\":\"assume-srgb\",\"colorPolicy\":\"assume-srgb\"", StringComparison.Ordinal));
            Protocol.ReadHeader(stream);
        }));
        Run("request/duplicate-nested-image", () => Reject<ProtocolException>(() =>
        {
            using var stream = Frame(JsonSerializer.Serialize(Request()).Replace("\"width\":1", "\"width\":1,\"width\":1", StringComparison.Ordinal));
            Protocol.ReadHeader(stream);
        }));
        Run("request/duplicate-inside-array", () => Reject<ProtocolException>(() =>
        {
            using var stream = Frame("{\"protocol\":1,\"params\":[{\"x\":1,\"x\":2}]}");
            Protocol.ReadHeader(stream);
        }));
        Run("request/excessive-json-depth", () => Reject<JsonException>(() =>
        {
            using var stream = Frame("{\"protocol\":1,\"params\":" + new string('[', 40) + "0" + new string(']', 40) + "}");
            Protocol.ReadHeader(stream);
        }));
        Run("request/truncated-prefix", () => Reject<EndOfStreamException>(() =>
        {
            using var stream = new MemoryStream("PCADREQ1"u8.ToArray());
            Protocol.ReadHeader(stream);
        }));
        Run("request/bad-magic", () => Reject<ProtocolException>(() =>
        {
            using var stream = Frame("{\"protocol\":1}");
            stream.WriteByte(0);
            stream.Position = 0;
            Protocol.ReadHeader(stream);
        }));
        foreach (uint length in new[] { 0u, (uint)Protocol.MaxHeaderBytes + 1, uint.MaxValue })
            Run($"request/header-length-{length}", () => Reject<ProtocolException>(() =>
            {
                byte[] prefix = new byte[12];
                "PCADREQ1"u8.CopyTo(prefix);
                BinaryPrimitives.WriteUInt32LittleEndian(prefix.AsSpan(8), length);
                using var stream = new MemoryStream(prefix);
                Protocol.ReadHeader(stream);
            }));
        Run("request/truncated-json", () => Reject<EndOfStreamException>(() =>
        {
            using var stream = Frame("{\"protocol\":1}");
            stream.SetLength(stream.Length - 1);
            Protocol.ReadHeader(stream);
        }));
        Run("request/control-extra-fields", () => Reject<ProtocolException>(() => ReadControl(JsonSerializer.Serialize(Request()))));
        Run("request/control-trailing-bytes", () => Reject<ProtocolException>(() =>
        {
            using var stream = Frame("{\"protocol\":1}", 0);
            Protocol.ValidateControlRequest(Protocol.ReadHeader(stream), stream);
        }));

        foreach (var (name, metadata) in new (string, ImageMetadata)[]
        {
            ("zero-width", Pixel with { width = 0 }),
            ("negative-height", Pixel with { height = -1 }),
            ("side-cap", Pixel with { width = Protocol.MaxSide + 1 }),
            ("pixel-cap", Pixel with { width = 6001, height = 6000 }),
            ("x-overflow", Pixel with { x = int.MaxValue }),
            ("y-overflow", Pixel with { y = int.MaxValue }),
            ("channel-count", Pixel with { channels = 4 }),
            ("wrong-color-mode", Pixel with { color_mode = "cmyk" }),
            ("wrong-depth", Pixel with { source_depth = 12 }),
            ("icc-negative", Pixel with { icc_profile = [-1] }),
            ("icc-over-byte", Pixel with { icc_profile = [256] }),
            ("icc-null", Pixel with { icc_profile = null! }),
            ("icc-malformed", Pixel with { icc_profile = new int[128] }),
        })
            Run("request/range/" + name, () => Reject<ProtocolException>(() => ReadImage(Request(metadata), 0, 0, 0)));
        foreach (float sample in new[] { float.NaN, float.PositiveInfinity, float.NegativeInfinity, -0.01f, 1.01f })
        {
            Run("request/rgb-sample/" + sample, () => Reject<ProtocolException>(() => ReadImage(Request(), sample, 0, 0)));
            Run("request/selection-sample/" + sample, () => Reject<ProtocolException>(() =>
                ReadImage(Request(selection: Protocol.MaskMetadata(Pixel)), 0, 0, 0, sample)));
        }
        Run("request/selection-geometry-mismatch", () => Reject<ProtocolException>(() =>
            ReadImage(Request(selection: Protocol.MaskMetadata(Pixel) with { x = 0 }), 0, 0, 0, 0)));
        Run("request/payload-truncated", () => Reject<ProtocolException>(() => ReadImage(Request(), 0, 0)));
        Run("request/payload-trailing", () => Reject<ProtocolException>(() => ReadImage(Request(), 0, 0, 0, 0)));
        Run("request/untagged-needs-explicit-policy", () => Reject<ProtocolException>(() =>
            ReadImage(Request() with { @params = JsonSerializer.SerializeToElement(new { }) }, 0, 0, 0)));
        Run("request/wrong-device", () => Reject<ProtocolException>(() => ReadImage(Request() with { device = "qnn.npu" }, 0, 0, 0)));
        Run("request/resource-not-accepted", () => Reject<ProtocolException>(() => ReadImage(Request() with { resource = "model.onnx" }, 0, 0, 0)));
    }

    private static void WriterRegressions(string directory)
    {
        var ok = new ResponseHeader(1, "ok", "fixture: ok", new MaskOutput("mask", Protocol.MaskMetadata(Source)),
            new Execution([Protocol.DeviceId], Protocol.Backend, null));
        Run("writer/missing-mask", () => Reject<ProtocolException>(() => Protocol.WriteResponse(Path.Combine(directory, "invalid-missing-mask.bin"), ok)));
        Run("writer/unexpected-mask", () => Reject<ProtocolException>(() => Protocol.WriteResponse(Path.Combine(directory, "invalid-extra-mask.bin"), new(1, "ready", "ready"), Mask)));
        Run("writer/mask-length", () => Reject<ProtocolException>(() => Protocol.WriteResponse(Path.Combine(directory, "invalid-mask-length.bin"), ok, [0])));
        Run("writer/header-cap", () => Reject<ProtocolException>(() => Protocol.WriteResponse(Path.Combine(directory, "invalid-header.bin"), new(1, "error", new string('x', Protocol.MaxHeaderBytes)))));
        Run("writer/no-overwrite", () =>
        {
            string path = Path.Combine(directory, "ready.bin");
            byte[] before = File.ReadAllBytes(path);
            Reject<IOException>(() => Protocol.WriteResponse(path, new(1, "error", "do not overwrite")));
            Check(File.ReadAllBytes(path).SequenceEqual(before), "Existing response changed.");
        });
        Run("writer/no-invalid-files-or-temporaries", () =>
        {
            Check(!Directory.EnumerateFiles(directory, "invalid-*.bin").Any(), "Rejected writer input published a response.");
            Check(!Directory.EnumerateFiles(directory, ".photocraft-response-*").Any(), "Temporary response leaked.");
        });
    }

    private static void VerifyRustFixtures(string rustDir)
    {
        Run("rust-request/control/exact-wire-contract", () =>
        {
            string path = Path.Combine(rustDir, "control-request.bin");
            using var stream = Protocol.OpenRequest(path);
            var header = Protocol.ReadHeader(stream);
            Protocol.ValidateControlRequest(header, stream);
            Check(header.protocol == 1, "Expected protocol 1.");
        });
        foreach (var (name, depth, alpha, selection, tagged, automatic) in new[]
        {
            ("rgb-8", 8, false, false, false, true),
            ("rgb-16", 16, false, false, false, false),
            ("rgb-32", 32, false, false, false, false),
            ("rgba-selection", 16, true, true, false, false),
            ("rgb-icc", 16, false, false, true, false),
        })
            Run($"rust-request/{name}/metadata-and-every-sample", () =>
                VerifyRustImage(Path.Combine(rustDir, name + ".bin"), depth, alpha, selection, tagged, automatic));
    }

    private static void VerifyRustImage(string path, int depth, bool alpha, bool selection, bool tagged, bool automatic)
    {
        using var stream = Protocol.OpenRequest(path);
        var header = Protocol.ReadHeader(stream);
        Check(header.protocol == 1 && header.operation == Protocol.Operation && header.resource is null &&
            header.device == (automatic ? null : Protocol.DeviceId) && !header.allow_fallback, "Rust request policy changed.");
        var metadata = header.image ?? throw new InvalidOperationException("Missing Rust source metadata.");
        Check(metadata is { x: -7, y: 11, width: 3, height: 2, color_mode: "rgb" } &&
            metadata.channels == (alpha ? 4 : 3) && metadata.source_depth == depth && metadata.alpha == alpha,
            "Rust source geometry/depth/alpha changed.");
        int[] expectedIcc = tagged ? new int[128] : [];
        if (tagged)
        {
            expectedIcc[3] = 128;
            foreach (var (offset, value) in new[] { (12, "mntr"), (16, "RGB "), (36, "acsp") })
                for (int i = 0; i < value.Length; i++) expectedIcc[offset + i] = value[i];
        }
        Check(metadata.icc_profile.SequenceEqual(expectedIcc), "Synthetic ICC bytes changed (no WIC transform tested).");
        ExactKeys(header.@params, tagged ? [] : ["colorPolicy"]);
        if (!tagged) Check(header.@params.GetProperty("colorPolicy").GetString() == "assume-srgb", "Color policy changed.");
        if (selection)
        {
            Check(header.selection is { x: -7, y: 11, width: 3, height: 2, channels: 1, color_mode: "gray", source_depth: 32, alpha: false } &&
                header.selection.icc_profile.Length == 0, "Rust selection metadata changed.");
        }
        else Check(header.selection is null, "Unexpected selection.");

        long payloadStart = stream.Position;
        using (var reader = new BinaryReader(stream, Encoding.UTF8, leaveOpen: true))
        {
            for (int pixel = 0; pixel < 6; pixel++)
                foreach (float expected in alpha ? new[] { 0f, 0.5f, 1f, 0.25f } : new[] { 0f, 0.5f, 1f })
                    Check(reader.ReadSingle() == expected, "Rust source f32le sample changed.");
            if (selection)
                foreach (float expected in new[] { 0f, 0.25f, 0.5f, 0.75f, 1f, 0.5001f })
                    Check(reader.ReadSingle() == expected, "Rust selection f32le sample changed.");
            Protocol.RequireEnd(stream);
        }
        // Independently run the production reader over the same bytes; it validates and
        // consumes selection even though Windows AI does not use it as an inference seed.
        stream.Position = payloadStart;
        var image = Protocol.ReadImage(header, stream);
        ushort[] expectedPixel = [0, 32768, 65535, alpha ? (ushort)16384 : ushort.MaxValue];
        Check(image.Rgba64.SequenceEqual(Enumerable.Range(0, 6).SelectMany(_ => expectedPixel)), "Rgba64 staging changed.");
    }
}
