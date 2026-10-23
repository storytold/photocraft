//! File drag and drop (receiving side) through `wl_data_device`.
//!
//! PhotoCraft fork: winit 0.30 has no Wayland drag and drop. A drag that offers `text/uri-list`
//! is read asynchronously (the pipe is a calloop source, so a slow source never blocks the loop)
//! and reported as the same events X11 sends: `HoveredFile` for each path once the list arrives,
//! `DroppedFile` on drop, `HoveredFileCancelled` when the drag leaves. The drag position is
//! reported as `CursorMoved`, since `wl_pointer` gets no events during a drag.

use std::io::{ErrorKind, Read};
use std::path::PathBuf;

use calloop::{PostAction, RegistrationToken};
use sctk::compositor::SurfaceData;
use sctk::data_device_manager::data_device::DataDeviceHandler;
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::DataSourceHandler;
use sctk::data_device_manager::WritePipe;
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use tracing::warn;

use crate::dpi::LogicalPosition;
use crate::event::WindowEvent;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::{self, DeviceId, WindowId};

const URI_LIST: &str = "text/uri-list";

/// A uri-list bigger than this is not a list of files someone dragged.
const MAX_URI_LIST: usize = 16 << 20;

/// The drag currently over one of our windows.
#[derive(Debug)]
pub struct DndState {
    window_id: WindowId,
    data_device: WlDataDevice,
    /// Reading the uri-list; `None` once it's read (or failed).
    reader: Option<RegistrationToken>,
    buf: Vec<u8>,
    /// The paths, once the uri-list is read.
    paths: Option<Vec<PathBuf>>,
    /// The drop happened (maybe before the list finished arriving).
    dropped: bool,
}

impl WinitState {
    fn drag_offer(data_device: &WlDataDevice) -> Option<DragOffer> {
        data_device.data::<sctk::data_device_manager::data_device::DataDeviceData>()?.drag_offer()
    }

    fn dnd_cursor_moved(&mut self, window_id: WindowId, x: f64, y: f64) {
        let scale = match self.windows.get_mut().get(&window_id) {
            Some(window) => window.lock().unwrap().scale_factor(),
            None => return,
        };
        let position = LogicalPosition::new(x, y).to_physical(scale);
        let device_id = crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId));
        self.events_sink.push_window_event(WindowEvent::CursorMoved { device_id, position }, window_id);
    }

    /// End the drag: stop reading, cancel the hover if it was shown, forget the offer.
    fn dnd_end(&mut self, cancel_hover: bool) {
        let Some(dnd) = self.dnd.take() else { return };
        if let Some(token) = dnd.reader {
            self.loop_handle.remove(token);
        }
        if cancel_hover && dnd.paths.as_ref().is_some_and(|p| !p.is_empty()) {
            self.events_sink.push_window_event(WindowEvent::HoveredFileCancelled, dnd.window_id);
        }
        if cancel_hover {
            let device_id =
                crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId));
            self.events_sink.push_window_event(WindowEvent::CursorLeft { device_id }, dnd.window_id);
        }
        self.dispatched_events = true;
    }

    /// The uri-list finished arriving: show the hover, or deliver the drop if it already happened.
    fn dnd_list_read(&mut self) {
        let Some(dnd) = self.dnd.as_mut() else { return };
        dnd.reader = None;
        let paths = parse_uri_list(&dnd.buf);
        dnd.buf = Vec::new();
        let window_id = dnd.window_id;
        for path in &paths {
            self.events_sink.push_window_event(WindowEvent::HoveredFile(path.clone()), window_id);
        }
        dnd.paths = Some(paths);
        if dnd.dropped {
            self.dnd_deliver_drop();
        }
        self.dispatched_events = true;
    }

    fn dnd_deliver_drop(&mut self) {
        let Some(dnd) = self.dnd.take() else { return };
        let offer = Self::drag_offer(&dnd.data_device);
        let paths = dnd.paths.unwrap_or_default();
        for path in paths.iter() {
            self.events_sink.push_window_event(WindowEvent::DroppedFile(path.clone()), dnd.window_id);
        }
        if let Some(offer) = offer {
            if !paths.is_empty() && offer.selected_action == DndAction::Copy {
                offer.finish();
            }
            offer.destroy();
        }
        self.dispatched_events = true;
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        data_device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &WlSurface,
    ) {
        // A new drag replaces any unfinished one.
        self.dnd_end(true);

        let parent = surface.data::<SurfaceData>().and_then(|d| d.parent_surface()).unwrap_or(surface);
        let window_id = wayland::make_wid(parent);
        if !self.windows.get_mut().contains_key(&window_id) {
            return;
        }
        let Some(offer) = Self::drag_offer(data_device) else { return };
        if !offer.with_mime_types(|m| m.iter().any(|m| m == URI_LIST)) {
            // Not files: refuse, so the source shows "no drop here".
            offer.accept_mime_type(offer.serial, None);
            offer.set_actions(DndAction::empty(), DndAction::empty());
            return;
        }
        offer.accept_mime_type(offer.serial, Some(URI_LIST.into()));
        // Copy only: a move could make the source delete the file.
        offer.set_actions(DndAction::Copy, DndAction::Copy);

        let mut dnd = DndState {
            window_id,
            data_device: data_device.clone(),
            reader: None,
            buf: Vec::new(),
            paths: None,
            dropped: false,
        };
        match offer.receive(URI_LIST.into()) {
            Ok(pipe) => {
                // Level-triggered: one read per wakeup, so the (blocking) pipe never blocks.
                let token = self.loop_handle.insert_source(pipe, |_, pipe, state: &mut WinitState| {
                    let mut chunk = [0u8; 8192];
                    // SAFETY: the fd stays open and registered; we only read from it.
                    match unsafe { pipe.get_mut() }.read(&mut chunk) {
                        Ok(0) => {
                            state.dnd_list_read();
                            PostAction::Remove
                        },
                        Ok(n) => {
                            let Some(dnd) = state.dnd.as_mut() else { return PostAction::Remove };
                            if dnd.buf.len() + n > MAX_URI_LIST {
                                warn!("drag and drop: uri-list too large, ignoring");
                                dnd.buf.clear();
                                state.dnd_list_read();
                                return PostAction::Remove;
                            }
                            dnd.buf.extend_from_slice(&chunk[..n]);
                            PostAction::Continue
                        },
                        Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) => {
                            PostAction::Continue
                        },
                        Err(e) => {
                            warn!("drag and drop: reading the uri-list failed: {e}");
                            if let Some(dnd) = state.dnd.as_mut() {
                                dnd.buf.clear();
                            }
                            state.dnd_list_read();
                            PostAction::Remove
                        },
                    }
                });
                match token {
                    Ok(token) => dnd.reader = Some(token),
                    Err(e) => warn!("drag and drop: can't watch the uri-list pipe: {e}"),
                }
            },
            Err(e) => warn!("drag and drop: can't receive the uri-list: {e}"),
        }
        if dnd.reader.is_none() {
            dnd.paths = Some(Vec::new());
        }
        self.dnd = Some(dnd);
        self.dnd_cursor_moved(window_id, x, y);
        self.dispatched_events = true;
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        // After a drop the offer lives on until the list is read and delivered.
        if self.dnd.as_ref().is_some_and(|d| d.dropped) {
            return;
        }
        self.dnd_end(true);
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        if let Some(window_id) = self.dnd.as_ref().map(|d| d.window_id) {
            self.dnd_cursor_moved(window_id, x, y);
            self.dispatched_events = true;
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(dnd) = self.dnd.as_mut() else { return };
        dnd.dropped = true;
        if dnd.paths.is_some() {
            self.dnd_deliver_drop();
        }
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        _: DndAction,
    ) {
        if self.dnd.is_some() {
            offer.set_actions(DndAction::Copy, DndAction::Copy);
        }
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl DataSourceHandler for WinitState {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        _: WritePipe,
    ) {
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

/// The local paths in a `text/uri-list` (RFC 2483): `file:` URIs, percent-decoded; comments,
/// blank lines and other schemes are skipped.
pub(crate) fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let text = String::from_utf8_lossy(data);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|uri| {
            let rest = uri.strip_prefix("file://")?;
            // `file://host/path`: the host is empty or `localhost` for local files.
            let path = if rest.starts_with('/') {
                rest
            } else {
                let slash = rest.find('/')?;
                let host = &rest[..slash];
                if !host.is_empty() && host != "localhost" && !is_this_host(host) {
                    return None;
                }
                &rest[slash..]
            };
            let bytes = percent_decode(path.as_bytes());
            Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        })
        .collect()
}

fn is_this_host(host: &str) -> bool {
    std::fs::read_to_string("/proc/sys/kernel/hostname").is_ok_and(|h| h.trim() == host)
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_paths() {
        let list = b"# comment\r\nfile:///home/me/a%20b.png\r\nfile://localhost/tmp/x.psd\r\n\
            https://example.com/c.png\r\nfile:///tmp/%D1%84.jpg\r\nfile:///tmp/bad%2\n";
        let paths = parse_uri_list(list);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/me/a b.png"),
                PathBuf::from("/tmp/x.psd"),
                PathBuf::from("/tmp/ф.jpg"),
                PathBuf::from("/tmp/bad%2"),
            ]
        );
    }
}
