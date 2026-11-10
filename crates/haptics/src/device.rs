//! The haptic mouse itself, found and driven through hidapi (Windows).

use crate::descriptor::{
    HAPTICS_USAGE_PAGE, HapticsLayout, SIMPLE_HAPTIC_CONTROLLER_USAGE, build_trigger_report, parse_haptics_descriptor, parse_waveform_list,
};
use hidapi::{DeviceInfo, HidApi, HidDevice, MAX_REPORT_DESCRIPTOR_SIZE};
use std::collections::HashMap;

/// Large enough for any Waveform List a mouse sends (a few 16-bit ordinals).
const FEATURE_REPORT_BUFFER: usize = 64;

pub enum Played {
    Sent,
    /// The device doesn't list this waveform, or its report can't carry it.
    Unsupported,
    /// The write failed: unplugged, re-paired or asleep.
    Gone,
}

pub struct Device {
    device: HidDevice,
    layout: HapticsLayout,
    ordinals: HashMap<u16, u32>,
    /// Behind a Logitech receiver: the way to wake the mouse before a pulse.
    wake: Option<crate::wake::Wake>,
}

impl Device {
    /// The first HID Simple Haptic Controller that can be driven, if any.
    pub fn find() -> Option<Self> {
        let api = match HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                log::warn!("haptics: HID unavailable: {e}");
                return None;
            }
        };
        let found = api
            .device_list()
            .filter(|info| info.usage_page() == HAPTICS_USAGE_PAGE && info.usage() == SIMPLE_HAPTIC_CONTROLLER_USAGE)
            .find_map(|info| Self::open(&api, info));
        if found.is_none() {
            log::info!("haptics: no haptic mouse found");
        }
        found
    }

    pub fn play(&self, usage: u16, strength: f64) -> Played {
        let Some(ordinal) = self.ordinals.get(&usage).copied() else { return Played::Unsupported };
        let Some(report) = build_trigger_report(&self.layout, ordinal, strength) else { return Played::Unsupported };
        log::debug!("haptics: play {usage:#06x} (ordinal {ordinal}) at strength {strength:.2}: report {report:02x?}");
        if let Some(wake) = &self.wake {
            wake.ping();
        }
        match self.device.write(&report) {
            Ok(_) => Played::Sent,
            Err(_) => Played::Gone,
        }
    }

    fn open(api: &HidApi, info: &DeviceInfo) -> Option<Self> {
        let device = info.open_device(api).ok()?;
        let mut descriptor = vec![0u8; MAX_REPORT_DESCRIPTOR_SIZE];
        let len = device.get_report_descriptor(&mut descriptor).ok()?;
        let layout = parse_haptics_descriptor(descriptor.get(..len)?)?;
        let mut report = [0u8; FEATURE_REPORT_BUFFER];
        report[0] = layout.waveform_report_id;
        let len = device.get_feature_report(&mut report).ok()?;
        let ordinals = parse_waveform_list(&layout, report.get(..len)?);
        if ordinals.is_empty() {
            return None;
        }
        log::info!(
            "haptics: using {:04x}:{:04x} ({} waveforms: {})",
            info.vendor_id(),
            info.product_id(),
            ordinals.len(),
            crate::descriptor::describe_waveforms(&ordinals)
        );
        log::debug!(
            "haptics: trigger report {} ({} bits), trigger {:?}, intensity {:?} up to {}, cutoff {:?} from {}",
            layout.trigger_report_id,
            layout.trigger_report_bits,
            layout.trigger,
            layout.intensity,
            layout.intensity_max,
            layout.cutoff,
            layout.cutoff_min
        );
        let wake = crate::wake::Wake::find(api, info.vendor_id(), info.product_id());
        Some(Self { device, layout, ordinals, wake })
    }
}
