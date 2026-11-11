//! Windows: the share sheet through `DataTransferManager`, anchored to the app's own foreground
//! window. The package's `ShareCompleted` and `ShareCanceled` events answer the call — Success
//! when the share completed, Dismissed when the sheet was dismissed — and every failure on the
//! way resolves once as a `Backend` error carrying the HRESULT. Nothing can be shared into the
//! app, so receiving answers empty.

use std::{
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread,
};

use windows::{
    ApplicationModel::DataTransfer::{
        DataPackage, DataRequestedEventArgs, DataTransferManager, ShareCompletedEventArgs,
    },
    Foundation::{TypedEventHandler, Uri},
    Storage::{IStorageItem, StorageFile},
    Win32::{
        Foundation::HWND,
        System::Threading::GetCurrentProcessId,
        System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        UI::Shell::IDataTransferManagerInterop,
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
    core::{HSTRING, IInspectable, Interface, factory},
};

use windows_collections::IIterable;

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::SharedMedia,
    share::{ShareResult, ShareResultStatus},
};

pub(crate) struct NativeBackend;

/// Balances one `RoInitialize` with `RoUninitialize` on drop; each session thread joins the MTA
/// for its own lifetime instead of leaving an init reference on a pooled thread.
struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn new_multithreaded() -> Self {
        // SAFETY: Ok covers S_OK and S_FALSE and means this thread owes an RoUninitialize;
        // Err (RPC_E_CHANGED_MODE) means the thread is already in another mode, so leave it.
        Self {
            should_uninit: unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            // SAFETY: paired with the RoInitialize that set `should_uninit`.
            unsafe { RoUninitialize() };
        }
    }
}

/// What the sheet shows, resolved once up front: the caption Windows requires (`title`, then
/// `subject`, then the body, the order share_plus uses) and the body itself, where a URI stands
/// in for the text, as share_plus maps it.
#[derive(Default, Clone, PartialEq, Debug)]
struct Sheet {
    /// The sheet's caption; Windows refuses a share without one.
    title: Option<String>,
    /// The description and text payload: the URI, else the text.
    body: Option<String>,
    /// A URI handed to `SetUri` as well, when it parses as one.
    uri: Option<String>,
}

fn sheet_from_params(params: &ShareParams) -> Sheet {
    let content = |value: &Option<String>| {
        value
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let text = content(&params.text);
    let uri = content(&params.uri);
    let title = content(&params.title)
        .or_else(|| content(&params.subject))
        .or_else(|| text.clone().or_else(|| uri.clone()));
    Sheet {
        title,
        body: uri.clone().or(text),
        uri,
    }
}

/// One sheet's answer channel: the facade's callback, taken by the first terminal event, and the
/// wake-up that ends the session thread's park.
struct Session {
    done: Mutex<Option<ShareDone>>,
    wake: mpsc::Sender<()>,
}

impl Session {
    fn new(done: ShareDone, wake: mpsc::Sender<()>) -> Self {
        Self {
            done: Mutex::new(Some(done)),
            wake,
        }
    }

    /// Hands the facade its one answer, then wakes the session thread. A second call finds the
    /// callback gone and changes nothing.
    fn resolve(&self, answer: Result<ShareResult, ShareError>) {
        let done = self
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(done) = done {
            done(answer);
        }
        let _ = self.wake.send(());
    }
}

/// The answer a sheet event carries: no payload, just the disposition.
fn answered(status: ShareResultStatus) -> Result<ShareResult, ShareError> {
    Ok(ShareResult {
        raw: String::new(),
        status,
    })
}

fn backend_error(error: windows::core::Error) -> ShareError {
    ShareError::new(ErrorKind::Backend).with_status(i64::from(error.code().0))
}

/// Resolves the session with the failure and returns it, so a failed step cannot also hang the
/// call waiting for an event that will never come.
fn bail(session: &Session, error: windows::core::Error) -> windows::core::Error {
    session.resolve(Err(backend_error(error.clone())));
    error
}

/// The foreground window when this process owns it, else `None`. This crate cannot reach the
/// shell's window registry, so the app's own foreground window is the only anchor available.
fn foreground_window() -> Option<HWND> {
    // SAFETY: GetForegroundWindow only reads the window manager's state; the handle stays valid
    // while the window lives, which outlives this share.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut process = 0u32;
    // SAFETY: GetWindowThreadProcessId only writes through the out pointer, valid for the call.
    let owner = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    // SAFETY: GetCurrentProcessId only reads this process's kernel id.
    (owner != 0 && process == unsafe { GetCurrentProcessId() }).then_some(hwnd)
}

/// Stages every listed path as a read-only `StorageFile` before the sheet opens, so the
/// `DataRequested` handler only hands over prepared items inside its deadline. `join` parks this
/// session thread alone — the file providers complete without this thread pumping — never an
/// STA or the engine's thread.
fn stage_files(paths: &[String]) -> windows::core::Result<Vec<IStorageItem>> {
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        let opened = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_str()))?.join()?;
        items.push(opened.cast::<IStorageItem>()?);
    }
    Ok(items)
}

/// Fills the package the system created for the sheet: the caption, the description and text,
/// the URI when it parses, and the staged files.
fn fill_package(
    package: &DataPackage,
    sheet: &Sheet,
    items: &[IStorageItem],
) -> windows::core::Result<()> {
    let properties = package.Properties()?;
    if let Some(title) = &sheet.title {
        properties.SetTitle(&HSTRING::from(title.as_str()))?;
    }
    if let Some(body) = &sheet.body {
        properties.SetDescription(&HSTRING::from(body.as_str()))?;
        package.SetText(&HSTRING::from(body.as_str()))?;
    }
    if let Some(uri) = &sheet.uri {
        // The URI also rides as text: many targets read only the text format. A string that
        // does not parse as a URI is still shared as text, never dropped.
        if let Ok(uri) = Uri::CreateUri(&HSTRING::from(uri.as_str())) {
            package.SetUri(&uri)?;
        }
    }
    if !items.is_empty() {
        package.SetStorageItemsReadOnly(&IIterable::from(items.to_vec()))?;
    }
    Ok(())
}

/// Shows one sheet on its own thread and resolves the session exactly once. Every failure maps
/// to a coded `Backend` error with the HRESULT; the park only ends when the session has answered.
fn sheet_flow(
    session: &Arc<Session>,
    hwnd: HWND,
    sheet: &Sheet,
    paths: &[String],
    wake: mpsc::Receiver<()>,
) -> windows::core::Result<()> {
    let items = stage_files(paths)?;
    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    // SAFETY: `hwnd` was checked live and ours above; GetForWindow only copies it into the
    // manager it returns, which the caller keeps alive for the whole share.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }?;
    let requested: TypedEventHandler<DataTransferManager, DataRequestedEventArgs> =
        TypedEventHandler::new({
            let session = Arc::clone(session);
            let sheet = sheet.clone();
            let items = Arc::new(items);
            move |_, args| {
                let args = args.ok().map_err(|error| bail(&session, error))?;
                let request = args.Request().map_err(|error| bail(&session, error))?;
                let package = request.Data().map_err(|error| bail(&session, error))?;
                // A fill failure has no later event to wait for, so it resolves here and the
                // sheet is told why it got nothing.
                if let Err(error) = fill_package(&package, &sheet, &items) {
                    session.resolve(Err(backend_error(error.clone())));
                    let _ = request
                        .FailWithDisplayText(&HSTRING::from("The share could not be prepared."));
                    return Err(error);
                }
                let completed: TypedEventHandler<DataPackage, ShareCompletedEventArgs> =
                    TypedEventHandler::new({
                        let session = session.clone();
                        move |_, _| {
                            session.resolve(answered(ShareResultStatus::Success));
                            Ok(())
                        }
                    });
                // A dismissed sheet is not an error: the answer is Dismissed.
                let canceled: TypedEventHandler<DataPackage, IInspectable> =
                    TypedEventHandler::new({
                        let session = session.clone();
                        move |_, _| {
                            session.resolve(answered(ShareResultStatus::Dismissed));
                            Ok(())
                        }
                    });
                package
                    .ShareCompleted(&completed)
                    .map_err(|error| bail(&session, error))?;
                package
                    .ShareCanceled(&canceled)
                    .map_err(|error| bail(&session, error))?;
                Ok(())
            }
        });
    let token = manager.DataRequested(&requested)?;
    // SAFETY: the same live `hwnd`; ShowShareUIForWindow only reads it to pick the anchor.
    unsafe { interop.ShowShareUIForWindow(hwnd) }?;
    // The shell hosts the sheet's UI; this thread only lends the MTA the events arrive in, so it
    // parks here until the system's answer resolves the session.
    let _ = wake.recv();
    let _ = manager.RemoveDataRequested(token);
    Ok(())
}

/// The session thread: owns the MTA for one share, then exits. `start_share` has already
/// returned, so any failure of the flow resolves through the session instead.
fn run_session(
    session: &Arc<Session>,
    hwnd: HWND,
    sheet: &Sheet,
    paths: &[String],
    wake: mpsc::Receiver<()>,
) {
    let _com = ComGuard::new_multithreaded();
    if let Err(error) = sheet_flow(session, hwnd, sheet, paths, wake) {
        session.resolve(Err(backend_error(error)));
    }
}

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(async {
            // IsSupported answers for the process; the guard lends this poll's thread an MTA
            // for the factory call only.
            let _com = ComGuard::new_multithreaded();
            DataTransferManager::IsSupported().map_err(backend_error)
        })
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        // Windows has no inbound share surface in scope here.
        Box::pin(std::future::ready(Ok::<bool, ShareError>(false)))
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        // Without a window of ours in the foreground the sheet has nowhere to anchor: the
        // no-activity edge, answered before anything is shown.
        let Some(hwnd) = foreground_window() else {
            return Err(ShareError::new(ErrorKind::NoActivity));
        };
        let sheet = sheet_from_params(params);
        let paths: Vec<String> = params.files.iter().map(|file| file.path.clone()).collect();
        let (wake, wake_rx) = mpsc::channel();
        let session = Arc::new(Session::new(done, wake));
        // The session thread hosts the share off the engine's threads; the callback leaves with
        // it, so a spawn failure must still answer.
        let spawned = {
            let session = Arc::clone(&session);
            thread::Builder::new()
                .name("nsh-share-sheet".into())
                .spawn(move || run_session(&session, hwnd, &sheet, &paths, wake_rx))
        };
        match spawned {
            Ok(_) => Ok(()),
            // The thread never started, so no sheet exists; answer now rather than hang `done`.
            Err(_) => {
                session.resolve(Err(ShareError::new(ErrorKind::Backend)));
                Ok(())
            }
        }
    }

    fn cancel_share(&self) {
        // Windows' share UI has no cancel API, so this is a no-op: the system's own answer still
        // resolves the call through `done` (cancel_share_best_effort).
    }

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        // Windows has no inbound share surface in scope here.
        Box::pin(std::future::ready(Ok::<Vec<SharedMedia>, ShareError>(
            Vec::new(),
        )))
    }

    fn start_listen(&self, _done: MediaDone) -> Result<(), ShareError> {
        // Nothing can arrive from outside, so a listener could only ever hang; refusing keeps
        // the call resolved.
        Err(ShareError::new(ErrorKind::NotAvailable))
    }

    fn stop_listen(&self) {}
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    // The parent binds ShareParams only, so ShareFile comes from its own module directly; a
    // parent-level binding would sit unused in the non-test build.
    use crate::options::ShareFile;

    use super::{
        Backend, ErrorKind, NativeBackend, ShareDone, ShareError, ShareParams, Sheet,
        foreground_window, sheet_from_params,
    };

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn sheet_title_prefers_title_then_subject_then_body() {
        let both = ShareParams {
            title: Some("Title".into()),
            subject: Some("Subject".into()),
            text: Some("Text".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            sheet_from_params(&both),
            Sheet {
                title: Some("Title".into()),
                body: Some("Text".into()),
                uri: None,
            }
        );
        let subject_only = ShareParams {
            subject: Some("Subject".into()),
            text: Some("Text".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            sheet_from_params(&subject_only).title,
            Some("Subject".into())
        );
        let bare = ShareParams {
            uri: Some("https://example.com/x".into()),
            ..ShareParams::default()
        };
        // Windows refuses a share without a caption, so the body stands in as the last resort.
        assert_eq!(
            sheet_from_params(&bare).title,
            Some("https://example.com/x".into())
        );
    }

    #[test]
    fn sheet_uri_wins_over_text_as_the_body() {
        let params = ShareParams {
            text: Some("See this".into()),
            uri: Some("https://example.com/x".into()),
            ..ShareParams::default()
        };
        let sheet = sheet_from_params(&params);
        assert_eq!(sheet.body.as_deref(), Some("https://example.com/x"));
        assert_eq!(sheet.uri.as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn sheet_ignores_blank_fields() {
        let params = ShareParams {
            title: Some(String::new()),
            text: Some("   ".into()),
            ..ShareParams::default()
        };
        assert_eq!(sheet_from_params(&params), Sheet::default());
    }

    #[test]
    fn start_share_without_a_window_of_ours_answers_no_activity() {
        // The test process owns no window, so the pid check leaves no anchor to show a sheet on.
        if foreground_window().is_some() {
            return;
        }
        let backend = NativeBackend;
        let params = ShareParams {
            text: Some("Hello".into()),
            files: vec![ShareFile {
                path: "C:\\temp\\photo.png".into(),
                mime_type: None,
            }],
            ..ShareParams::default()
        };
        let (sent, received) = std::sync::mpsc::channel::<Result<_, ShareError>>();
        let done: ShareDone = Box::new(move |answer| {
            let _ = sent.send(answer);
        });
        let error = backend.start_share(&params, done).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NoActivity);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn receiving_is_off_on_windows() {
        let backend = NativeBackend;
        assert!(!run(backend.can_receive()).unwrap());
        assert!(run(backend.get_initial_media()).unwrap().is_empty());
        assert_eq!(
            backend.start_listen(Box::new(|_| {})).unwrap_err().kind(),
            ErrorKind::NotAvailable
        );
    }
}
