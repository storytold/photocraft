//! RGB and grayscale 8-bit preview drawing through a filter's Windows device context.
use std::ffi::c_void;
use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY, StretchDIBits};

// SAFETY: callers supply a valid pixel-map record for the duration of the callback.
unsafe fn read<T: Copy>(base: *const c_void, at: usize) -> T {
    unsafe { base.cast::<u8>().add(at).cast::<T>().read_unaligned() }
}

/// # Safety
/// Non-null records and their pixel data must remain readable for the callback, and `dc`
/// must identify a valid Windows drawing context owned by the calling plug-in.
pub unsafe extern "C" fn display(map: *const c_void, rect: *const c_void, row: i32, col: i32, dc: *mut c_void) -> i16 {
    if map.is_null() || rect.is_null() || dc.is_null() {
        return -1;
    }
    // SAFETY: filter callback contract provides the map and 32-bit source rectangle. Invalid
    // pointers can terminate only this disposable worker, never the editor process.
    let (top, left, bottom, right, mt, ml, mb, mr, mode, stride, step, plane, base) = unsafe {
        (
            read::<i32>(rect, 0),
            read::<i32>(rect, 4),
            read::<i32>(rect, 8),
            read::<i32>(rect, 12),
            read::<i32>(map, 4),
            read::<i32>(map, 8),
            read::<i32>(map, 12),
            read::<i32>(map, 16),
            read::<i32>(map, 20),
            read::<i32>(map, 24),
            read::<i32>(map, 28),
            read::<i32>(map, 32),
            read::<*const u8>(map, if cfg!(target_pointer_width = "64") { 40 } else { 36 }),
        )
    };
    let Some(w) = right.checked_sub(left) else {
        return -1;
    };
    let Some(h) = bottom.checked_sub(top) else {
        return -1;
    };
    if (mode != 1 && mode != 3)
        || w <= 0
        || h <= 0
        || w > 32767
        || h > 32767
        || top < mt
        || left < ml
        || bottom > mb
        || right > mr
        || stride <= 0
        || step <= 0
        || (mode == 3 && plane <= 0)
        || base.is_null()
    {
        return -1;
    }
    let need = (i64::from(bottom) - i64::from(mt) - 1)
        .checked_mul(i64::from(stride))
        .and_then(|a| (i64::from(right) - i64::from(ml) - 1).checked_mul(i64::from(step)).and_then(|b| a.checked_add(b)))
        .and_then(|n| n.checked_add(if mode == 3 { 2 * i64::from(plane) } else { 0 }))
        .and_then(|n| n.checked_add(1));
    let Some(need) = need else {
        return -1;
    };
    let pitch = ((w as usize) * 3 + 3) & !3;
    let size = (pitch as u64) * (h as u64);
    if need <= 0 || need > crate::wire::MAX_PIXELS as i64 || size > crate::wire::MAX_PIXELS as u64 {
        return -1;
    }
    // SAFETY: the validated map describes this accessible byte range by callback contract.
    let source = unsafe { std::slice::from_raw_parts(base, need as usize) };
    let mut bgr = Vec::new();
    if bgr.try_reserve_exact(size as usize).is_err() {
        return -1;
    }
    bgr.resize(size as usize, 0);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let pos = ((i64::from(top) - i64::from(mt)) as usize + y) * stride as usize + ((i64::from(left) - i64::from(ml)) as usize + x) * step as usize;
            let to = y * pitch + x * 3;
            for c in 0..3 {
                // Grayscale has only one sample; plane spacing has no meaning there.
                let channel = if mode == 3 { (2 - c) * plane as usize } else { 0 };
                let Some(v) = source.get(pos + channel) else {
                    return -1;
                };
                let Some(dst) = bgr.get_mut(to + c) else {
                    return -1;
                };
                *dst = *v;
            }
        }
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB,
            biSizeImage: size as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the HDC is supplied by the filter; image and header remain alive for this call.
    let n = unsafe { StretchDIBits(dc, col, row, w, h, 0, 0, w, h, bgr.as_ptr().cast(), &info, DIB_RGB_COLORS, SRCCOPY) };
    if n == 0 || n == -1 { -1 } else { 0 }
}
