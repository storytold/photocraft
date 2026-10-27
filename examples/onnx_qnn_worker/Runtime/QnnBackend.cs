using Microsoft.ML.OnnxRuntime;
using Microsoft.ML.OnnxRuntime.Tensors;

namespace PhotoCraft.OnnxQnn;

/// <summary>Production backend: actual ONNX Runtime QNN, never the fake test backend.</summary>
internal sealed class QnnBackend : IInferenceBackend
{
    public void CheckPayload(string root) => Payload.Check(root);

    public IInferenceSession Create(string root, VerifiedModel model)
    {
        CheckPayload(root);
        string backendPath = Path.GetFullPath(Path.Combine(root, "QnnHtp.dll"));
        using var options = new SessionOptions();
        options.AddSessionConfigEntry("session.disable_cpu_ep_fallback", "1");
        options.AppendExecutionProvider("QNN", new Dictionary<string, string>
        {
            ["backend_path"] = backendPath,
            ["offload_graph_io_quantization"] = "0",
        });
        // The only model path is local, allowlisted, SHA256-verified and held read-locked.
        // Unsupported graphs/driver failures are errors, never retried on CPU or another EP.
        var session = new InferenceSession(model.Path, options);
        try { return new QnnSession(session); }
        catch { session.Dispose(); throw; }
    }
}

internal sealed class QnnSession : IInferenceSession
{
    private readonly InferenceSession session;
    public TensorBinding[] Inputs { get; }
    public TensorBinding[] Outputs { get; }
    internal QnnSession(InferenceSession session)
    {
        this.session = session;
        Inputs = Bindings(session.InputMetadata);
        Outputs = Bindings(session.OutputMetadata);
    }
    private static TensorBinding[] Bindings(IReadOnlyDictionary<string, NodeMetadata> metadata) =>
        metadata.Select(pair => new TensorBinding(pair.Key,
            pair.Value.IsTensor && pair.Value.ElementType == typeof(float) ? "float32" : "unsupported",
            pair.Value.IsTensor ? pair.Value.Dimensions.ToArray() : [])).ToArray();

    public TensorData Run(string inputName, string outputName, TensorData input)
    {
        var tensor = new DenseTensor<float>(input.Samples, input.Shape);
        var value = NamedOnnxValue.CreateFromTensor(inputName, tensor);
        using var results = session.Run(new[] { value }, new[] { outputName });
        if (results.Count != 1) throw new WorkerException("QNN returned an unexpected number of outputs.");
        var result = results.First();
        if (result.Name != outputName) throw new WorkerException("QNN returned an unexpected output binding.");
        var output = result.AsTensor<float>();
        if (output.Length > Profiles.MaxTensorElements) throw new WorkerException("QNN output exceeds worker tensor limit.");
        return new(output.ToArray(), output.Dimensions.ToArray());
    }
    public void Dispose() => session.Dispose();
}
