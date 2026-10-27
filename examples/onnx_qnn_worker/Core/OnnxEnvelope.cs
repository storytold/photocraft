using System.Text;

namespace PhotoCraft.OnnxQnn;

/// <summary>
/// Conservative envelope restriction, NOT a complete ONNX/protobuf validator or sandbox.
/// Field numbers/wires: https://github.com/onnx/onnx/blob/v1.20.1/onnx/onnx.proto .
/// Visit supported graph/tensor-bearing fields; reject unknown container fields, wrong wires,
/// duplicate singulars, external data, custom domains, functions and device/context graphs.
/// Raw values, type information, topology and operator semantics remain ORT's responsibility.
/// Reviewed, trusted models only: this does not make native ORT safe against malicious models.
/// </summary>
internal static class OnnxEnvelope
{
    private enum Message { Model, Graph, Node, Attribute, Tensor, Sparse, Opset }
    private sealed class Budget { internal int Fields; internal bool Graph; internal bool Q; internal bool Dq; internal bool Opset; }
    private static readonly UTF8Encoding StrictUtf8 = new(false, true);
    internal const int MaxFields = 1_000_000;
    internal const int MaxDepth = 32;

    internal static void Validate(Stream stream)
    {
        if (!stream.CanRead || !stream.CanSeek || stream.Position != 0 || stream.Length is <= 0 or > Profiles.MaxModelBytes)
            throw new WorkerException("Expected a bounded seekable ONNX file at offset zero.");
        var budget = new Budget();
        Scan(stream, stream.Length, Message.Model, 0, budget);
        if (!budget.Graph || !budget.Q || !budget.Dq || !budget.Opset)
            throw new WorkerException("Only single-file standard-domain QDQ ONNX graphs with a standard opset import are supported; Q/DQ presence is not proof of HTP compatibility.");
    }

    private static void Scan(Stream stream, long end, Message message, int depth, Budget budget)
    {
        if (depth > MaxDepth) throw new WorkerException("ONNX graph nesting limit exceeded.");
        var singular = new HashSet<ulong>();
        bool op = false, version = false;
        while (stream.Position < end)
        {
            if (++budget.Fields > MaxFields) throw new WorkerException("ONNX protobuf field budget exceeded.");
            ulong tag = Varint(stream, end);
            int wire = (int)(tag & 7);
            ulong field = tag >> 3;
            if (field is 0 or > 536870911) throw new WorkerException("Invalid ONNX protobuf tag.");
            int allowed = Wires(message, field);
            if (allowed == 0 || wire > 5 || (allowed & (1 << wire)) == 0)
                throw new WorkerException("Unknown, forbidden or wrong-wire ONNX envelope field.");
            if (!Repeated(message, field) && !singular.Add(field))
                throw new WorkerException("Duplicate singular ONNX envelope field.");
            if (wire == 0)
            {
                ulong value = Varint(stream, end);
                if (message == Message.Tensor && field == 14 && value != 0)
                    throw new WorkerException("ONNX external tensor data is forbidden.");
                if (message == Message.Model && field == 1 && (value is 0 or > 10))
                    throw new WorkerException("This transition worker accepts only ONNX IR 1..10; newer IR needs a new envelope review.");
                if (message == Message.Opset && field == 2)
                {
                    if (value is 0 or > 30) throw new WorkerException("Unsupported ONNX opset version.");
                    version = true;
                }
            }
            else if (wire == 1) Skip(stream, end, 8);
            else if (wire == 5) Skip(stream, end, 4);
            else
            {
                ulong length = Varint(stream, end);
                if (length > (ulong)(end - stream.Position)) throw new WorkerException("Truncated ONNX protobuf field.");
                long childEnd = checked(stream.Position + (long)length);
                Message? child = (message, field) switch
                {
                    (Message.Model, 7) => Message.Graph,
                    (Message.Model, 8) => Message.Opset,
                    (Message.Graph, 1) => Message.Node,
                    (Message.Graph, 5) => Message.Tensor,
                    (Message.Graph, 15) => Message.Sparse,
                    (Message.Node, 5) => Message.Attribute,
                    (Message.Attribute, 5 or 10) => Message.Tensor,
                    (Message.Attribute, 6 or 11) => Message.Graph,
                    (Message.Attribute, 22 or 23) => Message.Sparse,
                    (Message.Sparse, 1 or 2) => Message.Tensor,
                    _ => null,
                };
                if (child is { } kind)
                {
                    if (message == Message.Model && field == 7) budget.Graph = true;
                    if (message == Message.Model && field == 8)
                    {
                        if (budget.Opset) throw new WorkerException("Only one standard opset import is supported.");
                        budget.Opset = true;
                    }
                    Scan(stream, childEnd, kind, depth + 1, budget);
                }
                else if ((message == Message.Node && (field is 4 or 7)) || (message == Message.Opset && field == 1))
                {
                    if (length > 256) throw new WorkerException("Oversized ONNX operator/domain name.");
                    var bytes = new byte[(int)length];
                    stream.ReadExactly(bytes);
                    string name;
                    try { name = StrictUtf8.GetString(bytes); }
                    catch (DecoderFallbackException) { throw new WorkerException("Invalid UTF-8 ONNX operator/domain name."); }
                    if ((message == Message.Opset || field == 7) && name is not ("" or "ai.onnx"))
                        throw new WorkerException("Custom-domain/EPContext operators and opsets are unsupported.");
                    if (message == Message.Node && field == 4)
                    {
                        if (string.IsNullOrWhiteSpace(name) || name == "EPContext" || name.Any(char.IsControl))
                            throw new WorkerException("Invalid operator name or unsupported EPContext model.");
                        op = true;
                        budget.Q |= name == "QuantizeLinear";
                        budget.Dq |= name == "DequantizeLinear";
                    }
                }
                else stream.Position = childEnd; // Known non-file-bearing data/type/metadata only.
            }
        }
        if (stream.Position != end) throw new WorkerException("ONNX protobuf boundary mismatch.");
        if (message == Message.Model && !singular.Contains(1)) throw new WorkerException("Missing ONNX IR version.");
        if (message == Message.Node && !op) throw new WorkerException("Missing ONNX operator name.");
        if (message == Message.Opset && !version) throw new WorkerException("Missing ONNX opset version.");
    }

    // Bitset of wire types: 0=varint, 1=fixed64, 2=length-delimited, 5=fixed32.
    // Unknown fields are rejected, not guessed to be harmless future nested messages.
    private static int Wires(Message m, ulong f) => (m, f) switch
    {
        (Message.Model, 1 or 5) => 1,
        (Message.Model, 2 or 3 or 4 or 6 or 7 or 8 or 14) => 4,
        (Message.Graph, 1 or 2 or 5 or 10 or 11 or 12 or 13 or 14 or 15 or 16) => 4,
        (Message.Node, 1 or 2 or 3 or 4 or 5 or 6 or 7 or 9) => 4,
        (Message.Attribute, 1 or 4 or 5 or 6 or 9 or 10 or 11 or 13 or 14 or 15 or 22 or 23) => 4,
        (Message.Attribute, 2) => 32,
        (Message.Attribute, 3 or 20) => 1,
        (Message.Attribute, 7) => 32 | 4,
        (Message.Attribute, 8) => 1 | 4,
        (Message.Tensor, 1 or 5 or 7 or 11) => 1 | 4,
        (Message.Tensor, 2 or 14) => 1,
        (Message.Tensor, 4) => 32 | 4,
        (Message.Tensor, 10) => 2 | 4,
        (Message.Tensor, 6 or 8 or 9 or 12 or 16) => 4,
        (Message.Sparse, 1 or 2) => 4,
        (Message.Sparse, 3) => 1 | 4,
        (Message.Opset, 1) => 4,
        (Message.Opset, 2) => 1,
        _ => 0, // Includes Tensor.external_data(13), segment(3), functions/training/config/overload.
    };
    private static bool Repeated(Message m, ulong f) => (m, f) switch
    {
        (Message.Model, 8 or 14) => true,
        (Message.Graph, 1 or 5 or 11 or 12 or 13 or 14 or 15 or 16) => true,
        (Message.Node, 1 or 2 or 5 or 9) => true,
        (Message.Attribute, 7 or 8 or 9 or 10 or 11 or 15 or 23) => true,
        (Message.Tensor, 1 or 4 or 5 or 6 or 7 or 10 or 11 or 16) => true,
        (Message.Sparse, 3) => true,
        _ => false,
    };
    private static void Skip(Stream stream, long end, int bytes)
    {
        if (end - stream.Position < bytes) throw new WorkerException("Truncated ONNX field.");
        stream.Position += bytes;
    }
    private static ulong Varint(Stream stream, long end)
    {
        ulong value = 0;
        for (int shift = 0; shift < 70; shift += 7)
        {
            if (stream.Position >= end) throw new WorkerException("Truncated ONNX varint.");
            int b = stream.ReadByte();
            if (b < 0 || (shift == 63 && b > 1)) throw new WorkerException("Invalid ONNX varint.");
            value |= (ulong)(b & 127) << shift;
            if ((b & 128) == 0) return value;
        }
        throw new WorkerException("Oversized ONNX varint.");
    }
}