//! The Win32 side: winspool (printers, driver settings) and GDI (page geometry, printing).
//! Every `unsafe` block says why it is sound. Buffers the system writes into are `u64`-backed so
//! the structures read from them are aligned.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, DEVMODEW, DIB_RGB_COLORS, DM_IN_BUFFER, DM_IN_PROMPT, DM_OUT_BUFFER, DeleteDC, GetDeviceCaps, HALFTONE,
    HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY, SetBrushOrgEx, SetStretchBltMode,
    StretchDIBits, VERTRES,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE,
    PRINTER_INFO_4W,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::ColorSystem::{ICM_OFF, SetICMMode};

use crate::{DevMode, Job, Page, PrintError, Printed, Printer, Result, RgbImage};

const IDOK: i32 = 1;
const IDCANCEL: i32 = 2;
/// Wide strings read from the system stop here at the latest.
const MAX_WCHARS: usize = 32 * 1024;

fn last_error(what: &str) -> PrintError {
    PrintError::System(format!("{what} failed: {}", std::io::Error::last_os_error()))
}

fn wide(s: &str) -> Result<Vec<u16>> {
    if s.contains('\0') {
        return Err(PrintError::Invalid("a name contains a NUL character".into()));
    }
    Ok(s.encode_utf16().chain(Some(0)).collect())
}

/// A zeroed, 8-byte aligned buffer of at least `bytes` bytes.
fn aligned(bytes: usize) -> Vec<u64> {
    vec![0u64; bytes.div_ceil(8).max(1)]
}

fn bytes_of(buf: &[u64], len: usize) -> Vec<u8> {
    buf.iter().flat_map(|w| w.to_le_bytes()).take(len).collect()
}

/// Reads a NUL-terminated wide string.
///
/// # Safety
/// `p` is null or points to a NUL-terminated UTF-16 string that stays alive for the call.
unsafe fn read_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    // SAFETY: the caller guarantees a NUL-terminated string; we stop at the NUL (or the cap).
    while n < MAX_WCHARS && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: the `n` units before the NUL were just read and are valid.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

pub fn printers() -> Result<Vec<Printer>> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    // SAFETY: a size query: no buffer (null, 0 bytes); the out-params are valid locals.
    unsafe { EnumPrintersW(flags, null(), 4, null_mut(), 0, &mut needed, &mut count) };
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buf = aligned(needed as usize);
    // SAFETY: `buf` holds at least `needed` writable bytes, aligned for PRINTER_INFO_4W.
    let ok = unsafe { EnumPrintersW(flags, null(), 4, buf.as_mut_ptr().cast(), needed, &mut needed, &mut count) };
    if ok == 0 {
        return Err(last_error("listing printers"));
    }
    let default = default_printer().ok().flatten();
    let infos = buf.as_ptr().cast::<PRINTER_INFO_4W>();
    let fits = (needed as usize) / std::mem::size_of::<PRINTER_INFO_4W>();
    let mut out = Vec::new();
    for i in 0..(count as usize).min(fits) {
        // SAFETY: the system wrote `count` PRINTER_INFO_4W at the start of `buf` (bounded by
        // the buffer size above); their name pointers point into `buf`, which is alive.
        let name = unsafe { read_wide((*infos.add(i)).pPrinterName) };
        if !name.is_empty() {
            out.push(Printer { default: default.as_deref() == Some(name.as_str()), name });
        }
    }
    Ok(out)
}

pub fn default_printer() -> Result<Option<String>> {
    let mut len = 0u32;
    // SAFETY: a size query with a null buffer; `len` is a valid local.
    unsafe { GetDefaultPrinterW(null_mut(), &mut len) };
    if len == 0 || len as usize > MAX_WCHARS {
        return Ok(None);
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` writable wide chars.
    if unsafe { GetDefaultPrinterW(buf.as_mut_ptr(), &mut len) } == 0 {
        return Ok(None);
    }
    // SAFETY: on success the buffer holds a NUL-terminated name.
    Ok(Some(unsafe { read_wide(buf.as_ptr()) }))
}

/// An open printer handle, closed on drop.
struct OpenPrinter(PRINTER_HANDLE);

impl OpenPrinter {
    fn open(name: &[u16]) -> Result<Self> {
        let mut h = PRINTER_HANDLE { Value: null_mut() };
        // SAFETY: `name` is NUL-terminated; `h` is a valid out-param; no defaults.
        if unsafe { OpenPrinterW(name.as_ptr(), &mut h, null()) } == 0 || h.Value.is_null() {
            return Err(last_error("opening the printer"));
        }
        Ok(OpenPrinter(h))
    }
}

impl Drop for OpenPrinter {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenPrinterW and is closed once.
        unsafe { ClosePrinter(self.0) };
    }
}

/// Settings the driver can take as input: `current`, copied aligned, when it is this printer's.
fn input_settings(printer: &str, current: Option<&DevMode>) -> Option<Vec<u64>> {
    let d = current?;
    let b = d.as_bytes();
    // dmDeviceName: the first 32 wide chars (31 + NUL), so long names are truncated.
    let device: Vec<u16> = b.get(..64)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|c| *c != 0).collect();
    let want: Vec<u16> = printer.encode_utf16().take(31).collect();
    if device != want {
        return None;
    }
    let mut buf = aligned(b.len());
    for (i, chunk) in b.chunks(8).enumerate() {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        if let Some(slot) = buf.get_mut(i) {
            *slot = u64::from_le_bytes(w);
        }
    }
    Some(buf)
}

pub fn properties(printer: &str, current: Option<&DevMode>, parent: Option<isize>, prompt: bool) -> Result<Option<DevMode>> {
    let name = wide(printer)?;
    let p = OpenPrinter::open(&name)?;
    let hwnd: HWND = parent.map_or(null_mut(), |h| h as HWND);
    // SAFETY: with fmode 0 DocumentPropertiesW only returns the size the settings need.
    let needed = unsafe { DocumentPropertiesW(hwnd, p.0, name.as_ptr(), null_mut(), null(), 0) };
    if needed <= 0 || needed as usize > (1 << 20) {
        return Err(last_error("reading the printer's settings"));
    }
    let mut out = aligned(needed as usize);
    let input = input_settings(printer, current);
    let mut mode = DM_OUT_BUFFER;
    if input.is_some() {
        mode |= DM_IN_BUFFER;
    }
    if prompt {
        mode |= DM_IN_PROMPT;
    }
    let inp: *const DEVMODEW = input.as_ref().map_or(null(), |b| b.as_ptr().cast());
    // SAFETY: `out` holds `needed` aligned writable bytes as the size query asked for; `inp` is
    // null or an aligned copy of settings written by this printer's driver.
    let r = unsafe { DocumentPropertiesW(hwnd, p.0, name.as_ptr(), out.as_mut_ptr().cast(), inp, mode) };
    match r {
        IDOK => {
            let all = bytes_of(&out, needed as usize);
            let word = |at: usize| all.get(at..at + 2).map_or(0, |w| usize::from(u16::from_le_bytes([w[0], w[1]])));
            let len = (word(68) + word(70)).min(all.len());
            DevMode::from_bytes(all.into_iter().take(len).collect()).map(Some)
        }
        IDCANCEL => Ok(None),
        _ => Err(last_error("the printer's settings dialog")),
    }
}

/// A printer device context, deleted on drop.
struct Dc(HDC);

impl Dc {
    fn create(printer: &str, settings: Option<&DevMode>) -> Result<Self> {
        let name = wide(printer)?;
        let dm = input_settings(printer, settings);
        let dmp: *const DEVMODEW = dm.as_ref().map_or(null(), |b| b.as_ptr().cast());
        // SAFETY: `name` is NUL-terminated; `dmp` is null or aligned settings from this
        // printer's driver, alive for the call.
        let hdc = unsafe { CreateDCW(null(), name.as_ptr(), null(), dmp) };
        if hdc.is_null() {
            return Err(last_error(&format!("opening printer “{printer}”")));
        }
        Ok(Dc(hdc))
    }

    fn cap(&self, index: u32) -> i32 {
        // SAFETY: a valid printer DC and a documented capability index.
        unsafe { GetDeviceCaps(self.0, index as i32) }
    }

    fn page(&self) -> Result<Page> {
        let (dx, dy) = (self.cap(LOGPIXELSX), self.cap(LOGPIXELSY));
        if dx <= 0 || dy <= 0 {
            return Err(PrintError::System("the printer reported no resolution".into()));
        }
        let (fx, fy) = (72.0 / f64::from(dx), 72.0 / f64::from(dy));
        let (pw, ph) = (self.cap(PHYSICALWIDTH), self.cap(PHYSICALHEIGHT));
        let (ox, oy) = (self.cap(PHYSICALOFFSETX), self.cap(PHYSICALOFFSETY));
        let (hw, vh) = (self.cap(HORZRES), self.cap(VERTRES));
        let (pw, ph) = if pw > 0 && ph > 0 { (pw, ph) } else { (hw, vh) };
        Ok(Page {
            paper: (f64::from(pw) * fx, f64::from(ph) * fy),
            printable: (f64::from(ox) * fx, f64::from(oy) * fy, f64::from(hw) * fx, f64::from(vh) * fy),
            dpi: (dx as u32, dy as u32),
        })
    }
}

impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: the DC came from CreateDCW and is deleted once.
        unsafe { DeleteDC(self.0) };
    }
}

pub fn page(printer: &str, settings: Option<&DevMode>) -> Result<Page> {
    Dc::create(printer, settings)?.page()
}

/// RGB rows top-down → a bottom-up 24-bit DIB (BGR, rows padded to 4 bytes).
fn dib(image: &RgbImage<'_>) -> Result<(BITMAPINFO, Vec<u8>)> {
    let (w, h) = (image.width as usize, image.height as usize);
    let stride = (w.checked_mul(3).ok_or_else(|| PrintError::Invalid("image too large".into()))? + 3) & !3;
    let total = stride.checked_mul(h).filter(|t| *t <= i32::MAX as usize).ok_or_else(|| PrintError::Invalid("image too large to print in one piece".into()))?;
    let mut bits = vec![0u8; total];
    for (y, src) in image.data.chunks_exact(w * 3).enumerate() {
        let start = (h - 1 - y) * stride;
        let Some(dst) = bits.get_mut(start..start + w * 3) else { continue };
        for (d, s) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<3>().0) {
            *d = [s[2], s[1], s[0]];
        }
    }
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: image.width as i32,
        biHeight: image.height as i32,
        biPlanes: 1,
        biBitCount: 24,
        biCompression: BI_RGB,
        biSizeImage: total as u32,
        ..Default::default()
    };
    Ok((BITMAPINFO { bmiHeader: header, bmiColors: [Default::default()] }, bits))
}

pub fn print(job: &Job<'_>, image: &RgbImage<'_>, rect: (f64, f64, f64, f64)) -> Result<Printed> {
    if image.width > i32::MAX as u32 || image.height > i32::MAX as u32 {
        return Err(PrintError::Invalid("image too large".into()));
    }
    let dc = Dc::create(job.printer, job.settings)?;
    let page = dc.page()?;
    let (x, y, w, h) = crate::device_rect(&page, rect);
    let (bmi, bits) = dib(image)?;
    let title = wide(job.title)?;
    let output = job.output_file.map(wide).transpose()?;
    let info = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: title.as_ptr(),
        lpszOutput: output.as_ref().map_or(null(), |o| o.as_ptr()),
        lpszDatatype: null(),
        fwType: 0,
    };
    // SAFETY: a valid printer DC. ICM off: the pixels are already in the printer's space
    // (or deliberately unmanaged), so GDI must not convert them again.
    unsafe { SetICMMode(dc.0, ICM_OFF) };
    // SAFETY: a valid DC; HALFTONE resampling needs the brush origin reset afterwards.
    unsafe {
        SetStretchBltMode(dc.0, HALFTONE);
        SetBrushOrgEx(dc.0, 0, 0, null_mut());
    }
    // SAFETY: `info` and the strings it points to live until EndDoc/AbortDoc below.
    if unsafe { StartDocW(dc.0, &info) } <= 0 {
        return Err(last_error("starting the print job"));
    }
    let fail = |what: &str| {
        let e = last_error(what);
        // SAFETY: the job was started on this DC.
        unsafe { AbortDoc(dc.0) };
        e
    };
    for _ in 0..job.copies.clamp(1, 999) {
        // SAFETY: inside a started document on a valid DC.
        if unsafe { StartPage(dc.0) } <= 0 {
            return Err(fail("starting a page"));
        }
        // SAFETY: `bits` is a bottom-up DIB exactly as `bmi` describes (24-bit, padded rows,
        // `biSizeImage` bytes); the source rectangle is the whole image.
        let lines = unsafe {
            StretchDIBits(dc.0, x, y, w, h, 0, 0, image.width as i32, image.height as i32, bits.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY)
        };
        if lines <= 0 {
            return Err(fail("sending the image to the printer"));
        }
        // SAFETY: closes the page started above.
        if unsafe { EndPage(dc.0) } <= 0 {
            return Err(fail("finishing a page"));
        }
    }
    // SAFETY: closes the document started above.
    if unsafe { EndDoc(dc.0) } <= 0 {
        return Err(last_error("finishing the print job"));
    }
    Ok(Printed { device_rect: (x, y, w, h), dpi: page.dpi })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_is_bottom_up_bgr_with_padded_rows() {
        // 2×2: red, green / blue, white.
        let data = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let (bmi, bits) = dib(&RgbImage { width: 2, height: 2, data: &data }).unwrap();
        assert_eq!(bmi.bmiHeader.biHeight, 2);
        assert_eq!(bits.len(), 16, "6 bytes per row padded to 8");
        // First stored row is the bottom one: blue, white.
        assert_eq!(&bits[0..6], &[255, 0, 0, 255, 255, 255]);
        assert_eq!(&bits[8..14], &[0, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn settings_from_another_printer_are_not_passed_to_the_driver() {
        let mut b = vec![0u8; 220];
        for (i, c) in "Printer A".encode_utf16().enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        b[68..70].copy_from_slice(&220u16.to_le_bytes());
        let d = DevMode::from_bytes(b).unwrap();
        assert!(input_settings("Printer A", Some(&d)).is_some());
        assert!(input_settings("Printer B", Some(&d)).is_none());
        assert!(input_settings("Printer A", None).is_none());
    }

    /// A real job through Windows' "Microsoft Print to PDF" driver, written to a file (needs that
    /// printer installed; run by hand).
    #[test]
    #[ignore]
    fn prints_through_a_real_driver_to_a_file() {
        let out = std::env::temp_dir().join(format!("pc-winprint-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let settings = crate::default_settings("Microsoft Print to PDF").unwrap();
        let (w, h) = (300u32, 200u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| if (i % w) < w / 2 { [220, 30, 30] } else { [30, 30, 220] }).collect();
        let out_s = out.to_string_lossy().into_owned();
        let job = Job { printer: "Microsoft Print to PDF", settings: Some(&settings), title: "PhotoCraft test", output_file: Some(&out_s), copies: 2 };
        let done = crate::print(&job, &RgbImage { width: w, height: h, data: &data }, (72.0, 72.0, 216.0, 144.0)).unwrap();
        assert_eq!(done.device_rect.2, done.dpi.0 as i32 * 3, "3 inches wide");
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF"), "the driver wrote a PDF");
        assert!(pdf.len() > 1000, "the page has content");
        let _ = std::fs::remove_file(&out);
    }

    /// Lists this machine's printers and their pages (needs real printers; run by hand).
    #[test]
    #[ignore]
    fn lists_real_printers() {
        for p in printers().unwrap() {
            println!("{}{} {:?}", p.name, if p.default { " (default)" } else { "" }, page(&p.name, None));
        }
    }
}
