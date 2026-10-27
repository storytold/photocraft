//! Original offscreen GDI tests of the native preview callback, on both Windows ABIs.
#![cfg(windows)]
#![allow(dead_code)]
#[path = "../src/pixels.rs"]
mod pixels;
#[path = "../src/preview.rs"]
mod preview;
#[path = "../src/record.rs"]
mod record;
#[path = "../../../crates/plugins/src/native_8bf/wire.rs"]
mod wire;

use std::{ffi::c_void, ptr};
use windows_sys::Win32::Graphics::Gdi::*;

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Canvas {
    fn new() -> Self {
        // SAFETY: test-owned offscreen bitmap; selected only into this memory context.
        unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            assert!(!dc.is_null());
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 6,
                    biHeight: -4,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            assert!(!bitmap.is_null() && !bits.is_null());
            bits.cast::<u8>().write_bytes(0, 6 * 4 * 4);
            let previous = SelectObject(dc, bitmap);
            Self { dc, bitmap, previous }
        }
    }
    fn pixel(&self, x: i32, y: i32) -> u32 {
        // SAFETY: this live context owns a 6x4 bitmap for the entire test.
        unsafe { GetPixel(self.dc, x, y) }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: restore original object before releasing our bitmap and context.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn map(bytes: &[u8], bounds: [i32; 4], mode: i32, row: i32, column: i32, plane: i32) -> record::Record {
    let mut map = record::Record::new();
    for (i, value) in bounds.into_iter().enumerate() {
        map.set32(4 + i * 4, value);
    }
    for (at, value) in [(20, mode), (24, row), (28, column), (32, plane)] {
        map.set32(at, value);
    }
    map.set_ptr(if cfg!(target_pointer_width = "64") { 40 } else { 36 }, bytes.as_ptr() as usize);
    map
}

#[test]
fn grayscale_preview_reads_one_sample_without_interplane_bytes() {
    let target = Canvas::new();
    let sample = [77];
    let bounds = [0i32, 0, 1, 1];
    let mut image = map(&sample, bounds, 1, 1, 1, 0);
    // SAFETY: map, bounds, one-byte source and GDI target remain live through display.
    assert_eq!(unsafe { preview::display(image.ptr(), bounds.as_ptr().cast::<c_void>(), 1, 2, target.dc) }, 0);
    assert_eq!(target.pixel(2, 1), 0x004d4d4d);
    assert_eq!(target.pixel(1, 1), 0);
    assert_eq!(target.pixel(3, 1), 0);
}

#[test]
fn grayscale_preview_honors_crop_origin_padded_rows_and_column_stride() {
    let target = Canvas::new();
    let source = [0, 99, 30, 99, 90, 99, 0, 99, 99, 99, 0, 99, 150, 99, 210, 99, 0, 99];
    let mut image = map(&source, [10, 20, 12, 24], 1, 10, 2, 1);
    let crop = [10i32, 21, 12, 23];
    // SAFETY: cropped samples and declared row/column offsets are within source.
    assert_eq!(unsafe { preview::display(image.ptr(), crop.as_ptr().cast(), 1, 2, target.dc) }, 0);
    assert_eq!(target.pixel(2, 1), 0x001e1e1e);
    assert_eq!(target.pixel(3, 1), 0x005a5a5a);
    assert_eq!(target.pixel(2, 2), 0x00969696);
    assert_eq!(target.pixel(3, 2), 0x00d2d2d2);
    assert_eq!(target.pixel(1, 1), 0);
    assert_eq!(target.pixel(4, 2), 0);
}

#[test]
fn rgb_preview_retains_planar_and_interleaved_channel_order() {
    let bounds = [0i32, 0, 1, 2];
    for (source, row, step, plane) in [([10, 20, 30, 40, 50, 60], 2, 1, 2), ([10, 30, 50, 20, 40, 60], 6, 3, 1)] {
        let target = Canvas::new();
        let mut image = map(&source, bounds, 3, row, step, plane);
        // SAFETY: both six-byte test layouts describe exactly two RGB pixels.
        assert_eq!(unsafe { preview::display(image.ptr(), bounds.as_ptr().cast(), 0, 0, target.dc) }, 0);
        assert_eq!(target.pixel(0, 0), 0x00321e0a);
        assert_eq!(target.pixel(1, 0), 0x003c2814);
    }
}
