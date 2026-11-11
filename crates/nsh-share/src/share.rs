//! `Share`: the calls an app makes, with one sheet at a time and one answer per call.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use tokio::sync::oneshot;

use crate::{
    backend::{Backend, ShareDone},
    error::{ErrorKind, ShareError},
    options::{Rect, ShareFile, ShareParams},
};

type Answer = Result<ShareResult, ShareError>;

/// What the system answered about one sheet, after `share_plus`'s `ShareResult`.
#[derive(PartialEq, Debug, Clone)]
pub struct ShareResult {
    /// The platform's own answer, raw: the activity type chosen, the target package, or empty.
    pub raw: String,
    /// What became of the sheet.
    pub status: ShareResultStatus,
}

/// What became of one sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShareResultStatus {
    /// The user picked a target and the system handed the share over.
    Success,
    /// The user dismissed the sheet. Not an error: nothing was shared and nothing failed.
    Dismissed,
    /// The share did not happen, for a reason the raw answer names.
    Unavailable,
}

impl ShareResultStatus {
    /// The name TSX matches on, `share_plus`'s own.
    pub fn as_str(self) -> &'static str {
        match self {
            ShareResultStatus::Success => "success",
            ShareResultStatus::Dismissed => "dismissed",
            ShareResultStatus::Unavailable => "unavailable",
        }
    }
}

/// The sheet gate. `busy` lasts until the system answers, even after the call was cancelled, so a
/// second sheet never stacks over one the system still shows.
///
/// `waiting` names the generation that owns the gate, so a late second answer from an older
/// generation cannot resolve a newer call.
#[derive(Default)]
struct Gate {
    busy: bool,
    next_id: u64,
    waiting: Option<(u64, oneshot::Sender<Answer>)>,
}

struct Shared {
    backend: Arc<dyn Backend>,
    gate: Mutex<Gate>,
}

impl Shared {
    fn gate(&self) -> MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The share sheet, as `share_plus` names it.
///
/// Clones share one sheet gate. [`Share::new`] returns the platform's shared handle, so two calls
/// in one app share it too.
#[derive(Clone)]
pub struct Share {
    shared: Arc<Shared>,
}

impl Share {
    /// The platform's share sheet, with one gate for the whole process.
    pub fn new() -> Self {
        static PLATFORM: OnceLock<Share> = OnceLock::new();
        PLATFORM
            .get_or_init(|| Self::from_arc(crate::platform::default_backend()))
            .clone()
    }

    /// Sharing through `backend`, with its own gate. Tests and fakes use this.
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self::from_arc(backend)
    }

    fn from_arc(backend: Arc<dyn Backend>) -> Self {
        Self {
            shared: Arc::new(Shared {
                backend,
                gate: Mutex::new(Gate::default()),
            }),
        }
    }

    /// Whether the platform can show a share sheet here.
    pub async fn is_available(&self) -> Result<bool, ShareError> {
        self.shared.backend.is_available().await
    }

    /// Shows the sheet with `params` and waits for the system's answer.
    ///
    /// A share needs a text, a URI or files, and plain file paths. A second call while a sheet is
    /// up gets `ERR_SHARE_IN_PROGRESS`. Dropping the future asks the system to dismiss the sheet;
    /// the call stays busy until the system answers, and that answer frees the gate.
    pub async fn share(&self, params: ShareParams) -> Result<ShareResult, ShareError> {
        validate(&params)?;
        let (sender, receiver) = oneshot::channel();
        let generation = {
            let mut gate = self.shared.gate();
            if gate.busy {
                return Err(ShareError::new(ErrorKind::InProgress));
            }
            gate.busy = true;
            let generation = gate.next_id;
            gate.next_id += 1;
            gate.waiting = Some((generation, sender));
            generation
        };
        let shared = Arc::clone(&self.shared);
        let done: ShareDone = Box::new(move |answer| answered(&shared, generation, answer));
        if let Err(error) = self.shared.backend.start_share(&params, done) {
            // Clear only our own registration: a backend that fired done before failing freed the
            // gate already, and a newer call may have taken it in the window.
            let mut gate = self.shared.gate();
            if matches!(&gate.waiting, Some((id, _)) if *id == generation) {
                gate.busy = false;
                gate.waiting = None;
            }
            return Err(error);
        }
        let mut owner = Owner {
            shared: Arc::clone(&self.shared),
            generation,
            armed: true,
        };
        let answer = receiver.await;
        owner.armed = false;
        answer.unwrap_or_else(|_| Err(ShareError::new(ErrorKind::Backend)))
    }

    /// Shares one URI, with an optional title.
    pub async fn share_uri(
        &self,
        uri: impl Into<String>,
        title: Option<String>,
    ) -> Result<ShareResult, ShareError> {
        let params = ShareParams {
            uri: Some(uri.into()),
            title,
            ..ShareParams::default()
        };
        self.share(params).await
    }

    /// Shares files, with optional text and an anchor for popover sheets.
    pub async fn share_files(
        &self,
        files: Vec<ShareFile>,
        text: Option<String>,
        origin: Option<Rect>,
    ) -> Result<ShareResult, ShareError> {
        let params = ShareParams {
            files,
            text,
            share_position_origin: origin,
            ..ShareParams::default()
        };
        self.share(params).await
    }
}

impl Default for Share {
    fn default() -> Self {
        Self::new()
    }
}

/// Refuses what no platform can share: nothing to share, a URI where a path belongs, or a missing
/// file. Runs before the gate, so a refused share leaves no state behind.
fn validate(params: &ShareParams) -> Result<(), ShareError> {
    let has_text = params.text.as_deref().is_some_and(|t| !t.is_empty());
    let has_uri = params.uri.as_deref().is_some_and(|u| !u.is_empty());
    if !has_text && !has_uri && params.files.is_empty() {
        return Err(ShareError::new(ErrorKind::InvalidArgument));
    }
    for file in &params.files {
        // Bridges stage the file and build the provider URI themselves; a URI here means the
        // caller did not stage it.
        if file.path.contains("://") || file.path.starts_with("file:") {
            return Err(ShareError::new(ErrorKind::InvalidArgument));
        }
        // The web bridge decides which paths exist; std::path is cfg'd out on wasm32 anyway.
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !std::path::Path::new(&file.path).exists() {
                return Err(ShareError::new(ErrorKind::FileNotFound));
            }
        }
    }
    Ok(())
}

/// Hands the system's answer to the waiting call, and frees the gate. An answer for a generation
/// the gate no longer holds is dropped, without touching a newer call that owns it.
fn answered(shared: &Shared, generation: u64, answer: Answer) {
    let sender = {
        let mut gate = shared.gate();
        if matches!(&gate.waiting, Some((id, _)) if *id == generation) {
            gate.busy = false;
            gate.waiting.take().map(|(_, sender)| sender)
        } else {
            None
        }
    };
    if let Some(sender) = sender {
        let _ = sender.send(answer);
    }
}

/// Asks the system to dismiss the sheet if the call's future is dropped before the answer arrives.
/// The gate stays busy: only the system's answer, which still arrives, frees it.
struct Owner {
    shared: Arc<Shared>,
    generation: u64,
    armed: bool,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if self.armed {
            let mine = {
                let gate = self.shared.gate();
                matches!(&gate.waiting, Some((id, _)) if *id == self.generation)
            };
            if mine {
                self.shared.backend.cancel_share();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        future::{Future, ready},
        pin::Pin,
        sync::{Mutex, MutexGuard},
        task::{Context, Waker},
        time::Duration,
    };

    use crate::{
        backend::BackendFuture,
        fake::FakeBackend,
        share::{ShareResult, ShareResultStatus},
    };

    /// A share call the test polls in by hand, inside the backend's window.
    type SecondCall = Pin<Box<dyn Future<Output = Result<ShareResult, ShareError>> + Send>>;

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Yields until `ready` holds, so a spawned call has reached its sheet.
    async fn settle(ready: impl Fn() -> bool) {
        for _ in 0..1000 {
            if ready() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the sheet never started");
    }

    /// Starts a share on its own task, and waits until its sheet is up.
    async fn start_share(
        share: &Share,
        fake: &FakeBackend,
        params: ShareParams,
    ) -> tokio::task::JoinHandle<Result<ShareResult, ShareError>> {
        let call = share.clone();
        let handle = tokio::spawn(async move { call.share(params).await });
        settle(|| fake.pending_share()).await;
        handle
    }

    fn dismissed() -> Result<ShareResult, ShareError> {
        Ok(ShareResult {
            raw: String::new(),
            status: ShareResultStatus::Dismissed,
        })
    }

    #[test]
    fn a_share_with_no_text_uri_or_files_is_refused_without_a_sheet() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let error = share.share(ShareParams::default()).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidArgument);
            assert_eq!(error.code(), "ERR_SHARE_INVALID_ARGUMENT");
            assert_eq!(fake.shares_started(), 0);
        });
    }

    #[test]
    fn a_file_uri_is_refused_without_a_sheet() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let params = |path: &str| ShareParams {
                text: Some("Look".to_owned()),
                files: vec![ShareFile {
                    path: path.to_owned(),
                    mime_type: None,
                }],
                ..ShareParams::default()
            };
            for path in ["file:///tmp/a.png", "content://media/1", "file:a.png"] {
                let error = share.share(params(path)).await.unwrap_err();
                assert_eq!(
                    error.kind(),
                    ErrorKind::InvalidArgument,
                    "{path} was shared"
                );
            }
            assert_eq!(fake.shares_started(), 0);
        });
    }

    #[test]
    fn a_missing_file_is_refused_without_a_sheet() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let params = ShareParams {
                text: Some("Look".to_owned()),
                files: vec![ShareFile {
                    path: "/nsh-share/does-not-exist.png".to_owned(),
                    mime_type: None,
                }],
                ..ShareParams::default()
            };
            let error = share.share(params).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::FileNotFound);
            assert_eq!(error.code(), "ERR_SHARE_FILE_NOT_FOUND");
            assert_eq!(fake.shares_started(), 0);
        });
    }

    #[test]
    fn an_existing_file_reaches_the_backend() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let params = ShareParams {
                text: Some("Look".to_owned()),
                files: vec![ShareFile {
                    path: "/tmp".to_owned(),
                    mime_type: Some("image/png".to_owned()),
                }],
                ..ShareParams::default()
            };
            let call = start_share(&share, &fake, params.clone()).await;
            assert_eq!(fake.last_params(), Some(params));
            fake.deliver_share(dismissed());
            let answer = call.await.unwrap().unwrap();
            assert_eq!(answer.status, ShareResultStatus::Dismissed);
        });
    }

    #[test]
    fn a_second_share_is_refused_while_a_sheet_is_up() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let first = start_share(&share, &fake, text_params("One")).await;
            let second = share.share(text_params("Two")).await.unwrap_err();
            assert_eq!(second.kind(), ErrorKind::InProgress);
            assert_eq!(fake.shares_started(), 1);
            fake.deliver_share(dismissed());
            assert_eq!(
                first.await.unwrap().unwrap().status,
                ShareResultStatus::Dismissed
            );
        });
    }

    #[test]
    fn a_share_resolves_once_and_a_late_second_answer_is_ignored() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = start_share(&share, &fake, text_params("One")).await;
            assert!(fake.deliver_share(dismissed()));
            assert_eq!(
                call.await.unwrap().unwrap().status,
                ShareResultStatus::Dismissed
            );
            assert!(
                !fake.deliver_share(dismissed()),
                "nothing waits for a second answer"
            );

            let next = start_share(&share, &fake, text_params("Two")).await;
            fake.deliver_share(dismissed());
            assert_eq!(
                next.await.unwrap().unwrap().status,
                ShareResultStatus::Dismissed
            );
        });
    }

    #[test]
    fn a_stale_answer_from_an_older_generation_cannot_resolve_a_newer_call() {
        let backend = LateAnswerBackend::new();
        let share = Share::with_backend(Arc::new(backend.clone()));
        run(async {
            // The first start fails, but the bridge keeps its callback and fires it later anyway.
            backend.fail_next_start();
            let first = tokio::spawn({
                let call = share.clone();
                async move { call.share(text_params("One")).await }
            });
            settle(|| backend.shares_started() == 1).await;
            assert_eq!(
                first.await.unwrap().unwrap_err().code(),
                "ERR_SHARE_BACKEND"
            );

            let second = tokio::spawn({
                let call = share.clone();
                async move { call.share(text_params("Two")).await }
            });
            settle(|| backend.shares_started() == 2).await;
            // The older generation's late answer arrives while the newer call owns the gate.
            assert!(backend.answer(0, Ok(success("stale"))));
            assert!(
                !second.is_finished(),
                "the newer call must still be waiting"
            );
            assert!(backend.answer(1, Ok(success("notes"))));
            assert_eq!(second.await.unwrap().unwrap().raw, "notes");
        });
    }

    #[test]
    fn a_backend_that_answers_then_fails_the_start_cannot_wreck_the_next_call() {
        let backend = FireAndFailBackend::new();
        let share = Share::with_backend(Arc::new(backend.clone()));
        run(async {
            // The second call's future is polled in by the hook, inside the window the answer
            // freed and the failed start has not cleared yet.
            let slot: Arc<Mutex<Option<SecondCall>>> = Arc::new(Mutex::new(None));
            let hook_slot = Arc::clone(&slot);
            let hook_share = share.clone();
            backend.fire_fail_with(Box::new(move || {
                let mut second: SecondCall = Box::pin({
                    let call = hook_share.clone();
                    async move { call.share(text_params("Two")).await }
                });
                let waker = Waker::noop();
                let mut cx = Context::from_waker(waker);
                assert!(
                    second.as_mut().poll(&mut cx).is_pending(),
                    "the second call must register and wait"
                );
                *hook_slot.lock().unwrap() = Some(second);
            }));

            let first = tokio::spawn({
                let call = share.clone();
                async move { call.share(text_params("One")).await }
            });
            settle(|| backend.shares_started() == 2).await;
            assert_eq!(
                first.await.unwrap().unwrap_err().code(),
                "ERR_SHARE_BACKEND"
            );

            assert!(backend.answer(1, Ok(success("notes"))));
            let second = slot.lock().unwrap().take().expect("the hook ran");
            let answer = tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .expect("the newer call must resolve with its own answer");
            assert_eq!(answer.unwrap().raw, "notes");
        });
    }

    #[test]
    fn cancelling_the_call_does_not_resolve_it_and_the_answer_frees_the_gate() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = start_share(&share, &fake, text_params("One")).await;
            call.abort();
            let _ = call.await;
            assert_eq!(
                fake.cancels(),
                1,
                "dropping the call asks to dismiss the sheet"
            );

            let refused = share.share(text_params("Two")).await.unwrap_err();
            assert_eq!(refused.kind(), ErrorKind::InProgress);

            fake.deliver_share(dismissed());
            let next = start_share(&share, &fake, text_params("Three")).await;
            fake.deliver_share(dismissed());
            assert!(next.await.unwrap().is_ok());
        });
    }

    #[test]
    fn a_share_that_fails_to_start_frees_the_gate() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            fake.fail_next_share(ShareError::new(ErrorKind::NoActivity));
            let error = share.share(text_params("One")).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::NoActivity);
            assert_eq!(fake.shares_started(), 0);
            assert!(!fake.pending_share());

            let next = start_share(&share, &fake, text_params("Two")).await;
            fake.deliver_share(dismissed());
            assert!(next.await.unwrap().is_ok());
        });
    }

    #[test]
    fn a_platform_error_reaches_the_caller_with_its_status() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = start_share(&share, &fake, text_params("One")).await;
            fake.deliver_share(Err(ShareError::new(ErrorKind::Backend).with_status(-13)));
            let error = call.await.unwrap().unwrap_err();
            assert_eq!(error.status(), Some(-13));
        });
    }

    #[test]
    fn the_convenience_calls_carry_what_they_are_given() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = {
                let share = share.clone();
                tokio::spawn(async move {
                    share
                        .share_uri("https://nsh.dev", Some("NSH".to_owned()))
                        .await
                })
            };
            settle(|| fake.pending_share()).await;
            let params = fake.last_params().unwrap();
            assert_eq!(params.uri.as_deref(), Some("https://nsh.dev"));
            assert_eq!(params.title.as_deref(), Some("NSH"));
            fake.deliver_share(dismissed());
            assert!(call.await.unwrap().is_ok());
        });

        run(async {
            let call = {
                let share = share.clone();
                tokio::spawn(async move {
                    share
                        .share_files(
                            vec![ShareFile {
                                path: "/tmp".to_owned(),
                                mime_type: None,
                            }],
                            Some("A file".to_owned()),
                            Some(Rect {
                                x: 1.0,
                                y: 2.0,
                                width: 3.0,
                                height: 4.0,
                            }),
                        )
                        .await
                })
            };
            settle(|| fake.pending_share()).await;
            let params = fake.last_params().unwrap();
            assert_eq!(params.files.len(), 1);
            assert_eq!(params.text.as_deref(), Some("A file"));
            assert_eq!(
                params.share_position_origin,
                Some(Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                })
            );
            fake.deliver_share(dismissed());
            assert!(call.await.unwrap().is_ok());
        });
    }

    #[test]
    fn the_availability_query_passes_through_the_backend() {
        let fake = FakeBackend::new();
        let share = Share::with_backend(Arc::new(fake.clone()));
        run(async {
            assert!(share.is_available().await.unwrap());
            fake.set_available(Err(ShareError::new(ErrorKind::Backend)));
            assert_eq!(
                share.is_available().await.unwrap_err().code(),
                "ERR_SHARE_BACKEND"
            );
        });
    }

    fn text_params(text: &str) -> ShareParams {
        ShareParams {
            text: Some(text.to_owned()),
            ..ShareParams::default()
        }
    }

    fn success(raw: &str) -> ShareResult {
        ShareResult {
            raw: raw.to_owned(),
            status: ShareResultStatus::Success,
        }
    }

    /// A backend that breaks the contract on purpose: a start can fail while the callback is kept,
    /// and that stale answer still fires later — the case the generation gate exists for.
    #[derive(Clone)]
    struct LateAnswerBackend {
        state: Arc<Mutex<LateState>>,
    }

    struct LateState {
        started: usize,
        fail_next: bool,
        // Slot per start order; a slot goes empty when its answer fires.
        dones: Vec<Option<ShareDone>>,
    }

    impl LateAnswerBackend {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(LateState {
                    started: 0,
                    fail_next: false,
                    dones: Vec::new(),
                })),
            }
        }

        fn lock(&self) -> MutexGuard<'_, LateState> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn shares_started(&self) -> usize {
            self.lock().started
        }

        /// Makes the next start fail, keeping its callback for a late answer.
        fn fail_next_start(&self) {
            self.lock().fail_next = true;
        }

        /// Answers one sheet by start order, however old. Returns `false` when it was answered
        /// already.
        fn answer(&self, sheet: usize, answer: Result<ShareResult, ShareError>) -> bool {
            let done = self.lock().dones.get_mut(sheet).and_then(Option::take);
            match done {
                Some(done) => {
                    done(answer);
                    true
                }
                None => false,
            }
        }
    }

    impl Backend for LateAnswerBackend {
        fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
            Box::pin(ready(Ok(true)))
        }

        fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
            Box::pin(ready(Ok(false)))
        }

        fn start_share(&self, _params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
            let mut state = self.lock();
            state.started += 1;
            state.dones.push(Some(done));
            if state.fail_next {
                state.fail_next = false;
                Err(ShareError::new(ErrorKind::Backend))
            } else {
                Ok(())
            }
        }

        fn cancel_share(&self) {}

        fn get_initial_media(
            &self,
        ) -> BackendFuture<'_, Result<Vec<crate::SharedMedia>, ShareError>> {
            Box::pin(ready(Ok(Vec::new())))
        }

        fn start_listen(&self, _done: crate::MediaDone) -> Result<(), ShareError> {
            Ok(())
        }

        fn stop_listen(&self) {}
    }

    /// A backend that breaks the contract on purpose: its first start answers, runs a hook that
    /// polls a newer call into the freed window, and only then fails the start — the case the
    /// guarded clear in `share` must survive.
    #[derive(Clone)]
    struct FireAndFailBackend {
        state: Arc<Mutex<FireState>>,
    }

    struct FireState {
        started: usize,
        armed: bool,
        dones: Vec<Option<ShareDone>>,
        interleave: Option<Box<dyn Fn() + Send>>,
    }

    impl FireAndFailBackend {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(FireState {
                    started: 0,
                    armed: false,
                    dones: Vec::new(),
                    interleave: None,
                })),
            }
        }

        fn lock(&self) -> MutexGuard<'_, FireState> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn shares_started(&self) -> usize {
            self.lock().started
        }

        /// Arms the contract break: the next start answers, runs `interleave`, then fails.
        fn fire_fail_with(&self, interleave: Box<dyn Fn() + Send>) {
            let mut state = self.lock();
            state.armed = true;
            state.interleave = Some(interleave);
        }

        /// Answers one sheet by start order, however old. Returns `false` when it was answered
        /// already.
        fn answer(&self, sheet: usize, answer: Result<ShareResult, ShareError>) -> bool {
            let done = self.lock().dones.get_mut(sheet).and_then(Option::take);
            match done {
                Some(done) => {
                    done(answer);
                    true
                }
                None => false,
            }
        }
    }

    impl Backend for FireAndFailBackend {
        fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
            Box::pin(ready(Ok(true)))
        }

        fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
            Box::pin(ready(Ok(false)))
        }

        fn start_share(&self, _params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
            let interleave = {
                let mut state = self.lock();
                state.started += 1;
                if state.armed && state.started == 1 {
                    state.armed = false;
                    state.dones.push(None);
                    state.interleave.take()
                } else {
                    state.dones.push(Some(done));
                    return Ok(());
                }
            };
            // The contract break: answer, let a newer call take the freed gate, then fail.
            done(dismissed());
            if let Some(run) = interleave {
                run();
            }
            Err(ShareError::new(ErrorKind::Backend))
        }

        fn cancel_share(&self) {}

        fn get_initial_media(
            &self,
        ) -> BackendFuture<'_, Result<Vec<crate::SharedMedia>, ShareError>> {
            Box::pin(ready(Ok(Vec::new())))
        }

        fn start_listen(&self, _done: crate::MediaDone) -> Result<(), ShareError> {
            Ok(())
        }

        fn stop_listen(&self) {}
    }
}
