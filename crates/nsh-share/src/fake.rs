//! A backend with scripted answers. Tests use it, and so can an app that runs without a sheet.

use std::{
    future::ready,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::ShareError,
    options::ShareParams,
    receive::SharedMedia,
    share::ShareResult,
};

/// A backend whose answers the test sets, and whose sheet the test finishes.
///
/// Like the platform sheets, a cancel does not finish the sheet: the test answers it, as the system
/// would. Clones share one state.
#[derive(Clone)]
pub struct FakeBackend {
    state: Arc<Mutex<FakeState>>,
}

struct FakeState {
    available: Result<bool, ShareError>,
    can_receive: Result<bool, ShareError>,
    next_start_error: Option<ShareError>,
    waiting_share: Option<ShareDone>,
    waiting_media: Option<MediaDone>,
    initial_media: Vec<SharedMedia>,
    shares: usize,
    cancels: usize,
    listens: usize,
    last_params: Option<ShareParams>,
}

impl Default for FakeBackend {
    /// A device with a share sheet and no inbound channel.
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    /// A device that can share and cannot receive.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                available: Ok(true),
                can_receive: Ok(false),
                next_start_error: None,
                waiting_share: None,
                waiting_media: None,
                initial_media: Vec::new(),
                shares: 0,
                cancels: 0,
                listens: 0,
                last_params: None,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets what `is_available` answers.
    pub fn set_available(&self, answer: Result<bool, ShareError>) {
        self.lock().available = answer;
    }

    /// Sets what `can_receive` answers.
    pub fn set_can_receive(&self, answer: Result<bool, ShareError>) {
        self.lock().can_receive = answer;
    }

    /// Makes the next share fail to start with `error`, as a platform does when it cannot show one.
    pub fn fail_next_share(&self, error: ShareError) {
        self.lock().next_start_error = Some(error);
    }

    /// Whether a sheet is waiting for its answer.
    pub fn pending_share(&self) -> bool {
        self.lock().waiting_share.is_some()
    }

    /// Answers the waiting sheet. Returns `false` when none waits.
    pub fn deliver_share(&self, answer: Result<ShareResult, ShareError>) -> bool {
        let waiting = self.lock().waiting_share.take();
        match waiting {
            Some(done) => {
                done(answer);
                true
            }
            None => false,
        }
    }

    /// Delivers one inbound item to the waiting listener. Returns `false` when none waits.
    pub fn deliver_media(&self, answer: Result<SharedMedia, ShareError>) -> bool {
        let waiting = self.lock().waiting_media.take();
        match waiting {
            Some(done) => {
                done(answer);
                true
            }
            None => false,
        }
    }

    /// Sets what `get_initial_media` answers.
    pub fn set_initial_media(&self, media: Vec<SharedMedia>) {
        self.lock().initial_media = media;
    }

    /// How many sheets were started.
    pub fn shares_started(&self) -> usize {
        self.lock().shares
    }

    /// How many times a sheet was asked to cancel.
    pub fn cancels(&self) -> usize {
        self.lock().cancels
    }

    /// How many listeners were started.
    pub fn listens_started(&self) -> usize {
        self.lock().listens
    }

    /// The params of the last sheet started.
    pub fn last_params(&self) -> Option<ShareParams> {
        self.lock().last_params.clone()
    }
}

impl Backend for FakeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(self.lock().available.clone()))
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(self.lock().can_receive.clone()))
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        let mut state = self.lock();
        state.last_params = Some(params.clone());
        if let Some(error) = state.next_start_error.take() {
            return Err(error);
        }
        state.shares += 1;
        state.waiting_share = Some(done);
        Ok(())
    }

    fn cancel_share(&self) {
        self.lock().cancels += 1;
    }

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        Box::pin(ready(Ok(self.lock().initial_media.clone())))
    }

    fn start_listen(&self, done: MediaDone) -> Result<(), ShareError> {
        let mut state = self.lock();
        state.listens += 1;
        state.waiting_media = Some(done);
        Ok(())
    }

    fn stop_listen(&self) {
        // The waiter's answer still arrives through its done, as on a platform.
    }
}
