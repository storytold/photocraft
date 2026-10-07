//! AppKit local event monitor for tablet data. The only module in the workspace that allows
//! `unsafe`: calls into AppKit's Objective-C API that objc2 can't prove sound on its own
//! (each has a `SAFETY:` comment). Everything the monitor reads goes through the pure, tested
//! mapping in [`crate::appkit`].
//!
//! A *local* monitor sees this app's events only, on the main thread, inside `-[NSApplication
//! sendEvent:]` before the event reaches winit's view, so the sample is current when the UI
//! handles the matching pointer event. The monitor passes every event through unchanged.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use crate::motion::{Aligner, Feed};
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{NSEvent, NSEventMask};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperatingSystemVersion, NSOperationQueue, NSProcessInfo};
use objc2_game_controller::{GCDevice, GCMouse, GCMouseDidConnectNotification, GCMouseDidDisconnectNotification, GCMouseInput};

use crate::appkit::{RawEvent, State};
use crate::{Error, Sample, Update, deliver};

/// Keeps the monitor installed; dropping it removes the monitor. Main-thread only (not `Send`).
pub struct Monitor {
    token: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>,
    motion: Rc<RefCell<GcMotion>>,
}

impl Monitor {
    /// Install the monitor. `callback` gets `Some(sample)` for pen events and `None` when a mouse
    /// moves; it runs on the main thread. Call on the main thread (before or after the event loop
    /// starts).
    pub fn install(callback: impl Fn(Option<Sample>) + 'static) -> Result<Self, Error> {
        Self::with_motion(callback, Feed::default())
    }

    pub fn with_motion(callback: impl Fn(Option<Sample>) + 'static, feed: Feed) -> Result<Self, Error> {
        if MainThreadMarker::new().is_none() {
            return Err(Error::NotMainThread);
        }
        let state = RefCell::new(State::default());
        let motion = Rc::new(RefCell::new(GcMotion::new(feed)));
        let events_motion = motion.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit calls a local monitor's handler with the event it is about to
            // dispatch, a valid `NSEvent` that it keeps alive for the duration of the call; we
            // only borrow it within the call.
            let e: &NSEvent = unsafe { event.as_ref() };
            let raw = read(e);
            if let Ok(mut motion) = events_motion.try_borrow_mut() {
                motion.event(e, &raw);
            }
            // The handler can't re-enter itself (AppKit runs it synchronously on the main
            // thread), but `try_borrow_mut` keeps a re-entrant call from panicking anyway.
            if let Ok(mut st) = state.try_borrow_mut()
                && let Update::Set(sample) = st.handle(&raw)
            {
                deliver(&callback, sample);
            }
            // Pass the event on unchanged.
            event.as_ptr()
        });
        let mask = NSEventMask(crate::appkit::event_mask() | (1 << 14) | (1 << 9));
        // SAFETY: the handler returns the (non-null, valid) event it was given, as the API
        // requires ("block's return must be a valid pointer or null"). We keep the block alive
        // in `Monitor` as well, although AppKit copies it.
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };
        let token = token.ok_or_else(|| Error::Platform("addLocalMonitorForEventsMatchingMask returned nil".into()))?;
        Ok(Self { token, _handler: handler, motion })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // Stop callbacks before releasing the Objective-C token, which can outlive this owner.
        if let Ok(mut motion) = self.motion.try_borrow_mut() {
            motion.stop();
        }
        // SAFETY: `token` is exactly the object `addLocalMonitorForEventsMatchingMask:handler:`
        // returned, removed once (here). `Monitor` is not `Send`, so this runs on the main thread.
        unsafe { NSEvent::removeMonitor(&self.token) };
    }
}

/// GC callbacks and OS anchors share the main queue. Receipt order is explicit; no
/// assumption is made about GC profile timestamps or delayed physical-device samples.
struct GcMotion {
    state: Rc<RefCell<Aligner>>,
    devices: Vec<(u64, Retained<GCMouse>)>,
    next_device: u64,
    devices_dirty: Rc<Cell<bool>>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    scope: Option<(usize, [f64; 2])>,
    previous: Option<[f64; 2]>,
    feed: Feed,
    available: bool,
    pressed: bool,
}
impl GcMotion {
    fn new(feed: Feed) -> Self {
        let available =
            NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion { majorVersion: 14, minorVersion: 0, patchVersion: 0 });
        let devices_dirty = Rc::new(Cell::new(true));
        let mut observers = Vec::new();
        if available {
            // SAFETY: Foundation copies the blocks. Main-queue delivery confines Rc/Cell to
            // this thread; observer tokens are retained and removed in stop().
            unsafe {
                for name in [GCMouseDidConnectNotification, GCMouseDidDisconnectNotification] {
                    let dirty = devices_dirty.clone();
                    let block = RcBlock::new(move |_: NonNull<NSNotification>| dirty.set(true));
                    observers.push(NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                        Some(name),
                        None,
                        Some(&NSOperationQueue::mainQueue()),
                        &block,
                    ));
                }
            }
        }
        Self {
            state: Rc::default(),
            devices: Vec::new(),
            next_device: 0,
            devices_dirty,
            observers,
            scope: None,
            previous: None,
            feed,
            available,
            pressed: false,
        }
    }
    fn boundary(&mut self) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.reset();
        }
        self.previous = None;
        self.scope = None;
    }
    fn reset(&mut self) {
        self.boundary();
        self.pressed = false;
        self.feed.clear();
    }
    fn stop(&mut self) {
        // SAFETY: Each input object is retained by its device; nil removes our copied callback.
        unsafe {
            for observer in self.observers.drain(..) {
                NSNotificationCenter::defaultCenter().removeObserver((*observer).as_ref());
            }
            for (_, device) in &self.devices {
                if let Some(input) = device.mouseInput() {
                    input.setMouseMovedHandler(std::ptr::null_mut());
                }
            }
        }
        self.devices.clear();
        self.reset();
    }
    fn refresh(&mut self) {
        // SAFETY: GC retained device list is valid; callbacks are explicitly dispatched on
        // the main queue and own only main-thread Rc state. Blocks are copied by the setter.
        unsafe {
            let mice = GCMouse::mice();
            let mut changed = false;
            self.devices.retain(|(_, device)| {
                if mice.iter().any(|mouse| std::ptr::eq(&*mouse, &**device)) {
                    true
                } else {
                    if let Some(input) = device.mouseInput() {
                        input.setMouseMovedHandler(std::ptr::null_mut());
                    }
                    changed = true;
                    false
                }
            });
            for mouse in &mice {
                if self.devices.len() >= 16 || self.devices.iter().any(|(_, d)| std::ptr::eq(&**d, &*mouse)) {
                    continue;
                }
                let Some(input) = mouse.mouseInput() else { continue };
                self.next_device = self.next_device.saturating_add(1);
                let id = self.next_device;
                mouse.setHandlerQueue(dispatch2::DispatchQueue::main());
                let state = self.state.clone();
                let handler = RcBlock::new(move |_input: NonNull<GCMouseInput>, dx: f32, dy: f32| {
                    if let Ok(mut state) = state.try_borrow_mut() {
                        state.push(id, [f64::from(dx), -f64::from(dy)]);
                    }
                });
                input.setMouseMovedHandler(RcBlock::as_ptr(&handler));
                self.devices.push((id, mouse));
                changed = true;
            }
            if changed {
                self.reset();
            }
        }
    }
    fn event(&mut self, e: &NSEvent, raw: &RawEvent) {
        if !self.available || self.feed.view() == 0 {
            return;
        }
        // Focus changes, exit, button boundaries and *all* pen provenance invalidate GC motion,
        // irrespective of the application's pressure preference.
        if !is_mouse(raw.kind) || raw.subtype != 0 {
            self.reset();
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(window) = e.window(mtm).filter(|w| w.isKeyWindow()) else {
            self.reset();
            return;
        };
        let Some(view) = window.contentView() else {
            self.reset();
            return;
        };
        let id = Retained::as_ptr(&view) as usize;
        if id != self.feed.view() {
            self.reset();
            return;
        }
        // Discovery runs once and on connection notifications, not on every mouse move.
        if self.devices_dirty.replace(false) {
            self.refresh();
        }
        let down = matches!(raw.kind, crate::appkit::event_type::LEFT_MOUSE_DOWN | crate::appkit::event_type::RIGHT_MOUSE_DOWN);
        if !self.pressed && !down {
            return;
        }
        let bounds = view.bounds();
        let p = view.convertPoint_fromView(e.locationInWindow(), None);
        let size = [bounds.size.width, bounds.size.height];
        let point = [p.x - bounds.origin.x, if view.isFlipped() { p.y - bounds.origin.y } else { bounds.size.height - (p.y - bounds.origin.y) }];
        let scope = (id, size);
        let moved = matches!(
            raw.kind,
            crate::appkit::event_type::MOUSE_MOVED | crate::appkit::event_type::LEFT_MOUSE_DRAGGED | crate::appkit::event_type::RIGHT_MOUSE_DRAGGED
        );
        let up = matches!(raw.kind, crate::appkit::event_type::LEFT_MOUSE_UP | crate::appkit::event_type::RIGHT_MOUSE_UP);
        if down {
            self.boundary();
            self.pressed = true;
            self.feed.press(point, u8::from(raw.kind == crate::appkit::event_type::RIGHT_MOUSE_DOWN));
        } else if up {
            self.boundary();
            self.pressed = false;
            return;
        } else if self.scope.is_some_and(|old| old != scope) {
            self.reset();
        } else if !moved {
            self.boundary();
            self.pressed = false;
        }
        // Down establishes the first anchor. Hover never accumulates/fits discarded curves.
        if !self.pressed {
            return;
        }
        let now = NSProcessInfo::processInfo().systemUptime();
        let extras = if let Ok(mut state) = self.state.try_borrow_mut() {
            state.endpoint(point, e.timestamp(), now)
        } else {
            self.previous = None;
            self.scope = None;
            self.feed.clear();
            return;
        };
        if moved
            && self.pressed
            && let Some(from) = self.previous
        {
            self.feed.publish(from, point, extras);
        }
        self.previous = Some(point);
        self.scope = Some(scope);
    }
}

impl Drop for GcMotion {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Read the tablet fields of an event. Every getter is safe for every event type: AppKit
/// returns 0 / zero points for fields an event doesn't carry (`subtype` and the proximity fields
/// are only read for the event types that define them, see [`crate::appkit`]).
fn read(e: &NSEvent) -> RawEvent {
    let kind = e.r#type().0;
    let tabletish = matches!(kind, crate::appkit::event_type::TABLET_POINT | crate::appkit::event_type::TABLET_PROXIMITY);
    let mouse = !tabletish && is_mouse(kind);
    // `subtype` raises an exception for event types without one; ask only mouse events.
    let subtype = if mouse { e.subtype().0 } else { 0 };
    let proximity = kind == crate::appkit::event_type::TABLET_PROXIMITY || (mouse && subtype == crate::appkit::subtype::TABLET_PROXIMITY);
    let point = kind == crate::appkit::event_type::TABLET_POINT || (mouse && subtype == crate::appkit::subtype::TABLET_POINT);
    let (pressure, tilt, rotation) = if point {
        let t = e.tilt();
        (e.pressure(), (t.x, t.y), e.rotation())
    } else {
        (0.0, (0.0, 0.0), 0.0)
    };
    let (device, entering) = if proximity { (e.pointingDeviceType().0, e.isEnteringProximity()) } else { (0, false) };
    RawEvent { kind, subtype, pressure, tilt, rotation, device, entering }
}

fn is_mouse(kind: usize) -> bool {
    use crate::appkit::event_type::*;
    matches!(
        kind,
        LEFT_MOUSE_DOWN
            | LEFT_MOUSE_UP
            | RIGHT_MOUSE_DOWN
            | RIGHT_MOUSE_UP
            | MOUSE_MOVED
            | LEFT_MOUSE_DRAGGED
            | RIGHT_MOUSE_DRAGGED
            | OTHER_MOUSE_DOWN
            | OTHER_MOUSE_UP
            | OTHER_MOUSE_DRAGGED
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use objc2::MainThreadOnly;
    use objc2_core_graphics::{CGEvent, CGEventField, CGEventMouseSubtype, CGEventType, CGMouseButton};
    use objc2_foundation::NSPoint;

    #[test]
    fn install_off_the_main_thread_is_an_error() {
        // `cargo test` runs tests on worker threads, never on the main thread.
        let r = std::thread::spawn(|| Monitor::install(|_| {}).err()).join().unwrap();
        assert_eq!(r, Some(Error::NotMainThread));
    }

    /// A real `NSEvent` built from a Quartz event, as the tablet driver posts them.
    fn event(kind: CGEventType, set: impl Fn(&CGEvent)) -> objc2::rc::Retained<NSEvent> {
        // Quartz builds mouse events only; other types start blank and get their type set.
        let cg = CGEvent::new_mouse_event(None, kind, NSPoint::new(10.0, 10.0), CGMouseButton::Left)
            .or_else(|| {
                let e = CGEvent::new(None)?;
                CGEvent::set_type(Some(&e), kind);
                Some(e)
            })
            .expect("CGEvent");
        set(&cg);
        NSEvent::eventWithCGEvent(&cg).expect("NSEvent")
    }

    fn int(e: &CGEvent, f: CGEventField, v: i64) {
        CGEvent::set_integer_value_field(Some(e), f, v);
    }

    fn dbl(e: &CGEvent, f: CGEventField, v: f64) {
        CGEvent::set_double_value_field(Some(e), f, v);
    }

    // Control focus without ordering/activating a window over the user's editor.
    objc2::define_class!(
        #[unsafe(super(objc2_app_kit::NSWindow))]
        #[name = "PhotoCraftMotionTestWindow"]
        #[thread_kind = MainThreadOnly]
        #[ivars = Cell<bool>]
        struct MotionTestWindow;
        impl MotionTestWindow {
            #[unsafe(method(isKeyWindow))]
            fn is_key(&self) -> bool {
                use objc2::DefinedClass;
                self.ivars().get()
            }
        }
    );

    /// Called by the harness-free integration executable: AppKit windows require the main thread.
    #[allow(dead_code)]
    pub fn bound_motion_on_main_thread() {
        use objc2::DefinedClass;
        use objc2_app_kit::{NSApplication, NSBackingStoreType, NSEventModifierFlags, NSEventType, NSWindowStyleMask};
        use objc2_foundation::{NSRect, NSSize};
        if !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion { majorVersion: 14, minorVersion: 0, patchVersion: 0 }) {
            return; // Production deliberately retains OS-only input on older macOS.
        }
        // Covers cleanup when construction succeeds but monitor installation never does.
        let dirty = objc2::rc::autoreleasepool(|_| {
            let motion = GcMotion::new(Feed::default());
            let dirty = motion.devices_dirty.clone();
            drop(motion);
            dirty
        });
        assert_eq!(Rc::strong_count(&dirty), 1, "notification observers are removed on drop");
        let mtm = MainThreadMarker::new().expect("integration main thread");
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: constructed on the main thread, retained locally, and not autoreleased on close.
        let window: Retained<MotionTestWindow> = unsafe {
            let allocated = MotionTestWindow::alloc(mtm).set_ivars(Cell::new(true));
            let window: Retained<MotionTestWindow> = objc2::msg_send![super(allocated),
                initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 200.0)),
                styleMask: NSWindowStyleMask::Titled, backing: NSBackingStoreType::Buffered, defer: false];
            window.setReleasedWhenClosed(false);
            window
        };
        assert!(window.isKeyWindow());
        let view = window.contentView().expect("content view");
        let feed = Feed::default();
        feed.bind_view(Retained::as_ptr(&view) as usize);
        let monitor = Monitor::with_motion(|_| {}, feed.clone()).expect("bound monitor");
        let send = |kind, point: [f64; 2], age| {
            let bounds = view.bounds();
            let point = NSPoint::new(point[0], if view.isFlipped() { point[1] } else { bounds.size.height - point[1] });
            let location = view.convertPoint_toView(point, None);
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                location,
                NSEventModifierFlags::empty(),
                NSProcessInfo::processInfo().systemUptime() - age,
                window.windowNumber(),
                None,
                1,
                1,
                1.0,
            )
            .expect("window mouse event");
            app.sendEvent(&event);
        };
        let push = || {
            let state = monitor.motion.borrow().state.clone();
            state.borrow_mut().push(1, [1.0, 1.0]);
            state.borrow_mut().push(1, [1.0, -1.0]);
        };
        let (a, b) = ([10.0, 20.0], [14.0, 20.0]);
        // All native events arrive before the UI reads them, as in a single UI frame.
        send(NSEventType::LeftMouseDown, a, 0.0);
        assert!(monitor.motion.borrow().pressed);
        push();
        send(NSEventType::LeftMouseDragged, b, 0.0);
        send(NSEventType::LeftMouseUp, b, 0.0);
        feed.begin(a, 0);
        assert_eq!(feed.take(a, b), vec![[12.0, 22.0]], "native window coordinates and clock map a curve");
        assert!(feed.take(a, b).is_empty(), "consumed once");
        send(NSEventType::MouseMoved, a, 0.0);
        assert!(monitor.motion.borrow().previous.is_none(), "hover does not anchor GC motion");
        // Stale OS events cannot use otherwise valid GC candidates.
        send(NSEventType::LeftMouseDown, a, 0.0);
        push();
        send(NSEventType::LeftMouseDragged, b, 0.1);
        feed.begin(a, 0);
        assert!(feed.take(a, b).is_empty());
        // Tablet provenance clears mouse candidates independently of UI pressure settings.
        send(NSEventType::LeftMouseDown, a, 0.0);
        push();
        app.sendEvent(&event(CGEventType::LeftMouseDragged, |e| {
            int(e, CGEventField::MouseEventSubtype, i64::from(CGEventMouseSubtype::TabletPoint.0));
        }));
        assert!(!monitor.motion.borrow().pressed);
        assert!(feed.take(a, b).is_empty());
        // A non-key window cannot publish a drawing interval.
        send(NSEventType::LeftMouseDown, a, 0.0);
        push();
        window.ivars().set(false);
        assert!(!window.isKeyWindow());
        send(NSEventType::LeftMouseDragged, b, 0.0);
        assert!(!monitor.motion.borrow().pressed);
        assert!(feed.take(a, b).is_empty());
        drop(monitor);
        window.close();
    }

    /// Tablet mouse events read back through the same path as live ones.
    #[test]
    fn tablet_events_read_through_the_monitor_path() {
        let mut st = State::default();
        let drag = event(CGEventType::LeftMouseDragged, |e| {
            int(e, CGEventField::MouseEventSubtype, i64::from(CGEventMouseSubtype::TabletPoint.0));
            dbl(e, CGEventField::MouseEventPressure, 0.25);
            dbl(e, CGEventField::TabletEventPointPressure, 0.25);
            dbl(e, CGEventField::TabletEventTiltX, 0.5);
            dbl(e, CGEventField::TabletEventTiltY, -0.5);
            dbl(e, CGEventField::TabletEventRotation, 45.0);
        });
        let raw = read(&drag);
        assert_eq!((raw.kind, raw.subtype), (6, 1));
        let Update::Set(Some(s)) = st.handle(&raw) else { panic!("{raw:?}") };
        assert!((s.pressure - 0.25).abs() < 0.01, "{s:?}"); // Quartz keeps 8 bits of mouse pressure.
        assert!((s.tilt_x - 30.0).abs() < 0.1 && (s.tilt_y - 30.0).abs() < 0.1, "{s:?}");
        assert!((s.rotation - 45.0).abs() < 0.1, "{s:?}");
        assert!(!s.eraser);

        // Eraser end entering proximity, then a press with it.
        let prox = event(CGEventType::TabletProximity, |e| {
            int(e, CGEventField::TabletProximityEventPointerType, crate::appkit::device::ERASER as i64);
            int(e, CGEventField::TabletProximityEventEnterProximity, 1);
        });
        let raw = read(&prox);
        assert_eq!((raw.kind, raw.device, raw.entering), (24, 3, true));
        assert!(matches!(st.handle(&raw), Update::Set(Some(Sample { eraser: true, .. }))));

        // A plain mouse event (subtype 0) is a mouse, whatever its pressure.
        let mouse = event(CGEventType::LeftMouseDown, |e| dbl(e, CGEventField::MouseEventPressure, 0.7));
        let raw = read(&mouse);
        assert_eq!((raw.kind, raw.subtype), (1, 0));
        assert_eq!(st.handle(&raw), Update::Set(None));
    }
}
