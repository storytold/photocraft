//! XInput2 tablet valuators → [`Sample`]s. Pure (no X calls), so it is tested on every platform;
//! `x11.rs` queries the devices and feeds raw events here.
//!
//! A tablet tool is a slave pointer device with an "Abs Pressure" valuator (the xf86-input-wacom
//! and xf86-input-libinput drivers and Xwayland all label their axes with the standard
//! `AXIS_LABEL_PROP_*` atom names). Each valuator is normalised from its advertised `min..max`:
//!
//! - pressure → 0..1;
//! - tilt: drivers report degrees (libinput, Xwayland: -64..63; wacom: -64..63 units of about a
//!   degree), used as-is when the range fits in ±90, otherwise scaled to ±[`TILT_DEGREES`];
//! - "Abs Rotary Z" (Art Pen barrel rotation) → 0..360 from the valuator range;
//! - "Abs Wheel" (Xwayland's label for the same axis on tablet tools) → degrees as
//!   `raw / 5 − 90` (mod 360); raw `0` means no rotation was reported and leaves rotation at 0.
//!
//! The eraser is a separate device whose name says so ("… Pen eraser", "… Eraser (0x…)"), as
//! GTK and Krita detect it. Raw events only carry the valuators that changed, so the last value
//! of each axis is kept per device.

use std::collections::HashMap;

use crate::{Sample, Update};

/// Tilt for valuators that don't report degrees: the extremes of the range.
pub const TILT_DEGREES: f64 = 60.0;

/// Valuator numbers beyond this are ignored (the protocol allows 65535; real devices have < 16).
pub const MAX_AXES: usize = 64;

/// Devices kept (hostile servers could report thousands).
pub const MAX_DEVICES: usize = 256;

/// Which pen axis a valuator carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AxisKind {
    Pressure,
    TiltX,
    TiltY,
    Rotation,
}

impl AxisKind {
    /// From a valuator's label atom name.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Abs Pressure" => Some(AxisKind::Pressure),
            "Abs Tilt X" => Some(AxisKind::TiltX),
            "Abs Tilt Y" => Some(AxisKind::TiltY),
            "Abs Rotary Z" | "Abs Wheel" => Some(AxisKind::Rotation),
            _ => None,
        }
    }
}

/// One valuator of a device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub kind: AxisKind,
    /// Valuator number (index into a raw event's valuator mask).
    pub number: u16,
    pub min: f64,
    pub max: f64,
    /// Xwayland labels barrel rotation "Abs Wheel" (5 units per degree, offset 90°).
    pub abs_wheel_rotation: bool,
}

impl Axis {
    /// Map a raw value of this axis to the sample's units.
    fn map(&self, v: f64) -> f32 {
        if !v.is_finite() {
            return match self.kind {
                AxisKind::Pressure => 1.0,
                _ => 0.0,
            };
        }
        let span = self.max - self.min;
        let unit = if span.is_finite() && span > 0.0 { ((v - self.min) / span).clamp(0.0, 1.0) } else { v.clamp(0.0, 1.0) };
        match self.kind {
            AxisKind::Pressure => unit as f32,
            AxisKind::TiltX | AxisKind::TiltY => {
                if self.min >= -90.0 && self.max <= 90.0 && span > 0.0 {
                    v.clamp(-90.0, 90.0) as f32
                } else {
                    ((unit * 2.0 - 1.0) * TILT_DEGREES) as f32
                }
            }
            AxisKind::Rotation => (unit * 360.0) as f32,
        }
    }
}

/// Xwayland and xf86-input-wacom send barrel rotation on "Abs Wheel" as
/// `remainder(angle + 90, 360) * 5`. Raw `0` means the pen did not report rotation.
pub fn map_abs_wheel_rotation(raw: f64) -> Option<f32> {
    if !raw.is_finite() {
        return Some(0.0);
    }
    if raw == 0.0 {
        return None;
    }
    Some((raw / 5.0 - 90.0).rem_euclid(360.0) as f32)
}

/// XI2 delivers each raw event twice to `XIAllDevices` clients. The master copy has
/// `deviceid` = master and `sourceid` = slave; the slave copy has both equal to the slave.
#[inline]
pub fn raw_event_is_slave_copy(deviceid: u16, sourceid: u16) -> bool {
    deviceid == sourceid
}

/// A slave pointer device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub id: u16,
    /// `XIQueryDevice` name (for logs).
    pub name: String,
    /// The eraser end of a pen.
    pub eraser: bool,
    /// Pen axes (other valuators are not kept).
    pub axes: Vec<Axis>,
}

impl Device {
    /// Build from what `XIQueryDevice` reports: the name and each valuator's (number, label,
    /// min, max).
    pub fn new(id: u16, name: &str, valuators: impl IntoIterator<Item = (u16, String, f64, f64)>) -> Self {
        let listed: Vec<_> = valuators.into_iter().filter(|(n, ..)| usize::from(*n) < MAX_AXES).take(MAX_AXES).collect();
        let has_pressure = listed.iter().any(|(_, label, ..)| label == "Abs Pressure");
        let axes = listed
            .into_iter()
            .filter_map(|(number, label, min, max)| {
                let kind = AxisKind::from_label(&label)?;
                // Tablet pads expose Abs Wheel without pressure; only tablet tools use it as rotation.
                if kind == AxisKind::Rotation && label == "Abs Wheel" && !has_pressure {
                    return None;
                }
                let abs_wheel_rotation = kind == AxisKind::Rotation && label == "Abs Wheel";
                Some(Axis { kind, number, min, max, abs_wheel_rotation })
            })
            .collect();
        Self { id, name: name.to_string(), eraser: name.to_ascii_lowercase().contains("eraser"), axes }
    }

    /// A pen (tip or eraser): has a pressure axis.
    pub fn is_tablet(&self) -> bool {
        self.axes.iter().any(|a| a.kind == AxisKind::Pressure)
    }
}

/// The values a raw event carries: valuator numbers set in `mask` (32 per word, LSB first)
/// paired, in order, with `values`. Extra mask bits beyond the values are ignored.
pub fn valuators<'a>(mask: &'a [u32], values: &'a [f64]) -> impl Iterator<Item = (u16, f64)> + 'a {
    let numbers = mask
        .iter()
        .enumerate()
        .flat_map(|(w, bits)| (0..32u32).filter(move |b| bits & (1 << b) != 0).map(move |b| w.saturating_mul(32).saturating_add(b as usize)));
    numbers.zip(values.iter().copied()).filter_map(|(n, v)| u16::try_from(n).ok().map(|n| (n, v)))
}

/// Devices and the last value of each pen axis.
#[derive(Clone, Debug, Default)]
pub struct State {
    devices: HashMap<u16, Device>,
    last: HashMap<u16, [Option<f32>; 4]>,
}

impl State {
    /// Replace the device list (after `XIQueryDevice`, on start and hierarchy changes).
    pub fn set_devices(&mut self, devices: impl IntoIterator<Item = Device>) {
        self.devices = devices.into_iter().take(MAX_DEVICES).map(|d| (d.id, d)).collect();
        self.last.retain(|id, _| self.devices.contains_key(id));
    }

    pub fn device(&self, id: u16) -> Option<&Device> {
        self.devices.get(&id)
    }

    /// A raw motion or button event from slave device `id` and its changed valuators. Tablet
    /// devices give a sample. Emulated pointers (e.g. `xwayland-pointer`) do not clear the pen
    /// feed on motion; a mouse button still means `None`.
    pub fn handle(&mut self, id: u16, valuators: impl IntoIterator<Item = (u16, f64)>, is_button: bool) -> Update {
        let Some(dev) = self.devices.get(&id) else { return Update::Keep };
        if !dev.is_tablet() {
            if is_button {
                return Update::Set(None);
            }
            return Update::Keep;
        }
        let last = self.last.entry(id).or_default();
        for (n, v) in valuators {
            for a in dev.axes.iter().filter(|a| a.number == n) {
                let slot = match a.kind {
                    AxisKind::Pressure => 0,
                    AxisKind::TiltX => 1,
                    AxisKind::TiltY => 2,
                    AxisKind::Rotation => 3,
                };
                let mapped = if a.abs_wheel_rotation { map_abs_wheel_rotation(v) } else { Some(a.map(v)) };
                if let (Some(s), Some(dest)) = (mapped, last.get_mut(slot)) {
                    *dest = Some(s);
                }
            }
        }
        let [pressure, tilt_x, tilt_y, rotation] = *last;
        Update::Set(Some(
            Sample {
                pressure: pressure.unwrap_or(0.0),
                tilt_x: tilt_x.unwrap_or(0.0),
                tilt_y: tilt_y.unwrap_or(0.0),
                rotation: rotation.unwrap_or(0.0),
                eraser: dev.eraser,
            }
            .sanitized(),
        ))
    }
}

/// A 32.32 fixed-point X value as f64.
pub fn fp3232(integral: i32, frac: u32) -> f64 {
    f64::from(integral) + f64::from(frac) / 4_294_967_296.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wacom(id: u16, name: &str) -> Device {
        Device::new(
            id,
            name,
            [
                (0, "Abs X".to_string(), 0.0, 44704.0),
                (1, "Abs Y".to_string(), 0.0, 27940.0),
                (2, "Abs Pressure".to_string(), 0.0, 65535.0),
                (3, "Abs Tilt X".to_string(), -64.0, 63.0),
                (4, "Abs Tilt Y".to_string(), -64.0, 63.0),
                (5, "Abs Rotary Z".to_string(), -900.0, 899.0),
            ],
        )
    }

    fn mouse(id: u16) -> Device {
        Device::new(id, "Logitech USB Optical Mouse", [(0, "Rel X".to_string(), -1.0, -1.0), (1, "Rel Y".to_string(), -1.0, -1.0)])
    }

    fn emulated_pointer(id: u16) -> Device {
        Device::new(id, "xwayland-pointer", [(0, "Rel X".to_string(), -1.0, -1.0), (1, "Rel Y".to_string(), -1.0, -1.0)])
    }

    fn sample(u: Update) -> Sample {
        match u {
            Update::Set(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        }
    }

    #[test]
    fn raw_event_slave_copy_detection() {
        assert!(!raw_event_is_slave_copy(2, 9));
        assert!(raw_event_is_slave_copy(9, 9));
    }

    #[test]
    fn abs_wheel_rotation_mapping() {
        assert_eq!(map_abs_wheel_rotation(0.0), None);
        assert!((map_abs_wheel_rotation(450.0).unwrap() - 0.0).abs() < 0.01);
        assert!((map_abs_wheel_rotation(675.0).unwrap() - 45.0).abs() < 0.01);
        assert!((map_abs_wheel_rotation(900.0).unwrap() - 90.0).abs() < 0.01);
        assert_eq!(map_abs_wheel_rotation(f64::NAN), Some(0.0));
    }

    #[test]
    fn mask_pairs_numbers_with_values() {
        let v: Vec<_> = valuators(&[0b10_0101, 1], &[1.0, 2.0, 3.0, 4.0]).collect();
        assert_eq!(v, vec![(0, 1.0), (2, 2.0), (5, 3.0), (32, 4.0)]);
        // Fewer values than mask bits (malformed): stop at the values.
        assert_eq!(valuators(&[u32::MAX; 4], &[7.0]).count(), 1);
        assert_eq!(valuators(&[], &[7.0]).count(), 0);
    }

    #[test]
    fn devices_and_eraser_detection() {
        assert!(wacom(9, "Wacom Intuos Pro M Pen stylus").is_tablet());
        assert!(!wacom(9, "Wacom Intuos Pro M Pen stylus").eraser);
        assert!(wacom(10, "Wacom Intuos Pro M Pen eraser").eraser);
        assert!(wacom(11, "HUION Kamvas Pen Eraser (0x1)").eraser);
        assert!(!mouse(12).is_tablet());
        let pad = Device::new(13, "Wacom Intuos Pro M Pad pad", [(5, "Abs Wheel".to_string(), 0.0, 71.0)]);
        assert!(!pad.is_tablet() && pad.axes.is_empty());
    }

    #[test]
    fn valuators_normalise_to_pressure_degrees_and_rotation() {
        let mut st = State::default();
        st.set_devices([wacom(9, "Pen stylus"), wacom(10, "Pen eraser"), mouse(12), emulated_pointer(14)]);
        assert_eq!(st.handle(12, [(0, 3.0)], false), Update::Keep);
        assert_eq!(st.handle(12, [(0, 1.0)], true), Update::Set(None));
        assert_eq!(st.handle(2, [(2, 3.0)], false), Update::Keep);
        let s = sample(st.handle(9, [(0, 100.0), (2, 32767.5), (3, -30.0), (4, 45.0), (5, 0.0)], false));
        assert_eq!(s.pressure, 0.5);
        assert_eq!((s.tilt_x, s.tilt_y), (-30.0, 45.0));
        assert!((s.rotation - 180.0).abs() < 0.2, "{}", s.rotation);
        assert!(!s.eraser);
        // Raw events only carry changed axes: the others keep their last value.
        let s = sample(st.handle(9, [(2, 65535.0)], false));
        assert_eq!((s.pressure, s.tilt_x, s.tilt_y), (1.0, -30.0, 45.0));
        // The eraser device has its own state and flag.
        let s = sample(st.handle(10, [(2, 0.0)], false));
        assert!(s.eraser && s.pressure == 0.0 && s.tilt_x == 0.0);
        // Xwayland also moves xwayland-pointer while the pen is down: keep the pen sample.
        let _ = sample(st.handle(9, [(2, 40000.0)], false));
        assert_eq!(st.handle(14, [(0, 3.0)], false), Update::Keep);
        assert_eq!(st.handle(14, [(0, 1.0)], true), Update::Set(None));
    }

    #[test]
    fn tilt_ranges_that_are_not_degrees_are_scaled() {
        let d =
            Device::new(1, "pen", [(2, "Abs Pressure".into(), 0.0, 1023.0), (3, "Abs Tilt X".into(), 0.0, 127.0), (4, "Abs Tilt Y".into(), -1000.0, 1000.0)]);
        let mut st = State::default();
        st.set_devices([d]);
        let s = sample(st.handle(1, [(3, 127.0), (4, -1000.0)], false));
        assert_eq!((s.tilt_x, s.tilt_y), (60.0, -60.0));
    }

    #[test]
    fn hostile_device_data_does_not_panic() {
        let mut st = State::default();
        // Empty, inverted, zero-width and non-finite ranges; huge valuator numbers.
        let d = Device::new(
            1,
            "",
            [
                (2, "Abs Pressure".into(), 5.0, 5.0),
                (3, "Abs Tilt X".into(), 90.0, -90.0),
                (4, "Abs Tilt Y".into(), f64::NEG_INFINITY, f64::INFINITY),
                (5, "Abs Rotary Z".into(), f64::NAN, f64::NAN),
                (u16::MAX, "Abs Pressure".into(), 0.0, 1.0),
            ],
        );
        assert_eq!(d.axes.len(), 4, "out-of-range valuator numbers are dropped");
        st.set_devices([d]);
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, -0.0, 1e300] {
            let s = sample(st.handle(1, [(2, v), (3, v), (4, v), (5, v)], false));
            assert!((0.0..=1.0).contains(&s.pressure), "{v}: {s:?}");
            assert!((-90.0..=90.0).contains(&s.tilt_x) && (-90.0..=90.0).contains(&s.tilt_y), "{v}: {s:?}");
            assert!((0.0..360.0).contains(&s.rotation), "{v}: {s:?}");
        }
        // Lots of devices are capped.
        st.set_devices((0..2000u16).map(|i| Device::new(i, "x", [])));
        assert!(st.devices.len() <= MAX_DEVICES);
        // Valuator values with no matching device axis are ignored.
        st.set_devices([wacom(3, "pen")]);
        let s = sample(st.handle(3, valuators(&[u32::MAX; 2048], &[0.5; 4096]).collect::<Vec<_>>(), false));
        assert!(s.pressure < 0.001);
    }

    #[test]
    fn xwayland_tablet_stylus_maps_abs_wheel_as_rotation() {
        let dev = Device::new(11, "xwayland-tablet stylus", [(2, "Abs Pressure".into(), 0.0, 65535.0), (5, "Abs Wheel".into(), -900.0, 899.0)]);
        let mut st = State::default();
        st.set_devices([dev]);
        let s = sample(st.handle(11, [(5, 0.0)], false));
        assert!(s.rotation.abs() < 0.01);
        let s = sample(st.handle(11, [(5, 900.0)], false));
        assert!((s.rotation - 90.0).abs() < 0.2);
    }

    #[test]
    fn fixed_point() {
        assert_eq!(fp3232(3, 1 << 31), 3.5);
        assert_eq!(fp3232(-2, 0), -2.0);
    }
}
