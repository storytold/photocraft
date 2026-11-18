# Bundled camera calibration profiles

The native app embeds **9 DCPs for 9 exact model identifiers**, losslessly compressed with gzip.
They provide independent camera calibration, not factory JPEG or Picture Control looks.
RAW decoder support is separate. The web build does not embed this bundle.

All nine original files declare `ProfileCopyright = RawTherapee CC0` and
`ProfileEmbedPolicy = 3`. Files marked only `public domain` are excluded until an explicit
upstream dedication covering those files is available. No permission is inferred from the
RawTherapee program-code license or from the embed policy alone. No upstream program code
is included.

Source: [RawTherapee at the pinned revision](https://github.com/RawTherapee/RawTherapee/tree/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles).
The [CC0 license](LICENSE-CC0.txt) accompanies the files. [manifest.json](manifest.json)
records the unchanged notice, source URL, upstream Git blob, original SHA-256, compressed
SHA-256 and sizes for each file. Decompression restores the exact upstream bytes, including
the notices. Compression uses gzip level 9, mtime=0 and no original filename header.

Keep the manifest and `crates/io/src/bundled_profiles_data.rs` synchronized. IO tests verify
that the manifest, embedded catalog and on-disk files contain exactly these nine CC0 profiles.
The loader also checks the actual DCP notice, usage policy and model before applying it.

See the [exact model coverage](../../docs/camera-profile-coverage.md) and
[selection and rendering documentation](../../docs/camera-profiles.md).
