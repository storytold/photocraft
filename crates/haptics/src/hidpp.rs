//! Just enough of Logitech's HID++ 2.0 to wake a mouse behind a Logitech receiver.
//!
//! A mouse that shares a receiver with a Logitech keyboard doesn't get a standard haptics output
//! report sent right after a key press until it moves again: the report waits on the receiver.
//! A HID++ request is answered by the mouse itself, so sending one first (a ping, which does
//! nothing else) brings the link up, and the pulse that follows plays at once. Seen on an
//! MX Master 4 and MX Keys sharing a Bolt receiver; a mouse on Bluetooth has no receiver to wait on.
//!
//! The messages are HID++ 2.0 long reports: report ID, device index on the receiver, feature
//! index, function and software ID (one byte), then 16 bytes of parameters.

/// Logitech's USB vendor id.
pub const LOGITECH_VENDOR_ID: u16 = 0x046D;
/// The vendor usage page of a receiver's HID++ collections.
pub const HIDPP_USAGE_PAGE: u16 = 0xFF00;
/// The HID++ collection that carries long (20-byte) reports.
pub const HIDPP_LONG_USAGE: u16 = 0x0002;

const SHORT_REPORT: u8 = 0x10;
const LONG_REPORT: u8 = 0x11;
/// A long report: report ID, device index, feature index, function and software ID, 16 parameters.
pub const LONG_REPORT_LEN: usize = 20;
/// Our software ID, so replies to other programs on the same receiver (Options+) aren't ours.
const SOFTWARE_ID: u8 = 0x0B;
/// The root feature is always at index 0.
const ROOT: u8 = 0x00;
/// Root function 0: the index of a feature on the device (0 when it doesn't have it).
const GET_FEATURE: u8 = 0;
/// Root function 1: ping, echoing its third parameter.
const PING: u8 = 1;
/// The feature that tells a haptic mouse apart from the other devices on the receiver.
pub const HAPTICS_FEATURE: u16 = 0x19B0;
/// Receivers pair up to six devices, at indices 1 to 6.
pub const DEVICE_INDICES: std::ops::RangeInclusive<u8> = 1..=6;

/// A request to the device at `index` on the receiver.
fn request(index: u8, feature: u8, function: u8, params: &[u8]) -> [u8; LONG_REPORT_LEN] {
    let mut report = [0u8; LONG_REPORT_LEN];
    report[0] = LONG_REPORT;
    report[1] = index;
    report[2] = feature;
    report[3] = (function << 4) | SOFTWARE_ID;
    for (slot, value) in report[4..].iter_mut().zip(params) {
        *slot = *value;
    }
    report
}

/// Asks the device at `index` where it keeps `feature`.
pub fn get_feature(index: u8, feature: u16) -> [u8; LONG_REPORT_LEN] {
    let [hi, lo] = feature.to_be_bytes();
    request(index, ROOT, GET_FEATURE, &[hi, lo])
}

/// A ping to the device at `index`: it answers, echoing `tag`, and does nothing else. A new tag
/// for each ping tells its answer apart from a late answer to an earlier one.
pub fn ping(index: u8, tag: u8) -> [u8; LONG_REPORT_LEN] {
    request(index, ROOT, PING, &[0, 0, tag])
}

/// What an input report says about a request we sent to `index` (with function `function` of the
/// root feature).
#[derive(Debug, PartialEq, Eq)]
pub enum Reply<'a> {
    /// The answer: its parameters.
    Answer(&'a [u8]),
    /// The device refused, or isn't there.
    Error,
    /// Some other report: another program's reply, another device, an event.
    Other,
}

/// Reads `report` (report ID first, as hidapi returns it) as a reply to our root `function` call
/// to the device at `index`.
fn reply(report: &[u8], index: u8, function: u8) -> Reply<'_> {
    let call = (function << 4) | SOFTWARE_ID;
    match report {
        // HID++ 1.0 error: 0x8F, then the refused feature index and function.
        [SHORT_REPORT, i, 0x8F, ROOT, c, ..] if *i == index && *c == call => Reply::Error,
        // HID++ 2.0 error: feature index 0xFF, then the refused one.
        [LONG_REPORT | SHORT_REPORT, i, 0xFF, ROOT, c, ..] if *i == index && *c == call => Reply::Error,
        [LONG_REPORT | SHORT_REPORT, i, ROOT, c, params @ ..] if *i == index && *c == call => Reply::Answer(params),
        _ => Reply::Other,
    }
}

/// [`reply`] to [`get_feature`]: the feature's index, 0 when the device doesn't have it.
pub fn feature_reply(report: &[u8], index: u8) -> Option<Result<u8, ()>> {
    match reply(report, index, GET_FEATURE) {
        Reply::Answer(params) => Some(Ok(params.first().copied().unwrap_or(0))),
        Reply::Error => Some(Err(())),
        Reply::Other => None,
    }
}

/// [`reply`] to [`ping`] with `tag`: true once the device answered it (or refused, which also
/// means it's awake). An echo of another tag is a late answer to an earlier ping.
pub fn ping_answered(report: &[u8], index: u8, tag: u8) -> bool {
    match reply(report, index, PING) {
        Reply::Answer(params) => params.get(2) == Some(&tag),
        Reply::Error => true,
        Reply::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_long_reports_with_our_software_id() {
        let r = get_feature(2, HAPTICS_FEATURE);
        assert_eq!(&r[..6], &[0x11, 2, 0x00, 0x0B, 0x19, 0xB0]);
        assert!(r[6..].iter().all(|b| *b == 0));
        let p = ping(3, 0x5A);
        assert_eq!(&p[..7], &[0x11, 3, 0x00, 0x1B, 0, 0, 0x5A]);
        assert_eq!(p.len(), LONG_REPORT_LEN);
    }

    #[test]
    fn reads_the_feature_index_from_the_answer() {
        let mut answer = [0u8; LONG_REPORT_LEN];
        answer[..5].copy_from_slice(&[0x11, 2, 0x00, 0x0B, 0x0D]);
        assert_eq!(feature_reply(&answer, 2), Some(Ok(0x0D)));
        // Another device, another program's software id: not ours.
        assert_eq!(feature_reply(&answer, 3), None);
        answer[3] = 0x0A;
        assert_eq!(feature_reply(&answer, 2), None);
    }

    #[test]
    fn errors_are_recognised_in_both_protocol_versions() {
        assert_eq!(feature_reply(&[0x10, 4, 0x8F, 0x00, 0x0B, 0x09, 0], 4), Some(Err(())));
        let mut long = [0u8; LONG_REPORT_LEN];
        long[..6].copy_from_slice(&[0x11, 4, 0xFF, 0x00, 0x0B, 0x05]);
        assert_eq!(feature_reply(&long, 4), Some(Err(())));
    }

    #[test]
    fn a_ping_counts_as_answered_on_a_reply_or_an_error() {
        let mut echo = [0u8; LONG_REPORT_LEN];
        echo[..7].copy_from_slice(&[0x11, 1, 0x00, 0x1B, 4, 5, 0x5A]);
        assert!(ping_answered(&echo, 1, 0x5A));
        assert!(!ping_answered(&echo, 1, 0x5B), "a late answer to an earlier ping");
        assert!(ping_answered(&[0x10, 1, 0x8F, 0x00, 0x1B, 0x09, 0], 1, 0x5B));
        assert!(!ping_answered(&echo, 2, 0x5A));
        assert!(!ping_answered(&[0x11, 1, 0x05, 0x0B, 0, 0], 1, 0x5A), "an event from another feature");
        assert!(!ping_answered(&[], 1, 0x5A));
    }
}
