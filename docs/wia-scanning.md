# Scanning with Windows Image Acquisition

PhotoCraft imports scans through Windows Image Acquisition (WIA) on native Windows builds.
Install the scanner manufacturer's Windows driver, connect and power on the scanner, and choose
**File > Import > WIA Support…**. Select the scanner and scan settings in the Windows dialog;
the acquired image opens as a new, unsaved PhotoCraft document. Save it with Save As.

Scanning runs in a desktop background job, so the PhotoCraft window remains responsive. Cancel
in the scanner dialog or cancel the PhotoCraft job to stop acquisition. Existing documents are
not replaced. Cancellation and driver errors do not open a partially acquired document.

The native backend uses Windows PowerShell, WIA discovery/settings dialogs, and the current
Windows `ImageScanner` API for flatbed acquisition on Windows 10/11. It requires no additional
runtime installation; the scanner's WIA driver is still required. WIA
acquisition is unavailable on macOS, Linux and the web build. TWAIN is not implemented.

## Command and automation

The engine command is `file.import.wiaSupport`. Calling it with `{}` or `{"action":"list"}`
enumerates WIA scanners without opening a scan dialog. The result includes `available` and a
`devices` array; each device has `id`, `name`, `manufacturer`, `description` and `type`.
An empty list means Windows did not expose a scanner to WIA. Device IDs are opaque strings:
use the exact `id` returned by enumeration rather than a display name or list position.

The menu invokes `{"action":"acquire"}`. Automation can request the same interactive scan or
acquire from an explicit device without scanner dialogs:

```json
{
  "action": "acquire",
  "deviceId": "<exact id from devices>",
  "showUI": false,
  "dpi": 300,
  "mode": "color"
}
```

| Parameter | Accepted values | Default and behaviour |
|---|---|---|
| `action` | `list`, `acquire` | `list`; enumeration does not scan |
| `deviceId` | Exact enumerated scanner ID | Interactive acquisition lets the user select a scanner; acquisition without dialogs requires an explicit ID |
| `showUI` | Boolean | `true`; `false` transfers without scanner settings or progress dialogs |
| `dpi` | Integer, 1–12800 | Optional; omission uses the driver or dialog resolution |
| `mode` | `color`, `gray`, `text` | `color`; requests the corresponding WIA acquisition intent |

The DPI bounds validate the request, not the scanner's capabilities. A driver can accept only a
list or range of resolutions, or have a read-only resolution. Unsupported values return an
error rather than silently changing the requested resolution. When `dpi` is provided with
`showUI:true`, PhotoCraft applies it after the settings dialog and before transfer. The returned
scan metadata reports the resolution supplied by the driver. [Microsoft documents WIA resolution
constraints](https://learn.microsoft.com/en-us/windows-hardware/drivers/image/wia-ips-xres).

Interactive cancellation is a normal result with `acquired:false` and `cancelled:true`. Successful
acquisition reports `acquired:true` and scan/document metadata. No scanner, an unavailable selected
device, an unsupported setting, a busy driver or a failed transfer produces an actionable error.
Non-Windows enumeration reports `available:false` and an empty `devices` array.

In the desktop control channel, pass `wait:false` for scans that may take longer than a request
timeout, then poll `jobs.list` and cancel with `jobs.cancel {"job":<job id>}`:

```json
{"id":1,"method":"engine.execute","params":{"command":"file.import.wiaSupport","params":{"action":"list"}}}
{"id":2,"method":"engine.execute","params":{"command":"file.import.wiaSupport","params":{"action":"acquire","deviceId":"<exact id>","showUI":false,"dpi":300,"mode":"color"},"wait":false}}
{"id":3,"method":"jobs.list","params":{}}
{"id":4,"method":"jobs.cancel","params":{"job":1}}
```

Use the job ID returned by request 2 in request 4. Direct `Session::execute` calls are synchronous;
desktop dispatch uses the job infrastructure. Cancelling a running job stops the WIA helper
process and discards its output. See [background jobs](control-protocol.md#background-jobs-210)
for the control/MCP lifecycle.

## Image fidelity and driver behaviour

PhotoCraft selects a format the scanner supports, preferring TIFF, PNG, BMP, then JPEG. The
V500 tested here exposes BMP only. Acquisition uses `ImageScanner.ScanFilesToFolderAsync`:
the legacy WIA Automation `Item.Transfer` returned access denied on this working scanner,
including when requesting BMP. The
scan goes through the existing image importer, retaining the bit depth and embedded colour
profile that it supports. Temporary acquisition files are cleaned up; they do not become the
document's save path. Metadata or colour profiles that the driver omits cannot be recovered.
[Microsoft documents the current scanner API](https://learn.microsoft.com/en-us/uwp/api/windows.devices.scanners.imagescanner).

PhotoCraft converts the dialog's pixel rectangle to inches before applying a DPI override and
clamps it to the reported platen dimensions. Verify the same physical area at different
resolutions with each driver; PhotoCraft does not assume a fixed platen size.
[Microsoft requires drivers to update scan coordinates and extents after resolution changes](https://learn.microsoft.com/en-us/windows-hardware/drivers/image/wia-ips-xextent).

Close Epson Scan or other scanner software before testing if WIA reports a busy or locked device.
For an offline or communication error, check power and USB, then enumerate again. Missing devices
usually require checking the installed WIA driver and Windows Image Acquisition service.
[Microsoft's WIA error reference](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/wia/-wia-error-codes)
explains the device status codes included in error diagnostics.

## Epson Perfection V500 validation

Use the driver appropriate to the exact V500 model and Windows version from
[Epson's V500 Photo support page](https://epson.com/Support/Scanners/Perfection-Series/Epson-Perfection-V500-Photo/s/SPT_B11B189011).
The V500's original documentation states that WIA exposes fewer scanner features than Epson
Scan/TWAIN. Film and slide scanning, DIGITAL ICE and the full optical-resolution range must be
verified separately; this implementation does not promise those features.
[Epson's V500 guide](https://files.support.epson.com/pdf/prv5ph/prv5phug.pdf).

Test the following on a physical V500, recording Windows version, driver version, device ID,
requested/returned DPI, returned format, dimensions, bit depth and any HRESULT:

1. Power on and connect USB; confirm enumeration contains the V500. If another scanner is
   present, acquire by the V500's exact ID and verify that the intended scanner runs.
2. Put a photo on the platen and scan through the menu at 300 dpi. Confirm preview/settings,
   transfer, document dimensions, orientation, colour and Save As all work.
3. Scan the same physical area at 300 and 600 dpi. Check that dimensions approximately double
   while the selected area stays the same. Verify `color`, `gray` and `text` requests.
4. Acquire at 300 dpi with `showUI:false` and the exact device ID. Confirm no scanner dialogs
   appear, and compare scan area, DPI and image fidelity with the interactive scan.
5. Cancel the settings dialog and an active PhotoCraft acquisition job. Confirm the job
   ends, the helper exits, no document is added and an immediate retry can scan.
6. Power off or disconnect the scanner; test enumeration and acquisition with the earlier ID.
   Keep an existing document open and verify it survives each error.
7. Request a resolution the driver rejects, then retry a supported resolution. Test busy-device
   handling with other scanner software using the device.
8. Check supported higher resolutions, film/slide modes and any driver-supplied 16-bit image or
   embedded ICC profile only after the basic platen workflow passes.

Validation status, 2026-10-12: connected Epson Perfection V500 discovery and a physical 300 dpi
colour scan succeeded through the current Windows scanner API: 2550 × 3510 pixels, 24-bit BMP,
26,858,574 bytes. The release desktop app completed acquisition and opened the scan as a new
unsaved document. The selected-area configuration also passed a no-transfer driver check after
avoiding redundant Epson property writes. The user also confirmed the rebuilt menu/settings
workflow physically scanned and opened its image: 850 × 1170 pixels at 100 dpi, RGB, 8-bit per
channel; the desktop job completed successfully with no import warnings. Scanner regression
tests (11), UI scanner tests (5), engine tests, 124 automation tests, formatting, layering and
wasm checks passed. Strict Clippy found existing `collapsible_match` warnings in untouched code;
the focused check passed with that lint excluded. The broader adversarial command test timed out
on printing; a parallel crop GPU test crashed and passed when retried serially. Native dialog
settings passed the menu test. Cancellation during physical transfer, retry and film/slide capabilities still need
hardware validation. Feeder/TWAIN acquisition is not implemented.

Upstream integration validation (main `d42f9da94`): all 19 focused scanner/UI/automation tests
passed, including profile-prompt admission and keeping a scan unsaved after the prompt. Full
engine (2,257) and automation (89) library tests passed. UI library tests passed 4,069 tests;
26 failed because the local font setup lacks CJK/Arabic glyphs, and 18 were ignored. Formatting,
layering, wasm and the adversarial command check passed. Strict Clippy found pre-existing
nonminimal-boolean, manual-range and collapsible-match findings in untouched code. The physical
transfer helper is unchanged from the V500-tested portable build; the upstream integration adds
stable document identity and the current queued colour-profile prompt flow.