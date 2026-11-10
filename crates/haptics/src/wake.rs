//! Waking a haptic mouse behind a Logitech receiver before a pulse (see [`crate::hidpp`]).

use crate::hidpp::{
    DEVICE_INDICES, HAPTICS_FEATURE, HIDPP_LONG_USAGE, HIDPP_USAGE_PAGE, LOGITECH_VENDOR_ID, LONG_REPORT_LEN, feature_reply, get_feature, ping, ping_answered,
};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

/// How long to wait for the device that owns the haptics feature to answer. Every paired device
/// is asked at once; a keyboard was seen answering after half a second.
const FIND_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a ping may hold a pulse back. The mouse answers in a few milliseconds when it is
/// awake and later when its link has to come up; past this the pulse goes anyway, as it would
/// without waking.
const PING_TIMEOUT: Duration = Duration::from_millis(1000);
/// Input reports read while waiting for ours (other programs' replies, events). The wait is
/// bounded by its timeout; this only bounds a flood.
const MAX_READS: usize = 512;
/// Old input reports dropped before a ping: everything the receiver sent since the last one.
const MAX_STALE: usize = 512;

/// The receiver's HID++ collection and the index of the haptic mouse on it.
pub struct Wake {
    device: HidDevice,
    index: u8,
    /// The tag of the last ping (see [`ping`]).
    tag: std::cell::Cell<u8>,
}

impl Wake {
    /// The HID++ link to the haptic mouse behind the receiver `vendor:product`, when that is a
    /// Logitech receiver and the mouse is paired to it. `None` otherwise: nothing to wake.
    pub fn find(api: &HidApi, vendor: u16, product: u16) -> Option<Self> {
        if vendor != LOGITECH_VENDOR_ID {
            return None;
        }
        let mut candidates = 0;
        let device = api
            .device_list()
            .filter(|info| info.vendor_id() == vendor && info.product_id() == product)
            .inspect(|info| log::debug!("haptics: receiver collection {:#06x}/{:#06x}", info.usage_page(), info.usage()))
            .filter(|info| info.usage_page() == HIDPP_USAGE_PAGE && info.usage() == HIDPP_LONG_USAGE)
            .find_map(|info| {
                candidates += 1;
                info.open_device(api).inspect_err(|e| log::debug!("haptics: can't open the receiver's HID++ collection: {e}")).ok()
            });
        let Some(device) = device else {
            log::debug!("haptics: no usable HID++ collection on the receiver ({candidates} found)");
            return None;
        };
        let Some(index) = haptic_device(&device) else {
            log::debug!("haptics: no device on the receiver answered with the haptics feature");
            return None;
        };
        log::info!("haptics: waking the mouse through its Logitech receiver (device {index})");
        Some(Self { device, index, tag: std::cell::Cell::new(0) })
    }

    /// Pings the mouse and waits (briefly) for its answer, so a pulse written next isn't held on
    /// the receiver. Failing is fine: the pulse then goes as it would have anyway.
    pub fn ping(&self) {
        drain(&self.device);
        let tag = self.tag.get().wrapping_add(1);
        self.tag.set(tag);
        if self.device.write(&ping(self.index, tag)).is_err() {
            return;
        }
        let answered = wait(&self.device, PING_TIMEOUT, |report| ping_answered(report, self.index, tag).then_some(()));
        if answered.is_none() {
            log::debug!("haptics: the mouse didn't answer the wake-up ping in time");
        }
    }
}

/// The index of the paired device that has the haptics feature. Asks them all at once, then takes
/// the first that answers with it (the others answer 0, an error, or not at all).
fn haptic_device(device: &HidDevice) -> Option<u8> {
    for index in DEVICE_INDICES {
        if let Err(e) = device.write(&get_feature(index, HAPTICS_FEATURE)) {
            log::debug!("haptics: HID++ request to device {index} failed: {e}");
        }
    }
    wait(device, FIND_TIMEOUT, |report| {
        log::debug!("haptics: HID++ report {report:02x?}");
        DEVICE_INDICES.into_iter().find(|index| matches!(feature_reply(report, *index), Some(Ok(feature)) if feature != 0))
    })
}

/// Drops the input reports queued since the last read, so a wait sees only what comes next.
fn drain(device: &HidDevice) {
    let mut buffer = [0u8; LONG_REPORT_LEN];
    for _ in 0..MAX_STALE {
        match device.read_timeout(&mut buffer, 0) {
            Ok(len) if len > 0 => {}
            _ => return,
        }
    }
}

/// Reads input reports until `pick` accepts one or `timeout` passes.
fn wait<T>(device: &HidDevice, timeout: Duration, mut pick: impl FnMut(&[u8]) -> Option<T>) -> Option<T> {
    let start = Instant::now();
    let mut buffer = [0u8; LONG_REPORT_LEN];
    for _ in 0..MAX_READS {
        let left = timeout.checked_sub(start.elapsed())?;
        let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let len = device.read_timeout(&mut buffer, millis).inspect_err(|e| log::debug!("haptics: HID++ read failed: {e}")).ok()?;
        if len == 0 {
            return None;
        }
        if let Some(found) = buffer.get(..len).and_then(&mut pick) {
            return Some(found);
        }
    }
    None
}
