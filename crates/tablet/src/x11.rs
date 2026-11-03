//! XInput2 raw tablet events on X11, over a separate pure-Rust X connection (x11rb's
//! `RustConnection`, no `unsafe`, no libxcb).
//!
//! winit selects XI2 pointer events on its window for the master pointer, whose events carry the
//! valuators of whichever slave moved but whose valuator layout changes with it, and which go to
//! winit alone while a drag holds the implicit grab. Raw events (`XI_RawMotion`,
//! `XI_RawButtonPress/Release`) selected on the root window have neither problem: since XI 2.1
//! every client that selects them gets them, grabbed or not, tagged with the slave device. So
//! this thread listens to those, plus hierarchy and device changes to keep the device list
//! current, and maps them with [`crate::xi`].
//!
//! Raw events don't say which window the pen is over. That's fine for pressure: the UI only uses
//! the sample while its own canvas receives pointer events, and any other pointer moving resets
//! it to "mouse".

use std::thread::JoinHandle;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, DeviceType, XIEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::xi::{Device, State, fp3232, valuators};
use crate::{Error, Sample, Update, deliver};

/// `XIAllDevices`: select for every device, slaves included.
const ALL_DEVICES: u16 = 0;

/// Query failures can repeat on device changes; keep diagnostics bounded for this reader.
const MAX_QUERY_WARNINGS: u8 = 4;

/// Start listening on `display` (`None` = `$DISPLAY`, the display winit uses). The connection
/// and the XI2 version are checked before this returns; events are then read on a background
/// thread that calls `callback` with `Some(sample)` for pen events and `None` when another
/// pointer moves. The thread ends when the X connection closes.
pub fn spawn(display: Option<&str>, callback: impl Fn(Option<Sample>) + Send + 'static) -> Result<JoinHandle<()>, Error> {
    let (conn, screen) = RustConnection::connect(display).map_err(|e| Error::Unsupported(format!("cannot connect to the X server: {e}")))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| Error::Platform(format!("X screen {screen} not found")))?;
    // Raw events for everyone (not just the grabbing client) need XI 2.1; ask for 2.2.
    let version =
        conn.xinput_xi_query_version(2, 2).map_err(|e| Error::Platform(e.to_string()))?.reply().map_err(|e| Error::Unsupported(format!("no XInput2: {e}")))?;
    if (version.major_version, version.minor_version) < (2, 1) {
        return Err(Error::Unsupported(format!("XInput {}.{} (raw events need 2.1)", version.major_version, version.minor_version)));
    }
    let mask = XIEventMask::RAW_MOTION | XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_BUTTON_RELEASE | XIEventMask::HIERARCHY | XIEventMask::DEVICE_CHANGED;
    conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: ALL_DEVICES, mask: vec![mask] }])
        .map_err(|e| Error::Platform(e.to_string()))?
        .check()
        .map_err(|e| Error::Platform(format!("XISelectEvents: {e}")))?;
    let mut state = State::default();
    let mut query_warnings = 0;
    let devices = query_devices(&conn, &mut query_warnings);
    std::thread::Builder::new()
        .name("photocraft-tablet-x11".into())
        .spawn(move || {
            log_tablet_devices(&devices);
            state.set_devices(devices);
            run(&conn, &mut state, &callback, query_warnings);
        })
        .map_err(|e| Error::Platform(format!("cannot start the tablet thread: {e}")))
}

/// Read until the connection closes; diagnostic budgets live for this reader, not each stroke.
fn run(conn: &RustConnection, state: &mut State, callback: &dyn Fn(Option<Sample>), mut query_warnings: u8) {
    let mut reported_pressure = false;
    loop {
        let event = match conn.wait_for_event() {
            Ok(event) => event,
            Err(error) => {
                log::warn!("tablet: XInput2 reader stopped: {error}");
                break;
            }
        };
        let mut raw_ids = None;
        let update = match event {
            Event::XinputRawMotion(e) | Event::XinputRawButtonPress(e) | Event::XinputRawButtonRelease(e) => {
                raw_ids = Some((e.deviceid, e.sourceid));
                let values: Vec<f64> = e.axisvalues.iter().map(|v| fp3232(v.integral, v.frac)).collect();
                state.handle(e.deviceid, valuators(&e.valuator_mask, &values))
            }
            Event::XinputHierarchy(_) | Event::XinputDeviceChanged(_) => {
                state.set_devices(query_devices(conn, &mut query_warnings));
                Update::Keep
            }
            _ => Update::Keep,
        };
        if let Update::Set(sample) = update {
            deliver(callback, sample);
            // Hovering at zero must not hide the first pressure-bearing sample. One receipt
            // proves this reader reached the callback, without logging every event or stroke.
            if !reported_pressure
                && let (Some(sample), Some((device, source))) = (sample, raw_ids)
                && sample.pressure > 0.0
            {
                reported_pressure = true;
                log::debug!("tablet: first XInput2 pressure callback: device={device} source={source} pressure={}", sample.pressure);
            }
        }
    }
}

/// The slave pointer devices with their pen valuators (empty on query error).
fn query_devices(conn: &RustConnection, warnings: &mut u8) -> Vec<Device> {
    let reply = match conn.xinput_xi_query_device(ALL_DEVICES).map_err(|e| e.to_string()).and_then(|c| c.reply().map_err(|e| e.to_string())) {
        Ok(reply) => reply,
        Err(error) => {
            query_warning(warnings, format_args!("device query failed: {error}"));
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for info in reply.infos.iter().take(crate::xi::MAX_DEVICES) {
        if info.type_ != DeviceType::SLAVE_POINTER && info.type_ != DeviceType::FLOATING_SLAVE {
            continue;
        }
        let name = String::from_utf8_lossy(&info.name);
        let valuators = info.classes.iter().take(crate::xi::MAX_AXES).filter_map(|c| match &c.data {
            DeviceClassData::Valuator(v) => {
                let label = atom_name(conn, v.label, info.deviceid, warnings)?;
                Some((v.number, label, fp3232(v.min.integral, v.min.frac), fp3232(v.max.integral, v.max.frac)))
            }
            _ => None,
        });
        out.push(Device::new(info.deviceid, &name, valuators.collect::<Vec<_>>()));
    }
    out
}

/// Atom zero is legitimately unlabeled; failed lookups use the shared query-warning budget.
fn atom_name(conn: &RustConnection, atom: u32, device: u16, warnings: &mut u8) -> Option<String> {
    if atom == 0 {
        return None;
    }
    let reply = match conn.get_atom_name(atom).map_err(|e| e.to_string()).and_then(|c| c.reply().map_err(|e| e.to_string())) {
        Ok(reply) => reply,
        Err(error) => {
            query_warning(warnings, format_args!("label atom {atom} for device {device} could not be read: {error}"));
            return None;
        }
    };
    Some(String::from_utf8_lossy(&reply.name).into_owned())
}

/// A shared per-reader budget covers query and label errors, including hotplug refreshes.
fn query_warning(warnings: &mut u8, message: std::fmt::Arguments<'_>) {
    if *warnings >= MAX_QUERY_WARNINGS {
        return;
    }
    *warnings += 1;
    let suffix = if *warnings == MAX_QUERY_WARNINGS { "; further query diagnostics suppressed" } else { "" };
    log::warn!("tablet: XInput2 {message}{suffix}");
}

/// Log the initial pen inventory on the reader thread, after its creation succeeded.
fn log_tablet_devices(devices: &[Device]) {
    const MAX_PENS: usize = 16;
    const MAX_NAME_CHARS: usize = 128;
    let total = devices.iter().filter(|d| d.is_tablet()).count();
    let pens: Vec<_> = devices
        .iter()
        .filter(|d| d.is_tablet())
        .take(MAX_PENS)
        .map(|d| {
            let mut chars = d.name.chars();
            let mut name: String = chars.by_ref().take(MAX_NAME_CHARS).collect();
            if chars.next().is_some() {
                let _ = name.pop();
                name.push('…');
            }
            format!("{name} (id {})", d.id)
        })
        .collect();
    if pens.is_empty() {
        log::info!("tablet: XInput2 reader started; no pen devices with an Abs Pressure valuator yet");
        return;
    }
    let suffix = if total > pens.len() { format!("; {} further pen devices suppressed", total - pens.len()) } else { String::new() };
    log::info!("tablet: XInput2 reader started; pen devices: {} ({total} total){suffix}", pens.join(", "));
}
