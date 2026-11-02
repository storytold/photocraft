//! Completion alerts (ShotoPhopEX, plan Phase 5): a short synthesized blip when a generation
//! finishes or fails, as Preferences › AI Integrations › Alerts says (`off`, `done`,
//! `doneAndFailed`; off by default). The status line says what happened either way.
//!
//! The sound itself needs the `alerts` cargo feature (rodio on a background thread; the release
//! packages turn it on, CI and the web build leave it off). Without it this module still decides
//! which alert an event earns, so the choice is tested everywhere. No sound files: the two blips
//! are generated here.

use std::f32::consts::TAU;

/// Which blip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    /// Two rising notes (880 Hz, then 1320 Hz).
    Done,
    /// One low note (220 Hz).
    Failed,
}

impl Alert {
    pub fn key(self) -> &'static str {
        match self {
            Alert::Done => "done",
            Alert::Failed => "failed",
        }
    }
}

/// The alert a generation's end earns under the preference: `done` plays for results,
/// `doneAndFailed` also for failures; a cancel never plays.
pub fn for_outcome(pref: &str, done: bool, failed: bool) -> Option<Alert> {
    match (pref, done, failed) {
        ("done" | "doneAndFailed", true, _) => Some(Alert::Done),
        ("doneAndFailed", _, true) => Some(Alert::Failed),
        _ => None,
    }
}

/// The blip as mono samples at `rate` Hz: sines with 5-ms fades at a quarter of full scale.
pub fn samples(alert: Alert, rate: u32) -> Vec<f32> {
    let notes: &[(f32, f32)] = match alert {
        Alert::Done => &[(880.0, 0.090), (1320.0, 0.090)],
        Alert::Failed => &[(220.0, 0.160)],
    };
    let rate = rate.max(8000) as f32;
    let mut out = Vec::new();
    for (freq, secs) in notes {
        let n = (secs * rate) as usize;
        let fade = (0.005 * rate).max(1.0);
        for i in 0..n {
            let t = i as f32 / rate;
            let edge = (i as f32 / fade).min((n - i) as f32 / fade).min(1.0);
            out.push((TAU * freq * t).sin() * 0.25 * edge);
        }
    }
    out
}

/// Play `alert` without waiting for it (a no-op without the `alerts` feature or on the web).
/// A blip still playing makes the next one wait its turn rather than overlap.
pub fn play(alert: Alert) {
    #[cfg(all(feature = "alerts", not(target_arch = "wasm32")))]
    native::play(alert);
    #[cfg(not(all(feature = "alerts", not(target_arch = "wasm32"))))]
    let _ = alert;
}

#[cfg(all(feature = "alerts", not(target_arch = "wasm32")))]
mod native {
    use std::sync::Mutex;

    use super::{Alert, samples};

    /// One blip at a time.
    static PLAYING: Mutex<()> = Mutex::new(());

    pub fn play(alert: Alert) {
        let spawned = std::thread::Builder::new().name("photocraft-alert".into()).spawn(move || {
            let Ok(_turn) = PLAYING.lock() else { return };
            // No output device (a server, a muted session): nothing to do.
            let Ok((_stream, handle)) = rodio::OutputStream::try_default() else { return };
            let Ok(sink) = rodio::Sink::try_new(&handle) else { return };
            let rate = 44_100;
            sink.append(rodio::buffer::SamplesBuffer::new(1, rate, samples(alert, rate)));
            sink.sleep_until_end();
        });
        if let Err(e) = spawned {
            log::warn!("alert sound: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preference_decides_which_outcomes_play() {
        assert_eq!(for_outcome("off", true, false), None);
        assert_eq!(for_outcome("done", true, false), Some(Alert::Done));
        assert_eq!(for_outcome("done", false, true), None, "failures stay quiet under `done`");
        assert_eq!(for_outcome("doneAndFailed", false, true), Some(Alert::Failed));
        assert_eq!(for_outcome("doneAndFailed", false, false), None, "a cancel never plays");
        assert_eq!(for_outcome("loud", true, true), None);
    }

    #[test]
    fn the_blips_are_short_quiet_and_at_their_pitch() {
        let rate = 44_100;
        let done = samples(Alert::Done, rate);
        let failed = samples(Alert::Failed, rate);
        assert_eq!(done.len(), 2 * (0.09 * rate as f32) as usize);
        assert_eq!(failed.len(), (0.16 * rate as f32) as usize);
        assert!(done.iter().chain(&failed).all(|s| s.abs() <= 0.25 + 1e-6));
        assert!(done[0].abs() < 1e-3 && failed.last().unwrap().abs() < 0.01, "faded in and out");
        // Zero crossings give the pitch: 2 per period.
        let crossings = |s: &[f32]| s.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32;
        let half = done.len() / 2;
        let low = crossings(&done[..half]) / 2.0 / 0.09;
        let high = crossings(&done[half..]) / 2.0 / 0.09;
        assert!((low - 880.0).abs() < 25.0 && (high - 1320.0).abs() < 25.0, "{low} {high}");
        assert!((crossings(&failed) / 2.0 / 0.16 - 220.0).abs() < 15.0);
        play(Alert::Done); // never blocks, never panics (silent without the feature)
    }
}
