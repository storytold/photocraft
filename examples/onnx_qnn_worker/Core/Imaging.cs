namespace PhotoCraft.OnnxQnn;

internal sealed record TensorData(float[] Samples, int[] Shape);

/// <summary>CPU-side explicit SDR RGB preprocessing and same-geometry scalar mask postprocessing.</summary>
internal static class Imaging
{
    internal static TensorData Preprocess(InputImage image, ModelProfile profile)
    {
        Profiles.Validate(profile);
        var source = image.Header.image;
        int pixels = Protocol.Geometry(source);
        if (source.color_mode != "rgb" || source.channels != (source.alpha ? 4 : 3) ||
            source.source_depth is not (8 or 16 or 32) || source.icc_profile is null || source.icc_profile.Length != 0 ||
            image.Samples.Length != checked(pixels * source.channels))
            throw new WorkerException("Unsupported input metadata or samples; no implicit color conversion.");
        foreach (float value in image.Samples) Protocol.Sdr(value);
        var input = profile.input;
        var (width, height) = Profiles.Shape(input.layout, input.shape, 3);
        var result = new float[checked(width * height * 3)];
        // RGB channels remain straight; alpha is explicitly ignored for analysis, not premultiplied.
        for (int y = 0; y < height; y++)
        for (int x = 0; x < width; x++)
        for (int c = 0; c < 3; c++)
        {
            float rgb = Sample(image.Samples, source.width, source.height, source.channels, c,
                x, y, width, height, input.resize);
            float value = (rgb - input.mean[c]) * input.scale[c];
            if (!float.IsFinite(value)) throw new WorkerException("Normalization overflowed; review model mean/scale.");
            int index = input.layout == "nchw" ? (c * height + y) * width + x : (y * width + x) * 3 + c;
            result[index] = value;
        }
        return new(result, (int[])input.shape.Clone());
    }

    internal static float[] Postprocess(TensorData output, ModelProfile profile, ImageMetadata source)
    {
        Profiles.Validate(profile);
        int pixels = Protocol.Geometry(source);
        var settings = profile.output;
        var (width, height) = Profiles.Shape(settings.layout, settings.shape, 1);
        if (!output.Shape.SequenceEqual(settings.shape) || output.Samples.Length != checked(width * height))
            throw new WorkerException("Runtime output tensor shape/length differs from the configured single-channel mask.");
        var probabilities = new float[output.Samples.Length];
        for (int i = 0; i < probabilities.Length; i++)
        {
            float value = output.Samples[i];
            if (!float.IsFinite(value)) throw new WorkerException("Runtime output contains NaN/Infinity.");
            if (settings.postprocess == "sigmoid")
            {
                // Stable sigmoid for positive and negative finite logits; no exp overflow.
                float e = MathF.Exp(-MathF.Abs(value));
                value = value >= 0 ? 1 / (1 + e) : e / (1 + e);
            }
            probabilities[i] = Protocol.Sdr(value); // probability mode rejects out-of-range, never hides a bad model by clipping.
        }
        var mask = new float[pixels];
        for (int y = 0; y < source.height; y++)
        for (int x = 0; x < source.width; x++)
            mask[y * source.width + x] = Protocol.Sdr(Sample(probabilities, width, height, 1, 0,
                x, y, source.width, source.height, settings.resize));
        return mask;
    }

    // Pixel-center mapping: s=(d+0.5)*source/dest-0.5, edge clamped. Nearest uses
    // floor((d+0.5)*source/dest). Bilinear interpolates both axes, no antialias/letterbox.
    private static float Sample(float[] samples, int width, int height, int channels, int channel,
        int x, int y, int destinationWidth, int destinationHeight, string method)
    {
        if (method == "nearest")
        {
            int sx = Math.Min(width - 1, (int)(((double)x + 0.5) * width / destinationWidth));
            int sy = Math.Min(height - 1, (int)(((double)y + 0.5) * height / destinationHeight));
            return samples[(sy * width + sx) * channels + channel];
        }
        double px = Math.Clamp(((double)x + 0.5) * width / destinationWidth - 0.5, 0, width - 1);
        double py = Math.Clamp(((double)y + 0.5) * height / destinationHeight - 0.5, 0, height - 1);
        int x0 = (int)Math.Floor(px), y0 = (int)Math.Floor(py);
        int x1 = Math.Min(width - 1, x0 + 1), y1 = Math.Min(height - 1, y0 + 1);
        double fx = px - x0, fy = py - y0;
        double a = samples[(y0 * width + x0) * channels + channel] * (1 - fx) + samples[(y0 * width + x1) * channels + channel] * fx;
        double b = samples[(y1 * width + x0) * channels + channel] * (1 - fx) + samples[(y1 * width + x1) * channels + channel] * fx;
        return (float)(a * (1 - fy) + b * fy);
    }
}
