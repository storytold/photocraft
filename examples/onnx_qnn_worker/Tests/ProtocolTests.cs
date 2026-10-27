using System.Buffers.Binary;
using System.Text;
using System.Text.Json;
using PhotoCraft.OnnxQnn;

namespace PhotoCraft.OnnxQnn.Tests;

internal static partial class Program
{
    private static MemoryStream Frame(string json, params float[] samples)
    {
        var stream = new MemoryStream();
        using (var writer = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: true))
        {
            byte[] header = Encoding.UTF8.GetBytes(json);
            writer.Write("PCADREQ1"u8); writer.Write((uint)header.Length); writer.Write(header);
            foreach (float value in samples) writer.Write(value);
        }
        stream.Position = 0; return stream;
    }
    private static InputImage Decode(RunHeader header, float[] samples)
    {
        using var stream = Frame(JsonSerializer.Serialize(header, Json.Options), samples);
        using var document = Protocol.ReadHeader(stream);
        return Protocol.ReadImage(document.RootElement, stream);
    }
    private static void TestProtocol()
    {
        Run("json/duplicate-and-nested-duplicate", () =>
        {
            Reject<WorkerException>(() => Json.Parse("{\"protocol\":1,\"protocol\":1}"u8.ToArray()));
            Reject<WorkerException>(() => Json.Parse("{\"p\":{\"a\":1,\"a\":2}}"u8.ToArray()));
        });
        Run("json/depth-and-unknown-fields", () =>
        {
            Reject<JsonException>(() => Json.Parse(Encoding.UTF8.GetBytes(new string('[', 33) + "0" + new string(']', 33))));
            using var doc = Json.Parse("{\"protocol\":1,\"extra\":0}"u8.ToArray());
            Reject<WorkerException>(() => Protocol.Control(doc.RootElement, new MemoryStream()));
        });
        Run("protocol/control-exact-fields-and-eof", () =>
        {
            using var frame = Frame("{\"protocol\":1}");
            using var doc = Protocol.ReadHeader(frame); Protocol.Control(doc.RootElement, frame);
            using var trailing = Frame("{\"protocol\":1}", 0);
            using var t = Protocol.ReadHeader(trailing);
            Reject<WorkerException>(() => Protocol.Control(t.RootElement, trailing));
        });
        Run("protocol/magic-version-short-prefix", () =>
        {
            using var shortFrame = new MemoryStream([1, 2]);
            Reject<EndOfStreamException>(() => Protocol.ReadHeader(shortFrame));
            using var wrong = Frame("{\"protocol\":2}");
            Reject<WorkerException>(() => Protocol.ReadHeader(wrong));
            using var badMagic = Frame("{\"protocol\":1}"); badMagic.GetBuffer()[0] = 0;
            Reject<WorkerException>(() => Protocol.ReadHeader(badMagic));
        });
        Run("protocol/header-zero-over-limit-and-truncation", () =>
        {
            foreach (uint length in new[] { 0u, (uint)Protocol.MaxHeaderBytes + 1, 100u })
            {
                byte[] prefix = new byte[12]; "PCADREQ1"u8.CopyTo(prefix);
                BinaryPrimitives.WriteUInt32LittleEndian(prefix.AsSpan(8), length);
                using var frame = new MemoryStream(prefix);
                Reject<WorkerException>(() => Protocol.ReadHeader(frame));
            }
        });
        foreach (int depth in new[] { 8, 16, 32 })
        foreach (bool alpha in new[] { false, true })
            Run($"protocol/f32le-depth-{depth}-alpha-{alpha}", () =>
            {
                var image = Image(depth, alpha);
                var decoded = Decode(image.Header, image.Samples);
                Check(decoded.Header.image.x == -7 && decoded.Header.image.source_depth == depth);
                Check(decoded.Samples.SequenceEqual(image.Samples));
            });
        Run("protocol/payload-short-trailing-and-nonfinite", () =>
        {
            var image = Image();
            Reject<WorkerException>(() => Decode(image.Header, image.Samples[..^1]));
            Reject<WorkerException>(() => Decode(image.Header, [.. image.Samples, 0f]));
            foreach (float value in new[] { float.NaN, float.PositiveInfinity, float.NegativeInfinity, -0.001f, 1.001f })
            {
                float[] samples = (float[])image.Samples.Clone(); samples[0] = value;
                Reject<WorkerException>(() => Decode(image.Header, samples));
            }
        });
        Run("protocol/geometry-overflow-and-budgets", () =>
        {
            foreach (var meta in new[] { Metadata() with { width = 0 }, Metadata() with { width = int.MaxValue },
                Metadata() with { width = 4097, height = 4096 }, Metadata() with { x = int.MaxValue }, Metadata() with { y = int.MaxValue } })
                Reject<WorkerException>(() => Protocol.Geometry(meta));
            Check(Protocol.Geometry(Metadata() with { x = int.MinValue, y = int.MinValue }) == 4);
            Check(Protocol.Geometry(Metadata() with { width = 4096, height = 4096 }) == Protocol.MaxPixels);
        });
        Run("protocol/resource-device-color-depth-restrictions", () =>
        {
            var image = Image();
            foreach (var h in new[] { image.Header with { resource = "../bad.onnx" }, image.Header with { device = "cpu" },
                image.Header with { operation = "filter" }, image.Header with { image = Metadata() with { icc_profile = [1] } },
                image.Header with { image = Metadata() with { color_mode = "cmyk" } }, image.Header with { image = Metadata() with { source_depth = 7 } },
                image.Header with { @params = JsonSerializer.SerializeToElement(new { colorPolicy = "ignore-icc" }) },
                image.Header with { @params = JsonSerializer.SerializeToElement(new { colorPolicy = "assume-srgb", extra = 1 }) } })
                Reject<WorkerException>(() => Decode(h, image.Samples));
            Check(Decode(image.Header with { allow_fallback = true }, image.Samples).Header.allow_fallback); // Permission does not create fallback.
        });
        Run("protocol/selection-validated-not-multiplied", () =>
        {
            var image = Image();
            var seed = Protocol.MaskMetadata(image.Header.image);
            var decoded = Decode(image.Header with { selection = seed }, [.. image.Samples, 0, 0, 0, 0]);
            Check(decoded.Samples.SequenceEqual(image.Samples));
            Reject<WorkerException>(() => Decode(image.Header with { selection = seed with { x = 0 } }, [.. image.Samples, 0, 0, 0, 0]));
            Reject<WorkerException>(() => Decode(image.Header with { selection = seed }, [.. image.Samples, 0, float.NaN, 0, 0]));
        });
        Run("protocol/status-response-keys-and-no-payload", () =>
        {
            foreach (string state in new[] { "ready", "needs_model", "unavailable", "error" })
            {
                using var bytes = new MemoryStream(); Protocol.Write(bytes, Reply.Status(state, "scope"));
                byte[] data = bytes.ToArray(); Check(data.AsSpan(0, 8).SequenceEqual("PCADRES1"u8));
                int length = checked((int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(8)));
                using var doc = JsonDocument.Parse(data.AsMemory(12, length));
                Json.Fields(doc.RootElement, "protocol", "status", "message", "output", "execution");
                Check(data.Length == 12 + length && doc.RootElement.GetProperty("output").ValueKind == JsonValueKind.Null);
            }
            Reject<WorkerException>(() => Protocol.Write(new MemoryStream(), Reply.Status("ready", new string('x', 1025))));
            Reject<WorkerException>(() => Protocol.Write(new MemoryStream(), new Reply(new ResponseHeader(1, "error", "bad"), [0])));
        });
        Run("protocol/ok-response-f32le-and-no-fallback", () =>
        {
            using var bytes = new MemoryStream();
            var reply = new Reply(new(1, "ok", "", new("mask", Protocol.MaskMetadata(Metadata())), new([Protocol.Device], Protocol.Backend, null)), [0.5001f, 0.5002f, 0f, 1f]);
            Protocol.Write(bytes, reply); byte[] data = bytes.ToArray();
            int offset = 12 + checked((int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(8)));
            Near(0.5001f, BinaryPrimitives.ReadSingleLittleEndian(data.AsSpan(offset)));
            Check(data.Length == offset + 16);
            Reject<WorkerException>(() => Protocol.Write(new MemoryStream(), reply with { Mask = [float.NaN, 0, 0, 0] }));
            Reject<WorkerException>(() => Protocol.Write(new MemoryStream(), reply with { Header = reply.Header with { execution = new(["cpu"], "cpu", "fallback") } }));
        });
        Run("arguments/reject-unc-ads-relative-and-wrong-order", () =>
        {
            Reject<WorkerException>(() => Arguments.Parse([]));
            foreach (string bad in new[] { "relative", "\\\\server\\share\\x", "C:\\request:stream" })
                Reject<WorkerException>(() => Arguments.Parse(["--action", "probe", "--request", bad, "--response", "C:\\job\\response"]));
        });
    }
}
