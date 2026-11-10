//! Haptic feedback on haptic mice: a short pulse when something snaps to a guide, a slider hits
//! its end, or a selection or edit lands.
//!
//! - **Windows**: Windows 11 exposes a haptic mouse (for example the Logitech MX Master 4 on its
//!   current firmware) as a standard HID Simple Haptic Controller (Haptics page `0x0E`). PhotoCraft
//!   opens that collection with hidapi's pure-Rust backend and writes the Manual Trigger output
//!   report itself: no vendor SDK, plug-in or background service. The user's "Haptic signals"
//!   switch and intensity (Settings › Bluetooth & devices › Mouse) are honored, since Windows only
//!   applies them to its own haptics.
//!   A mouse behind a Logitech receiver is first woken with a HID++ ping ([`hidpp`]): sharing the
//!   receiver with a Logitech keyboard, it otherwise gets a pulse sent right after a key press
//!   only once it moves.
//! - **Elsewhere**: a no-op.
//!
//! The waveforms and when to use them follow Microsoft's input haptics guidance:
//! <https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/input-haptics-implementation-guide>.
//!
//! [`Haptics::play`] never blocks and never fails: a device that is missing, asleep or unplugged
//! just means no pulse.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod descriptor;
pub mod hidpp;
pub mod setting;

#[cfg(target_os = "windows")]
mod device;
#[cfg(target_os = "windows")]
mod wake;

/// The waveforms PhotoCraft plays, from the HID Haptics page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Waveform {
    /// A slider reached its end.
    Hover,
    /// An object snapped to an alignment.
    Step,
    /// A change of state: a lasso closed, Object Selection selected, a Content-Aware Move dropped.
    Success,
}

impl Waveform {
    /// The waveform's usage id on the Haptics page.
    pub fn usage(self) -> u16 {
        match self {
            Waveform::Success => 0x1009,
            Waveform::Hover => 0x1008,
            Waveform::Step => 0x1014,
        }
    }
}

/// Plays waveforms on the first haptic mouse, from a background thread so HID I/O never stalls a
/// frame. Cheap to share: clones send to the same thread.
#[derive(Clone)]
pub struct Haptics {
    #[cfg(target_os = "windows")]
    tx: std::sync::mpsc::SyncSender<Waveform>,
    /// PhotoCraft's own switch (Preferences), on top of Windows' haptic signals setting.
    #[cfg(target_os = "windows")]
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Haptics {
    /// Starts the haptics thread (Windows). The device is looked up on the first pulse.
    pub fn start() -> Self {
        #[cfg(target_os = "windows")]
        {
            // One pending pulse at most: while a write is in flight, newer pulses are dropped
            // rather than queued, so a burst never plays late.
            let (tx, rx) = std::sync::mpsc::sync_channel::<Waveform>(1);
            let spawned = std::thread::Builder::new().name("photocraft-haptics".into()).spawn(move || worker(&rx));
            if let Err(e) = spawned {
                log::warn!("haptics: no thread: {e}");
            }
            Self { tx, enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)) }
        }
        #[cfg(not(target_os = "windows"))]
        Self {}
    }

    /// Plays `waveform` at the user's haptics strength. Returns at once; does nothing without a
    /// haptic mouse, outside Windows, or with Windows' haptic signals turned off.
    pub fn play(&self, waveform: Waveform) {
        #[cfg(target_os = "windows")]
        if self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = self.tx.try_send(waveform);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = waveform;
    }
}

impl Haptics {
    /// Turns PhotoCraft's pulses on or off (Preferences), whatever Windows' setting says.
    pub fn set_enabled(&self, on: bool) {
        #[cfg(target_os = "windows")]
        self.enabled.store(on, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_os = "windows"))]
        let _ = on;
    }
}

/// How long a failed device lookup stands before a pulse looks again (a mouse paired or switched
/// on after start).
#[cfg(target_os = "windows")]
const LOOKUP_AGAIN_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(target_os = "windows")]
fn worker(rx: &std::sync::mpsc::Receiver<Waveform>) {
    let mut device: Option<device::Device> = None;
    let mut last_lookup: Option<std::time::Instant> = None;
    while let Ok(waveform) = rx.recv() {
        let setting = setting::read();
        if !setting.enabled {
            continue;
        }
        // A write that fails means the device went away (re-paired, woke from sleep): look it up
        // again at once, one time.
        for first_try in [true, false] {
            if device.is_none() && last_lookup.is_none_or(|t| t.elapsed() >= LOOKUP_AGAIN_AFTER) {
                last_lookup = Some(std::time::Instant::now());
                device = device::Device::find();
            }
            let Some(d) = device.as_ref() else { break };
            match d.play(waveform.usage(), setting.strength) {
                device::Played::Sent | device::Played::Unsupported => break,
                device::Played::Gone => {
                    device = None;
                    if first_try {
                        last_lookup = None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_usages_are_the_haptics_page_ids() {
        assert_eq!(Waveform::Hover.usage(), 0x1008);
        assert_eq!(Waveform::Success.usage(), 0x1009);
        assert_eq!(Waveform::Step.usage(), 0x1014);
    }

    #[test]
    fn playing_without_a_device_returns_at_once() {
        let haptics = Haptics::start();
        haptics.set_enabled(false);
        haptics.play(Waveform::Step);
        haptics.set_enabled(true);
        for _ in 0..100 {
            haptics.play(Waveform::Step);
        }
    }
}
