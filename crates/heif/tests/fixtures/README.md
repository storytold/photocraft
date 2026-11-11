# HEVC 4:2:2 regression vector

`intra-422-10.hevc` is a synthetic 128×128 FFmpeg `testsrc2` pattern encoded
with x265 as one 10-bit 4:2:2 intra picture. It exercises the Table 8-3 chroma
mode mapping fixed by heic-rs PR #10.

Source: https://github.com/jaredt17/heic-rs/blob/e3d0a074978323ccf49089703233d4831076ccb0/tests/fixtures/hevc/intra-422-10.hevc

MIT OR Apache-2.0; the upstream licence files are retained alongside this vector.
The expected FNV-1a hash `c9324cae793f082f` covers Y/Cb/Cr u16 little-endian
samples and was measured by the upstream vector's author with FFmpeg's independent
decoder. See that revision's `scripts/make-422-vector.sh` for reproduction.
No camera photographs or CRX sensor streams are included.
