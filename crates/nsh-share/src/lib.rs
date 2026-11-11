//! The share sheet for Native Shell apps.
//!
//! [`Share`] follows Flutter's `share_plus`: give the sheet text, a URI or files, then wait for the
//! system's answer, which resolves exactly once — `Ok` with a status, or a [`ShareError`] with a
//! stable code. A dismissed sheet is an answer, not an error. [`ReceiveSharingIntent`] follows
//! `receive_sharing_intent`: read the media the app was launched with, then wait for the next
//! inbound item, one waiter at a time.

mod backend;
mod error;
mod fake;
mod native;
mod options;
mod platform;
mod receive;
mod share;

use nsh_core::prelude::*;

pub use backend::{Backend, BackendFuture, MediaDone, ShareDone};
pub use error::{ErrorKind, ShareError};
pub use fake::FakeBackend;
pub use native::{
    NativeRect, NativeShare, NativeShareFile, NativeShareParams, NativeShareResult,
    NativeSharedMedia,
};
pub use options::{Rect, ShareFile, ShareParams};
pub use receive::{ReceiveSharingIntent, SharedMedia, SharedMediaType};
pub use share::{Share, ShareResult, ShareResultStatus};

/// The crate's UI set: what TSX imports from `nsh-share/native`.
#[native_module("nsh-share/native")]
fn native(registry: &mut Registry) {
    registry.class::<NativeShare>();
}
