//! The Wintab packet reader: the pen path when the driver's "Use Windows Ink" is off.
//!
//! Off, the Wacom driver serves the pen through the Wintab API instead (`wintab32.dll`, installed
//! by the driver: Tip-Feel'ed pressure at the sensor's full resolution) and keeps synthesizing
//! the mouse itself. This module synthesizes no mouse messages and applies no feel curve: it
//! opens a context on the window (`CXO_MESSAGES | CXO_SYSTEM`), subclasses it for `WT_PACKET` and
//! `WT_PROXIMITY`, and reports each reading through the same callback as the other platforms —
//! pen in range (hovering or touching) is `Some`, back to mouse is `None`. The DLL is loaded
//! dynamically, so a machine without a Wintab driver fails [`Monitor::install`] cleanly. Packets
//! always ask for pressure, plus `PK_ORIENTATION` when the context device's `DVC_ORIENTATION`
//! axes say it has tilt or twist — a plain pen gets a pressure-only context rather than one
//! `WTOpen` refuses. Where reported, a tilt-capable pen's azimuth/altitude become the sample's
//! `tilt_x`/`tilt_y` and its barrel twist the `rotation`; the move mask carries pressure and
//! orientation too, so stationary changes still deliver.
//!
//! As in `appkit` and `xi`, all decoding (message parameters, pressure scale, eraser bits, hover
//! vs contact) is pure and unit-tested on every platform; the Windows glue is the gated `win`
//! module.

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

/// `WTI_DEVICES` (wintab.h): the base `WTInfo` category of the device records; each device has
/// its own ([`CATEGORY_LIMIT`] slots: 100..200, then `WTI_CURSORS`).
const WTI_DEVICES: u32 = 100;
/// `DVC_NPRESSURE` (wintab.h): the normal (tip) pressure `AXIS` entry. Only the Windows glue
/// queries it (the tests fake that query), so it exists exactly where something uses it: ungated,
/// it is dead code on every other platform (and warnings are denied).
#[cfg(any(target_os = "windows", test))]
const DVC_NPRESSURE: u32 = 15;
/// `DVC_ORIENTATION` (wintab.h): the `AXIS[3]` orientation entry (azimuth, altitude, twist). Gated
/// like [`DVC_NPRESSURE`].
#[cfg(any(target_os = "windows", test))]
const DVC_ORIENTATION: u32 = 17;

/// `WTI_CURSORS` (wintab.h): the base `WTInfo` category of the cursors. Each category spans
/// [`CATEGORY_LIMIT`] slots (the cursors' 200..300; 300, `WTI_EXTENSIONS`, starts the next).
const WTI_CURSORS: u32 = 200;
/// How many slots one `WTInfo` category holds (the bases are 100 apart).
const CATEGORY_LIMIT: u32 = 100;

/// The `WTInfo` category of a packet cursor's `CSR_TYPE` entry: `WTI_CURSORS` plus the cursor
/// index, or `None` when the index leaves the cursor category ([`CATEGORY_LIMIT`] slots) — a packet
/// field must never select another category's memory.
pub fn cursor_type_entry(cursor: u32) -> Option<u32> {
    if cursor < CATEGORY_LIMIT { Some(WTI_CURSORS + cursor) } else { None }
}

/// The `WTInfo` category of the context's device (`lcDevice`): `WTI_DEVICES` plus the index,
/// bounded to the device category ([`CATEGORY_LIMIT`] slots — a query must never read another
/// category). An out-of-range index falls back to device 0's entries: Wacom's `WTI_DEFCONTEXT`
/// carries `lcDevice = 0xFFFFFFFF` for "the default device", and reading nothing for it dropped
/// orientation and mis-scaled pressure with the 10-bit fallback.
pub fn device_category(device: u32) -> u32 {
    WTI_DEVICES + if device < CATEGORY_LIMIT { device } else { 0 }
}

/// The pressure scale from an `AXIS` maximum (10-bit Wintab when the driver reports none).
pub fn pressure_scale(ax_max: i32) -> f32 {
    if ax_max > 0 { ax_max as f32 } else { 1023.0 }
}

/// A raw pressure 0..ax_max as 0..1 (total on its own: a driver's max may be 0 or NaN).
fn pressure_unit(pressure: u32, pressure_max: f32) -> f32 {
    if pressure_max > 0.0 && pressure_max.is_finite() { (pressure as f32 / pressure_max).clamp(0.0, 1.0) } else { 0.0 }
}

/// The orientation axes' maxima from the selected device's `DVC_ORIENTATION` (`AXIS[3]`:
/// azimuth, altitude, twist), in the driver's own units (tenths of a degree). All zero when the
/// driver reports no such axes — a pen without tilt or barrel rotation — which maps everything
/// to zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrientationScales {
    pub azimuth: f32,
    pub altitude: f32,
    pub twist: f32,
}

impl OrientationScales {
    /// The three `AXIS.ax_max` (an absent or negative axis reads as 0).
    pub fn new(axes: [i32; 3]) -> Self {
        let f = |ax_max: i32| if ax_max > 0 { ax_max as f32 } else { 0.0 };
        Self { azimuth: f(axes[0]), altitude: f(axes[1]), twist: f(axes[2]) }
    }
}

/// The device's report ranges, read at install from the context's device: the pressure axis'
/// maximum as the divisor for 0..1 ([`pressure_scale`]) and the orientation axes'.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scales {
    pub pressure_max: f32,
    pub orientation: OrientationScales,
}

/// The packet data bits (`PK_*`, wintab.h) in bit order (a packet lays its set fields out so).
const PK_CURSOR: u32 = 0x0020;
const PK_BUTTONS: u32 = 0x0040;
const PK_X: u32 = 0x0080;
const PK_Y: u32 = 0x0100;
const PK_NORMAL_PRESSURE: u32 = 0x0400;
const PK_ORIENTATION: u32 = 0x1000;

/// The packet fields every context asks for: cursor, buttons, position and pressure. Pressure
/// rides the layout unconditionally — every pen reports it, and Wacom's `DVC_PKTDATA` is a default
/// mask (0x1FF), not a capability list, so gating on it would have dropped pressure and orientation
/// from a fully capable context.
const BASE_FIELDS: u32 = PK_CURSOR | PK_BUTTONS | PK_X | PK_Y | PK_NORMAL_PRESSURE;

/// Whether the orientation axes say the device can actually report tilt or twist: nonzero azimuth
/// and altitude (tilt) or twist maxima (a Z-rotary stub with zero tilt reads as none).
fn orientation_usable(axes: [i32; 3]) -> bool {
    (axes[0] > 0 && axes[1] > 0) || axes[2] > 0
}

/// The `lcPktData` layout and `lcMoveMask` for a device whose `DVC_ORIENTATION` axes are `axes`
/// (`None` when the driver reports none).
///
/// The layout may only name supported fields — Wintab fails `WTOpen` on an unsupported item — so
/// `PK_ORIENTATION` is requested only when the device has usable orientation axes. The move mask
/// names position, pressure and orientation so stationary changes still generate a packet;
/// buttons and cursor have their own events.
pub fn packet_masks(axes: Option<[i32; 3]>) -> (u32, u32) {
    let pkt_data = match axes {
        Some(axes) if orientation_usable(axes) => BASE_FIELDS | PK_ORIENTATION,
        _ => BASE_FIELDS,
    };
    (pkt_data, pkt_data & (PK_X | PK_Y | PK_NORMAL_PRESSURE | PK_ORIENTATION))
}

/// The `WTInfo` entries [`context_settings`] reads from the context's device.
pub trait DeviceInfo {
    /// The normal-pressure axis' maximum (`DVC_NPRESSURE`), in driver units.
    fn pressure_max(&mut self, category: u32) -> Option<i32>;
    /// The orientation axes' maxima (`DVC_ORIENTATION`: azimuth, altitude, twist); `None` when
    /// the device reports no axes.
    fn orientation_max(&mut self, category: u32) -> Option<[i32; 3]>;
}

/// The context's tuned fields and its device's report ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextSettings {
    /// `lcPktData`: the fields in each packet.
    pub pkt_data: u32,
    /// `lcMoveMask`: the fields whose change generates a packet.
    pub move_mask: u32,
    /// The selected device's ranges.
    pub scales: Scales,
}

/// The settings for a context on `device` (`lcDevice`), querying only that device's `WTInfo`
/// category: another device's ranges would scale its packets wrongly. [`DeviceInfo`] supplies the
/// answers, keeping the multi-device and capability logic testable on every platform.
pub fn context_settings(device: u32, info: &mut impl DeviceInfo) -> ContextSettings {
    let category = device_category(device);
    let axes = info.orientation_max(category);
    let (pkt_data, move_mask) = packet_masks(axes);
    // The driver's Tip-Feel'ed output range (0..ax_max); missing reads as the 10-bit fallback.
    let pressure_max = pressure_scale(info.pressure_max(category).unwrap_or(0));
    // No PK_ORIENTATION in the layout: its fields stay zero, so the axes are not read.
    let orientation = if pkt_data & PK_ORIENTATION != 0 { OrientationScales::new(axes.unwrap_or([0; 3])) } else { OrientationScales::default() };
    ContextSettings { pkt_data, move_mask, scales: Scales { pressure_max, orientation } }
}

/// One packet's decoded fields: what the stream reads out of a `WT_PACKET`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub cursor: u32,
    pub pressure: u32,
    /// `ORIENTATION`: `(azimuth, altitude, twist)`, in the axes' own units (tenths of a degree);
    /// all zero for a pen without orientation.
    pub orientation: (i32, i32, i32),
}

/// A packet's `ORIENTATION` fields as `(tilt_x, tilt_y, rotation)` in [`Sample`]'s conventions
/// (tilt in degrees, W3C signs; rotation in degrees clockwise 0..360). Pure, so every corner is
/// unit-tested on every platform; [`Sample::sanitized`] clamps downstream regardless.
///
/// Wintab: `orAzimuth` is an angle about the tablet's normal, 0 pointing away from the user;
/// `orAltitude` the angle off the tablet surface (0 lying flat, the axis range perpendicular,
/// its sign saying which way up the pen is held — Qt takes its absolute value too). The pen axis
/// is that direction in (x right, y away from the user, z out of the tablet); the tilt is its
/// angle off the vertical per axis, with Wintab's +y flipped to the sample's towards-the-user.
/// `orTwist` is the barrel rotation, which the driver counts the other way around, so it is
/// negated exactly like Qt's `rotation = 360 - orTwist / 10` — a direction verified with a real
/// Art Pen (QTBUG-39570) and matched by Krita's own Wintab reader.
///
/// A zero `azimuth`/`altitude` or `twist` maximum maps that axis to zero: that is how a driver
/// granting no orientation fields degrades, and stray fields from such a driver stay ignored.
pub fn orientation(azimuth: i32, altitude: i32, twist: i32, scales: OrientationScales) -> (f32, f32, f32) {
    // Barrel twist, clockwise-positive: 0 with no twist axis (or for a pen that reports none).
    let rotation = if scales.twist > 0.0 { (360.0 * (1.0 - twist as f32 / scales.twist)).rem_euclid(360.0) } else { 0.0 };
    if !(scales.azimuth > 0.0 && scales.altitude > 0.0) {
        return (0.0, 0.0, rotation);
    }
    let az = std::f32::consts::TAU * azimuth as f32 / scales.azimuth;
    let alt = (std::f32::consts::FRAC_PI_2 * altitude.unsigned_abs() as f32 / scales.altitude).min(std::f32::consts::FRAC_PI_2);
    let (sin_az, cos_az) = az.sin_cos();
    let (sin_alt, cos_alt) = alt.sin_cos();
    let tilt_x = (sin_az * cos_alt).atan2(sin_alt).to_degrees();
    let tilt_y = -(cos_az * cos_alt).atan2(sin_alt).to_degrees();
    (tilt_x, tilt_y, rotation)
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
    /// stale serial) leaves the sample alone. Tilt and barrel rotation ride along either way (the
    /// Art Pen's twist matters at contact; hover carries it for the brush cursor).
    pub fn packet(&mut self, reading: Reading, csr_type: u32, scales: Scales) -> Update {
        let eraser = is_eraser(reading.cursor, csr_type);
        let (azimuth, altitude, twist) = reading.orientation;
        let (tilt_x, tilt_y, rotation) = orientation(azimuth, altitude, twist, scales.orientation);
        if reading.pressure > 0 {
            self.in_range = true;
            let pressure = pressure_unit(reading.pressure, scales.pressure_max);
            return Update::Set(Some(Sample { pressure, tilt_x, tilt_y, rotation, eraser }));
        }
        if self.in_range {
            return Update::Set(Some(Sample { pressure: 0.0, tilt_x, tilt_y, rotation, eraser }));
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
/// so `unsafe`, allowed on this module alone (the crate denies it everywhere else).
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod win {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    use windows::core::w;

    use super::{
        DVC_NPRESSURE, DVC_ORIENTATION, DeviceInfo, PenStream, Reading, Sample, Scales, Update, context_settings, cursor_type_entry, packet_for_context,
        packet_serial, proximity_entering,
    };
    use crate::{Error, deliver};

    // `wintab.h`: the default message base (`WT_PACKET`/`WT_PROXIMITY` are its offsets), the
    // default context to copy, and the cursor's type entry.
    const WT_DEFBASE: u32 = 0x7FF0;
    const WT_PACKET: u32 = WT_DEFBASE;
    const WT_PROXIMITY: u32 = WT_DEFBASE + 5;
    const WTI_DEFCONTEXT: u32 = 3;
    const CSR_TYPE: u32 = 20;
    /// `CXO_MESSAGES`: want `WT_*` messages. `CXO_SYSTEM`: this context also controls the system
    /// cursor and clicks while it is topmost. The spec processes contexts in overlap order and
    /// lets *the topmost context* control the cursor only with `CXO_SYSTEM`: a messages-only
    /// context on top of the window suppresses the driver's own system context instead of
    /// coexisting with it, and the pen's cursor stops moving over the app ([OBSERVED 2026-10-26,
    /// Windows 11, Wacom driver]). With `CXO_SYSTEM` the driver keeps synthesizing the cursor
    /// motion and mouse messages for the pen (what the unchecked "Use Windows Ink" box promises),
    /// while the packets bring the pressure.
    const CXO_MESSAGES: u32 = 0x0004;
    const CXO_SYSTEM: u32 = 0x0001;
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

    /// `AXIS` (wintab.h): a device axis range (the pressure and orientation scales).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    #[allow(clippy::upper_case_acronyms)]
    struct AXIS {
        ax_min: i32,
        ax_max: i32,
        ax_units: u32,
        ax_resolution: u32,
    }

    /// `ORIENTATION` (wintab.h): the transducer's azimuth and altitude about the tablet normal
    /// and its barrel twist, each in tenths of a degree (all zero when the pen reports none).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    #[allow(clippy::upper_case_acronyms)]
    struct ORIENTATION {
        azimuth: i32,
        altitude: i32,
        twist: i32,
    }

    /// One packet, the context's layout (`lcPktData`) in bit order. `WTPacket` writes only the
    /// granted fields, so the rest of a `default()` packet stays zero — an unrequested or
    /// ungranted `PK_ORIENTATION` reads as no tilt rather than stale data.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Packet {
        cursor: u32,
        buttons: u32,
        x: u32,
        y: u32,
        pressure: u32,
        orientation: ORIENTATION,
    }

    // The structs mirror wintab.h (LOGCONTEXTW 212 bytes, AXIS 16, the packet 32); a mismatch
    // would silently misread driver memory, so it must not compile.
    const _: () = assert!(std::mem::size_of::<LOGCONTEXT>() == 212 && std::mem::size_of::<AXIS>() == 16 && std::mem::size_of::<Packet>() == 32);

    /// The proc's state: the callback, the calls and context, the report scales, and the pen
    /// state between messages. Only the window's thread touches it.
    struct Data {
        callback: Box<dyn Fn(Option<Sample>)>,
        wt_info: WTInfoFn,
        wtpacket: WTPacketFn,
        wt_close: WTCloseFn,
        hctx: HCTX,
        scales: Scales,
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

    /// The context device's `WTInfo` entries, as [`context_settings`] asks for them.
    impl DeviceInfo for Dll {
        fn pressure_max(&mut self, category: u32) -> Option<i32> {
            self.entry::<AXIS>(category, DVC_NPRESSURE).map(|axis| axis.ax_max)
        }

        fn orientation_max(&mut self, category: u32) -> Option<[i32; 3]> {
            self.entry::<[AXIS; 3]>(category, DVC_ORIENTATION).map(|axes| axes.map(|axis| axis.ax_max))
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
            let mut dll = Dll::load()?;
            // SAFETY: a plain API call; `0, 0, null` is the documented presence check.
            if unsafe { (dll.wt_info)(0, 0, std::ptr::null_mut()) } == 0 {
                return Err(Error::Unsupported("no Wintab devices (WTInfo reports none)".into()));
            }

            // The default context, tuned for this window: our packet fields as messages, and the
            // system-cursor option so the driver keeps moving the cursor (see `CXO_SYSTEM`).
            let mut lc: LOGCONTEXT = dll.entry(WTI_DEFCONTEXT, 0).ok_or_else(|| Error::Platform("WTInfo WTI_DEFCONTEXT failed".into()))?;
            for (i, ch) in "Photocraft".encode_utf16().take(39).enumerate() {
                lc.name[i] = ch;
            }
            lc.options = CXO_MESSAGES | CXO_SYSTEM;
            lc.msg_base = WT_DEFBASE;
            // The default context's own device supplies the layout (supported fields only), the
            // move mask and the scales — a context on device 1 must not read device 0's.
            let settings = context_settings(lc.device, &mut dll);
            lc.pkt_data = settings.pkt_data;
            lc.pkt_mode = 0;
            lc.move_mask = settings.move_mask;

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
                scales: settings.scales,
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
        /// when the packet's cursor index leaves the cursor category ([`cursor_type_entry`]), so a
        /// hostile field never reads another category's memory.
        fn csr_type(&self, cursor: u32) -> u32 {
            let (last, mut csr_type) = self.last_cursor.get();
            if last != cursor {
                csr_type = 0;
                if let Some(index) = cursor_type_entry(cursor) {
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
            // SAFETY: Microsoft's pattern for a subclassed window being destroyed: release the
            // subclass from inside the `WM_NCDESTROY` handler, while the window still exists.
            // (`Monitor::drop`'s own removal covers the window outliving the monitor.)
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
            }
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
                        let reading = Reading {
                            cursor: pkt.cursor,
                            pressure: pkt.pressure,
                            orientation: (pkt.orientation.azimuth, pkt.orientation.altitude, pkt.orientation.twist),
                        };
                        let mut stream = data.stream.get();
                        let update = stream.packet(reading, data.csr_type(pkt.cursor), data.scales);
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

    /// A packet with just a pressure (no orientation fields): the plain-pen case.
    fn reading(pressure: u32) -> Reading {
        Reading { pressure, ..Reading::default() }
    }

    /// A packet with orientation fields filled in.
    fn oriented(pressure: u32, azimuth: i32, altitude: i32, twist: i32) -> Reading {
        Reading { pressure, orientation: (azimuth, altitude, twist), ..Reading::default() }
    }

    /// Scales for a plain pen: the given pressure divisor, no orientation axes.
    fn plain_scales(pressure_max: f32) -> Scales {
        Scales { pressure_max, ..Scales::default() }
    }

    #[test]
    fn proximity_reads_the_low_word_of_lparam_not_wparam() {
        assert!(proximity_entering(1));
        assert!(proximity_entering(0x1234_0001));
        assert!(!proximity_entering(0));
        // The leave message's high word can be set — it is the hardware-proximity flag (nonzero
        // when the cursor enters or leaves the tablet's proximity), not this context's enter
        // flag. Decoding wParam (the context handle) said "entering" for every message.
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
    fn cursor_entries_stay_inside_the_cursor_category() {
        assert_eq!(cursor_type_entry(0), Some(200));
        assert_eq!(cursor_type_entry(99), Some(299), "the category's last slot");
        assert_eq!(cursor_type_entry(100), None, "300 starts the extension category");
        assert_eq!(cursor_type_entry(u32::MAX), None, "a hostile packet field reads nothing");
    }

    #[test]
    fn device_entries_stay_inside_the_device_category() {
        assert_eq!(device_category(0), 100, "device 0's own category");
        assert_eq!(device_category(1), 101, "a second device gets its own category");
        assert_eq!(device_category(99), 199, "the category's last slot");
        assert_eq!(device_category(100), 100, "200 starts the cursors: nothing to read, fall back");
        assert_eq!(device_category(u32::MAX), 100, "Wacom's lcDevice = 0xFFFFFFFF sentinel");
    }

    #[test]
    fn packet_fields_follow_the_devices_orientation_axes() {
        // A plain pen (no axes): pressure only, no PK_ORIENTATION claim (WTOpen fails on an
        // unsupported item), and the move mask still carries pressure.
        let (pkt_data, move_mask) = packet_masks(None);
        assert_eq!(pkt_data, BASE_FIELDS);
        assert_eq!(pkt_data & PK_ORIENTATION, 0, "no orientation claim without axes");
        assert_eq!(move_mask & PK_NORMAL_PRESSURE, PK_NORMAL_PRESSURE);
        assert_eq!(move_mask & PK_ORIENTATION, 0);
        // Axes reported but zero or twist-only tilt stubs: still no orientation.
        assert_eq!(packet_masks(Some([0, 0, 0])).0, BASE_FIELDS);
        assert_eq!(packet_masks(Some([0, 0, 3600])).0, BASE_FIELDS | PK_ORIENTATION, "Z-rotary-only");
        assert_eq!(packet_masks(Some([3600, 0, 0])).0, BASE_FIELDS, "no altitude: no tilt");
        // A tilt-capable pen: orientation requested, and in the move mask.
        let (pkt_data, move_mask) = packet_masks(Some([3600, 900, 3600]));
        assert_eq!(pkt_data, BASE_FIELDS | PK_ORIENTATION);
        assert_eq!(move_mask & PK_NORMAL_PRESSURE, PK_NORMAL_PRESSURE);
        assert_eq!(move_mask & PK_ORIENTATION, PK_ORIENTATION);
        assert_eq!(move_mask & !pkt_data, 0, "the move mask only names requested fields");
        // Wacom's real DVC_PKTDATA default mask (0x1FF) must not gate anything: it lists no
        // pressure or orientation bits even on a fully capable pen.
        assert_eq!(0x1FF & (PK_NORMAL_PRESSURE | PK_ORIENTATION), 0);
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
        assert_eq!(pen.packet(reading(0), 0, plain_scales(1023.0)), Update::Keep);
        // Entering: hovering, no pressure yet; a hover packet keeps the pen alive.
        assert_eq!(pen.proximity(true), hover);
        assert_eq!(pen.packet(reading(0), 0, plain_scales(1023.0)), hover);
        // Contact: the pressure is the sample.
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        let s = contact(pen.packet(reading(512), 0x0802, plain_scales(1024.0)));
        assert_eq!((s.pressure, s.eraser), (0.5, false));
        let s = contact(pen.packet(reading(1024), 0x082a, plain_scales(1024.0)));
        assert!(s.eraser && s.pressure == 1.0, "{s:?}");
        // Leaving ends the pen (the mouse takes over): a stray zero-pressure packet after the
        // leave must stay silent, or the mouse would paint at the pen's zero.
        assert_eq!(pen.proximity(false), Update::Set(None));
        assert_eq!(pen.packet(reading(0), 0x082a, plain_scales(1024.0)), Update::Keep);
        // Re-entering reports hover again.
        assert_eq!(pen.proximity(true), hover);
    }

    #[test]
    fn orientation_maps_azimuth_and_altitude_to_tilt() {
        let axes = OrientationScales { azimuth: 3600.0, altitude: 900.0, twist: 0.0 };
        let tilt = |azimuth, altitude| {
            let (tilt_x, tilt_y, _) = orientation(azimuth, altitude, 0, axes);
            (tilt_x, tilt_y)
        };
        let near = |(x, y): (f32, f32), want: (f32, f32)| (x - want.0).abs() < 0.01 && (y - want.1).abs() < 0.01;
        // Perpendicular (the full altitude range) is no tilt at any azimuth.
        for azimuth in [0, 900, 1800, 2700] {
            assert!(near(tilt(azimuth, 900), (0.0, 0.0)), "azimuth {azimuth}");
        }
        // A pen leaning 45° in the azimuth's direction: +x tilts right, +y towards the user.
        assert!(near(tilt(900, 450), (45.0, 0.0)), "azimuth east leans right");
        assert!(near(tilt(0, 450), (0.0, -45.0)), "azimuth north (away) leans up the screen");
        assert!(near(tilt(1800, 450), (0.0, 45.0)), "azimuth south leans towards the user");
        assert!(near(tilt(2700, 450), (-45.0, 0.0)), "azimuth west leans left");
        // The altitude's sign (an upside-down pen) tilts the same way, like Qt's absolute value.
        assert_eq!(tilt(900, -450), tilt(900, 450));
        // Out-of-range fields clamp to perpendicular, the most a packet can mean.
        assert!(near(tilt(900, 9000), (0.0, 0.0)));
        // A zero azimuth or altitude maximum is "no orientation axes": zero tilt, even with
        // stray fields in the packet (the ask: zero axes must give zero tilt).
        for zero in [OrientationScales { azimuth: 0.0, ..axes }, OrientationScales { altitude: 0.0, ..axes }] {
            assert_eq!(orientation(900, 450, 0, zero), (0.0, 0.0, 0.0), "{zero:?}");
        }
        assert_eq!(orientation(900, 450, 0, OrientationScales::default()), (0.0, 0.0, 0.0));
    }

    #[test]
    fn twist_maps_to_clockwise_rotation() {
        let axes = OrientationScales { twist: 3600.0, ..OrientationScales::default() };
        let rotation = |twist| orientation(0, 0, twist, axes).2;
        // The driver counts twist the other way around, so Qt's negation (`360 - twist/10`) is
        // what makes 90/180/270 read as a pen turned clockwise by 270/180/90.
        assert_eq!(rotation(0), 0.0);
        assert_eq!(rotation(900), 270.0);
        assert_eq!(rotation(1800), 180.0);
        assert_eq!(rotation(2700), 90.0);
        assert!((rotation(3599) - 0.1).abs() < 0.001, "just under a full turn");
        // No twist axis (a pen without barrel rotation), or an out-of-range field: zero or in
        // range, never a panic or a NaN.
        assert_eq!(orientation(0, 0, 900, OrientationScales::default()).2, 0.0);
        for twist in [9999, -1, i32::MIN, i32::MAX] {
            let r = orientation(0, 0, twist, axes).2;
            assert!(r.is_finite() && (0.0..=360.0).contains(&r), "{twist} -> {r}");
        }
    }

    #[test]
    fn packets_carry_tilt_and_rotation() {
        let mut pen = PenStream::default();
        let full = Scales { pressure_max: 1024.0, orientation: OrientationScales { azimuth: 3600.0, altitude: 900.0, twist: 3600.0 } };
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        // A pen leaning 45° east at half pressure.
        let s = contact(pen.packet(oriented(512, 900, 450, 0), 0x0802, full));
        assert!((s.pressure - 0.5).abs() < 0.001 && (s.tilt_x - 45.0).abs() < 0.01 && s.tilt_y.abs() < 0.01, "{s:?}");
        // Hovering reports the same tilt and the barrel twist.
        let s = contact(pen.packet(oriented(0, 900, 450, 900), 0x0802, full));
        assert!(s.pressure == 0.0 && (s.tilt_x - 45.0).abs() < 0.01, "{s:?}");
        assert!((s.rotation - 270.0).abs() < 0.01, "{s:?}");
        // A plain pen (no orientation axes) still reads zero tilt and rotation.
        let s = contact(pen.packet(reading(512), 0x0802, plain_scales(1024.0)));
        assert!(s.tilt_x == 0.0 && s.tilt_y == 0.0 && s.rotation == 0.0, "{s:?}");
    }

    #[test]
    fn stationary_pressure_and_tilt_changes_reach_the_stream() {
        // With pressure and orientation in the move mask, a pen resting at one spot still gets a
        // WT_PACKET when only those change (the reading carries no position at all); the old
        // PK_X|PK_Y-only mask delivered neither, freezing the sample.
        let (_, move_mask) = packet_masks(Some([3600, 900, 3600]));
        assert_eq!(move_mask & (PK_NORMAL_PRESSURE | PK_ORIENTATION), PK_NORMAL_PRESSURE | PK_ORIENTATION);
        let full = Scales { pressure_max: 8191.0, orientation: OrientationScales { azimuth: 3600.0, altitude: 900.0, twist: 3600.0 } };
        let mut pen = PenStream::default();
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        let s = contact(pen.packet(oriented(4095, 0, 900, 0), 0x0802, full));
        assert!((s.pressure - 0.5).abs() < 0.001, "half of the 8191 range: {s:?}");
        // Same spot, same pressure: only the tilt and twist moved, and both must come through.
        let s = contact(pen.packet(oriented(4095, 900, 450, 900), 0x0802, full));
        assert!((s.tilt_x - 45.0).abs() < 0.01 && (s.rotation - 270.0).abs() < 0.01, "{s:?}");
    }

    /// A fake `WTInfo` device table for [`context_settings`], recording every entry asked for.
    #[derive(Default)]
    struct FakeDevice {
        pressure_max: Option<i32>,
        orientation_max: Option<[i32; 3]>,
    }

    #[derive(Default)]
    struct FakeDevices {
        devices: Vec<FakeDevice>,
        /// Every `(category, entry)` queried, for asserting the right category was read.
        asked: Vec<(u32, u32)>,
    }

    impl FakeDevices {
        fn device(&mut self, category: u32) -> Option<&mut FakeDevice> {
            let index = usize::try_from(category.checked_sub(WTI_DEVICES)?).ok()?;
            self.devices.get_mut(index)
        }
    }

    impl DeviceInfo for FakeDevices {
        fn pressure_max(&mut self, category: u32) -> Option<i32> {
            self.asked.push((category, DVC_NPRESSURE));
            self.device(category)?.pressure_max
        }

        fn orientation_max(&mut self, category: u32) -> Option<[i32; 3]> {
            self.asked.push((category, DVC_ORIENTATION));
            self.device(category)?.orientation_max
        }
    }

    #[test]
    fn context_settings_come_from_the_contexts_own_device() {
        // The fixture that reported the bug: device 0 a coarse 10-bit, 1800-twist pen, device 1
        // an 8191-range, 3600-twist one. Reading device 1 with device 0's ranges turned pressure
        // 4095 into 1.0 instead of ~0.5 and twist 900 into 180° instead of 270°.
        let mut info = FakeDevices {
            devices: vec![
                FakeDevice { pressure_max: Some(1023), orientation_max: Some([3600, 900, 1800]) },
                FakeDevice { pressure_max: Some(8191), orientation_max: Some([3600, 900, 3600]) },
                FakeDevice { pressure_max: Some(2047), orientation_max: None },
            ],
            ..FakeDevices::default()
        };
        let settings = context_settings(1, &mut info);
        assert_eq!(settings.scales.pressure_max, 8191.0, "device 1's range, not device 0's 1023");
        assert_eq!(settings.scales.orientation, OrientationScales { azimuth: 3600.0, altitude: 900.0, twist: 3600.0 });
        assert_eq!(settings.pkt_data & PK_ORIENTATION, PK_ORIENTATION, "device 1 has orientation axes");
        assert_eq!(info.asked.len(), 2, "{:?}", info.asked);
        assert!(info.asked.iter().all(|&(category, _)| category == WTI_DEVICES + 1), "queried device 0's category: {:?}", info.asked);
        let mut pen = PenStream::default();
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        let s = contact(pen.packet(oriented(4095, 0, 900, 900), 0x0802, settings.scales));
        assert!((s.pressure - 0.5).abs() < 0.001, "4095 of device 1's 8191: {s:?}");
        assert!((s.rotation - 270.0).abs() < 0.01, "twist 900 of device 1's 3600: {s:?}");

        // The same reading through device 0's scales, for contrast: device 0's owner gets these.
        info.asked.clear();
        let settings = context_settings(0, &mut info);
        assert_eq!(settings.scales.pressure_max, 1023.0);
        assert_eq!(settings.scales.orientation.twist, 1800.0);
        assert!(info.asked.iter().all(|&(category, _)| category == WTI_DEVICES), "{:?}", info.asked);
        let mut pen = PenStream::default();
        let s = contact(pen.packet(oriented(4095, 0, 900, 900), 0x0802, settings.scales));
        assert_eq!(s.pressure, 1.0, "4095 clamps against device 0's own 1023");
        assert!((s.rotation - 180.0).abs() < 0.01, "twist 900 of device 0's 1800: {s:?}");

        // Device 2 reports no orientation axes: a pressure-only context.
        info.asked.clear();
        let settings = context_settings(2, &mut info);
        assert_eq!(settings.scales.pressure_max, 2047.0);
        assert_eq!(settings.scales.orientation, OrientationScales::default());
        assert_eq!(settings.pkt_data, BASE_FIELDS);

        // An index outside the device category falls back to device 0's entries — not "nothing".
        info.asked.clear();
        let settings = context_settings(200, &mut info);
        assert!(info.asked.iter().all(|&(category, _)| category == WTI_DEVICES), "{:?}", info.asked);
        assert_eq!(settings.scales.pressure_max, 1023.0);
        assert_eq!(settings.scales.orientation.twist, 1800.0);
    }

    #[test]
    fn wacom_default_context_device_sentinel_reads_the_default_device() {
        // The regression the real driver exposed: Wacom's `WTI_DEFCONTEXT` leaves
        // `lcDevice = 0xFFFFFFFF`, and treating it as "no device" dropped orientation from the
        // layout (tilt and rotation dead) and fell back to the 10-bit pressure scale against the
        // pen's real 32767 range, so pressure clamped to 1.0 almost at once.
        let real = FakeDevice { pressure_max: Some(32767), orientation_max: Some([3600, 900, 3600]) };
        let mut info = FakeDevices { devices: vec![real], ..FakeDevices::default() };
        let settings = context_settings(u32::MAX, &mut info);
        assert!(info.asked.iter().all(|&(category, _)| category == WTI_DEVICES), "{:?}", info.asked);
        assert_eq!(settings.pkt_data, BASE_FIELDS | PK_ORIENTATION, "orientation must ride the layout");
        assert_eq!(settings.scales.pressure_max, 32767.0);
        let mut pen = PenStream::default();
        let contact = |u| match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        };
        let s = contact(pen.packet(oriented(16383, 0, 900, 900), 0x0802, settings.scales));
        assert!((s.pressure - 0.5).abs() < 0.001, "16383 of the pen's 32767: {s:?}");
        assert!((s.rotation - 270.0).abs() < 0.01, "twist 900 of 3600: {s:?}");
    }
}
