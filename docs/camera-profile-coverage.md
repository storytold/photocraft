# Redistributable camera profile coverage

Updated: 2026-10-10. The native desktop app includes **55 profiles for 55 exact camera model identifiers**,
including Nikon Z f. No Adobe installation is required.

This list includes only profiles whose original `ProfileCopyright` explicitly declares
**RawTherapee CC0** or **public domain**, and whose `ProfileEmbedPolicy = 3` (No Restrictions).
The source is the RawTherapee profile database at revision
`5f486d3678b34c74ba0c63571c17babe20935019`. Of the 161 upstream profiles audited, 106 were
excluded because their individual license declarations were absent or ambiguous. Permission
is based on each profile's own declaration, not inferred from the repository's program-code license.

- **CC0-1.0**: the original profile declares RawTherapee CC0; [full license](../assets/camera-profiles/LICENSE-CC0.txt).
- **Public-Domain**: the original profile explicitly declares public domain; [declaration and provenance](../assets/camera-profiles/LICENSE-public-domain.txt).
- [Per-file original notices, source URLs and hashes](../assets/camera-profiles/manifest.json). Original bytes are preserved in lossless gzip files.

## Scope

These profiles provide independent camera colour calibration. They **do not guarantee the camera's
JPEG appearance, Picture Control processing, or Adobe Camera Raw's default rendering**.
A listed camera has a redistributable calibration profile; this does not mean that its RAW format
or every compression mode can be decoded. For example, CR3 and compressed RAF decoding remain separate work.
The web build does not currently include this native profile bundle.

Matching uses the model identifier stored in the profile, with normalization of case, spaces,
punctuation and make prefixes. Regional aliases are not guessed, and neighbouring models are
not substituted. For example, the source file `CANON EOS 250D.dcp` contains the model identifier
`CANON EOS REBEL SL3`; this list uses the internal identifier.

For cameras outside this list, valid calibration embedded in the RAW file can still be used,
or users can supply a matching profile they are permitted to use. When calibration is missing,
the app reports that limitation explicitly.

## Complete camera list

Each source-file link points to the original profile at the pinned revision, for comparison
with the hashes and declarations recorded in the manifest.

| No. | Profile camera model identifier | License/declaration | Source file |
|---|---|---|---|
| 1 | CANON EOS REBEL SL3 | Public-Domain | [`CANON EOS 250D.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/CANON%20EOS%20250D.dcp) |
| 2 | CANON EOS 800D | Public-Domain | [`CANON EOS 800D.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/CANON%20EOS%20800D.dcp) |
| 3 | CANON EOS M50 | Public-Domain | [`CANON EOS M50.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/CANON%20EOS%20M50.dcp) |
| 4 | CANON POWERSHOT G5 X MARK II | Public-Domain | [`CANON POWERSHOT G5 X MARK II.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/CANON%20POWERSHOT%20G5%20X%20MARK%20II.dcp) |
| 5 | Canon EOS 5D Mark II | CC0-1.0 | [`Canon EOS 5D Mark II.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%205D%20Mark%20II.dcp) |
| 6 | CANON EOS 5D MARK IV | Public-Domain | [`Canon EOS 5D Mark IV.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%205D%20Mark%20IV.dcp) |
| 7 | CANON EOS M6 MARK II | Public-Domain | [`Canon EOS M6 Mark II.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20M6%20Mark%20II.dcp) |
| 8 | CANON EOS R | Public-Domain | [`Canon EOS R.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20R.dcp) |
| 9 | CANON EOS R5 | Public-Domain | [`Canon EOS R5.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20R5.dcp) |
| 10 | CANON EOS R6 | Public-Domain | [`Canon EOS R6.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20R6.dcp) |
| 11 | Canon EOS R8 | CC0-1.0 | [`Canon EOS R8.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20R8.dcp) |
| 12 | Canon EOS RP | CC0-1.0 | [`Canon EOS RP.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20RP.dcp) |
| 13 | Canon EOS-1D X Mark III | CC0-1.0 | [`Canon EOS-1D X Mark III.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS-1D%20X%20Mark%20III.dcp) |
| 14 | Canon EOS-1Ds Mark II | CC0-1.0 | [`Canon EOS-1Ds Mark II.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS-1Ds%20Mark%20II.dcp) |
| 15 | CANON POWERSHOT G1 X MARK II | Public-Domain | [`Canon PowerShot G1 X Mark II.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20PowerShot%20G1%20X%20Mark%20II.dcp) |
| 16 | FUJIFILM DBP for GX680 | CC0-1.0 | [`FUJIFILM DBP for GX680.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20DBP%20for%20GX680.dcp) |
| 17 | FUJIFILM X-A5 | Public-Domain | [`FUJIFILM X-A5.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-A5.dcp) |
| 18 | FUJIFILM X-A7 | Public-Domain | [`FUJIFILM X-A7.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-A7.dcp) |
| 19 | FUJIFILM X-E3 | Public-Domain | [`FUJIFILM X-E3.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-E3.dcp) |
| 20 | FUJIFILM X-H1 | Public-Domain | [`FUJIFILM X-H1.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-H1.dcp) |
| 21 | FUJIFILM X-PRO3 | Public-Domain | [`FUJIFILM X-Pro3.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-Pro3.dcp) |
| 22 | FUJIFILM X-S10 | Public-Domain | [`FUJIFILM X-S10.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-S10.dcp) |
| 23 | FUJIFILM X-T3 | Public-Domain | [`FUJIFILM X-T3.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-T3.dcp) |
| 24 | FUJIFILM X-T4 | CC0-1.0 | [`FUJIFILM X-T4.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-T4.dcp) |
| 25 | NIKON D3300 | Public-Domain | [`NIKON D3300.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D3300.dcp) |
| 26 | NIKON D500 | Public-Domain | [`NIKON D500.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D500.dcp) |
| 27 | NIKON D5300 | Public-Domain | [`NIKON D5300.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D5300.dcp) |
| 28 | NIKON D610 | Public-Domain | [`NIKON D610.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D610.dcp) |
| 29 | NIKON D7100 | Public-Domain | [`NIKON D7100.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D7100.dcp) |
| 30 | NIKON D7500 | Public-Domain | [`NIKON D7500.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D7500.dcp) |
| 31 | NIKON D800 | Public-Domain | [`NIKON D800.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D800.dcp) |
| 32 | NIKON D850 | Public-Domain | [`NIKON D850.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20D850.dcp) |
| 33 | NIKON Z 5 | Public-Domain | [`NIKON Z 5.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%205.dcp) |
| 34 | NIKON Z 50 | Public-Domain | [`NIKON Z 50.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%2050.dcp) |
| 35 | NIKON Z 6 | Public-Domain | [`NIKON Z 6.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%206.dcp) |
| 36 | NIKON Z 6_2 | Public-Domain | [`NIKON Z 6_2.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%206_2.dcp) |
| 37 | NIKON Z 7 | Public-Domain | [`NIKON Z 7.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%207.dcp) |
| 38 | NIKON Z 8 | Public-Domain | [`NIKON Z 8.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%208.dcp) |
| 39 | NIKON Z 9 | Public-Domain | [`NIKON Z 9.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%209.dcp) |
| 40 | NIKON Z F | Public-Domain | [`NIKON Z F.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/NIKON%20Z%20F.dcp) |
| 41 | OLYMPUS E-M1 | Public-Domain | [`OLYMPUS E-M1.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/OLYMPUS%20E-M1.dcp) |
| 42 | OLYMPUS E-M5MARKII | Public-Domain | [`OLYMPUS E-M5MarkII.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/OLYMPUS%20E-M5MarkII.dcp) |
| 43 | PANASONIC DC-S5M2 | Public-Domain | [`PANASONIC DC-S5M2.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/PANASONIC%20DC-S5M2.dcp) |
| 44 | PANASONIC DMC-LX100 | Public-Domain | [`PANASONIC DMC-LX100.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/PANASONIC%20DMC-LX100.dcp) |
| 45 | PENTAX K-50 | Public-Domain | [`PENTAX K-50.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/PENTAX%20K-50.dcp) |
| 46 | SONY DSC-RX100M6 | Public-Domain | [`SONY DSC-RX100M6.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20DSC-RX100M6.dcp) |
| 47 | SONY ILCE-6400 | Public-Domain | [`SONY ILCE-6400.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-6400.dcp) |
| 48 | SONY ILCE-6600 | Public-Domain | [`SONY ILCE-6600.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-6600.dcp) |
| 49 | SONY ILCE-7 | Public-Domain | [`SONY ILCE-7.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7.dcp) |
| 50 | SONY ILCE-7C | Public-Domain | [`SONY ILCE-7C.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7C.dcp) |
| 51 | SONY ILCE-7M4 | CC0-1.0 | [`SONY ILCE-7M4.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7M4.dcp) |
| 52 | SONY ILCE-7RM4 | Public-Domain | [`SONY ILCE-7RM4.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7RM4.dcp) |
| 53 | SONY ILCE-7SM3 | Public-Domain | [`SONY ILCE-7SM3.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7SM3.dcp) |
| 54 | SONY ILCE-9 | Public-Domain | [`SONY ILCE-9.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-9.dcp) |
| 55 | samsung SM-G930V | CC0-1.0 | [`samsung SM-G930V.dcp`](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/samsung%20SM-G930V.dcp) |

## Checking the applied profile

`raw.profiles` lists available profiles. `bundledCount` is the number of bundled profiles in the
query result, and `source = bundled-dcp` identifies a bundled entry. Omit `model` to list all
models, or filter with a parameter such as `{"model":"Nikon"}`.

`document.inspect.rawProfile` reports the profile actually applied to an image, its source,
license declaration and limitations. Optional user profiles are outside this fixed list and
require their own permission checks. The app does not automatically scan Adobe installation
directories. See [camera profiles and rendering](camera-profiles.md) for selection and usage rules.
