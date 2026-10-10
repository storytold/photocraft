# Camera calibration profiles

The **native app includes 55 profiles for 55 exact model identifiers**, including Nikon Z f.
No Adobe installation is required. See the [complete coverage list](camera-profile-coverage.md)
and [per-file provenance manifest](../assets/camera-profiles/manifest.json).
Each included file explicitly declares CC0 or public domain and ProfileEmbedPolicy=3.
No Adobe profiles or upstream program code are bundled. The DCP parser and renderer are
implemented in Rust from the public DNG specification.

These profiles provide independent camera colour calibration. They are **not factory camera
JPEG/Picture Control looks**, and do not promise pixel-identical ACR output. Nikon Auto,
custom manual offsets and toning are recorded but not reproduced by the bundled profiles.
No JPEG fitting is used. A profile on this list does not add support for its RAW format;
CR3, RAF and other unsupported compression variants remain separate decoder work.

## Selection and coverage

1. `PHOTOCRAFT_CAMERA_PROFILE`: an explicitly selected DCP file, validated against the camera.
2. Profiles in PhotoCraft user directories or `PHOTOCRAFT_CAMERA_PROFILES_DIR`.
3. The bundled profile for the exact matching model.
4. Existing RAW/DNG calibration if available; otherwise the existing neutral fallback, with
   an explicit warning that colour calibration is missing. Neighbouring models are not substituted.

Model matching normalizes case, spaces, punctuation and make prefixes. Regional model aliases
are not guessed. For user profiles, Nikon Picture Control name/base metadata selects a matching
look when available; Camera Standard, Adobe Standard and then another same-model profile are
fallbacks, reported in the limitations. Auto's adaptive processing is not emulated.
An explicit invalid, wrong-model or usage-restricted profile produces an error.

The Camera Raw dialog displays the applied profile. `document.inspect.rawProfile` and
`ui.inspect.cameraRaw.openingRaw.profile` expose source, name, model, filename, license/notice,
usage policy, digest, calibration/table/curve information, Picture Control and limitations.
XMP preserves provenance, without embedding the profile payload or absolute filesystem path.

```json
{"command":"raw.profiles","params":{"model":"Nikon","refresh":true}}
```

Omit `model` to list all available profiles. `bundledCount` identifies the built-in portion;
each entry has `source` (`bundled-dcp` or `external-dcp`). User-directory inventory is a header
index, not confirmation that every external profile is licensed or renderable.

## User profile locations and usage policy

Adobe directories are **not scanned automatically**. Optional user locations:

- `~/.local/share/photocraft/camera-profiles`
- macOS: `~/Library/Application Support/Photocraft/camera-profiles`
- Windows: `%APPDATA%/Photocraft/camera-profiles`
- `PHOTOCRAFT_CAMERA_PROFILES_DIR`: explicit additional directory
- `PHOTOCRAFT_CAMERA_PROFILE`: explicit file override

Users must have permission for profiles they supply. Public DCP format documentation does
not license the contents of arbitrary DCP files. DNG 1.7.1 ProfileEmbedPolicy 0/1 restricts
processing to DNG input: PhotoCraft refuses these profiles for NEF/CR2/ARW and other non-DNG
inputs. Policy 2 permits external use but no embedding; PhotoCraft does not embed the payload.
Policy 3 declares no profile-specific restrictions; separate license terms still matter.

The directory index caches for 60 seconds; files are capped at 32 MiB, traversal is bounded,
and directory symlinks are not followed. Parsed user profiles are invalidated on size/mtime
changes. The web build neither embeds this bundle nor discovers desktop profile files.

## Rendering and limitations

1. Validate one/two-illuminant ColorMatrix and ForwardMatrix calibration.
2. Retain as-shot white balance and interpolate by inverse temperature.
3. Convert balanced camera RGB to XYZ D50, then linear ProPhoto (RIMM).
4. Apply HueSatMap calibration tables, including value encoding.
5. Apply RAW baseline exposure, profile BaselineExposureOffset and user exposure.
6. Apply LookTable with hue-wrap trilinear interpolation and linear/sRGB value indexing.
7. Apply the profile's cubic tone curve and encode 16-bit ProPhoto output.

Display ICC conversion is separate. Re-development after exposure/white-balance edits uses
the same selection path. HDR, four-plane legacy profiles, ProfileGainTableMap and automatic
black rendering requested by some profiles remain unsupported. All 55 bundled profiles are
covered by native parse/metadata/policy tests; synthetic tests cover hostile input and policy
rejection. The built-in Z f calibration has no factory Picture Control look: use the existing
Auto sliders or manual adjustments for exposure/saturation changes, with white balance retained.

References: [DNG 1.7.1 specification, ProfileEmbedPolicy](https://helpx.adobe.com/content/dam/help/en/camera-raw/digital-negative/jcr_content/root/content/flex/items/position/position-par/download_section_733958301/download-1/DNG_Spec_1_7_1_0.pdf),
[ExifTool Nikon tag tables](https://exiftool.org/TagNames/Nikon.html),
[Nikon Z f Picture Controls](https://onlinemanual.nikonimglib.com/zf/en/picture_controls_37.html).
