//! Image Capture on the main thread.
//!
//! Device delegates are documented to run on the main thread, so a private run loop on a worker
//! would miss them. [`list_devices`] and [`acquire`] pump `NSRunLoop` in short slices until the
//! delegate moves the scan forward or the timeout elapses. The browser's delegate pointer is
//! unretained: [`Session`] keeps both the browser and the delegate alive, and clears the pointer
//! before either is dropped.

#![allow(unsafe_code)]
// Image Capture's protocol methods keep Apple's names so they match the generated traits.
#![allow(non_snake_case)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class};
use objc2_foundation::{NSDate, NSError, NSRunLoop, NSString, NSURL};
use objc2_image_capture_core::{
    ICDevice, ICDeviceBrowser, ICDeviceBrowserDelegate, ICDeviceDelegate, ICDeviceLocationTypeMask, ICDeviceTypeMask, ICScannerBitDepth, ICScannerDevice,
    ICScannerDeviceDelegate, ICScannerFunctionalUnitType, ICScannerPixelDataType, ICScannerTransferMode,
};

use crate::decode::{apply_mode, decode_scan};
use crate::{Error, Mode, ScanImage, ScanRequest, Scanner};

type Model = Arc<Mutex<State>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Browse,
    Open,
    Select,
    Configure,
    Scan,
    Close,
    Done,
}

struct State {
    request: Option<ScanRequest>,
    devices: Vec<Scanner>,
    step: Step,
    error: Option<String>,
    file: Option<PathBuf>,
    pages: u32,
    dpi: u32,
    select_requested: bool,
    unit_ready: bool,
    close_requested: bool,
    local_done: bool,
    opened_id: Option<String>,
}

impl State {
    fn new(request: Option<ScanRequest>) -> Self {
        let dpi = request.as_ref().map(|r| r.resolution).unwrap_or(0);
        Self {
            request,
            devices: Vec::new(),
            step: Step::Browse,
            error: None,
            file: None,
            pages: 0,
            dpi,
            select_requested: false,
            unit_ready: false,
            close_requested: false,
            local_done: false,
            opened_id: None,
        }
    }
}

fn lock(model: &Model) -> std::sync::MutexGuard<'_, State> {
    model.lock().unwrap_or_else(PoisonError::into_inner)
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "PhotoCraftScanDelegate"]
    #[ivars = Model]
    struct ScanDelegate;

    unsafe impl NSObjectProtocol for ScanDelegate {}

    unsafe impl ICDeviceBrowserDelegate for ScanDelegate {
        #[unsafe(method(deviceBrowser:didAddDevice:moreComing:))]
        fn deviceBrowser_didAddDevice_moreComing(&self, _browser: &ICDeviceBrowser, device: &ICDevice, _more_coming: bool) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                remember(self.ivars(), device);
            }));
        }

        #[unsafe(method(deviceBrowser:didRemoveDevice:moreGoing:))]
        fn deviceBrowser_didRemoveDevice_moreGoing(&self, _browser: &ICDeviceBrowser, device: &ICDevice, _more_going: bool) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let Some(id) = device_id(device) else { return };
                lock(self.ivars()).devices.retain(|d| d.id != id);
            }));
        }

        #[unsafe(method(deviceBrowserDidEnumerateLocalDevices:))]
        fn deviceBrowserDidEnumerateLocalDevices(&self, _browser: &ICDeviceBrowser) {
            lock(self.ivars()).local_done = true;
        }
    }

    unsafe impl ICDeviceDelegate for ScanDelegate {
        #[unsafe(method(device:didOpenSessionWithError:))]
        fn device_didOpenSessionWithError(&self, _device: &ICDevice, error: Option<&NSError>) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                if state.step != Step::Open {
                    return;
                }
                match error_text(error) {
                    Some(err) => fail(&mut state, err),
                    None => state.step = Step::Select,
                }
            }));
        }

        #[unsafe(method(device:didCloseSessionWithError:))]
        fn device_didCloseSessionWithError(&self, _device: &ICDevice, error: Option<&NSError>) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                if state.error.is_none()
                    && let Some(err) = error_text(error)
                {
                    state.error = Some(err);
                }
                state.step = Step::Done;
            }));
        }

        #[unsafe(method(didRemoveDevice:))]
        fn didRemoveDevice(&self, device: &ICDevice) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                if state.opened_id.as_deref() == device_id(device).as_deref() && state.step != Step::Done && state.step != Step::Browse {
                    fail(&mut state, "the scanner was disconnected".into());
                }
            }));
        }
    }

    unsafe impl ICScannerDeviceDelegate for ScanDelegate {
        #[unsafe(method(scannerDevice:didSelectFunctionalUnit:error:))]
        fn scannerDevice_didSelectFunctionalUnit_error(
            &self,
            _scanner: &ICScannerDevice,
            _unit: &objc2_image_capture_core::ICScannerFunctionalUnit,
            error: Option<&NSError>,
        ) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                if state.step != Step::Select {
                    return;
                }
                match error_text(error) {
                    Some(err) => fail(&mut state, err),
                    None => state.unit_ready = true,
                }
            }));
        }

        #[unsafe(method(scannerDevice:didScanToURL:))]
        fn scannerDevice_didScanToURL(&self, _scanner: &ICScannerDevice, url: &NSURL) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                state.pages = state.pages.saturating_add(1);
                if state.file.is_none()
                    && let Some(path) = url_path(url)
                {
                    state.file = Some(path);
                }
            }));
        }

        #[unsafe(method(scannerDevice:didCompleteScanWithError:))]
        fn scannerDevice_didCompleteScanWithError(&self, _scanner: &ICScannerDevice, error: Option<&NSError>) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = lock(self.ivars());
                if state.step != Step::Scan && state.step != Step::Close {
                    return;
                }
                if state.error.is_none()
                    && let Some(err) = error_text(error)
                {
                    state.error = Some(err);
                }
                state.step = Step::Close;
            }));
        }
    }
);

impl ScanDelegate {
    fn new(model: Model) -> Retained<Self> {
        let this = Self::alloc().set_ivars(model);
        // SAFETY: the object was just allocated and its ivars set. `init` is NSObject's designated
        // initializer and returns the retained instance (or a substitute the runtime owns).
        unsafe { msg_send![super(this), init] }
    }
}

fn fail(state: &mut State, err: String) {
    state.error = Some(err);
    state.step = Step::Done;
}

fn remember(model: &Model, device: &ICDevice) {
    if !is_scanner(device) {
        return;
    }
    let Some(id) = device_id(device) else { return };
    let mut state = lock(model);
    if state.devices.iter().any(|d| d.id == id) {
        return;
    }
    state.devices.push(Scanner { id, name: device_name(device) });
}

fn is_scanner(device: &ICDevice) -> bool {
    // SAFETY: `type` is a plain integer getter. The value is a device type OR a location, so the
    // scanner bit is masked out rather than compared for equality.
    let kind = unsafe { device.r#type() };
    kind.0 & ICDeviceTypeMask::Scanner.0 != 0
}

fn device_id(device: &ICDevice) -> Option<String> {
    // SAFETY: `UUIDString` returns a retained string or nil. `to_string` copies it.
    let text = unsafe { device.UUIDString() }?.to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn device_name(device: &ICDevice) -> String {
    // SAFETY: `name` returns a retained string or nil.
    let name = unsafe { device.name() }.map(|s| s.to_string()).unwrap_or_default();
    if name.is_empty() { "Scanner".into() } else { name }
}

fn error_text(error: Option<&NSError>) -> Option<String> {
    let error = error?;
    let text = error.localizedDescription().to_string();
    Some(if text.is_empty() { "the scanner reported an error".into() } else { text })
}

fn url_path(url: &NSURL) -> Option<PathBuf> {
    let text = url.path()?.to_string();
    if text.is_empty() { None } else { Some(PathBuf::from(text)) }
}

struct Session {
    browser: Retained<ICDeviceBrowser>,
    delegate: Retained<ScanDelegate>,
    model: Model,
}

impl Session {
    fn start(request: Option<ScanRequest>) -> Result<Self, Error> {
        if MainThreadMarker::new().is_none() {
            return Err(Error::NotMainThread);
        }
        let model = Arc::new(Mutex::new(State::new(request)));
        let delegate = ScanDelegate::new(model.clone());
        // SAFETY: `new` is Image Capture's browser constructor. The delegate is retained by this
        // session for as long as the browser holds its unretained pointer. The mask is scanner
        // devices on the local bus and on the network (Remote covers Bonjour, shared and Bluetooth).
        let browser = unsafe { ICDeviceBrowser::new() };
        let browser_delegate: &ProtocolObject<dyn ICDeviceBrowserDelegate> = ProtocolObject::from_ref(&*delegate);
        let mask = ICDeviceTypeMask(ICDeviceTypeMask::Scanner.0 | ICDeviceLocationTypeMask::Local.0 | ICDeviceLocationTypeMask::Remote.0);
        unsafe {
            browser.setDelegate(Some(browser_delegate));
            browser.setBrowsedDeviceTypeMask(mask);
            browser.start();
        }
        Ok(Self { browser, delegate, model })
    }

    fn with_state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&lock(&self.model))
    }

    fn update<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut lock(&self.model))
    }

    fn pump_slice(&self) {
        // 50ms lets Image Capture deliver the callbacks it posts to this thread without spinning.
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        NSRunLoop::currentRunLoop().runUntilDate(&until);
    }

    fn find(&self, id: &str) -> Option<Retained<ICDevice>> {
        // SAFETY: `devices` is the browser's current array, or nil before the first add.
        let devices = unsafe { self.browser.devices() }?;
        let count = devices.count();
        for index in 0..count {
            let device = devices.objectAtIndex(index);
            if device_id(&device).as_deref() == Some(id) {
                return Some(device);
            }
        }
        None
    }

    fn open_device(&self, id: &str) -> Result<(), Error> {
        let Some(device) = self.find(id) else {
            return Err(Error::Platform("the scanner disappeared".into()));
        };
        self.update(|state| {
            state.opened_id = Some(id.to_string());
            state.step = Step::Open;
        });
        let delegate: &ProtocolObject<dyn ICDeviceDelegate> = ProtocolObject::from_ref(&*self.delegate);
        // SAFETY: the delegate outlives the open session; this session clears it in `Drop`.
        // `requestOpenSession` completes later on the main thread via `device:didOpenSessionWithError:`.
        unsafe {
            device.setDelegate(Some(delegate));
            device.requestOpenSession();
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let id = self.with_state(|state| state.opened_id.clone());
            // SAFETY: both pointers were set by this session. Clearing them before the delegate is
            // released stops Image Capture calling into a freed object. `stop` ends browsing.
            unsafe {
                if let Some(id) = id.as_deref()
                    && let Some(device) = self.find(id)
                {
                    device.setDelegate(None);
                }
                self.browser.setDelegate(None);
                self.browser.stop();
            }
        }));
    }
}

pub(crate) fn list_devices(timeout: Duration) -> Result<Vec<Scanner>, Error> {
    let session = Session::start(None)?;
    let timeout = timeout.min(Duration::from_secs(30));
    let started = Instant::now();
    loop {
        let ready = session.with_state(|state| {
            let elapsed = started.elapsed();
            elapsed >= timeout || (state.local_done && !state.devices.is_empty() && elapsed >= Duration::from_millis(400))
        });
        if ready {
            return Ok(session.with_state(|state| state.devices.clone()));
        }
        if let Some(err) = session.with_state(|state| state.error.clone()) {
            return Err(Error::Platform(err));
        }
        session.pump_slice();
    }
}

pub(crate) fn acquire(request: ScanRequest) -> Result<ScanImage, Error> {
    let timeout = request.timeout.clamp(Duration::from_secs(1), Duration::from_secs(180));
    let mode = request.mode;
    let session = Session::start(Some(request))?;
    let deadline = Instant::now() + timeout;
    let browse_deadline = Instant::now() + Duration::from_secs(8).min(timeout);
    loop {
        if Instant::now() >= deadline {
            let salvage = session.with_state(|state| state.step == Step::Close && state.file.is_some() && state.error.is_none());
            if salvage {
                break;
            }
            return Err(Error::Timeout);
        }
        advance(&session, browse_deadline)?;
        if session.with_state(|state| state.step == Step::Done) {
            break;
        }
        session.pump_slice();
    }
    finish(&session, mode)
}

fn advance(session: &Session, browse_deadline: Instant) -> Result<(), Error> {
    if let Some(err) = session.with_state(|state| state.error.clone()) {
        return Err(Error::Platform(err));
    }
    let step = session.with_state(|state| state.step);
    match step {
        Step::Browse => browse(session, browse_deadline),
        Step::Open | Step::Scan => Ok(()),
        Step::Select => select_unit(session),
        Step::Configure => configure(session),
        Step::Close => close_session(session),
        Step::Done => Ok(()),
    }
}

fn browse(session: &Session, browse_deadline: Instant) -> Result<(), Error> {
    let id = session.with_state(|state| state.request.as_ref().map(|r| r.device_id.clone()));
    let Some(id) = id else {
        return Err(Error::Platform("choose a scanner".into()));
    };
    let found = session.with_state(|state| state.devices.iter().any(|d| d.id == id));
    if found {
        return session.open_device(&id);
    }
    if Instant::now() >= browse_deadline {
        return Err(Error::Platform("that scanner is not connected".into()));
    }
    Ok(())
}

fn scanner_for(session: &Session) -> Result<Retained<ICScannerDevice>, Error> {
    let id = session.with_state(|state| state.opened_id.clone()).ok_or_else(|| Error::Platform("the scanner disappeared".into()))?;
    let device = session.find(&id).ok_or_else(|| Error::Platform("the scanner disappeared".into()))?;
    device.downcast().map_err(|_| Error::Platform("that device is not a scanner".into()))
}

fn select_unit(session: &Session) -> Result<(), Error> {
    let scanner = scanner_for(session)?;
    // SAFETY: `selectedFunctionalUnit` is the unit Image Capture already picked, and `type` is its
    // integer kind. Both are getters.
    let unit = unsafe { scanner.selectedFunctionalUnit() };
    let current = unsafe { unit.r#type() };
    let want = preferred_unit(&scanner);
    let (requested, ready) = session.with_state(|state| (state.select_requested, state.unit_ready));
    if current == want || (requested && ready) {
        session.update(|state| state.step = Step::Configure);
        return Ok(());
    }
    if !requested {
        session.update(|state| state.select_requested = true);
        // SAFETY: `requestSelectFunctionalUnit:` completes later via `scannerDevice:didSelectFunctionalUnit:error:`.
        unsafe { scanner.requestSelectFunctionalUnit(want) };
    }
    Ok(())
}

fn preferred_unit(scanner: &ICScannerDevice) -> ICScannerFunctionalUnitType {
    // SAFETY: the array is the scanner's functional-unit numbers.
    let types = unsafe { scanner.availableFunctionalUnitTypes() };
    let count = types.count();
    let mut fallback = None;
    for index in 0..count {
        let value = types.objectAtIndex(index).unsignedIntegerValue();
        if value == ICScannerFunctionalUnitType::Flatbed.0 {
            return ICScannerFunctionalUnitType::Flatbed;
        }
        if fallback.is_none() {
            fallback = Some(ICScannerFunctionalUnitType(value));
        }
    }
    fallback.unwrap_or(ICScannerFunctionalUnitType::Flatbed)
}

fn configure(session: &Session) -> Result<(), Error> {
    let scanner = scanner_for(session)?;
    let (mode, dpi) = session.with_state(|state| {
        let mode = state.request.as_ref().map(|r| r.mode).unwrap_or(Mode::Color);
        (mode, state.dpi)
    });
    // Mark the scan started before `requestScan`, which can call back into the delegate before it returns.
    session.update(|state| state.step = Step::Scan);
    if let Err(err) = apply_settings(&scanner, mode, dpi, session) {
        session.update(|state| fail(state, err.to_string()));
        return Err(err);
    }
    // SAFETY: transfer settings were just written. `requestScan` completes via `didScanToURL:` and
    // `didCompleteScanWithError:`.
    unsafe { scanner.requestScan() };
    Ok(())
}

fn apply_settings(scanner: &ICScannerDevice, mode: Mode, dpi: u32, session: &Session) -> Result<(), Error> {
    let dir = std::env::temp_dir();
    let dir_text = dir.to_string_lossy();
    let name = format!("PhotoCraftScan-{}", unique_millis());
    let directory = NSString::from_str(&dir_text);
    let document = NSString::from_str(&name);
    let uti = NSString::from_str("public.jpeg");
    // The setters copy the strings and the file URL.
    let url = NSURL::fileURLWithPath(&directory);
    unsafe {
        scanner.setTransferMode(ICScannerTransferMode::FileBased);
        scanner.setDownloadsDirectory(&url);
        scanner.setDocumentName(&document);
        scanner.setDocumentUTI(&uti);
    }
    let unit = unsafe { scanner.selectedFunctionalUnit() };
    let pixel = match mode {
        Mode::Color => ICScannerPixelDataType::RGB,
        Mode::Gray => ICScannerPixelDataType::Gray,
        Mode::BlackAndWhite => ICScannerPixelDataType::BW,
    };
    unsafe {
        unit.setPixelDataType(pixel);
        unit.setBitDepth(ICScannerBitDepth::Depth8Bits);
        if unit.bitDepth() != ICScannerBitDepth::Depth8Bits {
            unit.setBitDepth(ICScannerBitDepth::Depth1Bit);
        }
        unit.setResolution(dpi as usize);
        let actual = unit.resolution();
        if actual == 0 {
            unit.setResolution(300);
        }
        let actual = unit.resolution();
        if actual > 0 {
            session.update(|state| state.dpi = u32::try_from(actual).unwrap_or(dpi));
        }
        let size = unit.physicalSize();
        if size.width > 0.0 && size.height > 0.0 {
            unit.setScanArea(objc2_foundation::NSRect { origin: objc2_foundation::NSPoint { x: 0.0, y: 0.0 }, size });
        }
    }
    Ok(())
}

fn unique_millis() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn close_session(session: &Session) -> Result<(), Error> {
    let (already, id) = session.with_state(|state| (state.close_requested, state.opened_id.clone()));
    if already {
        return Ok(());
    }
    session.update(|state| state.close_requested = true);
    let Some(id) = id else {
        session.update(|state| state.step = Step::Done);
        return Ok(());
    };
    let Some(device) = session.find(&id) else {
        session.update(|state| state.step = Step::Done);
        return Ok(());
    };
    // SAFETY: `requestCloseSession` completes via `device:didCloseSessionWithError:`. The delegate is still alive.
    unsafe { device.requestCloseSession() };
    Ok(())
}

fn finish(session: &Session, mode: Mode) -> Result<ScanImage, Error> {
    let (file, pages, dpi, error) = session.with_state(|state| (state.file.clone(), state.pages, state.dpi, state.error.clone()));
    if let Some(err) = error {
        return Err(Error::Platform(err));
    }
    let Some(file) = file else {
        return Err(Error::Platform("the scanner finished without an image".into()));
    };
    let bytes = std::fs::read(&file).map_err(|e| Error::Platform(format!("could not read the scan ({e})")))?;
    let _ = std::fs::remove_file(&file);
    let mut image = decode_scan(&bytes)?;
    apply_mode(&mut image.rgba, mode);
    image.dpi = if dpi == 0 { 300 } else { dpi };
    image.pages = pages.max(1);
    Ok(image)
}
