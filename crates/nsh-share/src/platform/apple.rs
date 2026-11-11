//! Apple: `UIActivityViewController` on iOS, `NSSharingServicePicker` on macOS. Inbound items
//! arrive through the App Group container the app's own share-extension target writes; the
//! extension is app-side wiring, this bridge only reads it. The payload form is this bridge's
//! own contract — a property-list array of dictionaries, not receive_sharing_intent's JSON —
//! see the keys below for what the extension must write.

use std::{
    collections::HashSet,
    future::ready,
    path::PathBuf,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSMutableArray, NSString, NSURL};

#[cfg(any(target_os = "ios", test))]
use objc2_foundation::{NSDictionary, NSNumber};

#[cfg(any(target_os = "ios", test))]
use crate::{options::Rect, receive::SharedMediaType};

#[cfg(target_os = "ios")]
use std::collections::VecDeque;

#[cfg(target_os = "ios")]
use objc2::{
    AnyThread, ClassType, msg_send,
    runtime::{Bool, NSObjectProtocol},
    sel,
};
#[cfg(target_os = "ios")]
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
#[cfg(target_os = "ios")]
use objc2_foundation::{
    NSArray, NSBundle, NSError, NSNotification, NSNotificationCenter, NSUserDefaults,
};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{
    UIActivityType, UIActivityViewController, UIApplication,
    UIApplicationDidBecomeActiveNotification, UIDevice, UIUserInterfaceIdiom, UIViewController,
    UIWindowScene,
};

#[cfg(target_os = "macos")]
use std::cell::RefCell;

#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, ClassType, DefinedClass, define_class};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSSharingService, NSSharingServiceDelegate, NSSharingServicePicker,
    NSSharingServicePickerDelegate,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize};

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::SharedMedia,
    share::{ShareResult, ShareResultStatus},
};

pub(crate) struct NativeBackend;

/// Runs `work` on the main thread, without waiting for it. UIKit and AppKit sheets are
/// main-thread-only; nothing here parks a thread for their answers.
fn fire_on_main(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    // Blocks can be invoked more than once, so the once-only payload rides in a Mutex<Option>.
    let work = Mutex::new(Some(work));
    let block: RcBlock<dyn Fn()> = RcBlock::new(move || {
        let work = work.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(work) = work {
            // SAFETY: the main operation queue runs its blocks on the main thread.
            work(unsafe { MainThreadMarker::new_unchecked() });
        }
    });
    // SAFETY: the block only touches Send state it owns, which is what a queued block must be.
    unsafe {
        objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&block);
    }
}

/// The rect a popover anchors to. On iPad a missing anchor is refused: the system throws, and
/// presenting the sheet without one crashes the app.
#[cfg(any(target_os = "ios", test))]
fn anchor(origin: Option<Rect>, pad: bool) -> Result<Rect, ShareError> {
    if pad {
        origin.ok_or_else(|| ShareError::new(ErrorKind::InvalidArgument))
    } else {
        Ok(origin.unwrap_or_default())
    }
}

/// The sourceRect a popover anchors with, in the presenter view's own coordinates — share_plus
/// converts window coordinates first, which this bridge leaves to the caller's own space.
/// A zero-sized anchor is coerced to the centered 2x2 rect the reference falls back to.
#[cfg(any(target_os = "ios", test))]
fn source_rect(origin: Rect, bounds: Rect) -> Rect {
    if origin.width > 0.0 && origin.height > 0.0 {
        return origin;
    }
    Rect {
        x: bounds.x + bounds.width / 2.0 - 1.0,
        y: bounds.y + bounds.height / 2.0 - 1.0,
        width: 2.0,
        height: 2.0,
    }
}

/// The email subject the sheet carries; `share_plus` lets the title stand in for it.
fn share_subject(params: &ShareParams) -> Option<String> {
    params
        .title
        .clone()
        .or_else(|| params.subject.clone())
        .filter(|s| !s.is_empty())
}

static STAGING: AtomicU64 = AtomicU64::new(0);

/// A staging failure: a missing source is `FileNotFound`, anything else carries the OS's errno.
fn stage_error(error: std::io::Error) -> ShareError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return ShareError::new(ErrorKind::FileNotFound);
    }
    let status = error.raw_os_error().map(i64::from).unwrap_or_default();
    ShareError::new(ErrorKind::Backend).with_status(status)
}

/// The name the sheet shows: the caller's override when given, else the source's own.
fn staged_name(path: &str, name_override: Option<&str>) -> String {
    let override_name = name_override.unwrap_or_default().trim();
    if !override_name.is_empty() {
        return override_name.to_owned();
    }
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "shared".to_owned())
}

/// Keeps two staged files from overwriting each other when their names collide.
fn unique_name(name: String, used: &mut HashSet<String>) -> String {
    let mut candidate = name.clone();
    let mut attempt = 1;
    while used.contains(&candidate) {
        attempt += 1;
        candidate = format!("{name}-{attempt}");
    }
    used.insert(candidate.clone());
    candidate
}

/// Copies each listed file into a fresh temp directory under the name the sheet shows, so the
/// overrides apply and the system reads one stable copy. Apple shares files as file URLs, and
/// nothing deletes the staged copies: a target like AirDrop may still be reading them.
fn stage(params: &ShareParams) -> Result<Vec<PathBuf>, ShareError> {
    if params.files.is_empty() {
        return Ok(Vec::new());
    }
    let dir = std::env::temp_dir().join(format!(
        "nsh-share-{}-{}",
        std::process::id(),
        STAGING.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&dir).map_err(stage_error)?;
    let overrides = params
        .file_name_overrides
        .iter()
        .map(|name| Some(name.as_str()))
        .chain(std::iter::repeat(None));
    let mut used = HashSet::new();
    let mut staged = Vec::with_capacity(params.files.len());
    for (file, name_override) in params.files.iter().zip(overrides) {
        let target = dir.join(unique_name(
            staged_name(&file.path, name_override),
            &mut used,
        ));
        std::fs::copy(&file.path, &target).map_err(stage_error)?;
        staged.push(target);
    }
    Ok(staged)
}

/// The item list the sheet shares: the text, the URI, then each staged file as a file URL.
/// An unparseable URI contributes nothing, so a share of only that can end up with no items.
fn share_items(params: &ShareParams, staged: &[PathBuf]) -> Retained<NSMutableArray> {
    let items = NSMutableArray::new();
    if let Some(text) = params.text.as_deref().filter(|t| !t.is_empty()) {
        // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
        let text = unsafe { Retained::cast_unchecked::<AnyObject>(NSString::from_str(text)) };
        items.addObject(&*text);
    }
    if let Some(uri) = params.uri.as_deref().filter(|u| !u.is_empty())
        && let Some(url) = NSURL::URLWithString(&NSString::from_str(uri))
    {
        // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
        let url = unsafe { Retained::cast_unchecked::<AnyObject>(url) };
        items.addObject(&*url);
    }
    for path in staged {
        let path = NSString::from_str(&path.to_string_lossy());
        // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
        let url = unsafe { Retained::cast_unchecked::<AnyObject>(NSURL::fileURLWithPath(&path)) };
        items.addObject(&*url);
    }
    items
}

/// Refuses an empty item list: both Apple sheets abort the process on one, which would leave
/// `done` uncalled — the guard turns that into a coded answer before any sheet exists.
fn refuse_empty(count: usize) -> Result<(), ShareError> {
    if count == 0 {
        Err(ShareError::new(ErrorKind::InvalidArgument))
    } else {
        Ok(())
    }
}

// This bridge's handoff contract, deliberately not receive_sharing_intent's payload: the suite
// and key names are that reference's, but where its extension writes JSON-encoded Data, an
// extension written against this crate must store a property-list array of dictionaries under
// MEDIA_KEY, with the fields "path", "type" (image/video/text/file/url), "mimeType",
// "thumbnail", "duration" (ms) and "message"; MESSAGE_KEY holds any text the sender typed. The
// extension then opens the host app, which activates and reads the container here.
#[cfg(target_os = "ios")]
const GROUP_ID_KEY: &str = "AppGroupId";
#[cfg(target_os = "ios")]
const MEDIA_KEY: &str = "ShareKey";
#[cfg(target_os = "ios")]
const MESSAGE_KEY: &str = "ShareMessageKey";

/// One dictionary of the handoff payload as the Rust API hands it out. An entry without a path
/// is skipped, the way the reference compacts its decoded list.
#[cfg(any(target_os = "ios", test))]
fn shared_media(entry: &NSDictionary, message: Option<&str>) -> Option<SharedMedia> {
    let path = entry_str(entry, "path").filter(|p| !p.is_empty())?;
    Some(SharedMedia {
        kind: media_kind(entry_str(entry, "type").as_deref().unwrap_or("file")),
        thumbnail: entry_str(entry, "thumbnail").filter(|t| !t.is_empty()),
        duration: entry_u64(entry, "duration"),
        mime_type: entry_str(entry, "mimeType").filter(|m| !m.is_empty()),
        message: entry_str(entry, "message")
            .filter(|m| !m.is_empty())
            .or_else(|| message.map(str::to_owned)),
        path,
    })
}

/// The kind an entry names, as receive_sharing_intent spells them; an unknown kind is a file.
#[cfg(any(target_os = "ios", test))]
fn media_kind(value: &str) -> SharedMediaType {
    match value {
        "image" => SharedMediaType::Image,
        "video" => SharedMediaType::Video,
        "text" => SharedMediaType::Text,
        "url" => SharedMediaType::Url,
        _ => SharedMediaType::File,
    }
}

/// A string field of an entry, read through the untyped dictionary the container gives.
#[cfg(any(target_os = "ios", test))]
fn entry_str(entry: &NSDictionary, key: &str) -> Option<String> {
    // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
    let key = unsafe { Retained::cast_unchecked::<AnyObject>(NSString::from_str(key)) };
    entry
        .objectForKey(&key)
        .and_then(|value| value.downcast_ref::<NSString>().map(|s| s.to_string()))
}

/// A number field of an entry, the video duration in milliseconds.
#[cfg(any(target_os = "ios", test))]
fn entry_u64(entry: &NSDictionary, key: &str) -> Option<u64> {
    // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
    let key = unsafe { Retained::cast_unchecked::<AnyObject>(NSString::from_str(key)) };
    entry.objectForKey(&key).and_then(|value| {
        value
            .downcast_ref::<NSNumber>()
            .map(|n| n.doubleValue() as u64)
    })
}

/// The handoff container the share extension writes: the suite the Info.plist names, or the
/// default group under the bundle id. No suite means no extension wired, a plain launch.
#[cfg(target_os = "ios")]
fn handoff_defaults() -> Option<Retained<NSUserDefaults>> {
    let bundle = NSBundle::mainBundle();
    let group = bundle
        .objectForInfoDictionaryKey(&NSString::from_str(GROUP_ID_KEY))
        .and_then(|value| value.downcast_ref::<NSString>().map(|s| s.to_string()))
        .or_else(|| bundle.bundleIdentifier().map(|id| format!("group.{id}")))?;
    NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&NSString::from_str(&group)))
}

/// Reads the container once, then consumes it, so a later plain launch reads empty.
#[cfg(target_os = "ios")]
fn read_handoff() -> Vec<SharedMedia> {
    let Some(defaults) = handoff_defaults() else {
        return Vec::new();
    };
    let media = handoff_payload(&defaults);
    defaults.removeObjectForKey(&NSString::from_str(MEDIA_KEY));
    defaults.removeObjectForKey(&NSString::from_str(MESSAGE_KEY));
    media
}

/// The payload as it stands in the container: an array of dictionaries, or nothing.
#[cfg(target_os = "ios")]
fn handoff_payload(defaults: &NSUserDefaults) -> Vec<SharedMedia> {
    let Some(raw) = defaults.objectForKey(&NSString::from_str(MEDIA_KEY)) else {
        return Vec::new();
    };
    let Some(array) = raw.downcast_ref::<NSArray>() else {
        return Vec::new();
    };
    let message = defaults
        .stringForKey(&NSString::from_str(MESSAGE_KEY))
        .map(|m| m.to_string());
    (0..array.count())
        .filter_map(|i| {
            array
                .objectAtIndex(i)
                .downcast_ref::<NSDictionary>()
                .and_then(|entry| shared_media(entry, message.as_deref()))
        })
        .collect()
}

/// The listen state: items waiting for a waiter, the one waiter, and the activation observer.
#[cfg(target_os = "ios")]
struct Inbound {
    queue: VecDeque<SharedMedia>,
    waiter: Option<MediaDone>,
    observer: Option<Observer>,
}

#[cfg(target_os = "ios")]
static INBOUND: Mutex<Inbound> = Mutex::new(Inbound {
    queue: VecDeque::new(),
    waiter: None,
    observer: None,
});

/// The center's observation handle. Removing an observation from any thread is supported, so
/// carrying the pointer between threads is all this wrapper does.
#[cfg(target_os = "ios")]
struct Observer(Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>>);

// SAFETY: the handle is only ever handed back to NSNotificationCenter, never messaged.
#[cfg(target_os = "ios")]
unsafe impl Send for Observer {}

#[cfg(target_os = "ios")]
fn inbound() -> std::sync::MutexGuard<'static, Inbound> {
    INBOUND.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Registers the activation observation: the extension writes the container, then opens the
/// host, which activates — that activation is the moment the handoff becomes readable. Returns
/// the observation handle; the caller stores it under the inbound lock it already holds.
#[cfg(target_os = "ios")]
fn register_activation() -> Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>> {
    let block: RcBlock<dyn Fn(std::ptr::NonNull<NSNotification>)> =
        RcBlock::new(move |_note: std::ptr::NonNull<NSNotification>| {
            let items = read_handoff();
            let mut inbound = inbound();
            inbound.queue.extend(items);
            if let Some(item) = inbound.queue.pop_front() {
                if let Some(waiter) = inbound.waiter.take() {
                    drop(inbound);
                    waiter(Ok(item));
                } else {
                    inbound.queue.push_front(item);
                }
            }
        });
    // SAFETY: the block owns no state at all, so it is sendable, and it outlives the call.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(UIApplicationDidBecomeActiveNotification),
            None,
            None,
            &block,
        )
    }
}

#[cfg(target_os = "ios")]
fn start_listen_ios(done: MediaDone) -> Result<(), ShareError> {
    // One lock hold covers the check and the store, so two direct Backend callers cannot each
    // register an observer and leak the first token.
    let mut inbound = inbound();
    if inbound.observer.is_none() {
        let token = register_activation();
        inbound.observer = Some(Observer(token));
    }
    if inbound.waiter.is_some() {
        return Err(ShareError::new(ErrorKind::InProgress));
    }
    if let Some(item) = inbound.queue.pop_front() {
        drop(inbound);
        done(Ok(item));
    } else {
        inbound.waiter = Some(done);
    }
    Ok(())
}

#[cfg(target_os = "ios")]
fn stop_listen_ios() {
    let (token, waiter) = {
        let mut inbound = inbound();
        (inbound.observer.take(), inbound.waiter.take())
    };
    if let Some(token) = token {
        // SAFETY: the handle is the center's own, for exactly the block this bridge registered.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(token.0.as_ref()) };
    }
    // With the observer gone no activation can ever deliver, so a pending waiter is answered
    // now — the take keeps exactly-once, and the coded error frees the facade's gate instead
    // of leaving it busy for the life of the process.
    if let Some(waiter) = waiter {
        waiter(Err(ShareError::new(ErrorKind::Backend)));
    }
}

/// The key scene's root controller, without walking anything presented over it.
#[cfg(target_os = "ios")]
fn root_view_controller(mtm: MainThreadMarker) -> Option<Retained<UIViewController>> {
    let app = UIApplication::sharedApplication(mtm);
    for scene in app.connectedScenes().iter() {
        let Ok(scene) = scene.downcast::<UIWindowScene>() else {
            continue;
        };
        let Some(window) = scene.keyWindow() else {
            continue;
        };
        if let Some(root) = window.rootViewController() {
            return Some(root);
        }
    }
    None
}

/// Walks the presented chain to the topmost controller, which the activity sheet presents from.
#[cfg(target_os = "ios")]
fn top_of(root: Retained<UIViewController>) -> Retained<UIViewController> {
    let mut top = root;
    while let Some(presented) = top.presentedViewController() {
        top = presented;
    }
    top
}

/// Whether every controller of the chain is clear of a transition: `presentViewController` is a
/// documented no-op while a presentation or dismissal is still in flight.
#[cfg(target_os = "ios")]
fn chain_settled(root: &Retained<UIViewController>) -> bool {
    let mut current = Retained::clone(root);
    loop {
        if current.isBeingPresented() || current.isBeingDismissed() {
            return false;
        }
        match current.presentedViewController() {
            Some(next) => current = next,
            None => return true,
        }
    }
}

/// Best-effort dismissal: find the activity sheet in the presented chain and tear it down; its
/// own completion handler answers `Dismissed` through `done`.
#[cfg(target_os = "ios")]
fn dismiss_activity(mtm: MainThreadMarker) {
    let Some(mut current) = root_view_controller(mtm) else {
        return;
    };
    while let Some(presented) = current.presentedViewController() {
        if presented.isKindOfClass(UIActivityViewController::class()) {
            current.dismissViewControllerAnimated_completion(true, None);
            return;
        }
        current = presented;
    }
}

/// The system's answer to one sheet: an error carries the framework's code, a completion is
/// `Success`, and everything else is a dismissal, which is not a failure.
///
/// # Safety
///
/// UIKit's completion contract: `activity` and `error` are each null or a valid object for the
/// duration of this one call, which runs on the main thread; each pointer is read at most once.
#[cfg(target_os = "ios")]
unsafe fn activity_answer(
    activity: *mut UIActivityType,
    completed: Bool,
    error: *mut NSError,
) -> Result<ShareResult, ShareError> {
    // SAFETY: UIKit passes null or a valid NSError, and we read it once, here.
    if !error.is_null() {
        let status = unsafe { (*error).code() } as i64;
        return Err(ShareError::new(ErrorKind::Backend).with_status(status));
    }
    if completed.as_bool() {
        // SAFETY: UIKit passes null or a valid activity name, and we read it once, here.
        let raw = if activity.is_null() {
            String::new()
        } else {
            unsafe { (*activity).to_string() }
        };
        Ok(ShareResult {
            raw,
            status: ShareResultStatus::Success,
        })
    } else {
        Ok(ShareResult {
            raw: String::new(),
            status: ShareResultStatus::Dismissed,
        })
    }
}

#[cfg(target_os = "ios")]
fn present(params: &ShareParams, staged: &[PathBuf], done: ShareDone, mtm: MainThreadMarker) {
    let pad = UIDevice::currentDevice(mtm).userInterfaceIdiom() == UIUserInterfaceIdiom::Pad;
    let origin = match anchor(params.share_position_origin, pad) {
        Ok(origin) => origin,
        Err(error) => {
            done(Err(error));
            return;
        }
    };
    let Some(root) = root_view_controller(mtm) else {
        done(Err(ShareError::new(ErrorKind::NotAvailable)));
        return;
    };
    // Presenting while the chain is mid-transition is a documented no-op whose completion never
    // fires, which would leave `done` uncalled; the chain is checked and refused instead.
    if !chain_settled(&root) {
        done(Err(ShareError::new(ErrorKind::NotAvailable)));
        return;
    }
    let presenter = top_of(root);
    let items = share_items(params, staged);
    if let Err(error) = refuse_empty(items.count()) {
        done(Err(error));
        return;
    }
    // SAFETY: the items array holds NSString and NSURL objects, which the sheet accepts.
    let sheet = unsafe {
        UIActivityViewController::initWithActivityItems_applicationActivities(
            UIActivityViewController::alloc(),
            items.as_super(),
            None,
        )
    };
    if let Some(subject) = share_subject(params)
        // share_plus reaches the sheet's private subject through public KVC; mail reads it. A
        // future iOS without that private setter would raise on the KVC call and abort with
        // `done` uncalled, so the setter is probed first and the subject simply dropped.
        && sheet.respondsToSelector(sel!(setSubject:))
    {
        // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
        let subject =
            unsafe { Retained::cast_unchecked::<AnyObject>(NSString::from_str(&subject)) };
        let key = NSString::from_str("subject");
        // SAFETY: setValue:forKey: on a valid object with a constant key and an NSString value;
        // the setSubject: selector it needs was just probed to exist.
        let _: () = unsafe { msg_send![&*sheet, setValue: Some(&*subject), forKey: &*key] };
    }
    if pad {
        match (sheet.popoverPresentationController(), presenter.view()) {
            (Some(popover), Some(view)) => {
                popover.setSourceView(Some(&*view));
                let bounds = view.bounds();
                let rect = source_rect(
                    origin,
                    Rect {
                        x: bounds.origin.x,
                        y: bounds.origin.y,
                        width: bounds.size.width,
                        height: bounds.size.height,
                    },
                );
                popover.setSourceRect(CGRect::new(
                    CGPoint::new(rect.x, rect.y),
                    CGSize::new(rect.width, rect.height),
                ));
            }
            _ => {
                // An iPad popover without an anchor throws when presented; refuse instead.
                done(Err(ShareError::new(ErrorKind::NotAvailable)));
                return;
            }
        }
    }
    let done = Mutex::new(Some(done));
    let handler: RcBlock<dyn Fn(*mut UIActivityType, Bool, *mut NSArray, *mut NSError)> =
        RcBlock::new(
            move |activity: *mut UIActivityType,
                  completed: Bool,
                  _items: *mut NSArray,
                  error: *mut NSError| {
                let done = done.lock().unwrap_or_else(PoisonError::into_inner).take();
                let Some(done) = done else {
                    return;
                };
                // SAFETY: UIKit's completion passes null-or-valid pointers, once, on the main
                // thread — the contract activity_answer's own # Safety section states.
                done(unsafe { activity_answer(activity, completed, error) });
            },
        );
    // SAFETY: the pointer is our RcBlock and stays alive across the call; the sheet copies the
    // block, and the system calls it once, on the main thread.
    unsafe {
        let handler = (&*handler as *const block2::Block<_>).cast_mut();
        sheet.setCompletionWithItemsHandler(handler);
    }
    // A Retained only derefs one class level, so the sheet crosses to its superclass by hand.
    presenter.presentViewController_animated_completion(sheet.as_super(), true, None);
}

/// The picker delegate's own state, kept alive through the sheet slot until the user chooses.
#[cfg(target_os = "macos")]
struct Ivars {
    subject: Option<String>,
    picker: RefCell<Option<Retained<NSSharingServicePicker>>>,
    done: RefCell<Option<ShareDone>>,
}

// The picker's delegate: it sets the email subject on the chosen service and answers `done`
// once, when the user chooses one or dismisses the picker.
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl NSSharingServicePickerDelegate for PickerDelegate {
        #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
        fn did_choose(&self, _picker: &NSSharingServicePicker, service: Option<&NSSharingService>) {
            let answer = match service {
                Some(service) => Ok(ShareResult {
                    raw: service.title().to_string(),
                    status: ShareResultStatus::Success,
                }),
                // A nil service is the dismissal: nothing was shared, nothing failed.
                None => Ok(ShareResult {
                    raw: String::new(),
                    status: ShareResultStatus::Dismissed,
                }),
            };
            let done = self.ivars().done.borrow_mut().take();
            let mut sheet = sheet_slot();
            if sheet.as_ref().is_some_and(|current| {
                Retained::as_ptr(current).cast::<()>() == std::ptr::from_ref(self).cast::<()>()
            }) {
                *sheet = None;
            }
            drop(sheet);
            if let Some(done) = done {
                done(answer);
            }
        }

        // The protocol's MainThreadMarker parameter is left to the macro, not written here.
        #[unsafe(method_id(sharingServicePicker:delegateForSharingService:))]
        fn delegate_for(
            &self,
            _picker: &NSSharingServicePicker,
            service: &NSSharingService,
        ) -> Option<Retained<ProtocolObject<dyn NSSharingServiceDelegate>>> {
            if let Some(subject) = self.ivars().subject.as_deref() {
                service.setSubject(Some(&NSString::from_str(subject)));
            }
            // The reference hands back the service's own delegate; nil is fine here, since
            // setting the subject is this hook's whole job for this bridge.
            None
        }
    }
);

// SAFETY: AppKit calls the delegate only on the main thread, and this bridge touches its ivars
// only inside main-thread work, so the Send and Sync impls never race anything.
#[cfg(target_os = "macos")]
unsafe impl Send for PickerDelegate {}

#[cfg(target_os = "macos")]
unsafe impl Sync for PickerDelegate {}

#[cfg(target_os = "macos")]
impl PickerDelegate {
    fn new(subject: Option<String>, done: ShareDone) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            subject,
            picker: RefCell::new(None),
            done: RefCell::new(Some(done)),
        });
        // SAFETY: NSObject's init on an allocated instance with its ivars set cannot fail.
        let obj: Retained<Self> = unsafe { objc2::msg_send![super(this), init] };
        obj
    }
}

/// The one live sheet, so the picker's weak delegate survives and cancel reaches the picker.
#[cfg(target_os = "macos")]
static SHEET: Mutex<Option<Retained<PickerDelegate>>> = Mutex::new(None);

#[cfg(target_os = "macos")]
fn sheet_slot() -> std::sync::MutexGuard<'static, Option<Retained<PickerDelegate>>> {
    SHEET.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(target_os = "macos")]
fn present(params: &ShareParams, staged: &[PathBuf], done: ShareDone, mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow() else {
        done(Err(ShareError::new(ErrorKind::NotAvailable)));
        return;
    };
    let Some(view) = window.contentView() else {
        done(Err(ShareError::new(ErrorKind::NotAvailable)));
        return;
    };
    let items = share_items(params, staged);
    if let Err(error) = refuse_empty(items.count()) {
        done(Err(error));
        return;
    }
    let delegate = PickerDelegate::new(share_subject(params), done);
    // SAFETY: the items array holds NSString and NSURL objects, which the picker accepts.
    let picker = unsafe {
        NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), items.as_super())
    };
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    delegate.ivars().picker.replace(Some(picker.clone()));
    *sheet_slot() = Some(delegate);
    // The rect is in the view's own coordinates; without an origin the picker anchors at the
    // view's corner, the same default share_plus uses.
    let rect = params.share_position_origin.unwrap_or_default();
    picker.showRelativeToRect_ofView_preferredEdge(
        NSRect::new(
            NSPoint::new(rect.x, rect.y),
            NSSize::new(rect.width, rect.height),
        ),
        &view,
        NSRectEdge::MaxY,
    );
}

/// Stages and presents the sheet off the caller: staging is file IO, presenting needs the main
/// thread, and every outcome reaches the caller through `done`, so nothing blocks.
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn start_sheet(params: ShareParams, done: ShareDone) -> Result<(), ShareError> {
    std::thread::Builder::new()
        .spawn(move || match stage(&params) {
            Ok(staged) => fire_on_main(move |mtm| present(&params, &staged, done, mtm)),
            Err(error) => done(Err(error)),
        })
        .map_err(|_| ShareError::new(ErrorKind::Backend))
        .map(|_| ())
}

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        // No main-queue round trip: a host whose executor runs on the main thread would never
        // service the answer block. The frameworks exist on both targets, and whether a sheet
        // can show right now is answered at presentation, through `done` as NotAvailable.
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        return Box::pin(ready(Ok(true)));
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        return Box::pin(ready(Ok::<bool, ShareError>(false)));
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        // Receiving is the iOS share-extension handoff; macOS has no inbound surface.
        #[cfg(target_os = "ios")]
        return Box::pin(ready(Ok(true)));
        #[cfg(not(target_os = "ios"))]
        return Box::pin(ready(Ok::<bool, ShareError>(false)));
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        return start_sheet(params.clone(), done);
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        {
            let _ = (params, done);
            Err(ShareError::new(ErrorKind::NotAvailable))
        }
    }

    fn cancel_share(&self) {
        #[cfg(target_os = "ios")]
        fire_on_main(dismiss_activity);
        #[cfg(target_os = "macos")]
        {
            let delegate = sheet_slot().clone();
            if let Some(delegate) = delegate {
                fire_on_main(move |_mtm| {
                    // close() answers didChoose(nil), which hands `done` Dismissed.
                    if let Some(picker) = delegate.ivars().picker.borrow().clone() {
                        picker.close();
                    }
                });
            }
        }
    }

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        // The cold-start read: the launch handoff if the extension delivered one, else empty.
        #[cfg(target_os = "ios")]
        return Box::pin(ready(Ok(read_handoff())));
        #[cfg(not(target_os = "ios"))]
        return Box::pin(ready(Ok(Vec::new())));
    }

    fn start_listen(&self, done: MediaDone) -> Result<(), ShareError> {
        #[cfg(target_os = "ios")]
        return start_listen_ios(done);
        #[cfg(not(target_os = "ios"))]
        {
            let _ = done;
            Err(ShareError::new(ErrorKind::NotAvailable))
        }
    }

    fn stop_listen(&self) {
        #[cfg(target_os = "ios")]
        stop_listen_ios();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipad_anchor_required() {
        let rect = Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        // On iPad the anchor is not optional: presenting a popover without one crashes.
        let error = anchor(None, true).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidArgument);
        assert_eq!(error.code(), "ERR_SHARE_INVALID_ARGUMENT");
        assert_eq!(anchor(Some(rect), true).unwrap(), rect);
        // Everywhere else the origin stays optional and an absent one anchors at zero.
        assert_eq!(
            anchor(None, false).unwrap(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0
            }
        );
        assert_eq!(anchor(Some(rect), false).unwrap(), rect);
    }

    #[test]
    fn staged_files_take_their_override_names() {
        assert_eq!(staged_name("/a/b/photo.png", None), "photo.png");
        assert_eq!(staged_name("/a/b/photo.png", Some("post.png")), "post.png");
        // A blank override is no override; a nameless source keeps a stable fallback.
        assert_eq!(staged_name("/a/b/photo.png", Some("  ")), "photo.png");
        assert_eq!(staged_name("/", None), "shared");
        let mut used = HashSet::new();
        assert_eq!(unique_name("a.png".to_owned(), &mut used), "a.png");
        assert_eq!(unique_name("a.png".to_owned(), &mut used), "a.png-2");
        assert_eq!(unique_name("b.png".to_owned(), &mut used), "b.png");
    }

    #[test]
    fn an_empty_item_list_is_refused_before_any_sheet_exists() {
        assert_eq!(
            refuse_empty(0).unwrap_err().kind(),
            ErrorKind::InvalidArgument
        );
        assert_eq!(
            refuse_empty(0).unwrap_err().code(),
            "ERR_SHARE_INVALID_ARGUMENT"
        );
        assert!(refuse_empty(1).is_ok());

        // The facade lets a uri-only share through, and a URI Foundation cannot parse (a space
        // in a schemed URL's authority) contributes no item: both Apple sheets abort the process
        // on an empty list, so the count is what stands between that and a coded answer.
        let bad = ShareParams {
            uri: Some("https://ex ample.com".to_owned()),
            ..Default::default()
        };
        assert_eq!(share_items(&bad, &[]).count(), 0);
        assert_eq!(
            refuse_empty(share_items(&bad, &[]).count())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidArgument
        );

        let good = ShareParams {
            uri: Some("https://nsh.dev".to_owned()),
            ..Default::default()
        };
        assert_eq!(share_items(&good, &[]).count(), 1);
        let text = ShareParams {
            text: Some("hello".to_owned()),
            ..Default::default()
        };
        assert_eq!(share_items(&text, &[]).count(), 1);
    }

    #[test]
    fn a_zero_sized_anchor_coerces_to_the_centered_fallback() {
        let solid = Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        let view = Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 600.0,
        };
        assert_eq!(source_rect(solid, view), solid);
        // A zero-sized origin anchors a 2x2 rect at the view's center, as the reference does.
        assert_eq!(
            source_rect(Rect::default(), view),
            Rect {
                x: 199.0,
                y: 299.0,
                width: 2.0,
                height: 2.0,
            }
        );
        // A flat or empty rect is just as unusable, and coerces the same way.
        assert_eq!(
            source_rect(
                Rect {
                    x: 5.0,
                    y: 5.0,
                    width: 0.0,
                    height: 9.0,
                },
                view,
            )
            .width,
            2.0
        );
    }

    #[test]
    fn handoff_entries_map_to_shared_media() {
        let ns = |s: &str| NSString::from_str(s);
        // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
        let any = |obj: Retained<NSString>| unsafe { Retained::cast_unchecked::<AnyObject>(obj) };
        let path = any(ns("/inbox/clip.mp4"));
        let kind = any(ns("video"));
        let duration =
            // SAFETY: every object is a valid AnyObject; objc2 documents this exact cast as safe.
            unsafe { Retained::cast_unchecked::<AnyObject>(NSNumber::numberWithDouble(4100.0)) };
        let k_path = ns("path");
        let k_type = ns("type");
        let k_duration = ns("duration");
        let typed = NSDictionary::<NSString, AnyObject>::from_slices(
            &[&*k_path, &*k_type, &*k_duration],
            &[&*path, &*kind, &*duration],
        );
        // SAFETY: the generic parameters are compile-time fiction over the same object.
        let entry: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(typed) };
        assert_eq!(
            shared_media(&entry, Some("look at this")),
            Some(SharedMedia {
                path: "/inbox/clip.mp4".to_owned(),
                thumbnail: None,
                duration: Some(4100),
                kind: SharedMediaType::Video,
                mime_type: None,
                message: Some("look at this".to_owned()),
            })
        );

        // An entry with no path key at all is skipped, and an unknown kind is a plain file.
        let typed = NSDictionary::<NSString, AnyObject>::from_slices(&[&*k_type], &[&*kind]);
        // SAFETY: the generic parameters are compile-time fiction over the same object.
        let entry: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(typed) };
        assert!(shared_media(&entry, None).is_none());

        let sticker = any(ns("sticker"));
        let typed = NSDictionary::<NSString, AnyObject>::from_slices(
            &[&*k_path, &*k_type],
            &[&*path, &*sticker],
        );
        // SAFETY: the generic parameters are compile-time fiction over the same object.
        let entry: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(typed) };
        assert_eq!(
            shared_media(&entry, None).map(|m| m.kind),
            Some(SharedMediaType::File)
        );
    }
}
