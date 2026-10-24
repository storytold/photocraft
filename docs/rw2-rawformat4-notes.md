# Panasonic RW2 RawFormat 4 (12-bit) — format research notes

**Status:** measurements only, no decoder. Nothing here was read from or derived from
dcraw, LibRaw, RawSpeed, rawler/dnglab or any other decoder implementation. Every number
below comes from the sample files listed in §6, analysed as byte sequences.

These notes exist because a clean-room decoder for this format is still out of reach, and
the reason turned out to be structural rather than a matter of effort (§5). If they shorten
the next attempt, they have earned their keep. If a decoder gets written anyway, please cite
the measurements rather than re-deriving them.

---

## 1. Container

| Field | Value | Source |
|---|---|---|
| Magic | `49 49 55 00` (`IIU\0`), TIFF variant with a Panasonic `U` | offset 0 |
| IFD0 offset | 24 | header |
| `Compression` (0x000b) | 34316 | IFD0 |
| `RawFormat` (0x002d) | 4 | IFD0 |
| `BitsPerSample` (0x000a) | 12 | IFD0 |
| `SensorWidth` / `SensorHeight` (0x0002 / 0x0003) | 4816 × 3464 | IFD0 |
| Active area (0x0004–0x0007) | top 8, left 8, bottom 3456, right 4600 | IFD0 |
| White level (0x000e–0x0010) | 4095, 4095, 4095 | IFD0 |
| `RawDataOffset` (0x0118) | 229376 | IFD0 |
| `StripOffsets` (0x0111) | `0xFFFFFFFF` (not used) | IFD0 |
| `StripByteCounts` (0x0117) | 0 (not used) | IFD0 |
| Rows per strip (0x0116) | 3464 (single strip) | IFD0 |
| Embedded JPEG preview | IFD0 tag 0x002e, `UNDEFINED`, starts `FF D8 FF E1` (Exif APP1) | — |

The RAW payload runs from `RawDataOffset` to **end of file** with nothing after it in the
four samples here (file size − 229376 = payload length exactly).

## 2. Payload geometry

Measured on the four GX7 samples:

| Quantity | Value |
|---|---|
| Payload length | 19,062,272 – 19,096,064 bytes (**varies per file**, see §5) |
| Length granularity | always a multiple of 16 bytes |
| Average bytes per pixel | ≈ 1.143 (12-bit is 1.5; uncompressed 8-bit would be 1.0) |
| 4816 ÷ 14 | 344 exactly — a row is an exact number of 16-byte groups |
| 19,070,976 (one sample) | = 231 × 82,560 = 231 × 15 rows × 5,504 bytes exactly |

The recurring `16 bytes` / `14 pixels` arithmetic (344 groups × 14 = 4816, and the 0x4000-byte
page × 231) is consistent across all four samples, but see §5 before treating it as a fixed
raster: the total length is content-dependent even though the per-row arithmetic is not.

A 15-row periodicity is directly visible in the byte stream: in transition areas, packets
separated by 15 rows share some byte positions exactly (byte 4, 6, 7, 8 and 13 in the samples
examined) while the remaining positions differ. 15 rows × 5,504 bytes = 82,560 bytes per
"page", and the total payload is an exact multiple of that.

## 3. Per-byte content sensitivity

For a packet stream, correlation with the same byte position in the neighbouring packet
(same row, lag 1 group) over a textured band:

| Byte | lag-1 autocorrelation |
|---|---|
| 7 | 0.850 |
| 5 | 0.648 |
| 4 | 0.420 (peak instead at lag 1032 = 3 rows: 0.420) |
| 8 | 0.180 (peak at lag 1032) |
| all others | ≤ 0.07 at every lag tested (1…2000) |

An FFT autocorrelation over 516,000 consecutive groups (1500 rows) puts the global peak of
byte 7 at lag 1, byte 5 at lag 1, byte 4 at lag 1032, byte 8 at lag 1032, and finds no
meaningful peak for the remaining twelve bytes.

Two regions with very different brightness (a flat dark switch body and a flat bright
background, both free of edges) differ in mean by **46.0 in byte 5** and by **less than 4 in
every other byte**.

Constant-byte groups exist where the image contains a constant gradient: 427 groups of all
`0x0D` (a +13 count step), 417 of all `0x00` and 20 of all `0x8C` in one sample. Transitions
into such a run begin at byte 8 of a group (half a group), so whatever field boundary exists
is byte-aligned at that point.

## 4. Quantisation behaviour

Within the constant-gradient regions, no group is within 15 bits of another: the smallest
Hamming distance to the all-`0x0D` pattern is **16 bits**, and single-byte-differing groups
(`8c 8b 8c 8c …`) exist elsewhere. Encoding states therefore change in large steps, and
there is no near-miss neighbourhood to interpolate between — a property worth knowing before
writing a matcher-based decoder.

## 5. Why random-access analysis fails: the stream is content-adaptive

Four frames of one static scene (same camera, same settings, seconds apart) were measured:

| File | File size | Payload length | Groups of 16 |
|---|---|---|---|
| P1340729 | 19,300,352 | 19,070,976 | 1,191,936 |
| P1340730 | 19,291,648 | 19,062,272 | 1,191,392 |
| P1340731 | 19,301,888 | 19,072,512 | 1,192,032 |
| P1340732 | 19,325,440 | 19,096,064 | 1,193,504 |

Two observations, both reproducible:

1. **The payload length differs between frames of the same scene** (spread 33,792 bytes,
   0.18 %). A fixed-layout raster of a fixed sensor size cannot do that.
2. **Group 0 already differs completely between the four files**, even though the stream
   starts in the masked border rows where the scene is identical and only sensor noise
   differs. The first 16 bytes of each file share nothing.

Together these mean the encoder is sequential and error-feedback-like: a small input
difference is amplified along the prediction chain instead of staying local, the coded length
drifts with content, and output at one position cannot be related to the image at one
position. That is why every positional measurement in §3 looks weak: correlating byte *j* of
group *k* against the pixels believed to sit there assumes addressability that this format
does not provide.

A consequence for future work: the decoder has to be built **sequentially and validated by
simulation** — guess a local rule, run it forward over the whole payload, and check whether
the decoded plane is smooth and matches the embedded JPEG. Positional correlation, field
entropy and bit-permutation searches all assume a fixed layout and cannot work here, no
matter how they are tuned.

## 6. Sample corpus used

Camera-body shots (product photography, GX7, 12-bit, RawFormat 4). These are private frames;
the hashes are given so the measurements can be re-checked against the maintainer's own
samples or a CC0 set, not so the files can be redistributed.

| File | Size | SHA-256 (first 32 hex) |
|---|---|---|
| P1340729.RW2 | 19,300,352 | `1f55f1d8adfa7ecdd27e6583c592d032` |
| P1340730.RW2 | 19,291,648 | `997a26cd326f942e2f9cdd0ca57126e9` |
| P1340731.RW2 | 19,301,888 | `f6eed8077a35c8338fe4a6711d2bc9b6` |
| P1340732.RW2 | 19,325,440 | `dc689522c6ee07e7a94ae39dca3e1141` |

Public CC0 samples of the same generation exist and were used to cross-check the container
fields only (all report RawFormat 4, 12-bit, Compression 34316):

- <https://raw.pixls.us/data/Panasonic/> — e.g. `DMC-GX7`, `DMC-GX7MK2`, `DMC-G7`,
  `DMC-GH3`, `DMC-GH4`, `DMC-GF1`, `DMC-GH2`, `DMC-G5`, `DMC-G6`, `DMC-LX7`

## 7. Open questions

- The exact rule that makes coded length depend on content (adaptive code length? state
  carried between groups?) is unresolved.
- The 15-row period is reproducible but its exact role (row permutation inside a page, bit
  plane interleave, or an artefact of the error-feedback state) is unresolved.
- The three content-sensitive byte positions (4, 5, 7) are located, but their mapping to
  pixel positions is unknown — and per §5 it may not be expressible as a mapping.
- Whether the first difference between two same-scene frames enters at byte 0 or is already
  present in the sensor's masked border was not separated.
