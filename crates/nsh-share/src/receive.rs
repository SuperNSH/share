//! `ReceiveSharingIntent`: the cold-start media the app was launched with, and the next inbound
//! share, one waiter at a time.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use tokio::sync::oneshot;

use crate::{
    backend::{Backend, MediaDone},
    error::{ErrorKind, ShareError},
};

/// One inbound shared item, after `receive_sharing_intent`'s `SharedMedia`.
#[derive(PartialEq, Debug, Clone)]
pub struct SharedMedia {
    /// Where the platform staged the item, as a plain filesystem path.
    pub path: String,
    /// A thumbnail of the item, for images and videos.
    pub thumbnail: Option<String>,
    /// How long the item plays, for audio and video.
    pub duration: Option<u64>,
    /// What the item is.
    pub kind: SharedMediaType,
    /// The item's MIME type, when the sender named one.
    pub mime_type: Option<String>,
    /// The text the sender wrote, when the share came with one.
    pub message: Option<String>,
}

/// What one inbound item is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SharedMediaType {
    /// A still image.
    Image,
    /// A video.
    Video,
    /// Plain text.
    Text,
    /// Any other file.
    File,
    /// A link.
    Url,
}

impl SharedMediaType {
    /// The name TSX matches on, `receive_sharing_intent`'s own.
    pub fn as_str(self) -> &'static str {
        match self {
            SharedMediaType::Image => "image",
            SharedMediaType::Video => "video",
            SharedMediaType::Text => "text",
            SharedMediaType::File => "file",
            SharedMediaType::Url => "url",
        }
    }
}

/// The listen gate: one waiter at a time, and only the backend's answer resolves it. `waiting`
/// names the generation that owns the gate, so a late or stale answer cannot resolve a newer
/// waiter, and a failed start cannot clear one.
#[derive(Default)]
struct Gate {
    busy: bool,
    next_id: u64,
    waiting: Option<(u64, oneshot::Sender<Result<SharedMedia, ShareError>>)>,
}

struct Shared {
    backend: Arc<dyn Backend>,
    gate: Mutex<Gate>,
    // The launch media the facade has taken from the backend; None until the first read.
    initial: Mutex<Option<Vec<SharedMedia>>>,
}

impl Shared {
    fn gate(&self) -> MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn initial(&self) -> MutexGuard<'_, Option<Vec<SharedMedia>>> {
        self.initial.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What other apps share into this app, as `receive_sharing_intent` names it.
///
/// Clones share one listen gate. [`ReceiveSharingIntent::new`] returns the platform's shared handle.
#[derive(Clone)]
pub struct ReceiveSharingIntent {
    shared: Arc<Shared>,
}

impl ReceiveSharingIntent {
    /// The platform's inbound channel, with one gate for the whole process.
    pub fn new() -> Self {
        static PLATFORM: OnceLock<ReceiveSharingIntent> = OnceLock::new();
        PLATFORM
            .get_or_init(|| Self::from_arc(crate::platform::default_backend()))
            .clone()
    }

    /// Receiving through `backend`, with its own gate. Tests and fakes use this.
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self::from_arc(backend)
    }

    fn from_arc(backend: Arc<dyn Backend>) -> Self {
        Self {
            shared: Arc::new(Shared {
                backend,
                gate: Mutex::new(Gate::default()),
                initial: Mutex::new(None),
            }),
        }
    }

    /// The items the app was launched with. An Android MAIN launch or a plain iOS launch answers
    /// an empty list, never an error. The backend's snapshot is read once;
    /// [`ReceiveSharingIntent::reset`] clears it.
    pub async fn get_initial_media(&self) -> Result<Vec<SharedMedia>, ShareError> {
        if let Some(media) = self.shared.initial().clone() {
            return Ok(media);
        }
        let media = self.shared.backend.get_initial_media().await?;
        *self.shared.initial() = Some(media.clone());
        Ok(media)
    }

    /// Waits for the next item another app shares in. One waiter at a time: a second call while
    /// one waits gets `ERR_SHARE_IN_PROGRESS`. Dropping the future stops the listener; the waiter
    /// resolves only when the backend delivers, which it still does.
    pub async fn receive(&self) -> Result<SharedMedia, ShareError> {
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
        let done: MediaDone = Box::new(move |answer| answered(&shared, generation, answer));
        if let Err(error) = self.shared.backend.start_listen(done) {
            // Clear only our own registration: a backend that fired done before failing freed the
            // gate already, and a newer waiter may have taken it in the window.
            let mut gate = self.shared.gate();
            if matches!(&gate.waiting, Some((id, _)) if *id == generation) {
                gate.busy = false;
                gate.waiting = None;
            }
            return Err(error);
        }
        let mut owner = Owner {
            shared: Arc::clone(&self.shared),
            armed: true,
        };
        let answer = receiver.await;
        owner.armed = false;
        answer.unwrap_or_else(|_| Err(ShareError::new(ErrorKind::Backend)))
    }

    /// Clears the stored launch media, so a later `get_initial_media` answers empty instead of the
    /// payload the app already consumed. Never touches a waiting `receive`.
    pub fn reset(&self) {
        *self.shared.initial() = Some(Vec::new());
    }
}

impl Default for ReceiveSharingIntent {
    fn default() -> Self {
        Self::new()
    }
}

/// Hands one inbound item to the waiting call, and frees the gate. An answer for a generation the
/// gate no longer holds is dropped, without touching a newer waiter that owns it.
fn answered(shared: &Shared, generation: u64, answer: Result<SharedMedia, ShareError>) {
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

/// Stops the listener if the call's future is dropped before an item arrives. The gate stays busy:
/// only the backend's delivery, which still happens, frees it.
struct Owner {
    shared: Arc<Shared>,
    armed: bool,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if self.armed {
            let busy = self.shared.gate().busy;
            if busy {
                self.shared.backend.stop_listen();
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
        task::{Context, Waker},
        time::Duration,
    };

    use crate::{backend::BackendFuture, fake::FakeBackend};

    /// A receive call the test polls in by hand, inside the backend's window.
    type Waiter = Pin<Box<dyn Future<Output = Result<SharedMedia, ShareError>> + Send>>;

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Yields until `ready` holds, so a spawned call has reached its listener.
    async fn settle(ready: impl Fn() -> bool) {
        for _ in 0..1000 {
            if ready() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the listener never started");
    }

    fn image(path: &str) -> SharedMedia {
        SharedMedia {
            path: path.to_owned(),
            thumbnail: None,
            duration: None,
            kind: SharedMediaType::Image,
            mime_type: Some("image/png".to_owned()),
            message: None,
        }
    }

    #[test]
    fn the_initial_media_resolve_once_from_the_backend_snapshot() {
        let fake = FakeBackend::new();
        fake.set_initial_media(vec![image("/tmp/inbox/1.png")]);
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            assert_eq!(
                receive.get_initial_media().await.unwrap(),
                vec![image("/tmp/inbox/1.png")]
            );
            // The backend is read once; a change after the first read never reaches the caller.
            fake.set_initial_media(vec![image("/tmp/inbox/2.png")]);
            assert_eq!(
                receive.get_initial_media().await.unwrap(),
                vec![image("/tmp/inbox/1.png")]
            );
        });
    }

    #[test]
    fn a_plain_launch_answers_empty_and_never_an_error() {
        let fake = FakeBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            assert!(receive.get_initial_media().await.unwrap().is_empty());
        });
    }

    #[test]
    fn reset_clears_the_stored_initial_media() {
        let fake = FakeBackend::new();
        fake.set_initial_media(vec![image("/tmp/inbox/1.png")]);
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            assert_eq!(receive.get_initial_media().await.unwrap().len(), 1);
            receive.reset();
            assert!(receive.get_initial_media().await.unwrap().is_empty());
        });
    }

    #[test]
    fn receive_waits_for_the_next_item_and_resolves_once() {
        let fake = FakeBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 1).await;
            assert!(fake.deliver_media(Ok(image("/tmp/inbox/1.png"))));
            assert_eq!(call.await.unwrap().unwrap(), image("/tmp/inbox/1.png"));
            // A second delivery with no waiter resolves nothing.
            assert!(!fake.deliver_media(Ok(image("/tmp/inbox/2.png"))));
        });
    }

    #[test]
    fn a_second_receive_is_refused_while_one_waits() {
        let fake = FakeBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 1).await;
            let refused = receive.receive().await.unwrap_err();
            assert_eq!(refused.kind(), ErrorKind::InProgress);
            assert_eq!(fake.listens_started(), 1);

            fake.deliver_media(Ok(image("/tmp/inbox/1.png")));
            assert!(call.await.unwrap().is_ok());

            let next = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 2).await;
            fake.deliver_media(Err(ShareError::new(ErrorKind::Backend)));
            assert_eq!(next.await.unwrap().unwrap_err().kind(), ErrorKind::Backend);
        });
    }

    #[test]
    fn dropping_the_wait_stops_the_listener_and_the_delivery_frees_the_gate() {
        let fake = FakeBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 1).await;
            call.abort();
            let _ = call.await;

            let refused = receive.receive().await.unwrap_err();
            assert_eq!(
                refused.kind(),
                ErrorKind::InProgress,
                "the drop must not free the gate; only the delivery does"
            );

            fake.deliver_media(Ok(image("/tmp/inbox/1.png")));
            let next = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 2).await;
            fake.deliver_media(Ok(image("/tmp/inbox/2.png")));
            assert_eq!(next.await.unwrap().unwrap(), image("/tmp/inbox/2.png"));
        });
    }

    #[test]
    fn reset_never_resolves_a_waiting_receive() {
        let fake = FakeBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
        run(async {
            let call = {
                let receive = receive.clone();
                tokio::spawn(async move { receive.receive().await })
            };
            settle(|| fake.listens_started() == 1).await;
            receive.reset();
            assert!(!call.is_finished(), "reset must not resolve the waiter");
            fake.deliver_media(Ok(image("/tmp/inbox/1.png")));
            assert_eq!(call.await.unwrap().unwrap(), image("/tmp/inbox/1.png"));
        });
    }

    #[test]
    fn a_backend_that_answers_then_fails_the_start_cannot_wreck_the_next_call() {
        let backend = FireAndFailBackend::new();
        let receive = ReceiveSharingIntent::with_backend(Arc::new(backend.clone()));
        run(async {
            // The second call's future is polled in by the hook, inside the window the answer
            // freed and the failed start has not cleared yet.
            let slot: Arc<Mutex<Option<Waiter>>> = Arc::new(Mutex::new(None));
            let hook_slot = Arc::clone(&slot);
            let hook_receive = receive.clone();
            backend.fire_fail_with(Box::new(move || {
                let mut second: Waiter = Box::pin({
                    let call = hook_receive.clone();
                    async move { call.receive().await }
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
                let call = receive.clone();
                async move { call.receive().await }
            });
            settle(|| backend.listens_started() == 2).await;
            assert_eq!(
                first.await.unwrap().unwrap_err().code(),
                "ERR_SHARE_BACKEND"
            );

            assert!(backend.deliver(1, Ok(image("/tmp/inbox/2.png"))));
            let second = slot.lock().unwrap().take().expect("the hook ran");
            let answer = tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .expect("the newer call must resolve with its own answer");
            assert_eq!(answer.unwrap(), image("/tmp/inbox/2.png"));
        });
    }

    /// A backend that breaks the contract on purpose: its first listen answers, runs a hook that
    /// polls a newer call into the freed window, and only then fails the start — the case the
    /// guarded clear in `receive` must survive.
    #[derive(Clone)]
    struct FireAndFailBackend {
        state: Arc<Mutex<FireState>>,
    }

    struct FireState {
        started: usize,
        armed: bool,
        dones: Vec<Option<MediaDone>>,
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

        fn listens_started(&self) -> usize {
            self.lock().started
        }

        /// Arms the contract break: the next listen answers, runs `interleave`, then fails.
        fn fire_fail_with(&self, interleave: Box<dyn Fn() + Send>) {
            let mut state = self.lock();
            state.armed = true;
            state.interleave = Some(interleave);
        }

        /// Delivers to one listener by start order, however old. Returns `false` when it was
        /// delivered to already.
        fn deliver(&self, listener: usize, answer: Result<SharedMedia, ShareError>) -> bool {
            let done = self.lock().dones.get_mut(listener).and_then(Option::take);
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
            Box::pin(ready(Ok(true)))
        }

        fn start_share(
            &self,
            _params: &crate::ShareParams,
            _done: crate::ShareDone,
        ) -> Result<(), ShareError> {
            Err(ShareError::new(ErrorKind::NotAvailable))
        }

        fn cancel_share(&self) {}

        fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
            Box::pin(ready(Ok(Vec::new())))
        }

        fn start_listen(&self, done: MediaDone) -> Result<(), ShareError> {
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
            done(Ok(image("/tmp/inbox/1.png")));
            if let Some(run) = interleave {
                run();
            }
            Err(ShareError::new(ErrorKind::Backend))
        }

        fn stop_listen(&self) {}
    }
}
