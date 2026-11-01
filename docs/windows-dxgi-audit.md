# Windows startup DXGI binding audit

Reviewed on 2026-11-01 for the vendor-preflight fast path in PR #10000. This is a
source review of the bindings reachable from this probe, not a security review of
every API exposed by winsafe or a hardware certification.

## Exact input and scope

The published `winsafe 0.0.29` archive was downloaded from crates.io and verified
against `Cargo.lock`: SHA-256
`9ef0ffc427f045c0cc9ebffd6f4f91153dcd2a1547b5c3c29ee3f541e16e95c6`.
Its `.cargo_vcs_info.json` identifies upstream revision
[`71ed88c2a0d18b03ee452f6d4261f4c22f443483`](https://github.com/rodrigocfd/winsafe/tree/71ed88c2a0d18b03ee452f6d4261f4c22f443483).
The dependency remains pinned at `=0.0.29`; no source or version was replaced.

Enabling `dxgi` also enables `ole`, then the already-enabled `user` and `kernel`
features. The package declares no Cargo dependencies or build script. The probe
uses only `CreateDXGIFactory1`, `EnumAdapters1`, and `GetDesc1`, plus their COM
ownership, vtable, HRESULT and structure helpers. Unused DXGI device/swapchain
APIs are outside this call path.

## Binding findings

- [`CreateDXGIFactory1`](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/dxgi/funcs.rs)
  initializes an owned null `IDXGIFactory1`, passes the interface's documented IID
  and a writable output pointer, and returns the interface only on `S_OK`.
  Other HRESULTs propagate. The factory IID and `extern "system"` signature match
  [Microsoft's contract](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-createdxgifactory1).
- [`EnumAdapters1`](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/dxgi/iterators.rs)
  holds a borrow of its factory, starts at index zero, and owns each returned
  adapter. Only `DXGI_ERROR_NOT_FOUND` ends enumeration successfully. Every other
  error is yielded once and terminates the iterator. The index cannot overflow:
  `u32::MAX` is the exhausted sentinel, checked before incrementing. The inherited
  [`IDXGIFactory1` vtable](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/dxgi/vts.rs)
  places `EnumAdapters1` after the `IDXGIFactory` entries as required.
- [`GetDesc1`](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/dxgi/com_interfaces/idxgiadapter1.rs)
  passes a zero-initialized, writable `#[repr(C)]` descriptor and returns it only
  after `S_OK`. `VendorId` is the SDK's `u32`; the description is 128 UTF-16 code
  units, memory sizes are pointer-sized, and the LUID/flags layout follows
  [`DXGI_ADAPTER_DESC1`](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ns-dxgi-dxgi_adapter_desc1).
  A native unit regression checks its size, alignment and the offsets used by the
  probe against that ABI.
- The [`com_interface!` macro](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/macros/com.rs)
  releases each non-null owned COM pointer on drop. Factory and adapters remain
  local to the synchronous scan; early Intel detection and error returns release
  them too. No raw pointers escape into PhotoCraft and no interface is sent to
  another thread by this path.
- `HrRet::to_hrresult` accepts only `S_OK`. That matches the success contract of
  these three calls. The app propagates factory, enumeration and descriptor
  failures to the original wgpu probe; a partial scan never proves Intel absent.
  Existing regressions check exactly-once fallback, early termination, and
  preservation of the original capability-filtered Intel decision.

No new defect was found in these reachable wrappers. This conclusion relies on
the Windows API honoring its documented output-pointer and HRESULT contracts.

## DLL availability is different from a failed factory call

The [`extern_sys!` macro](https://github.com/rodrigocfd/winsafe/blob/71ed88c2a0d18b03ee452f6d4261f4c22f443483/src/macros/ffis.rs)
uses a normal import library. A missing `dxgi.dll` or missing exported function
can fail in the Windows loader before Rust runs; it cannot be converted into the
factory-error fallback. The app must not claim otherwise.

The pinned `wgpu-hal 30.0.1` archive was also verified against its `Cargo.lock`
checksum. Its [HDR source](https://github.com/gfx-rs/wgpu/blob/40f4a34ebaf56f9a046231f54125ad046239d3f3/wgpu-hal/src/auxil/dxgi/hdr.rs)
already calls the statically imported
`windows::Win32::Graphics::Dxgi::CreateDXGIFactory1`; this HDR path is shared by DX12 and Windows Vulkan. Its
separate DX12 factory helper loads DXGI dynamically, so inspecting only that
helper is insufficient to establish the executable's loader requirements.

The Windows startup workflow records the executable's imports using Visual
Studio `dumpbin`, smoke-runs `--version`, builds the desktop application and runs
all its unit tests at the requested head SHA. Set the optional, non-secret
repository Actions variable `PHOTOCRAFT_STARTUP_IMPORT_BASELINE` to a full commit
SHA to also build that revision and fail if the candidate adds a mandatory DXGI
DLL import. The PR's one-off comparison uses the pre-change benchmark baseline
`2fccdcba8ea273dae3d997569bc888ed1df93470`. Logs and import reports are uploaded
with the tested SHA; the comparison restores the original source checkout.

An ordinary hosted Windows runner does not simulate removal of Windows DLLs or
certify every Server Core/VM image. A successful comparison establishes whether
this PR adds a loader dependency, not that the baseline works without DXGI.
Factory-call failure, unsupported adapters and missing GPUs remain distinct
cases covered by the original fallback and its synthetic regressions. Physical
Intel/hybrid testing is still useful and remains outside the hosted runner's
hardware coverage. The recorded performance samples are unchanged.
