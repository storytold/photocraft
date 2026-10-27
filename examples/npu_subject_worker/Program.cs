using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Windows.AI;
using Microsoft.Windows.AI.Imaging;
using Windows.ApplicationModel;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace PhotoCraft.WindowsAI;

internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        Arguments arguments;
        try { arguments = Arguments.Parse(args); }
        catch (Exception error)
        {
            Console.Error.WriteLine(error is ProtocolException ? error.Message : "Invalid companion command line.");
            return 2;
        }
        ResponseHeader response;
        byte[]? mask = null;
        try
        {
            (response, mask) = ExecuteAsync(arguments).GetAwaiter().GetResult();
        }
        catch (Exception error)
        {
            // Do not echo file paths, arbitrary JSON or opaque OS error messages to the host.
            string message = error is ProtocolException ? error.Message :
                error is EndOfStreamException ? "Truncated request file." :
                error is System.Text.Json.JsonException ? "Malformed request JSON or metadata types." :
                $"Windows AI operation failed ({error.GetType().Name}, HRESULT 0x{error.HResult:X8}); no fallback was used.";
            response = new(1, "error", message);
        }
        try
        {
            Protocol.WriteResponse(arguments.Response, response, mask);
            // A framed unavailable/error response is still a completed protocol transaction.
            return 0;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine($"Cannot publish companion response (HRESULT 0x{error.HResult:X8}).");
            return 3;
        }
    }

    private static async Task<(ResponseHeader, byte[]?)> ExecuteAsync(Arguments arguments)
    {
        using var request = Protocol.OpenRequest(arguments.Request);
        var header = Protocol.ReadHeader(request);
        if (arguments.Action != "run") Protocol.ValidateControlRequest(header, request);
        if (RuntimeInformation.ProcessArchitecture != Architecture.Arm64)
            return (new(1, "unavailable", "This companion build requires a native ARM64 Windows process."), null);
        try { _ = Package.Current.Id.FamilyName; }
        catch
        {
            return (new(1, "unavailable", "installRequired: install the signed MSIX and launch its registered execution alias; an unpackaged EXE has no systemAIModels identity."), null);
        }

        AIFeatureReadyState state;
        try { state = ImageForegroundExtractor.GetReadyState(); }
        catch (Exception error)
        {
            return (new(1, "unavailable", $"Windows AI activation is unavailable (HRESULT 0x{error.HResult:X8}); verify the installed MSIX, Windows build and Copilot+ NPU support."), null);
        }
        var readiness = DescribeState(state);
        if (arguments.Action == "probe")
        {
            // Deliberately do NOT call EnsureReadyAsync or CreateAsync here.
            return (readiness, null);
        }
        if (arguments.Action == "prepare")
        {
            if (state != AIFeatureReadyState.NotReady) return (readiness, null);
            // This is the ONLY download/preparation entry point, invoked with user consent.
            var result = await ImageForegroundExtractor.EnsureReadyAsync();
            if (result.Status != AIFeatureReadyResultState.Success)
            {
                string code = result.ExtendedError is { } error ? $"0x{error.HResult:X8}" : "none";
                return (new(1, "error", $"Explicit model preparation returned {result.Status} (extended HRESULT {code}); no inference was attempted."), null);
            }
            return (DescribeState(ImageForegroundExtractor.GetReadyState()), null);
        }
        if (state != AIFeatureReadyState.Ready) return (readiness, null);

        var input = Protocol.ReadImage(header, request);
        var metadata = header.image ?? throw new ProtocolException("Run requires image metadata.");
        byte[] bgra = ColorConversion.ToSrgbBgra(input, Path.GetDirectoryName(arguments.Request)!);
        using var writer = new DataWriter();
        writer.WriteBytes(bgra);
        var buffer = writer.DetachBuffer();
        using var bitmap = SoftwareBitmap.CreateCopyFromBuffer(buffer, BitmapPixelFormat.Bgra8,
            metadata.width, metadata.height, BitmapAlphaMode.Straight);
        using var extractor = await ImageForegroundExtractor.CreateAsync();
        using var nativeMask = extractor.GetMaskFromSoftwareBitmap(bitmap);
        if (nativeMask.BitmapPixelFormat != BitmapPixelFormat.Gray8 ||
            nativeMask.PixelWidth != metadata.width || nativeMask.PixelHeight != metadata.height)
            throw new ProtocolException("Windows AI returned an unexpected mask format or geometry; no resampling/substitution was performed.");
        int pixels = checked(metadata.width * metadata.height);
        var maskBuffer = new Windows.Storage.Streams.Buffer((uint)pixels) { Length = (uint)pixels };
        nativeMask.CopyToBuffer(maskBuffer);
        var maskBytes = new byte[pixels];
        using (var reader = DataReader.FromBuffer(maskBuffer)) reader.ReadBytes(maskBytes);
        return (new(1, "ok", "Windows AI ImageForegroundExtractor completed; Gray8 coverage normalized to f32 [0,1].",
            new MaskOutput("mask", Protocol.MaskMetadata(metadata)),
            new Execution([Protocol.DeviceId], Protocol.Backend, null)), maskBytes);
    }

    private static ResponseHeader DescribeState(AIFeatureReadyState state)
    {
        string name = state.ToString();
        return name switch
        {
            "Ready" => new(1, "ready", "Windows AI model reports Ready. Probe performs no model creation, preparation or download."),
            "NotReady" => new(1, "needs_model", "needsModelDownload: Windows AI reports NotReady; explicit prepare may download/install system-managed model components."),
            "CapabilityMissing" => new(1, "unavailable", "installRequired: package is missing effective systemAIModels capability; reinstall a correctly declared MSIX and check MaxVersionTested."),
            "DisabledByUser" => new(1, "unavailable", "Windows AI feature is disabled/removed by the user; this companion will not re-enable it."),
            "OSUpdateNeeded" => new(1, "unavailable", "Windows AI requires an OS update or major OS upgrade; this companion does not change update/channel policy."),
            "NotCompatibleWithSystemHardware" => new(1, "unavailable", "Windows AI model is incompatible with this hardware/NPU."),
            "NotSupportedOnCurrentSystem" => new(1, "unavailable", "Windows AI model is unsupported on this Windows build or hardware configuration."),
            _ => new(1, "unavailable", $"Windows AI readiness state {name} ({(int)state}) is not supported; no fallback was used."),
        };
    }
}
