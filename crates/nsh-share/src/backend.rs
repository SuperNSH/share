//! The seam between `Share` and a platform: the queries, one sheet that answers once, and the
//! inbound media.

use std::{future::Future, pin::Pin};

use crate::{error::ShareError, options::ShareParams, receive::SharedMedia, share::ShareResult};

/// A future a backend returns. Each call's future borrows the backend.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Receives the system's answer to one sheet, exactly once: the raw answer, or an error. A dismissal
/// arrives as `Ok` with [`ShareResultStatus::Dismissed`], not as an error.
pub type ShareDone = Box<dyn FnOnce(Result<ShareResult, ShareError>) + Send>;

/// Receives one inbound shared item, exactly once.
pub type MediaDone = Box<dyn FnOnce(Result<SharedMedia, ShareError>) + Send>;

/// What a platform provides. [`crate::Share`] adds the rules: one sheet at a time, one answer per
/// call.
pub trait Backend: Send + Sync {
    /// Whether the platform can show a share sheet here.
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>>;

    /// Whether the platform can receive shared items from other apps.
    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>>;

    /// Shows the sheet and returns at once. `done` is called once, when the system answers.
    /// An `Err` here means no sheet was shown, and `done` is not called.
    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError>;

    /// Asks the system to dismiss the sheet. Best-effort: where the platform cannot dismiss it,
    /// this does nothing, and the sheet's answer still arrives through `done`.
    fn cancel_share(&self);

    /// The items the app was launched with, if any. A plain launch answers an empty list, not an
    /// error.
    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>>;

    /// Listens for the next inbound item and returns at once. `done` is called once, when an item
    /// arrives. An `Err` here means no listener was started, and `done` is not called.
    fn start_listen(&self, done: MediaDone) -> Result<(), ShareError>;

    /// Stops listening for inbound items. A waiter's answer still arrives through its `done`.
    fn stop_listen(&self);
}
