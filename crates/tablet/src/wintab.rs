//! The Wintab packet reader: the pen path when the driver's "Use Windows Ink" is off.
//!
//! Off, the Wacom driver serves the pen through the Wintab API instead (`wintab32.dll`, installed
//! by the driver: Tip-Feel'ed pressure at the sensor's full resolution) and keeps synthesizing
//! the mouse itself. This module synthesizes no mouse messages and applies no feel curve: it
//! opens a context on the window (`CXO_MESSAGES`), subclasses it for `WT_PACKET` and
//! `WT_PROXIMITY`, and reports each reading through the same callback as the other platforms —
//! pen in range (hovering or touching) is `Some`, back to mouse is `None`. The DLL is loaded
//! dynamically, so a machine without a Wintab driver fails [`Monitor::install`] cleanly. Tilt is
//! not read (packets would need `PK_ORIENTATION`).
//!
//! As in `appkit` and `xi`, all decoding (message parameters, pressure scale, eraser bits, hover
//! vs contact) is pure and unit-tested on every platform; the Windows glue is the gated `win`
//! module.

#![allow(unsafe_code)]

use crate::{Sample, Update};

/// The eraser end of the pen? The cursor's classic inverted-id bit, or its `CSR_TYPE` (Wacom tool
/// families: bit `0x02` with bit `0x08` = an inverted stylus — tip `0x822`, eraser `0x82a`).
/// `csr_type` 0 means the driver named none.
pub fn is_eraser(cursor: u32, csr_type: u32) -> bool {
    cursor & 0x80 != 0 || (csr_type & 0x0006 == 0x0002 && csr_type & 0x0008 != 0)
}

/// Whether this `WT_PROXIMITY` message reports the pen entering hover range: lParam's low word
/// (wParam is the context handle). Reading wParam made every message an enter, so the pen never
/// left and a mouse painted at zero pressure afterwards.
pub fn proximity_entering(lparam: usize) -> bool {
    lparam as u32 & 0xFFFF != 0
}

/// A `WT_PACKET` serial: wParam's full 32 bits. Truncating to 16 bits made every `WTPacket`
/// lookup fail after 65,535 packets.
pub fn packet_serial(wparam: usize) -> u32 {
    wparam as u32
}

/// Is this `WT_PACKET` ours? Its lParam is the context handle (other contexts share the window).
pub fn packet_for_context(lparam: usize, hctx: usize) -> bool {
    lparam == hctx
}

/// The pressure scale from an `AXIS` maximum (10-bit Wintab when the driver reports none).
pub fn pressure_scale(ax_max: i32) -> f32 {
    if ax_max > 0 { ax_max as f32 } else { 1023.0 }
}

/// A raw pressure 0..ax_max as 0..1 (total on its own: a driver's max may be 0 or NaN).
fn pressure_unit(pressure: u32, pressure_max: f32) -> f32 {
    if pressure_max > 0.0 && pressure_max.is_finite() { (pressure as f32 / pressure_max).clamp(0.0, 1.0) } else { 0.0 }
}

/// The pen state the `WT_PACKET`/`WT_PROXIMITY` stream drives; pure, so every transition is
/// unit-tested on every platform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PenStream {
    /// The pen is in hover range or in contact (from a `WT_PROXIMITY` enter until its leave).
    in_range: bool,
}

impl PenStream {
    /// One `WT_PACKET`: pressure > 0 is contact, pressure 0 in range is hover, anything else (a
    /// stale serial) leaves the sample alone.
    pub fn packet(&mut self, cursor: u32, csr_type: u32, pressure: u32, pressure_max: f32) -> Update {
        let eraser = is_eraser(cursor, csr_type);
        if pressure > 0 {
            self.in_range = true;
            return Update::Set(Some(Sample { pressure: pressure_unit(pressure, pressure_max), tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser }));
        }
        if self.in_range {
            return Update::Set(Some(Sample { pressure: 0.0, eraser, ..Sample::default() }));
        }
        Update::Keep
    }

    /// One `WT_PROXIMITY`: entering reports the hovering pen (its end is learned with the first
    /// packet); leaving ends any contact, which returns input to the mouse.
    pub fn proximity(&mut self, entering: bool) -> Update {
        self.in_range = entering;
        Update::Set(entering.then_some(Sample { pressure: 0.0, ..Sample::default() }))
    }
}

#[cfg(target_os = "windows")]
pub use win::Monitor;

/// The Windows glue: load `wintab32.dll`, open the context, dispatch messages. Real Wintab calls,
/// so `unsafe` (this crate is the workspace's one exception).
#[cfg(target_os = "windows")]
mod win {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    use windows::core::w;

    use super::*;
    use crate::{Error, deliver};

    // `wintab.h`: the default message base (`WT_PACKET`/`WT_PROXIMITY` are its offsets), the
    // default context to copy, the cursor category and its type entry, the device's pressure
    // axis, and the packet fields this reader asks for (in the wintab bit order).
    const WT_DEFBASE: u32 = 0x7FF0;
    const WT_PACKET: u32 = WT_DEFBASE;
    const WT_PROXIMITY: u32 = WT_DEFBASE + 5;
    const WTI_DEFCONTEXT: u32 = 3;
    const WTI_DEVICES: u32 = 100;
    const WTI_CURSORS: u32 = 200;
    const DVC_NPRESSURE: u32 = 15;
    const CSR_TYPE: u32 = 20;
    const PK_CURSOR: u32 = 0x0020;
    const PK_BUTTONS: u32 = 0x0040;
    const PK_X: u32 = 0x0080;
    const PK_Y: u32 = 0x0100;
    const PK_NORMAL_PRESSURE: u32 = 0x0400;
    const PKT_FIELDS: u32 = PK_CURSOR | PK_BUTTONS | PK_X | PK_Y | PK_NORMAL_PRESSURE;
    /// `CXO_MESSAGES`: want `WT_*` messages; deliberately without `CXO_SYSTEM` (the driver's own
    /// system context keeps moving the cursor and clicking, which is what the unchecked box
    /// promises).
    const CXO_MESSAGES: u32 = 0x0004;
    /// The subclass id (`'pcwt'`), and `WM_NCDESTROY`, the last message a window receives.
    const SUBCLASS_ID: usize = 0x7063_7774;
    const WM_NCDESTROY: u32 = 0x0082;

    /// A Wintab context handle.
    #[allow(clippy::upper_case_acronyms)]
    type HCTX = usize;

    /// The spec's exports on `wintab32.dll`.
    type WTInfoFn = unsafe extern "system" fn(u32, u32, *mut c_void) -> u32;
    type WTOpenFn = unsafe extern "system" fn(HWND, *mut LOGCONTEXT, i32) -> HCTX;
    type WTCloseFn = unsafe extern "system" fn(HCTX) -> i32;
    type WTPacketFn = unsafe extern "system" fn(HCTX, u32, *mut c_void) -> i32;

    /// `LOGCONTEXTW` (wintab.h): the context description read as the default and modified.
    #[allow(clippy::upper_case_acronyms)]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LOGCONTEXT {
        name: [u16; 40], // LCNAMELEN
        options: u32,
        status: u32,
        locks: u32,
        msg_base: u32,
        device: u32,
        pkt_rate: u32,
        pkt_data: u32,
        pkt_mode: u32,
        move_mask: u32,
        btn_dn_mask: u32,
        btn_up_mask: u32,
        in_org_x: i32,
        in_org_y: i32,
        in_org_z: i32,
        in_ext_x: i32,
        in_ext_y: i32,
        in_ext_z: i32,
        out_org_x: i32,
        out_org_y: i32,
        out_org_z: i32,
        out_ext_x: i32,
        out_ext_y: i32,
        out_ext_z: i32,
        sens_x: i32,
        sens_y: i32,
        sens_z: i32,
        sys_mode: i32,
        sys_org_x: i32,
        sys_org_y: i32,
        sys_ext_x: i32,
        sys_ext_y: i32,
        sys_sens_x: i32,
        sys_sens_y: i32,
    }

    /// `AXIS` (wintab.h): a device axis range (the pressure scale).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    #[allow(clippy::upper_case_acronyms)]
    struct AXIS {
        ax_min: i32,
        ax_max: i32,
        ax_units: u32,
        ax_resolution: u32,
    }

    /// One packet, the fields of `PKT_FIELDS` in bit order.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Packet {
        cursor: u32,
        buttons: u32,
        x: u32,
        y: u32,
        pressure: u32,
    }

    // The structs mirror wintab.h (LOGCONTEXTW 212 bytes, AXIS 16, the packet prefix 20); a
    // mismatch would silently misread driver memory, so it must not compile.
    const _: () = assert!(std::mem::size_of::<LOGCONTEXT>() == 212 && std::mem::size_of::<AXIS>() == 16 && std::mem::size_of::<Packet>() == 20);

    /// The proc's state: the callback, the calls and context, the pressure scale, and the pen
    /// state between messages. Only the window's thread touches it.
    struct Data {
        callback: Box<dyn Fn(Option<Sample>)>,
        wt_info: WTInfoFn,
        wtpacket: WTPacketFn,
        wt_close: WTCloseFn,
        hctx: HCTX,
        pressure_max: f32,
        /// The last packet's cursor index and its `CSR_TYPE` (queried once per cursor:
        /// `WT_PROXIMITY` names no cursor, so packets are where the types are learned).
        last_cursor: Cell<(u32, u32)>,
        stream: Cell<PenStream>,
        /// 1 once the proc freed `self` (`WM_NCDESTROY`); `Monitor::drop` must then not touch it.
        freed: *mut AtomicUsize,
    }

    /// The loaded `wintab32.dll` and the four exports this reader needs — all resolved before any
    /// context is opened, so a shim missing one leaks nothing.
    struct Dll {
        wt_info: WTInfoFn,
        wt_open: WTOpenFn,
        wtpacket: WTPacketFn,
        wt_close: WTCloseFn,
    }

    impl Dll {
        /// `LoadLibraryExW` with `LOAD_LIBRARY_SEARCH_SYSTEM32` searches only the real System32:
        /// the application, current and `PATH` directories (where another `wintab32.dll` would
        /// hijack this process, the #712/#817 class of bug) are never searched.
        fn load() -> Result<Self, Error> {
            // SAFETY: the system32-only search path keeps a foreign same-named DLL out of the
            // process; a machine without the driver errors cleanly.
            let wintab = unsafe { LoadLibraryExW(w!("wintab32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|_| Error::Unsupported("wintab32.dll is not installed (no Wintab driver)".into()))?;
            let far = |name: &str| {
                let nul: Vec<u8> = name.bytes().chain([0]).collect();
                // SAFETY: `nul` is a live null-terminated string for the duration of the call.
                unsafe { GetProcAddress(wintab, windows::core::PCSTR(nul.as_ptr())) }
                    .ok_or_else(|| Error::Platform(format!("{name} not found in wintab32.dll")))
            };
            // SAFETY: each `far` call returned a non-null address of an export of the loaded
            // module, and every Wintab export is an `extern "system"` C function of the spec's
            // shape; the casts below pin which signature each name must have.
            let wt_info: WTInfoFn = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, WTInfoFn>(far("WTInfoW")?) };
            // SAFETY: as above.
            let wt_open: WTOpenFn = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, WTOpenFn>(far("WTOpenW")?) };
            // SAFETY: as above.
            let wtpacket: WTPacketFn = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, WTPacketFn>(far("WTPacket")?) };
            // SAFETY: as above.
            let wt_close: WTCloseFn = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, WTCloseFn>(far("WTClose")?) };
            Ok(Self { wt_info, wt_open, wtpacket, wt_close })
        }

        /// Read one `WTInfo` entry (see [`wt_info_entry`]).
        fn entry<T: Copy>(&self, category: u32, index: u32) -> Option<T> {
            // SAFETY: `self.wt_info` is the loaded shim's `WTInfoW` (see `Dll::load`).
            unsafe { wt_info_entry(self.wt_info, category, index) }
        }
    }

    /// Read one `WTInfo` entry. The driver-reported size is queried with a null output and checked
    /// before it writes, so a driver built against a larger structure can never write past the
    /// buffer; its first `size_of::<T>()` bytes are then copied out. `None` when the entry is
    /// absent, smaller than `T` or absurdly large.
    ///
    /// # Safety
    /// `wt_info` must be the loaded shim's `WTInfoW` (the module stays loaded).
    unsafe fn wt_info_entry<T: Copy>(wt_info: WTInfoFn, category: u32, index: u32) -> Option<T> {
        /// Larger than any real entry (the spec's structs are <= 212 bytes).
        const MAX_ENTRY: usize = 4096;
        let need = std::mem::size_of::<T>();
        // SAFETY: a null output is the documented size query; nothing is written. `wt_info` is
        // the caller's loaded `WTInfoW` (the safety contract).
        let size = unsafe { wt_info(category, index, std::ptr::null_mut()) } as usize;
        if size < need || size > MAX_ENTRY {
            return None;
        }
        let mut buf = vec![0u8; size];
        // SAFETY: `buf` is exactly the size the API reported, so it writes at most `size` bytes.
        let wrote = unsafe { wt_info(category, index, buf.as_mut_ptr() as *mut c_void) } as usize;
        if wrote < need {
            return None;
        }
        // SAFETY: the first `need` bytes are the requested plain-data structure.
        Some(unsafe { std::ptr::read(buf.as_ptr() as *const T) })
    }

    /// Keeps the Wintab context and the subclass alive; dropping it closes the context and removes
    /// the subclass. The proc runs on the window's thread, so install on that thread.
    pub struct Monitor {
        hwnd: HWND,
        /// The `Box<Data>` the proc reads; freed in `Drop` after the subclass is removed.
        data: *mut Data,
        /// 1 once the proc freed `data`; outside `Data` so `Drop` can read it without touching
        /// freed memory.
        freed: *mut AtomicUsize,
    }

    impl Monitor {
        /// Open a Wintab context on the window (the eframe/winit window's `HWND`) and report pen
        /// samples through `callback`. Fails cleanly on a machine without a Wintab driver
        /// (`wintab32.dll` or its exports missing, no devices).
        pub fn install(hwnd: *mut c_void, callback: impl Fn(Option<Sample>) + 'static) -> Result<Self, Error> {
            let hwnd = HWND(hwnd);
            if hwnd.is_invalid() {
                return Err(Error::Unsupported("no window handle".into()));
            }
            let dll = Dll::load()?;
            // SAFETY: a plain API call; `0, 0, null` is the documented presence check.
            if unsafe { (dll.wt_info)(0, 0, std::ptr::null_mut()) } == 0 {
                return Err(Error::Unsupported("no Wintab devices (WTInfo reports none)".into()));
            }

            // The default context, tuned for this window: our packet fields as messages, without
            // the system-cursor option.
            let mut lc: LOGCONTEXT = dll.entry(WTI_DEFCONTEXT, 0).ok_or_else(|| Error::Platform("WTInfo WTI_DEFCONTEXT failed".into()))?;
            for (i, ch) in "Photocraft".encode_utf16().take(39).enumerate() {
                lc.name[i] = ch;
            }
            lc.options = CXO_MESSAGES;
            lc.msg_base = WT_DEFBASE;
            lc.pkt_data = PKT_FIELDS;
            lc.pkt_mode = 0;
            lc.move_mask = PK_X | PK_Y;
            // The driver's Tip-Feel'ed output range (0..ax_max).
            let pressure_max = pressure_scale(dll.entry::<AXIS>(WTI_DEVICES, DVC_NPRESSURE).map_or(0, |axis| axis.ax_max));

            // SAFETY: the name/fields above satisfy the spec; `hwnd` is the live window the
            // caller keeps alive; `lc` lives for the call; the context is closed in `Drop` (or at
            // `WM_NCDESTROY`).
            let hctx = unsafe { (dll.wt_open)(hwnd, &mut lc, 1) };
            if hctx == 0 {
                return Err(Error::Platform("WTOpen failed".into()));
            }
            // SAFETY: a fresh `Box<AtomicUsize>`; it lives until the proc or `Drop` frees it,
            // exactly once, and is outside `Data` so `Drop` can read it after `Data` is gone.
            let freed = Box::into_raw(Box::new(AtomicUsize::new(0)));
            let data = Box::into_raw(Box::new(Data {
                callback: Box::new(callback),
                wt_info: dll.wt_info,
                wtpacket: dll.wtpacket,
                wt_close: dll.wt_close,
                hctx,
                pressure_max,
                last_cursor: Cell::new((u32::MAX, 0)),
                stream: Cell::new(PenStream::default()),
                freed,
            }));
            // SAFETY: `hwnd` is live; `data` is a `Box<Data>` the proc reads until `Drop` removes
            // the subclass; `subclass_proc` is a plain `extern "system"` fn.
            if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, data as usize) }.as_bool() {
                // SAFETY: the subclass was not installed, so the proc will never run: the freshly
                // opened context and both boxes must not leak.
                unsafe {
                    (dll.wt_close)(hctx);
                    drop(Box::from_raw(data));
                    drop(Box::from_raw(freed));
                }
                return Err(Error::Platform("SetWindowSubclass failed".into()));
            }
            Ok(Monitor { hwnd, data, freed })
        }
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            // SAFETY: `self.freed` is a live `AtomicUsize` until this function frees it exactly
            // once; `Acquire` pairs with the proc's `Release` store at `WM_NCDESTROY`, and a
            // nonzero flag means the proc already freed `data` and closed the context.
            unsafe {
                if (*self.freed).load(Ordering::Acquire) != 0 {
                    drop(Box::from_raw(self.freed));
                    return;
                }
                // The subclass was installed on this thread, and `Drop` runs there too, so
                // removing it ends the callbacks; if the window is already destroyed (`!IsWindow`)
                // no message can reach the proc either. Only then is `data` safe to free. If
                // removal failed while the window still lives the proc may still run at
                // `WM_NCDESTROY`, so leak both rather than free memory a message could read.
                if RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID).as_bool() || !IsWindow(Some(self.hwnd)).as_bool() {
                    let data = Box::from_raw(self.data);
                    (data.wt_close)(data.hctx);
                    drop(Box::from_raw(self.freed));
                }
            }
        }
    }

    impl Data {
        /// The cursor's `CSR_TYPE`, cached per cursor index. 0 when the driver names none — or
        /// when a hostile cursor index makes `WTI_CURSORS + cursor` overflow its category.
        fn csr_type(&self, cursor: u32) -> u32 {
            let (last, mut csr_type) = self.last_cursor.get();
            if last != cursor {
                csr_type = 0;
                if let Some(index) = WTI_CURSORS.checked_add(cursor) {
                    // SAFETY: `self.wt_info` is the loaded shim's, valid for the process's life.
                    csr_type = unsafe { wt_info_entry::<u32>(self.wt_info, index, CSR_TYPE) }.unwrap_or(0);
                }
                self.last_cursor.set((cursor, csr_type));
            }
            csr_type
        }
    }

    /// The subclass proc: reports Wintab pen packets through the callback and passes every other
    /// message on to winit.
    unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, dwref: usize) -> LRESULT {
        if msg == WM_NCDESTROY {
            if dwref != 0 {
                // SAFETY: `dwref` is the `Box<Data>` `install` leaked; `WM_NCDESTROY` is the last
                // message the window receives, so the proc frees it exactly once here, storing
                // the flag `Monitor::drop` reads before it would free, and closes the context.
                unsafe {
                    let data = Box::from_raw(dwref as *mut Data);
                    (*data.freed).store(1, Ordering::Release);
                    (data.wt_close)(data.hctx);
                }
            }
            // SAFETY: plain pass-through; the chain must still see the destruction.
            return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        }
        if dwref != 0 && (msg == WT_PACKET || msg == WT_PROXIMITY) {
            // SAFETY: `dwref` is the `Box<Data>` `install` leaked; it stays alive until this proc
            // frees it at `WM_NCDESTROY` or `Drop` removes the subclass, and no message arrives
            // after either. The body is panic-contained: an unwind out of `extern "system"`
            // aborts the process, and these messages carry driver data.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let data = unsafe { &*(dwref as *const Data) };
                if msg == WT_PACKET && packet_for_context(lparam.0 as usize, data.hctx) {
                    let mut pkt = Packet::default();
                    // SAFETY: `data.hctx` is the open context, `packet_serial` the message's full
                    // serial, and `pkt` lives for the call with exactly the fields `lc.pkt_data`
                    // asked for. A failed read is ignored: one unread packet must not lift the
                    // pen mid-stroke.
                    if unsafe { (data.wtpacket)(data.hctx, packet_serial(wparam.0), &mut pkt as *mut Packet as *mut c_void) } != 0 {
                        let mut stream = data.stream.get();
                        let update = stream.packet(pkt.cursor, data.csr_type(pkt.cursor), pkt.pressure, data.pressure_max);
                        data.stream.set(stream);
                        if let Update::Set(sample) = update {
                            deliver(&*data.callback, sample);
                        }
                    }
                } else if msg == WT_PROXIMITY && wparam.0 == data.hctx {
                    // Here wParam is the context and lParam's low word the entering flag.
                    let mut stream = data.stream.get();
                    let update = stream.proximity(proximity_entering(lparam.0 as usize));
                    data.stream.set(stream);
                    if let Update::Set(sample) = update {
                        deliver(&*data.callback, sample);
                    }
                }
            }));
            return LRESULT(0);
        }
        // SAFETY: plain pass-through; winit's wndproc must see every message that isn't ours.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proximity_reads_the_low_word_of_lparam_not_wparam() {
        assert!(proximity_entering(1));
        assert!(proximity_entering(0x1234_0001));
        assert!(!proximity_entering(0));
        // The leave message's lParam is the context handle (nonzero high word, zero low word);
        // decoding wParam said "entering" for every message.
        assert!(!proximity_entering(0x0001_0000));
        assert!(!proximity_entering(usize::MAX << 16));
    }

    #[test]
    fn packet_serials_are_not_truncated() {
        assert_eq!(packet_serial(0), 0);
        assert_eq!(packet_serial(0x1_2345), 0x1_2345);
        assert_eq!(packet_serial(0x1_0000), 0x1_0000, "the 16-bit truncation aliased this to 0");
        assert_eq!(packet_serial(65535), 65535);
        assert_eq!(packet_serial(65536), 65536, "the packet that made every WTPacket fail");
        assert_eq!(packet_serial(usize::MAX), u32::MAX);
    }

    #[test]
    fn packets_are_filtered_by_context() {
        assert!(packet_for_context(0x1234, 0x1234));
        assert!(!packet_for_context(0, 0x1234), "another context's packet on the same window");
        assert!(!packet_for_context(0x1234, 0));
    }

    #[test]
    fn eraser_bits() {
        assert!(!is_eraser(0, 0x0802), "Wacom stylus");
        assert!(is_eraser(0, 0x082a), "grip pen's eraser end");
        assert!(!is_eraser(0, 0x0822), "grip pen's tip");
        assert!(!is_eraser(0, 0x0804), "puck");
        assert!(is_eraser(0x80, 0), "the classic inverted-id bit alone");
        assert!(!is_eraser(0, 0), "no type named: a tip");
    }

    #[test]
    fn pressure_maps_to_the_unit_range() {
        assert_eq!(pressure_unit(0, 1023.0), 0.0);
        assert_eq!(pressure_unit(1023, 1023.0), 1.0);
        assert_eq!(pressure_unit(511, 1023.0), 511.0 / 1023.0);
        assert_eq!(pressure_unit(1024, 1023.0), 1.0, "over-range clamps");
        assert_eq!(pressure_unit(65535, 65535.0), 1.0);
        assert_eq!(pressure_unit(5, 0.0), 0.0, "no scale reads as no pressure");
        assert_eq!(pressure_unit(5, f32::NAN), 0.0);
        assert_eq!(pressure_scale(2047), 2047.0);
        assert_eq!(pressure_scale(0), 1023.0);
        assert_eq!(pressure_scale(-1), 1023.0);
    }

    #[test]
    fn hover_contact_and_the_leave_that_returns_to_the_mouse() {
        let mut pen = PenStream::default();
        let hover = Update::Set(Some(Sample { pressure: 0.0, ..Sample::default() }));
        // A packet with no pen in range reports nothing.
        assert_eq!(pen.packet(0, 0, 0, 1023.0), Update::Keep);
        // Entering: hovering, no pressure yet; a hover packet keeps the pen alive.
        assert_eq!(pen.proximity(true), hover);
        assert_eq!(pen.packet(0, 0, 0, 1023.0), hover);
        // Contact: the pressure is the sample.
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        let s = contact(pen.packet(0, 0x0802, 512, 1024.0));
        assert_eq!((s.pressure, s.eraser), (0.5, false));
        let s = contact(pen.packet(0, 0x082a, 1024, 1024.0));
        assert!(s.eraser && s.pressure == 1.0, "{s:?}");
        // Leaving ends the pen (the mouse takes over): a stray zero-pressure packet after the
        // leave must stay silent, or the mouse would paint at the pen's zero.
        assert_eq!(pen.proximity(false), Update::Set(None));
        assert_eq!(pen.packet(0, 0x082a, 0, 1024.0), Update::Keep);
        // Re-entering reports hover again.
        assert_eq!(pen.proximity(true), hover);
    }
}
