# Bundled camera calibration profiles

The native app embeds 55 DCPs (55 exact model identifiers), compressed losslessly with gzip.
No Adobe installation is needed. These are independent camera calibrations, not manufacturer
Picture Control presets or a promise of matching camera JPEGs. RAW decoder support is separate:
for example, a listed CR3/RAF model does not add a decoder for that format. The web build does
not embed this native bundle.

Source: [RawTherapee profile data at the pinned revision](https://github.com/RawTherapee/RawTherapee/tree/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles).
Only profiles whose original ProfileCopyright explicitly declares RawTherapee CC0 or public
domain, and whose ProfileEmbedPolicy is 3, are included. Of 161 upstream files audited, 106
with ambiguous or absent declarations were excluded. No permission is inferred from the
repository's program-code license. No upstream program code is included.

[manifest.json](manifest.json) records each original notice, pinned source URL, upstream Git
blob, original SHA-256 and compressed SHA-256. Original bytes, including notices, are preserved.
Licenses: [CC0](LICENSE-CC0.txt), [public-domain declarations](LICENSE-public-domain.txt).
The manifest and `crates/io/src/bundled_profiles_data.rs` must be updated together; the IO tests
check the complete bundle. Files are gzip level 9, mtime=0, with no original filename header.

Model matching normalizes case, separators and make prefixes, but does not guess regional
aliases or substitute a neighbouring model. The identifier in the DCP is authoritative even
when its source filename names a regional variant.

## Exact coverage

See the [55-model coverage and licensing table](../../docs/camera-profile-coverage.md).
The exact model identifiers, original filenames, declarations and hashes are also available
in [manifest.json](manifest.json).
