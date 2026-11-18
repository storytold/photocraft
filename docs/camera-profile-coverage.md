# Bundled camera profile coverage

The native build bundles exactly **9 CC0-1.0 profiles**. Every original `ProfileCopyright`
is `RawTherapee CC0` and every `ProfileEmbedPolicy` is `3`. Source filenames and model
identifiers can differ; matching uses the identifier inside the DCP, without guessing
regional aliases or neighbouring models.

| Exact model identifier | Original source file | License |
|---|---|---|
| Canon EOS 5D Mark II | [Canon EOS 5D Mark II.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%205D%20Mark%20II.dcp) | CC0-1.0 |
| Canon EOS R8 | [Canon EOS R8.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20R8.dcp) | CC0-1.0 |
| Canon EOS RP | [Canon EOS RP.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS%20RP.dcp) | CC0-1.0 |
| Canon EOS-1D X Mark III | [Canon EOS-1D X Mark III.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS-1D%20X%20Mark%20III.dcp) | CC0-1.0 |
| Canon EOS-1Ds Mark II | [Canon EOS-1Ds Mark II.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/Canon%20EOS-1Ds%20Mark%20II.dcp) | CC0-1.0 |
| FUJIFILM DBP for GX680 | [FUJIFILM DBP for GX680.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20DBP%20for%20GX680.dcp) | CC0-1.0 |
| FUJIFILM X-T4 | [FUJIFILM X-T4.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/FUJIFILM%20X-T4.dcp) | CC0-1.0 |
| SONY ILCE-7M4 | [SONY ILCE-7M4.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/SONY%20ILCE-7M4.dcp) | CC0-1.0 |
| samsung SM-G930V | [samsung SM-G930V.dcp](https://raw.githubusercontent.com/RawTherapee/RawTherapee/5f486d3678b34c74ba0c63571c17babe20935019/rtdata/dcpprofiles/samsung%20SM-G930V.dcp) | CC0-1.0 |

All sources are pinned to RawTherapee revision `5f486d3678b34c74ba0c63571c17babe20935019`.
See the [manifest](../assets/camera-profiles/manifest.json) for byte hashes and original notices.

The 46 previously proposed files marked only `public domain` are excluded pending explicit
upstream dedication. In particular, this bundle does **not** include Nikon Z f calibration.
Having a DCP does not add a RAW decoder or reproduce the camera manufacturer's JPEG look.
See [camera profiles](camera-profiles.md) for fallback behavior and implementation limits.
