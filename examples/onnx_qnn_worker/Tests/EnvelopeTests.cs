using System.Text;
using PhotoCraft.OnnxQnn;

namespace PhotoCraft.OnnxQnn.Tests;

internal static partial class Program
{
    private static byte[] Join(params byte[][] parts) => parts.SelectMany(p => p).ToArray();
    private static byte[] V(ulong value)
    {
        var bytes = new List<byte>();
        while (value >= 128) { bytes.Add((byte)((value & 127) | 128)); value >>= 7; }
        bytes.Add((byte)value); return bytes.ToArray();
    }
    private static byte[] Field(int field, byte[] content) => Join(V((ulong)(field * 8 + 2)), V((ulong)content.Length), content);
    private static byte[] Int(int field, ulong value) => Join(V((ulong)(field * 8)), V(value));
    private static byte[] Text(int field, string value) => Field(field, Encoding.UTF8.GetBytes(value));
    private static byte[] Node(string op, params byte[][] extra) => Join(Text(4, op), Join(extra));
    private static byte[] Graph(params byte[][] extra) => Join(Field(1, Node("QuantizeLinear")), Field(1, Node("DequantizeLinear")), Join(extra));
    private static byte[] Model(byte[]? graph = null, params byte[][] extra) =>
        Join(Int(1, 10), Field(8, Int(2, 13)), Field(7, graph ?? Graph()), Join(extra));
    private static void Envelope(byte[] bytes) { using var stream = new MemoryStream(bytes); OnnxEnvelope.Validate(stream); }
    private static void TestEnvelope()
    {
        Run("envelope/synthetic-qdq-not-an-inference-model", () => Envelope(Model()));
        Run("envelope/reject-missing-graph-opset-ir-qdq", () =>
        {
            foreach (byte[] bytes in new[] { Int(1, 10), Join(Int(1, 10), Field(7, Graph())),
                Join(Field(8, Int(2, 13)), Field(7, Graph())), Model(Field(1, Node("Identity"))),
                Model(extra: [Int(1, 10)]) })
                Reject<WorkerException>(() => Envelope(bytes));
        });
        Run("envelope/wrong-wire-root-and-node-security-fields", () =>
        {
            foreach (byte[] bytes in new[] { Join(Int(1, 10), Int(7, 0)), Model(Graph(Field(1, Node("Identity", Int(7, 0))))),
                Model(Graph(Field(1, Join(Int(4, 0))))), Model(Graph(Int(5, 0))) })
                Reject<WorkerException>(() => Envelope(bytes));
        });
        Run("envelope/truncated-tags-lengths-fixed-and-overflow", () =>
        {
            foreach (byte[] bytes in new[] { Array.Empty<byte>(), new byte[] { 0 }, new byte[] { 0x80 },
                new byte[] { 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02 },
                new byte[] { 0x3a, 0x80 }, new byte[] { 0x3a, 0xff, 0xff, 0xff, 0xff, 0x07 },
                Model(Graph(Field(1, Node("Identity", Field(5, new byte[] { 0x15, 0, 0 }))))),
                Join(Model(), new byte[] { 0x0b }) })
                Reject<WorkerException>(() => Envelope(bytes));
        });
        Run("envelope/duplicate-singular-root-node-tensor", () =>
        {
            foreach (byte[] bytes in new[] { Model(extra: [Field(7, Graph())]), Model(extra: [Field(8, Int(2, 13))]),
                Model(Graph(Field(1, Node("Identity", Text(7, ""), Text(7, "ai.onnx"))))),
                Model(Graph(Field(1, Node("Identity", Text(4, "EPContext"))))),
                Model(Graph(Field(5, Join(Int(14, 0), Int(14, 1))))) })
                Reject<WorkerException>(() => Envelope(bytes));
        });
        Run("envelope/custom-domain-context-functions-training-unknown", () =>
        {
            foreach (byte[] bytes in new[] { Model(Graph(Field(1, Node("Identity", Text(7, "com.microsoft"))))),
                Model(Graph(Field(1, Node("EPContext")))), Model(extra: [Field(20, [])]), Model(extra: [Field(25, [])]),
                Model(extra: [Field(26, [])]), Model(Graph(Field(1, Node("Identity", Field(10, []))))), Model(extra: [Field(99, [])]),
                Join(Int(1, 10), Field(8, Join(Text(1, "custom"), Int(2, 13))), Field(7, Graph())) })
                Reject<WorkerException>(() => Envelope(bytes));
        });
        Run("envelope/external-tensor-all-supported-nesting-routes", () =>
        {
            foreach (byte[] tensor in new[] { Field(13, []), Int(14, 1), Field(14, []), Int(13, 0) })
            {
                var routes = new List<byte[]> { Field(5, tensor), Field(15, Field(1, tensor)), Field(15, Field(2, tensor)) };
                foreach (int f in new[] { 5, 10 }) routes.Add(Field(1, Node("Constant", Field(5, Field(f, tensor)))));
                foreach (int f in new[] { 22, 23 })
                    foreach (int sparse in new[] { 1, 2 }) routes.Add(Field(1, Node("Constant", Field(5, Field(f, Field(sparse, tensor))))));
                foreach (int f in new[] { 6, 11 }) routes.Add(Field(1, Node("If", Field(5, Field(f, Graph(Field(5, tensor)))))));
                foreach (byte[] route in routes) Reject<WorkerException>(() => Envelope(Model(Graph(route))));
            }
        });
        Run("envelope/strict-utf8-and-name-size", () =>
        {
            Reject<WorkerException>(() => Envelope(Model(Graph(Field(1, Node("Identity", Field(7, [0xff])))))));
            Reject<WorkerException>(() => Envelope(Model(Graph(Field(1, Node(new string('x', 257)))))));
        });
        Run("envelope/raw-bytes-not-recursively-interpreted", () =>
        {
            // A raw-data byte pattern may look like external_data; it is still just tensor content.
            Envelope(Model(Graph(Field(5, Field(9, Field(13, [1, 2, 3]))))));
        });
        Run("envelope/depth-and-field-budget", () =>
        {
            byte[] graph = Graph();
            for (int i = 0; i < 20; i++) graph = Graph(Field(1, Node("If", Field(5, Field(6, graph)))));
            Reject<WorkerException>(() => Envelope(Model(graph)));
            // Repeated empty Node.input fields; total bytes remain ~2 MiB, but field work is bounded.
            byte[] many = new byte[(OnnxEnvelope.MaxFields + 1) * 2];
            for (int i = 0; i < many.Length; i += 2) many[i] = 0x0a;
            Reject<WorkerException>(() => Envelope(Model(Graph(Field(1, Node("Identity", many))))));
        });
    }
}
