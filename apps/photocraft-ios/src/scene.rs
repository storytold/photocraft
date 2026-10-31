//! Single-window UIKit scene bridge for winit 0.30.
//!
//! UIKit still posts application lifecycle notifications when scenes are enabled, so winit
//! receives its existing resume/suspend events. Do not synthesize duplicate notifications.
//! Remove this adapter when the pinned eframe/winit stack supports scenes itself.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::io;

use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSNumber, NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UIApplication, UICoordinateSpace, UIGestureRecognizerState, UIHoverGestureRecognizer, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession,
    UITouchType, UIView, UIWindow, UIWindowScene, UIWindowSceneDelegate,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use objc2::runtime::{AnyClass, ProtocolObject};
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_photos_ui::{PHPickerConfiguration, PHPickerConfigurationAssetRepresentationMode, PHPickerFilter, PHPickerResult};
use objc2_ui_kit::{
    UIAlertAction, UIAlertActionStyle, UIAlertController, UIAlertControllerStyle, UIDocumentPickerDelegate, UIDocumentPickerViewController, UIViewController,
};
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
use std::sync::{Arc, Mutex};

// extern_protocol also emits an undocumented internal unsafe helper trait.
#[allow(clippy::missing_safety_doc)]
mod photo_protocol {
    use super::*;
    objc2::extern_protocol!(
        /// iOS signature omitted by objc2-photos-ui 0.3.2's macOS-only controller binding.
        ///
        /// # Safety
        /// Implementations must provide the SDK's required callback on the main thread,
        /// with borrowed picker/results valid only for the duration of the callback.
        #[name = "PHPickerViewControllerDelegate"]
        pub(super) unsafe trait IosPhotosDelegate: NSObjectProtocol + MainThreadOnly {
            /// Receives the user's selection.
            ///
            /// # Safety
            /// UIKit must supply valid borrowed objects on the main thread.
            #[unsafe(method(picker:didFinishPicking:))]
            fn photos(&self, picker: &UIViewController, results: &NSArray<PHPickerResult>);
        }
    );
}
use photo_protocol::IosPhotosDelegate;

thread_local! {
    static PICK_REPLY: RefCell<Option<FileDialogReply>> = const { RefCell::new(None) };
    static PICK_DELEGATE: RefCell<Option<Retained<ImagePickerDelegate>>> = const { RefCell::new(None) };
    static PICK_SOURCE: RefCell<Option<bool>> = const { RefCell::new(None) };
}

fn finish_pick(answer: Option<FileDialogAnswer>) {
    let reply = PICK_REPLY.with(|slot| slot.borrow_mut().take());
    if let Some(reply) = reply {
        reply.send(answer);
    }
}

fn loaded_answer(result: Result<(String, Vec<u8>), String>) -> FileDialogAnswer {
    match result {
        Ok((name, bytes)) => FileDialogAnswer::Contents(name, bytes),
        Err(e) => FileDialogAnswer::Unavailable(e),
    }
}

define_class!(
    // SAFETY: No custom ivars or Drop; UIKit calls this NSObject delegate on the main thread.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "PhotoCraftImagePickerDelegate"]
    struct ImagePickerDelegate;
    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for ImagePickerDelegate {}
    // SAFETY: Exact UIKit document picker callback signatures, main thread only.
    unsafe impl UIDocumentPickerDelegate for ImagePickerDelegate {
        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            finish_pick(None);
        }
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn documents(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let path = urls.iter().next().and_then(|url| url.path()).map(|p| p.to_string());
            let Some(path) = path else {
                finish_pick(None);
                return;
            };
            let reply = PICK_REPLY.with(|slot| slot.borrow_mut().take());
            if let Some(reply) = reply {
                let shared = Arc::new(Mutex::new(Some(reply)));
                let worker = shared.clone();
                let result = std::thread::Builder::new().name("ipad-image-open".into()).spawn(move || {
                    let answer = loaded_answer(crate::picked_file::read_path(&path));
                    if let Some(reply) = worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                        reply.send(Some(answer));
                    }
                });
                if let Err(e) = result
                    && let Some(reply) = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
                {
                    reply.send(Some(FileDialogAnswer::Unavailable(e.to_string())));
                }
            }
        }
    }
    // SAFETY: The required PhotosUI callback is registered as a protocol implementation,
    // with the exact iOS SDK signature and main-thread-only objects.
    unsafe impl IosPhotosDelegate for ImagePickerDelegate {
        #[unsafe(method(picker:didFinishPicking:))]
        fn photos(&self, picker: &UIViewController, results: &NSArray<PHPickerResult>) {
            picker.dismissViewControllerAnimated_completion(true, None);
            let Some(result) = results.iter().next() else {
                finish_pick(None);
                return;
            };
            let reply = PICK_REPLY.with(|slot| slot.borrow_mut().take());
            let Some(reply) = reply else { return };
            let shared = Arc::new(Mutex::new(Some(reply)));
            let handler = block2::RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
                // SAFETY: NSItemProvider borrows URL/error for the completion's duration. The
                // temporary file is read here before returning, never passed to another worker.
                let result = unsafe {
                    if !error.is_null() {
                        Err("Could not download the selected photo. Check your connection and try again.".into())
                    } else {
                        url.as_ref()
                            .and_then(|u| u.path())
                            .ok_or_else(|| "No image file was returned.".to_string())
                            .and_then(|p| crate::picked_file::read_path(&p.to_string()))
                    }
                };
                if let Some(reply) = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    reply.send(Some(loaded_answer(result)));
                }
            });
            // SAFETY: SDK-defined provider API; callback owns only Send Rust state, and reads
            // the temporary URL only while valid. Compatible representation requests an image.
            unsafe {
                result.itemProvider().loadFileRepresentationForTypeIdentifier_completionHandler(&NSString::from_str("public.image"), &handler);
            }
        }
    }
);

fn picker_parent() -> Option<Retained<UIViewController>> {
    WINDOW.with(|slot| slot.borrow().as_ref().and_then(|window| window.rootViewController()))
}

pub fn show_file_dialog(request: FileDialogRequest, reply: FileDialogReply, ctx: &eframe::egui::Context) {
    let FileDialogRequest::Open { extensions, .. } = request else {
        reply.send(Some(FileDialogAnswer::Unavailable("Saving and folder selection are not yet supported on iPad.".into())));
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        reply.send(None);
        return;
    };
    let Some(parent) = picker_parent() else {
        reply.send(Some(FileDialogAnswer::Unavailable("The iPad window is not ready.".into())));
        return;
    };
    if parent.presentedViewController().is_some() {
        reply.send(Some(FileDialogAnswer::Unavailable("Close the current system dialog first.".into())));
        return;
    }
    PICK_REPLY.with(|slot| *slot.borrow_mut() = Some(reply));
    if extensions.is_some() {
        PICK_SOURCE.with(|slot| *slot.borrow_mut() = Some(false));
        ctx.request_repaint();
        return;
    }
    let alert =
        UIAlertController::alertControllerWithTitle_message_preferredStyle(Some(&NSString::from_str("Open image")), None, UIAlertControllerStyle::Alert, mtm);
    for (title, source) in [("Photos", Some(true)), ("Files", Some(false)), ("Cancel", None)] {
        let ctx = ctx.clone();
        let handler = block2::RcBlock::new(move |_: std::ptr::NonNull<UIAlertAction>| {
            if let Some(source) = source {
                PICK_SOURCE.with(|slot| *slot.borrow_mut() = Some(source));
                ctx.request_repaint();
            } else {
                finish_pick(None);
            }
        });
        alert.addAction(&UIAlertAction::actionWithTitle_style_handler(
            Some(&NSString::from_str(title)),
            if source.is_none() { UIAlertActionStyle::Cancel } else { UIAlertActionStyle::Default },
            Some(&handler),
            mtm,
        ));
    }
    alert.setModalInPresentation(true);
    parent.presentViewController_animated_completion(&alert, true, None);
}

pub fn poll_file_picker(ctx: &eframe::egui::Context) {
    if !PICK_SOURCE.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(parent) = picker_parent() else {
        PICK_SOURCE.with(|s| s.borrow_mut().take());
        finish_pick(None);
        return;
    };
    if parent.presentedViewController().is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }
    let photos = PICK_SOURCE.with(|slot| slot.borrow_mut().take()).unwrap_or(false);
    PICK_DELEGATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            // SAFETY: Inherited NSObject initializer; no ivars requiring initialization.
            *slot = Some(unsafe { msg_send![ImagePickerDelegate::alloc(mtm), init] });
        }
        let Some(delegate) = slot.as_ref() else { return };
        if photos {
            let Some(class) = AnyClass::get(c"PHPickerViewController") else {
                finish_pick(Some(FileDialogAnswer::Unavailable("Photos picker is unavailable.".into())));
                return;
            };
            // SAFETY: PhotosUI iOS SDK declares PHPickerViewController as UIViewController.
            // objc2 0.3.2 only exposes the macOS controller binding, so use its documented iOS
            // initializer/delegate selectors here. All objects stay on the main thread.
            let picker: Retained<UIViewController> = unsafe {
                let config = PHPickerConfiguration::new();
                config.setFilter(Some(&PHPickerFilter::imagesFilter()));
                config.setSelectionLimit(1);
                config.setPreferredAssetRepresentationMode(PHPickerConfigurationAssetRepresentationMode::Compatible);
                let allocated: objc2::rc::Allocated<UIViewController> = msg_send![class, alloc];
                let picker: Retained<UIViewController> = msg_send![allocated, initWithConfiguration: &*config];
                let _: () = msg_send![&*picker, setDelegate: ProtocolObject::<dyn IosPhotosDelegate>::from_ref(&**delegate)];
                picker
            };
            picker.setModalInPresentation(true);
            parent.presentViewController_animated_completion(&picker, true, None);
        } else {
            // SAFETY: Apple's immutable UTTypeData global is available on supported iPadOS.
            let types = NSArray::from_slice(&[unsafe { objc2_uniform_type_identifiers::UTTypeData }]);
            let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(UIDocumentPickerViewController::alloc(mtm), &types, true);
            picker.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
            picker.setAllowsMultipleSelection(false);
            picker.setModalInPresentation(true);
            parent.presentViewController_animated_completion(&picker, true, None);
        }
    });
}

thread_local! {
    // Retain the existing winit window across scene disconnection/reconnection. No second
    // renderer, event loop, or window is created, and UIKit objects never cross threads.
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
    static HOVER: RefCell<crate::hover::HoverInput> = RefCell::new(crate::hover::HoverInput::default());
    // UIKit gesture recognizers do not retain their target. Keep it for the app lifetime.
    static HOVER_TARGET: RefCell<Option<(Retained<PencilHoverTarget>, eframe::egui::Context)>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: No custom ivars or Drop; NSObject imposes no subclassing requirements.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "PhotoCraftPencilHoverTarget"]
    struct PencilHoverTarget;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for PencilHoverTarget {}

    impl PencilHoverTarget {
        // SAFETY: Exact target/action ABI for UIHoverGestureRecognizer, invoked on main thread.
        #[unsafe(method(pencilHover:))]
        fn pencil_hover(&self, recognizer: &UIHoverGestureRecognizer) {
            let state = recognizer.state();
            let position = if state == UIGestureRecognizerState::Began || state == UIGestureRecognizerState::Changed {
                let view = recognizer.view();
                let point = recognizer.locationInView(view.as_deref());
                Some([point.x, point.y])
            } else { None };
            HOVER.with(|slot| {
                if let Ok(mut hover) = slot.try_borrow_mut() { hover.update(position); }
            });
            HOVER_TARGET.with(|slot| {
                if let Ok(target) = slot.try_borrow()
                    && let Some((_, ctx)) = target.as_ref() { ctx.request_repaint(); }
            });
        }
    }
);

/// Register Pencil-only proximity input on the rendering view, preserving winit touch events.
pub fn install_pencil_hover(owner: &impl HasWindowHandle, ctx: &eframe::egui::Context) -> Result<(), io::Error> {
    let mtm = MainThreadMarker::new().ok_or_else(|| io::Error::other("Pencil hover must be installed on the main thread"))?;
    let handle = owner.window_handle().map_err(io::Error::other)?;
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return Err(io::Error::other("Expected UIKit rendering view"));
    };
    // SAFETY: Borrowed UIKit handle keeps this UIView alive; all access is main-thread-only.
    let view = unsafe { handle.ui_view.cast::<UIView>().as_ref() };
    // SAFETY: PencilHoverTarget inherits NSObject's init and has no ivars to initialize.
    let target: Retained<PencilHoverTarget> = unsafe { msg_send![PencilHoverTarget::alloc(mtm), init] };
    // SAFETY: Retained main-thread target implements the exact pencilHover: action above.
    let recognizer = unsafe { UIHoverGestureRecognizer::initWithTarget_action(UIHoverGestureRecognizer::alloc(mtm), Some(&target), Some(sel!(pencilHover:))) };
    recognizer.setAllowedTouchTypes(&NSArray::<NSNumber>::from_slice(&[&NSNumber::numberWithInteger(UITouchType::Pencil.0)]));
    recognizer.setCancelsTouchesInView(false);
    recognizer.setDelaysTouchesBegan(false);
    recognizer.setDelaysTouchesEnded(false);
    HOVER_TARGET.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| io::Error::other("Reentrant Pencil hover installation"))?;
        *slot = Some((target, ctx.clone()));
        Ok::<_, io::Error>(())
    })?;
    view.addGestureRecognizer(&recognizer);
    Ok(())
}

pub fn drain_pencil_hover(input: &mut eframe::egui::RawInput, zoom: f32) {
    HOVER.with(|slot| {
        if let Ok(mut hover) = slot.try_borrow_mut() {
            hover.drain(input, zoom);
        }
    });
}

define_class!(
    // SAFETY: NSObject imposes no subclassing requirements; there are no custom ivars or Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "PhotoCraftSceneDelegate"]
    struct SceneDelegate;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for SceneDelegate {}
    // SAFETY: All delegate callbacks use UIKit's exact signatures and run on the main thread.
    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
            let Some(scene) = scene.downcast_ref::<UIWindowScene>() else { return };
            let window = WINDOW.with(|slot| slot.try_borrow().ok().and_then(|w| w.clone()));
            if let Some(window) = window {
                attach(&window, scene);
            }
        }
    }
    // SAFETY: Optional window-scene methods are not required for our single-window adapter.
    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

/// Register the name referenced by Info.plist before winit enters UIApplicationMain.
pub fn register() {
    let _ = SceneDelegate::class();
    // Validate required Objective-C delegate methods during startup, not on first Open.
    let _ = ImagePickerDelegate::class();
}

/// Attach the exact window owned by eframe, rather than selecting a keyboard/system window.
pub fn attach_window(owner: &impl HasWindowHandle) -> Result<(), io::Error> {
    let mtm = MainThreadMarker::new().ok_or_else(|| io::Error::other("UIKit window must be attached on the main thread"))?;
    let handle = owner.window_handle().map_err(io::Error::other)?;
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return Err(io::Error::other("PhotoCraft expected a UIKit window"));
    };
    // SAFETY: HasWindowHandle guarantees the non-null UIKit UIView remains valid for the
    // borrowed handle's lifetime. We access it only on the main thread and do not take ownership.
    let view = unsafe { handle.ui_view.cast::<UIView>().as_ref() };
    let window = view.window().ok_or_else(|| io::Error::other("The editor view has no UIWindow"))?;
    let scenes = UIApplication::sharedApplication(mtm).connectedScenes();
    let scene =
        scenes.iter().find_map(|scene| scene.downcast::<UIWindowScene>().ok()).ok_or_else(|| io::Error::other("UIKit has not connected a window scene"))?;
    attach(&window, &scene);
    WINDOW.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| io::Error::other("Reentrant UIKit window attachment"))?;
        *slot = Some(window);
        Ok(())
    })
}

fn attach(window: &UIWindow, scene: &UIWindowScene) {
    window.setWindowScene(Some(scene));
    // iPadOS 17 is supported; effectiveGeometry is only available on newer systems.
    #[allow(deprecated)]
    window.setFrame(scene.coordinateSpace().bounds());
    window.makeKeyAndVisible();
}

/// Insets of the editor's window in UIKit points (not physical Retina pixels).
pub fn safe_area_insets() -> Option<[f64; 4]> {
    let _mtm = MainThreadMarker::new()?;
    let window = WINDOW.with(|slot| slot.try_borrow().ok().and_then(|w| w.clone()))?;
    let insets = window.safeAreaInsets();
    Some([insets.top, insets.left, insets.right, insets.bottom])
}
