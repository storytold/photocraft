//! COM implementation of `IThumbnailProvider` and `IInitializeWithStream`.
//!
//! Safety policy: nothing here may take down `explorer.exe` (or the `dllhost.exe` surrogate that
//! hosts thumbnail handlers). There is no `unwrap`, `expect` or `panic!`, every COM entry point
//! runs inside [`guard`], and every failure is reported as `E_FAIL`, so Explorer falls back to the
//! default icon.
//!
//! # Safety contract
//!
//! Everything here is `unsafe` because a COM server is, by definition, raw FFI: Windows calls into
//! this DLL through function pointers and hands it raw pointers. The code relies on four
//! invariants, which the functions below refer to by number:
//!
//! 1. **Layout.** [`Provider`] and `Factory` are `#[repr(C)]` with their vtable pointer(s) first,
//!    so a COM interface pointer is a pointer into the object, and `from_thumb` recovers the
//!    object from the second interface with `offset_of!`. The vtables are `static`, so they never
//!    dangle.
//! 2. **Lifetime.** An object is freed only when its reference count reaches zero, by the single
//!    `Box::from_raw` in `provider_release` / `factory_release`. `LIVE_COUNT` mirrors live objects
//!    so `DllCanUnloadNow` never lets the DLL unload under a live object.
//! 3. **Caller pointers.** Every pointer that Windows passes in (`this`, `riid`, `ppv`, `phbmp`,
//!    `stream`, ...) is checked for null before use, and every output pointer is set to null first,
//!    so a failing call never leaves the caller with a stale value. Windows guarantees that
//!    non-null interface pointers are valid for the duration of the call.
//! 4. **Resources.** Each GDI+ object, GDI `HBITMAP`, and COM stream acquired in a function is
//!    released on every path out of it; the one `HBITMAP` that is returned is owned by Explorer.
//!
//! The PSD parsing ([`crate::psd`]) is safe code and is the only code that looks at untrusted bytes.
#![allow(unsafe_code, non_snake_case, unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::mem::{offset_of, zeroed};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{addr_of_mut, null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK,
};
use windows_sys::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromScan0, GdipCreateBitmapFromStream, GdipCreateHBITMAPFromBitmap, GdipDeleteGraphics, GdipDisposeImage, GdipDrawImageRectI,
    GdipGetImageGraphicsContext, GdipGetImageHeight, GdipGetImageWidth, GdipSetInterpolationMode, GdipSetPixelOffsetMode, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GpBitmap, InterpolationModeHighQualityBicubic, PixelOffsetModeHalf,
};
use windows_sys::Win32::UI::Shell::SHCreateMemStream;
use windows_sys::core::{GUID, HRESULT};

use crate::psd;
use crate::registration::{self};

/// CLSID of the PhotoCraft PSD thumbnail provider (same value as `registration::CLSID_STR`).
const CLSID: GUID = GUID::from_u128(0x8D6B2D5C_3F41_4B7A_9C0E_5A17E2B94D63);

/// Never read more than this from the start of a file: its image resources have to fit.
const MAX_PREFIX_BYTES: usize = 16 * 1024 * 1024;
/// The first read, enough for the header of nearly every file and often the whole resources.
const FIRST_READ_BYTES: usize = 4096;
/// `HRESULT_FROM_WIN32(ERROR_ALREADY_INITIALIZED)`, as `IInitializeWithStream` specifies.
const E_ALREADY_INITIALIZED: HRESULT = 0x800700B7u32 as HRESULT;
/// How much larger than the embedded preview an enlargement may be; beyond this it only blurs.
const MAX_ENLARGE: u32 = 4;
/// Most pixels an enlarged bitmap may have (16 MiB at 32 bpp), whatever size Explorer asks for.
const MAX_ENLARGED_PIXELS: u64 = 2048 * 2048;
/// GDI+ `PixelFormat32bppPARGB`.
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;
/// `WTS_ALPHATYPE`: the bitmap has no meaningful alpha channel.
const WTSAT_RGB: i32 = 1;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
const IID_ICLASSFACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_C000_000000000046);
const IID_IINITIALIZE_WITH_STREAM: GUID = GUID::from_u128(0xB824B49D_22AC_4161_AC8A_9916E8FA3F7F);
const IID_ITHUMBNAIL_PROVIDER: GUID = GUID::from_u128(0xE357FCCD_A995_4576_B01F_234630154E96);

/// Live objects plus server locks; backs `DllCanUnloadNow`.
static LIVE_COUNT: AtomicI32 = AtomicI32::new(0);

/// Compares two GUIDs field by field.
fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// Runs `f`, turning a panic into `E_FAIL` instead of unwinding into COM callers.
fn guard(f: impl FnOnce() -> HRESULT) -> HRESULT {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(E_FAIL)
}

// ---------------------------------------------------------------- vtables

#[repr(C)]
struct IUnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// Only the `Read` slot of `IStream` (inherited from `ISequentialStream`) is needed.
#[repr(C)]
struct IStreamVtbl {
    base: IUnknownVtbl,
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> HRESULT,
}

#[repr(C)]
struct InitializeWithStreamVtbl {
    base: IUnknownVtbl,
    initialize: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> HRESULT,
}

#[repr(C)]
struct ThumbnailProviderVtbl {
    base: IUnknownVtbl,
    get_thumbnail: unsafe extern "system" fn(*mut c_void, u32, *mut HBITMAP, *mut i32) -> HRESULT,
}

#[repr(C)]
struct ClassFactoryVtbl {
    base: IUnknownVtbl,
    create_instance: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    lock_server: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
}

static INIT_VTBL: InitializeWithStreamVtbl =
    InitializeWithStreamVtbl { base: IUnknownVtbl { query_interface: init_query_interface, add_ref: init_add_ref, release: init_release }, initialize };

static THUMB_VTBL: ThumbnailProviderVtbl =
    ThumbnailProviderVtbl { base: IUnknownVtbl { query_interface: thumb_query_interface, add_ref: thumb_add_ref, release: thumb_release }, get_thumbnail };

static FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    base: IUnknownVtbl { query_interface: factory_query_interface, add_ref: factory_add_ref, release: factory_release },
    create_instance: factory_create_instance,
    lock_server: factory_lock_server,
};

// --------------------------------------------------------------- provider

/// One COM object exposing two interfaces, each with its own vtable pointer.
#[repr(C)]
struct Provider {
    init_vtbl: *const InitializeWithStreamVtbl,
    thumb_vtbl: *const ThumbnailProviderVtbl,
    refs: AtomicU32,
    /// Start of the PSD file, filled by `Initialize`.
    data: Mutex<Option<Vec<u8>>>,
}

impl Provider {
    /// A new provider with a reference count of one.
    fn create() -> *mut Provider {
        LIVE_COUNT.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(Box::new(Provider { init_vtbl: &INIT_VTBL, thumb_vtbl: &THUMB_VTBL, refs: AtomicU32::new(1), data: Mutex::new(None) }))
    }
}

/// The object behind an `IThumbnailProvider` interface pointer.
unsafe fn from_thumb(this: *mut c_void) -> *mut Provider {
    // SAFETY (invariant 1): `this` points at the `thumb_vtbl` field of a `Provider`.
    (this as *mut u8).sub(offset_of!(Provider, thumb_vtbl)) as *mut Provider
}

/// Shared `QueryInterface`: hands out the `IUnknown`, `IInitializeWithStream` or `IThumbnailProvider` vtable.
unsafe fn provider_query(p: *mut Provider, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if p.is_null() || riid.is_null() {
        return E_POINTER;
    }
    let riid = &*riid;
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_IINITIALIZE_WITH_STREAM) {
        *ppv = addr_of_mut!((*p).init_vtbl).cast();
    } else if guid_eq(riid, &IID_ITHUMBNAIL_PROVIDER) {
        *ppv = addr_of_mut!((*p).thumb_vtbl).cast();
    } else {
        return E_NOINTERFACE;
    }
    (*p).refs.fetch_add(1, Ordering::SeqCst);
    S_OK
}

/// Increments the provider's reference count and returns the new count.
unsafe fn provider_add_ref(p: *mut Provider) -> u32 {
    (*p).refs.fetch_add(1, Ordering::SeqCst) + 1
}

/// Decrements the reference count and frees the provider when it reaches zero.
unsafe fn provider_release(p: *mut Provider) -> u32 {
    let remaining = (*p).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if remaining == 0 {
        // SAFETY (invariant 2): the last reference is gone, so nothing else uses `p`; it came
        // from `Box::into_raw` in `Provider::create`.
        drop(Box::from_raw(p));
        LIVE_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
    remaining
}

/// `IInitializeWithStream::QueryInterface`.
unsafe extern "system" fn init_query_interface(this: *mut c_void, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    guard(|| provider_query(this as *mut Provider, riid, ppv))
}
/// `IInitializeWithStream::AddRef`.
unsafe extern "system" fn init_add_ref(this: *mut c_void) -> u32 {
    provider_add_ref(this as *mut Provider)
}
/// `IInitializeWithStream::Release`.
unsafe extern "system" fn init_release(this: *mut c_void) -> u32 {
    provider_release(this as *mut Provider)
}
/// `IThumbnailProvider::QueryInterface`.
unsafe extern "system" fn thumb_query_interface(this: *mut c_void, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    guard(|| provider_query(from_thumb(this), riid, ppv))
}
/// `IThumbnailProvider::AddRef`.
unsafe extern "system" fn thumb_add_ref(this: *mut c_void) -> u32 {
    provider_add_ref(from_thumb(this))
}
/// `IThumbnailProvider::Release`.
unsafe extern "system" fn thumb_release(this: *mut c_void) -> u32 {
    provider_release(from_thumb(this))
}

/// `IInitializeWithStream::Initialize`: buffers the start of the file.
unsafe extern "system" fn initialize(this: *mut c_void, stream: *mut c_void, _mode: u32) -> HRESULT {
    guard(|| {
        let provider = this as *mut Provider;
        if provider.is_null() || stream.is_null() {
            return E_POINTER;
        }
        // A repeated call is refused before any I/O, as `IInitializeWithStream` specifies.
        match (*provider).data.lock() {
            Ok(slot) if slot.is_some() => return E_ALREADY_INITIALIZED,
            Ok(_) => {}
            Err(_) => return E_FAIL,
        }
        let Some(bytes) = read_prefix(stream) else {
            return E_FAIL;
        };
        match (*provider).data.lock() {
            Ok(mut slot) if slot.is_none() => {
                *slot = Some(bytes);
                S_OK
            }
            Ok(_) => E_ALREADY_INITIALIZED,
            Err(_) => E_FAIL,
        }
    })
}

/// Reads the start of `stream` that holds the PSD's image resources: only as much as the file's own
/// header says they occupy (never more than [`MAX_PREFIX_BYTES`]), so a multi-gigabyte document is
/// not read through to show a 160 px preview. `None` when it is not a PSD/PSB.
unsafe fn read_prefix(stream: *mut c_void) -> Option<Vec<u8>> {
    // SAFETY (invariant 3): `stream` is a non-null `IStream` handed over by Explorer, whose first
    // field is its vtable pointer; `Read` is called with a buffer at least `step` bytes long.
    let vtbl = *(stream as *mut *const IStreamVtbl);
    if vtbl.is_null() {
        return None;
    }
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut want = FIRST_READ_BYTES;
    while out.len() < want {
        let step = (want - out.len()).min(chunk.len());
        let mut read = 0u32;
        let hr = ((*vtbl).read)(stream, chunk.as_mut_ptr().cast(), step as u32, &mut read);
        if hr < 0 {
            return None;
        }
        if read == 0 {
            break; // End of stream.
        }
        out.extend_from_slice(chunk.get(..(read as usize).min(step))?);
        if out.len() >= want {
            // Enough to see the header: now the exact amount the image resources need.
            let (psd::Extent::Known(end) | psd::Extent::NeedAtLeast(end)) =
                if out.starts_with(b"PK\x03\x04") { crate::pcraft::thumbnail_extent(&out)? } else { psd::resources_extent(&out)? };
            if end > MAX_PREFIX_BYTES {
                return None;
            }
            want = end;
        }
    }
    Some(out)
}

/// `IThumbnailProvider::GetThumbnail`: the embedded JFIF as an `HBITMAP` that Explorer owns.
unsafe extern "system" fn get_thumbnail(this: *mut c_void, cx: u32, phbmp: *mut HBITMAP, pdw_alpha: *mut i32) -> HRESULT {
    guard(|| {
        if this.is_null() || phbmp.is_null() || pdw_alpha.is_null() {
            return E_POINTER;
        }
        *phbmp = null_mut();
        let provider = &*from_thumb(this);
        let Ok(data) = provider.data.lock() else {
            return E_FAIL;
        };
        let Some(data) = data.as_deref() else {
            return E_UNEXPECTED; // `Initialize` was not called.
        };
        match psd::find_thumbnail_jpeg(data).or_else(|| crate::pcraft::find_thumbnail_png(data)).and_then(|image| jpeg_to_hbitmap(image, cx)) {
            Some(hbmp) => {
                *phbmp = hbmp;
                *pdw_alpha = WTSAT_RGB;
                S_OK
            }
            None => E_FAIL,
        }
    })
}

/// Redraws `bitmap` with its longest side at `cx` pixels (at most [`MAX_ENLARGE`] times the
/// original) using bicubic interpolation, within [`MAX_ENLARGED_PIXELS`]. Returns null when no enlargement is needed or possible,
/// in which case the caller uses the original; a non-null result is owned by the caller.
unsafe fn enlarge(bitmap: *mut GpBitmap, cx: u32) -> *mut GpBitmap {
    let (mut width, mut height) = (0u32, 0u32);
    if GdipGetImageWidth(bitmap.cast(), &mut width) != 0 || GdipGetImageHeight(bitmap.cast(), &mut height) != 0 {
        return null_mut();
    }
    let longest = width.max(height);
    let target = cx.min(longest.saturating_mul(MAX_ENLARGE));
    if longest == 0 || target <= longest {
        return null_mut();
    }
    let scale = |side: u32| ((u64::from(side) * u64::from(target)) / u64::from(longest)).max(1);
    if scale(width) * scale(height) > MAX_ENLARGED_PIXELS {
        return null_mut();
    }
    let (Ok(new_width), Ok(new_height)) = (i32::try_from(scale(width)), i32::try_from(scale(height))) else {
        return null_mut();
    };

    let mut result: *mut GpBitmap = null_mut();
    if GdipCreateBitmapFromScan0(new_width, new_height, 0, PIXEL_FORMAT_32BPP_PARGB, null(), &mut result) != 0 || result.is_null() {
        return null_mut();
    }
    let mut graphics = null_mut();
    let mut drawn = GdipGetImageGraphicsContext(result.cast(), &mut graphics) == 0 && !graphics.is_null();
    if drawn {
        // `Half` keeps the border pixels from fading out.
        drawn = GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic) == 0
            && GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf) == 0
            && GdipDrawImageRectI(graphics, bitmap.cast(), 0, 0, new_width, new_height) == 0;
    }
    if !graphics.is_null() {
        GdipDeleteGraphics(graphics);
    }
    if drawn {
        return result;
    }
    GdipDisposeImage(result.cast());
    null_mut()
}

/// Decodes a JPEG with GDI+, which ships with Windows, and enlarges it towards `cx` pixels when
/// it is smaller. The caller owns the returned `HBITMAP`.
// Invariant 4: `stream`, the GDI+ bitmaps, the GDI+ token and (on failure) `hbmp` are all released
// below on every path; only a successful `hbmp` leaves this function.
unsafe fn jpeg_to_hbitmap(jpeg: &[u8], cx: u32) -> Option<HBITMAP> {
    let stream = SHCreateMemStream(jpeg.as_ptr(), u32::try_from(jpeg.len()).ok()?);
    if stream.is_null() {
        return None;
    }

    let mut input: GdiplusStartupInput = zeroed();
    input.GdiplusVersion = 1;
    let mut token = 0usize;
    if GdiplusStartup(&mut token, &input, null_mut()) != 0 {
        release_stream(stream);
        return None;
    }

    let mut bitmap = null_mut();
    let mut hbmp: HBITMAP = null_mut();
    let mut ok = GdipCreateBitmapFromStream(stream, &mut bitmap) == 0 && !bitmap.is_null();
    let mut enlarged = null_mut();
    if ok {
        // Opaque white background; a JPEG has no alpha anyway. A preview smaller than the size
        // Explorer asks for is enlarged with a better filter than Explorer's own scaling.
        enlarged = enlarge(bitmap, cx);
        let source = if enlarged.is_null() { bitmap } else { enlarged };
        ok = GdipCreateHBITMAPFromBitmap(source, &mut hbmp, 0xFFFF_FFFF) == 0 && !hbmp.is_null();
    }
    if !enlarged.is_null() {
        GdipDisposeImage(enlarged.cast());
    }
    if !bitmap.is_null() {
        GdipDisposeImage(bitmap.cast());
    }
    GdiplusShutdown(token);
    release_stream(stream);
    if ok {
        return Some(hbmp);
    }
    if !hbmp.is_null() {
        DeleteObject(hbmp);
    }
    None
}

/// Releases the in-memory `IStream` that `jpeg_to_hbitmap` created for GDI+.
unsafe fn release_stream(stream: *mut c_void) {
    // SAFETY (invariants 3 and 4): `stream` is the non-null stream from `SHCreateMemStream`, still
    // holding the reference taken there; this releases it exactly once.
    let vtbl = *(stream as *mut *const IUnknownVtbl);
    if !vtbl.is_null() {
        ((*vtbl).release)(stream);
    }
}

// ---------------------------------------------------------- class factory

#[repr(C)]
struct Factory {
    vtbl: *const ClassFactoryVtbl,
    refs: AtomicU32,
}

/// `IClassFactory::QueryInterface`.
unsafe extern "system" fn factory_query_interface(this: *mut c_void, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if ppv.is_null() || riid.is_null() {
            return E_POINTER;
        }
        *ppv = null_mut();
        if guid_eq(&*riid, &IID_IUNKNOWN) || guid_eq(&*riid, &IID_ICLASSFACTORY) {
            factory_add_ref(this);
            *ppv = this;
            S_OK
        } else {
            E_NOINTERFACE
        }
    })
}

/// `IClassFactory::AddRef`.
unsafe extern "system" fn factory_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut Factory)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

/// `IClassFactory::Release`: frees the factory when the last reference goes.
unsafe extern "system" fn factory_release(this: *mut c_void) -> u32 {
    let factory = this as *mut Factory;
    let remaining = (*factory).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if remaining == 0 {
        // SAFETY (invariant 2): last reference; created by `Box::into_raw` in `DllGetClassObject`.
        drop(Box::from_raw(factory));
        LIVE_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
    remaining
}

/// `IClassFactory::CreateInstance`: creates a provider and returns the requested interface.
unsafe extern "system" fn factory_create_instance(_this: *mut c_void, outer: *mut c_void, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if ppv.is_null() {
            return E_POINTER;
        }
        *ppv = null_mut();
        if !outer.is_null() {
            return CLASS_E_NOAGGREGATION;
        }
        let provider = Provider::create();
        if provider.is_null() {
            return E_OUTOFMEMORY;
        }
        let hr = provider_query(provider, riid, ppv);
        provider_release(provider); // Drops the creation reference.
        hr
    })
}

/// `IClassFactory::LockServer`: keeps the DLL loaded while locked.
unsafe extern "system" fn factory_lock_server(_this: *mut c_void, lock: i32) -> HRESULT {
    LIVE_COUNT.fetch_add(if lock != 0 { 1 } else { -1 }, Ordering::SeqCst);
    S_OK
}

// ------------------------------------------------------------ DLL exports

/// COM entry point: hands out the class factory for [`CLSID`].
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return E_POINTER;
        }
        *ppv = null_mut();
        if !guid_eq(&*rclsid, &CLSID) {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        LIVE_COUNT.fetch_add(1, Ordering::SeqCst);
        let factory = Box::into_raw(Box::new(Factory { vtbl: &FACTORY_VTBL, refs: AtomicU32::new(1) }));
        let hr = factory_query_interface(factory.cast(), riid, ppv);
        factory_release(factory.cast());
        hr
    })
}

/// COM entry point: whether Windows may unload the DLL.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE_COUNT.load(Ordering::SeqCst) <= 0 { S_OK } else { S_FALSE }
}

/// `regsvr32` entry point: registers the provider for the current user.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    guard(|| match registration::current_module_path() {
        Some(path) if registration::register(&path, true).is_ok() => S_OK,
        _ => E_FAIL,
    })
}

/// `regsvr32 /u` entry point: removes the registration.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(|| match registration::unregister() {
        Ok(()) => S_OK,
        Err(_) => E_FAIL,
    })
}
