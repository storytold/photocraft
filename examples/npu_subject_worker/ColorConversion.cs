using System.IO;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace PhotoCraft.WindowsAI;

/// <summary>Managed WPF/WIC bridge, not a WPF window or alternative application UI.</summary>
internal static class ColorConversion
{
    internal static byte[] ToSrgbBgra(InputImage input, string jobDirectory)
    {
        var image = input.Header.image ?? throw new ProtocolException("Image metadata is required.");
        var source = BitmapSource.Create(image.width, image.height, 96, 96,
            PixelFormats.Rgba64, null, input.Rgba64, checked(image.width * 8));
        var srgb = new ColorContext(PixelFormats.Bgra32);
        string? profileFile = null;
        try
        {
            ColorContext sourceContext;
            if (image.icc_profile.Length != 0)
            {
                // ColorContext's public API accepts a profile Uri, not an arbitrary stream.
                // Keep the embedded profile in this job's private directory; never install it.
                profileFile = Path.Combine(jobDirectory, ".photocraft-profile-" + Guid.NewGuid().ToString("N") + ".icc");
                using (var stream = new FileStream(profileFile, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                    foreach (int value in image.icc_profile) stream.WriteByte((byte)value);
                sourceContext = new ColorContext(new Uri(profileFile, UriKind.Absolute));
            }
            else
            {
                // Protocol.ReadImage already requires explicit colorPolicy='assume-srgb'.
                sourceContext = srgb;
            }
            // A real Windows color transform, including an embedded source RGB ICC profile.
            // WIC throws for invalid/unsupported profiles: do not substitute sRGB on failure.
            var converted = new ColorConvertedBitmap(source, sourceContext, srgb, PixelFormats.Bgra32);
            var bytes = new byte[checked(image.width * image.height * 4)];
            converted.CopyPixels(bytes, checked(image.width * 4), 0);
            return bytes;
        }
        catch (Exception error) when (error is not OutOfMemoryException && error is not ProtocolException)
        {
            throw new ProtocolException($"Windows color conversion failed (HRESULT 0x{error.HResult:X8}); profile was not replaced or ignored.");
        }
        finally
        {
            if (profileFile is not null && File.Exists(profileFile)) File.Delete(profileFile);
        }
    }
}
