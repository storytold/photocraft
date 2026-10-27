using System.Runtime.InteropServices;
using System.Text.Json;

namespace PhotoCraft.OnnxQnn;

internal static class Program
{
    private static int Main(string[] args)
    {
        Arguments arguments;
        try { arguments = Arguments.Parse(args); }
        catch { Console.Error.WriteLine("Invalid QNN worker arguments; expected the six v1 arguments with absolute local job paths."); return 2; }
        Reply reply;
        try
        {
            using var input = new FileStream(arguments.Request, FileMode.Open, FileAccess.Read, FileShare.Read);
            using var document = Protocol.ReadHeader(input);
            InputImage? image = null;
            if (arguments.Action == "run") image = Protocol.ReadImage(document.RootElement, input);
            else Protocol.Control(document.RootElement, input);
            bool supported = OperatingSystem.IsWindows() && RuntimeInformation.ProcessArchitecture == Architecture.Arm64 &&
                RuntimeInformation.OSArchitecture == Architecture.Arm64;
            reply = Worker.Execute(arguments.Action, image, AppContext.BaseDirectory, supported, new QnnBackend());
        }
        catch (Exception error)
        {
            // Do not echo model paths, image data, user JSON, or opaque native error text to the host.
            string message = error switch
            {
                WorkerException => error.Message,
                NeedsModelException => error.Message,
                EndOfStreamException => "Truncated v1 request file.",
                JsonException => "Invalid request/registry JSON, fields or value types.",
                _ => $"QNN worker failed ({error.GetType().Name}, HRESULT 0x{error.HResult:X8}); no CPU/GPU graph fallback was attempted.",
            };
            reply = Reply.Status(error is NeedsModelException ? "needs_model" : "error", message);
        }
        try { Protocol.Publish(arguments.Response, reply); return 0; }
        catch { Console.Error.WriteLine("Cannot publish bounded QNN worker response."); return 3; }
    }
}
