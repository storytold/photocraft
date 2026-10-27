namespace PhotoCraft.OnnxQnn;

internal static class Payload
{
    // Actual Microsoft.ML.OnnxRuntime.QNN 1.24.4 win-arm64/native inventory, excluding link-time .lib.
    // Preserve QnnCpu/Gpu for source-payload completeness, but never select those backends.
    internal static readonly string[] RuntimeNames =
    [
        "onnxruntime.dll", "onnxruntime_providers_shared.dll", "onnxruntime_providers_qnn.dll",
        "QnnCpu.dll", "QnnGpu.dll", "QnnHtp.dll", "QnnHtpPrepare.dll", "QnnSystem.dll",
        "QnnHtpV73Stub.dll", "QnnHtpV81Stub.dll", "libQnnHtpV73Skel.so", "libQnnHtpV81Skel.so",
        "libqnnhtpv73.cat", "libqnnhtpv81.cat",
    ];

    internal static void Check(string root)
    {
        foreach (string name in RuntimeNames)
        {
            string path = Path.Combine(root, name);
            if (!File.Exists(path) || new FileInfo(path).Length == 0)
                throw new WorkerException("Incomplete app-local QNN native/DSP payload; reinstall the complete reviewed ARM64 folder.");
            LocalFiles.NoLinks(root, path);
        }
    }
}
