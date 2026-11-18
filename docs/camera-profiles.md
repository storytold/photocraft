# Camera calibration profiles

The **native app includes 9 CC0 profiles for 9 exact model identifiers**.
No Adobe installation is required. See the [complete coverage list](camera-profile-coverage.md)
and [per-file provenance manifest](../assets/camera-profiles/manifest.json).
Each included file has the exact original notice `RawTherapee CC0` and ProfileEmbedPolicy=3.
The 46 files marked only `public domain` are excluded pending explicit upstream dedication.
No Adobe profiles or upstream program code are bundled. The DCP parser and renderer are
implemented in Rust from the public DNG specification.

These profiles provide independent camera colour calibration. They are **not factory camera
JPEG/Picture Control looks**, and do not promise pixel-identical ACR output. Nikon Auto,
custom manual offsets and toning are recorded but not reproduced by the bundled profiles.
The DCP path does not fit the image to a JPEG. A profile on this list does not add support
for its RAW format; unsupported compression variants remain separate decoder work.

## Selection and coverage

Trusted native and local CLI direct opens explicitly authorize installed camera profiles.
Their selection order is:

1. `PHOTOCRAFT_CAMERA_PROFILE`: an explicitly selected DCP file, validated against the camera.
2. Profiles in PhotoCraft user directories or `PHOTOCRAFT_CAMERA_PROFILES_DIR`.
3. The bundled profile for the exact matching model.
4. The existing main-branch behavior: embedded/measured calibration, then a usable embedded
   JPEG fit for an uncalibrated camera, then the existing neutral fallback and warning.
   Neighbouring models are not substituted. Monochrome sensors skip DCP selection.

An explicit `DevelopOptions::camera_look` takes precedence over automatic DCP selection.
The low-level `develop_sensor_profile` API rejects combining a DCP and a supplied camera look.

Model matching normalizes case, spaces, punctuation and make prefixes. Regional model aliases
are not guessed. For user profiles, Nikon Picture Control name/base metadata selects a matching
look when available; Camera Standard, Adobe Standard and then another same-model profile are
fallbacks, reported in the limitations. Auto's adaptive processing is not emulated.
Automatic selection tries all candidates at each priority, then the next priority and finally
the bundled profile. A missing, unreadable, malformed, wrong-model or usage-restricted catalog
entry is skipped. When another DCP is selected, its report includes bounded diagnostics
containing filenames, never absolute paths. If no usable installed or bundled profile exists, development retains the
existing RAW calibration/fallback behavior. An explicit invalid, wrong-model or usage-restricted
`PHOTOCRAFT_CAMERA_PROFILE` produces an error; it is never silently replaced by another profile.

The RAW Develop dialog displays the applied profile. `document.inspect.rawProfile` and
`ui.inspect.cameraRaw.openingRaw.profile` expose source, name, model, filename, license/notice,
usage policy, digest, calibration/table/curve information, Picture Control and limitations.
XMP preserves provenance, without embedding the profile payload or absolute filesystem path.

```json
{"command":"raw.profiles","params":{"model":"Nikon","refresh":true}}
```

Omit `model` to list all available profiles. `bundledCount` identifies the built-in portion;
each entry has `source` (`bundled-dcp` or `external-dcp`). User-directory inventory is a header
index, not confirmation that every external profile is licensed or renderable.

## Profile-source authority

The byte-only import APIs and RAW re-development/curve helpers default to
`CameraProfilePolicy::BundledOnly`. They do not consult profile environment variables,
user directories or the external-profile cache. A trusted top-level open opts into
`CameraProfilePolicy::Installed` through `ImportOptions` / the policy-aware RAW helpers.
Profile authority is a typed caller decision, never taken from document XMP or provenance.
Generic UI and engine byte opens use this same bundled-only default, including bytes with
an optional display/save path. Trusted native file-open entry points explicitly authorize
installed profiles and carry that decision into background processing; the path metadata
alone grants no authority.

Capability-scoped headless and desktop automation use bundled-only imports, including
indirect RAW bytes embedded in other formats. Deferred prompts, background opens and RAW
Develop dialogs retain the original request's authority after the request gate is restored;
both deferred tone-curve previews and re-development narrow captured authority when the
current session has an authorization gate.

RAW Smart Object placement and recursive decoding use bundled calibration in both trusted
and scoped sessions. This keeps re-rendering, editing embedded contents and PSD export
independent of user-directory files and shared-cache history. To preserve a trusted external
profile rendering in a Smart Object, develop the RAW first and embed the developed document.

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
7. Apply raw-stage Shadows/Highlights/Clarity/Texture/Dehaze, then the profile's cubic tone
   curve and encode 16-bit ProPhoto output. The sensor's default tone curve is not applied twice.

Display ICC conversion is separate. Re-development after exposure/white-balance edits uses
the same selection path. HDR, four-plane legacy profiles, ProfileGainTableMap and automatic
black rendering requested by some profiles remain unsupported. All 9 bundled profiles are
covered by native parse/metadata/policy tests; synthetic tests cover hostile input and policy
rejection. Nikon Z f is not included in this CC0-only bundle.

`raw.profiles` scans user directories, so capability-scoped automation refuses this command
under the same policy as installed ICC-profile discovery; trusted local sessions can query it.

References: [DNG 1.7.1 specification, ProfileEmbedPolicy](https://helpx.adobe.com/content/dam/help/en/camera-raw/digital-negative/jcr_content/root/content/flex/items/position/position-par/download_section_733958301/download-1/DNG_Spec_1_7_1_0.pdf),
[ExifTool Nikon tag tables](https://exiftool.org/TagNames/Nikon.html),
[Nikon Z f Picture Controls](https://onlinemanual.nikonimglib.com/zf/en/picture_controls_37.html).

## Shared decoder integration

The RAW sensor model comes from the pinned `craft-raw` dependency, with DCP directories
and Nikon metadata read through `craft-tiff`. Retained `RawDecode` instances carry the
profile authority used to select their calibration. A re-development request with different
authority decodes and selects again; the retained-open cache only yields matching authority.
The DCP colour and tone stages run in both the fused band renderer and the full-frame path.
