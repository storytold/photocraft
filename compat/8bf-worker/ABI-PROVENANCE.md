# Independent Filter Foundry binary ABI observations

This records numeric interoperability observations from GPL binaries, not copied declarations. No SDK headers are included or used as implementation sources. See abi-observations.json and local disassembly for evidence.

| Field | x86 offset | x64 offset | Width/type |
|---|---:|---:|---|
| abortProc | 0x4 | 0x4 | pointer |
| progressProc | 0x8 | 0xc | pointer |
| parameters | 0xc | 0x14 | pointer |
| imageSize | 0x10 | 0x1c | v,h i16 |
| planes | 0x14 | 0x20 | i16 |
| filterRect | 0x16 | 0x22 | top,left,bottom,right i16 |
| maxSpace | 0x2c | 0x38 | i32 |
| inRect | 0x34 | 0x40 | four i16 |
| inLoPlane | 0x3c | 0x48 | i16 |
| inHiPlane | 0x3e | 0x4a | i16 |
| outRect | 0x40 | 0x4c | four i16 |
| outLoPlane | 0x48 | 0x54 | i16 |
| outHiPlane | 0x4a | 0x56 | i16 |
| inData | 0x4c | 0x58 | pointer |
| inRowBytes | 0x50 | 0x60 | i32 |
| outData | 0x54 | 0x64 | pointer |
| outRowBytes | 0x58 | 0x6c | i32 |
| maskRect | 0x60 | 0x74 | four i16 |
| hostSig | 0x78 | 0x90 | u32 |
| imageMode | 0x80 | 0x9c | i16 |
| platformData | 0xbc | 0xd8 | pointer |
| bufferProcs | 0xc0 | 0xe0 | pointer |
| displayPixels | 0xcc | 0xf8 | pointer |
| handleProcs | 0xd0 | 0x100 | pointer |
| advanceState | 0xe0 | 0x118 | pointer |
| propertyProcs | 0x140 | 0x188 | pointer |
| sSPBasic | 0x174 | 0x1d0 | pointer |
| depth | 0x17c | 0x1e0 | i32 |
| bigDocumentData | 0x190 | 0x1f8 | pointer |
| maxSpace64 | 0x1b0 | 0x228 | i64 |

PlatformData HWND and AboutRecord platform pointer are at offset zero. x86 entry and examined callbacks use cdecl; x64 uses Windows x64 ABI.

Canonical legacy-table version numbers, bufferSpace, maskData/maskRowBytes, and complete record size have not been independently established.

Resources and disassembly remain local research evidence. This does not establish broad commercial-filter compatibility.

## Reproduction sources and implementation boundary

The samples were Filter Foundry 1.7.0.25 from https://github.com/danielmarschall/filter_foundry/releases/tag/1.7.0.25 (GPL). The JSON records their hashes and machine-code evidence addresses. No Filter Foundry code is compiled, linked or redistributed by this project; the Rust implementation and test DLL are original. Only interoperable binary facts are recorded. The sample DLLs and disassembly remain local test/research inputs.

PSFilterPdn (https://github.com/0xC0000054/PSFilterPdn) and gimp-pspi (https://github.com/draekko/gimp-pspi) supplied architectural context. SDK-adapted ABI declarations discovered in these references were excluded. PhotoCraft requires no Adobe SDK, header, library, application or runtime. PiPL resource framing was independently checked against GPL sample resources and Nicholas Hayes's author-written MIT scanner implementation.

The version words offered by our legacy callback tables are compatibility choices, validated with the tested filters; they are not claimed as reconstructed canonical constants. The 4096-byte zero-filled record is a bounded compatibility envelope, not a claim about the complete interface size. Optional services remain absent. Unsupported or crashing native modules are reported to the editor as failed jobs.

Vicanek test downloads were used locally under their supplied usage license. They are not part of this repository and were not disassembled or redistributed.

## Grayscale preview and selector cleanup observations

Filter Foundry 1.7.0.25 tests grayscale image mode against numeric value `1`: x86 VA `0x40c88d` reads the image-mode field at offset `0x80`; x64 VA `0x180004ed8` reads offset `0x9c` and compares it to `1` at `0x180004edf`. Its independently authored `process.c` grayscale branch and `preview.c` pixel-map construction correlate the document mode with the preview mode. The preview uses the existing observed pixel-map layout with one grayscale sample per pixel. Filter Foundry supplies `planeBytes = 1`; accepting zero interplane spacing for a single-channel preview is a PhotoCraft implementation choice, not a claimed observation. Original offscreen GDI tests verify this choice and unchanged RGB channel order on both architectures.

The author-written selector control flow in [gimp-pspi](https://github.com/draekko/gimp-pspi/blob/c978ec574267cd82ad2b658e93fdf9d8b9d65fb7/src/pspi.c#L2300-L2346) and [PSFilterPdn](https://github.com/0xC0000054/PSFilterPdn/blob/69778b9b30fbd947cb0cac0ce5102d38b01f0d67/src/PSApi/LoadPsFilter.cs#L856-L944) calls Finish after successful Start and skips it when Start fails. PhotoCraft tracks the DLL result separately from host callback errors to preserve that sequence. The original fixture verifies Finish exactly once after successful Start, no Finish after Start failure/cancellation, and preservation of the original callback error if Finish also fails. No SDK declarations were used for this control-flow check.

## Grayscale/16-bit processing and watch property

Filter Foundry's author-written `process.c:106–131` and both GPL binaries establish Gray8 mode `1`, Gray16 mode `10`, RGB16 mode `11`, depth `16`, two little-endian bytes per sample, and a native maximum of `32768`. The x64 depth branch is at VA `0x1800049ec`, with the two-byte assignments at `0x180004a8c..4a96` and maximum assignments at `0x180004aa0..4aaa`; x86 VA `0x40ca5a..40caad` confirms the same. RGB16 mode comparisons appear at x86 `0x40af0d` and x64 `0x18000a6ca`; Gray16 at x86 `0x40af6c` and x64 `0x180004ee5`. Row strides remain byte counts. The Rust full-range/native conversions and preservation of unchanged original samples are original implementation choices verified by exhaustive identity tests and the conformance DLL.

The property-table pointer is at the recorded `propertyProcs` offset. Filter Foundry's author preview routine and x86 VA `0x40c62a..40c666` / x64 VA `0x18000928f..1800092d8` establish getter/setter slots at pointer offsets 1/2, signature `8BIM`, key `wtch`, index zero, and a pointer-sized simple value. The getter receives optional simple/complex output pointers; the setter receives the simple value and a complex handle. The preview saves, increments and restores the watch-suspension value. PhotoCraft provides that property only, with atomic storage and initialized failure outputs. Version/count words remain compatibility choices. The observed caller ignores the return, so its canonical declared width is not established; `0`/`-1` in EAX interoperates with either a 16-bit or 32-bit consumer. Original compiled tests verify both architectures, including a wide x64 simple value.

No complete modern Basic interface was established from these independent callers. Searches of other independently authored open-source filter routines added acquisition/release examples but not the remaining operations. SDK-adapted helper implementations were excluded. No partial Basic table or guessed property identifiers are advertised.

Filter Foundry's own `preview.c:86–89` assigns CMYK mode `4` to its one-channel Gray16/Gray32 preview. The x64 binary confirms this at VA `0x18000a6da..18000a6e6`. PhotoCraft continues to reject that unsupported preview map; successful Gray16 output must not be described as a working dialog preview.

## Legacy memory tables and unused mask rectangles (2026-10-15)

GIMP PSPI's author-written setup (`src/pspi.c:1718–1728`) supplies eight handle
callbacks, including a recovery hint and regular-handle disposal. Its author-written
implementations (`554–589`) establish a no-op recovery hint and disposal of owned
handles, foreign Windows global allocations, and indirect wrappers. Our original
Rust implementation uses checked ownership and Windows APIs; it does not copy
that implementation. Disposed owned pointer slots are retained until this one-job
worker exits, preventing address reuse from confusing duplicate disposal with a
new foreign allocation. The original conformance DLL checks the table envelope and
owned/direct/indirect disposal on both architectures.

Runtime page-guard observations show India Ink repeatedly reading only the buffer
table's two header words when offered five slots. With six slots and the last slot
null, it calls the five implemented functions. PSFilterPdn's author-written
`BufferSuite.CreateBufferProcsPointer` zero-initializes its table and populates only
those five functions. PhotoCraft therefore supplies a six-slot legacy envelope
with a null extension, without inventing a function signature. Version 2 remains
a tested compatibility choice. Diagnostic experiments with larger tables are not
included in the shipped worker.

The worker advertises no native selection mask. PSPI's independently authored
`advance_state_proc` processes input/output rectangles without rejecting an unused
mask rectangle. Runtime tracing showed India Ink populated that rectangle anyway;
our previous rejection left input pixels unavailable and the filter continued into
a null read. Ignoring unused mask requests fixes that path. The original fixture
checks mask-only completion, an explicit advance callback, and normal pixel
processing while an unused mask rectangle remains populated. The editor retains
responsibility for applying real selection coverage.

No proprietary plug-in code was disassembled or incorporated. Runtime diagnostics
recorded callback calls, record/table access offsets, and exception addresses only.
An unsolicited SDK mirror result surfaced during web search and was excluded as
an implementation source. No SDK material was added to the project.
