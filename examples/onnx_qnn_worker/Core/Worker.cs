namespace PhotoCraft.OnnxQnn;

internal interface IInferenceBackend
{
    void CheckPayload(string root);
    IInferenceSession Create(string root, VerifiedModel model);
}
internal interface IInferenceSession : IDisposable
{
    TensorBinding[] Inputs { get; }
    TensorBinding[] Outputs { get; }
    TensorData Run(string inputName, string outputName, TensorData input);
}

internal static class Worker
{
    internal static Reply Execute(string action, InputImage? image, string root, bool nativeWindowsArm64, IInferenceBackend backend)
    {
        if (action is not ("probe" or "prepare" or "run")) throw new WorkerException("Unsupported action.");
        if ((action == "run") != (image is not null)) throw new WorkerException("Action/request mismatch.");
        try
        {
            // Model absence is useful even on CI x64: do not initialize ORT or load native DLLs.
            // v1 probe/prepare are scoped to the local default profile, never all resources/devices.
            var profiles = Profiles.Load(root);
            var profile = profiles.Select(image?.Header.resource);
            using var model = profiles.Verify(profile);
            if (!nativeWindowsArm64)
                return Reply.Status("unavailable", "Configured model found, but QNN HTP requires native Windows ARM64 and a compatible Qualcomm NPU/driver. No inference fallback exists.");
            backend.CheckPayload(root);
            if (action == "probe")
                return Reply.Status("ready", "Package/default profile and model digest verified; probe creates no QNN session, performs no inference/download and does not attest NPU compatibility. Use prepare to validate the default session.");
            using var session = backend.Create(root, model);
            Profiles.Bindings(profile, session.Inputs, session.Outputs);
            if (action == "prepare")
                return Reply.Status("ready", "Default model QNN HTP session created and tensor bindings checked with CPU graph fallback disabled. No inference/download. v1 prepare has no resource; run re-creates its selected session.");
            if (image is null) throw new WorkerException("Missing run image.");
            var tensor = Imaging.Preprocess(image, profile);
            var output = session.Run(profile.input.name, profile.output.name, tensor);
            var mask = Imaging.Postprocess(output, profile, image.Header.image);
            return new Reply(new ResponseHeader(1, "ok", "", new MaskOutput("mask", Protocol.MaskMetadata(image.Header.image)),
                new Execution([Protocol.Device], Protocol.Backend, null)), mask);
        }
        catch (NeedsModelException error) { return Reply.Status("needs_model", error.Message); }
    }
}
