//! The errors a share or receive call reports. Each has a stable code; a message says what to do.

use std::fmt;

/// What went wrong, as a stable code; see [`ShareError::code`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// No share sheet exists here (the web, or a platform with no handler).
    NotAvailable,
    /// An argument is empty or not allowed, such as a share with no text, URI or files.
    InvalidArgument,
    /// Another sheet is already up. One sheet runs at a time.
    InProgress,
    /// The share has no screen to show on, for example Android with no running app.
    NoActivity,
    /// A listed file does not exist at its path.
    FileNotFound,
    /// The platform failed; `status` holds its own code where it gives one.
    Backend,
}

/// A failed share, receive or availability call.
///
/// `Display` names the message. Neither it nor the code holds the shared text or any file's content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .kind.message())]
pub struct ShareError {
    kind: ErrorKind,
    status: Option<i64>,
}

impl ShareError {
    /// An error of `kind`, with no platform status.
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, status: None }
    }

    /// The same error, carrying the platform's own status code.
    #[must_use]
    pub fn with_status(mut self, status: i64) -> Self {
        self.status = Some(status);
        self
    }

    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The platform's status code, when the platform gave one.
    pub fn status(&self) -> Option<i64> {
        self.status
    }

    /// The stable code, such as `ERR_SHARE_IN_PROGRESS`. Apps match on this.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// The message of this error; see [`ErrorKind::message`].
    pub fn message(&self) -> &'static str {
        self.kind.message()
    }
}

impl ErrorKind {
    /// The stable code of this kind.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::NotAvailable => "ERR_SHARE_NOT_AVAILABLE",
            ErrorKind::InvalidArgument => "ERR_SHARE_INVALID_ARGUMENT",
            ErrorKind::InProgress => "ERR_SHARE_IN_PROGRESS",
            ErrorKind::NoActivity => "ERR_SHARE_NO_ACTIVITY",
            ErrorKind::FileNotFound => "ERR_SHARE_FILE_NOT_FOUND",
            ErrorKind::Backend => "ERR_SHARE_BACKEND",
        }
    }

    /// What to do about it, in one sentence.
    pub fn message(self) -> &'static str {
        match self {
            ErrorKind::NotAvailable => "No share sheet is available on this device.",
            ErrorKind::InvalidArgument => {
                "Give the share a text, a URI or files, and plain file paths, not URIs."
            }
            ErrorKind::InProgress => "Another share sheet is already up. Wait for it to finish.",
            ErrorKind::NoActivity => "There is no screen to show the share sheet on right now.",
            ErrorKind::FileNotFound => "A file the share lists does not exist. Stage it first.",
            ErrorKind::Backend => "The platform's share service failed.",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
