# Synthetic AVIF fixture

Original mathematical gradient, CC0 (see LICENSE.txt); contains no photographs or third-party art.
Generated 2026-10-09 with FFmpeg/libaom, independently of PhotoCraft's ravif encoder.

32 × 18, RGB16 little endian input: R = x × 1901 + 73, G = y × 3511 + 107,
B = 23117. The AVIF is 12-bit YUV 4:4:4, limited range, BT.709 primaries and matrix,
sRGB transfer, one still picture. Generation:

```text
ffmpeg -f rawvideo -pixel_format rgb48le -video_size 32x18 -i gradient16.rgb
  -frames:v 1 -c:v libaom-av1 -still-picture 1 -pix_fmt yuv444p12le -crf 0
  -cpu-used 8 -color_primaries bt709 -color_trc iec61966-2-1 -colorspace bt709
  -f avif gradient12.avif
ffmpeg -i gradient12.avif -frames:v 1 -vf zscale=matrixin=709:rangein=limited:matrix=gbr:range=full
  -pix_fmt gbrp16le -f rawvideo gradient12.gbr16
```

The RGB reference uses FFmpeg's independent libdav1d decoder and zscale conversion with an explicit BT.709 limited-range input. The planar G/B/R
16-bit result is interleaved R/G/B into gradient12.rgb48le.
The test allows at most 128/65535 per channel for integer versus floating-point matrix rounding.
FFmpeg is fixture/oracle tooling only; PhotoCraft never invokes it or requires its installation.

`gradient-general-header.avif` uses the same original gradient and FFmpeg command,
with `-still-picture 0` and `-pix_fmt yuv444p10le`. It is a single still image whose
AV1 sequence header omits the optional still-picture optimization flags. This
FFmpeg build leaves its primaries and transfer unspecified (CICP 2/2), despite
the requested flags; the BT.709 matrix remains explicit. The test checks the
documented sRGB fallback and the original gradient within 0.01 normalized error.
It regresses the incorrect rejection of valid still items with general AV1 headers.
