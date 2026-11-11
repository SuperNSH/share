//! Linux has no system share sheet: text goes out as a `mailto:` URL handed to `xdg-open`,
//! the same honest fallback share_plus ships, and nothing can be shared into the app.

use std::{env, future::ready, os::unix::fs::PermissionsExt, process::Command};

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::SharedMedia,
    share::{ShareResult, ShareResultStatus},
};

pub(crate) struct NativeBackend;

/// A mailto query value: every byte outside RFC 6068's unreserved set goes out as `%XX`,
/// `=` and `&` included, so a value cannot smuggle a second query key.
fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0xf) as usize] as char);
            }
        }
    }
    out
}

/// The mailto URL a share falls back to, or the error that stops it before anything launches.
/// `subject` wins, `title` stands in when the caller set none; a uri rides in the body.
fn mailto_url(params: &ShareParams) -> Result<String, ShareError> {
    if !params.mail_to_fallback_enabled {
        return Err(ShareError::new(ErrorKind::NotAvailable));
    }
    let content = |value: &Option<String>| value.as_deref().filter(|value| !value.is_empty());
    let text = content(&params.text);
    let uri = content(&params.uri);
    // A bare desktop has no content provider, so a file has no URI to hand to a mail client;
    // files ride along only when text or a uri is there to carry the share.
    if !params.files.is_empty() && text.is_none() && uri.is_none() {
        return Err(ShareError::new(ErrorKind::NotAvailable));
    }
    let subject = content(&params.subject).or(content(&params.title));

    let mut body = String::new();
    if let Some(text) = text {
        body.push_str(text);
    }
    if let Some(uri) = uri {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(uri);
    }

    let mut query = String::new();
    if let Some(subject) = subject {
        query.push_str("subject=");
        query.push_str(&percent_encode(subject));
    }
    if !body.is_empty() {
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str("body=");
        query.push_str(&percent_encode(&body));
    }
    Ok(format!("mailto:?{query}"))
}

/// Whether an executable `name` sits on `PATH`: one stat per directory, nothing spawned.
fn in_path(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| {
        let Ok(metadata) = dir.join(name).metadata() else {
            return false;
        };
        !metadata.is_dir() && (metadata.permissions().mode() & 0o111) != 0
    })
}

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(Ok::<bool, ShareError>(in_path("xdg-open"))))
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(Ok::<bool, ShareError>(false)))
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        let url = mailto_url(params)?;
        // xdg-open hands the URL to the desktop's handler and exits; the child is never waited
        // on, so nothing here can block or hang, win or lose.
        match Command::new("xdg-open").arg(&url).spawn() {
            Ok(_child) => {
                // No answer ever comes back from a mail client, so the call resolves at once as
                // unavailable, the result share_plus reports on Linux.
                done(Ok(ShareResult {
                    raw: url,
                    status: ShareResultStatus::Unavailable,
                }));
                Ok(())
            }
            Err(_) => Err(ShareError::new(ErrorKind::Backend)),
        }
    }

    fn cancel_share(&self) {
        // The opener was fire-and-forget and the answer already went out; there is nothing to
        // cancel.
    }

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        Box::pin(ready(Ok::<Vec<SharedMedia>, ShareError>(Vec::new())))
    }

    fn start_listen(&self, _done: MediaDone) -> Result<(), ShareError> {
        // Linux delivers no inbound shares, so a listener could only ever hang; refusing keeps
        // the call resolved.
        Err(ShareError::new(ErrorKind::NotAvailable))
    }

    fn stop_listen(&self) {}
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::{
        Backend, ErrorKind, NativeBackend, ShareDone, ShareError, ShareParams, in_path, mailto_url,
        percent_encode,
    };
    use crate::options::ShareFile;

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn percent_encode_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(percent_encode("safe-._~"), "safe-._~");
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
        assert_eq!(percent_encode("line\nbreak"), "line%0Abreak");
    }

    #[test]
    fn mailto_subject_prefers_subject_then_title() {
        let both = ShareParams {
            subject: Some("Report".into()),
            title: Some("Ignored".into()),
            text: Some("Numbers".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&both).unwrap(),
            "mailto:?subject=Report&body=Numbers"
        );
        let titled = ShareParams {
            title: Some("Notes".into()),
            text: Some("Hello".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&titled).unwrap(),
            "mailto:?subject=Notes&body=Hello"
        );
    }

    #[test]
    fn mailto_appends_uri_after_text() {
        let with_text = ShareParams {
            text: Some("See this".into()),
            uri: Some("https://example.com/x".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&with_text).unwrap(),
            "mailto:?body=See%20this%0Ahttps%3A%2F%2Fexample.com%2Fx"
        );
        let uri_only = ShareParams {
            uri: Some("https://example.com/x".into()),
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&uri_only).unwrap(),
            "mailto:?body=https%3A%2F%2Fexample.com%2Fx"
        );
    }

    #[test]
    fn mailto_without_fallback_flag_is_not_available() {
        let params = ShareParams {
            text: Some("Hello".into()),
            mail_to_fallback_enabled: false,
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&params).unwrap_err().kind(),
            ErrorKind::NotAvailable
        );
    }

    #[test]
    fn mailto_of_files_alone_is_not_available() {
        let params = ShareParams {
            files: vec![ShareFile {
                path: "/tmp/photo.png".into(),
                mime_type: Some("image/png".into()),
            }],
            ..ShareParams::default()
        };
        assert_eq!(
            mailto_url(&params).unwrap_err().kind(),
            ErrorKind::NotAvailable
        );
    }

    #[test]
    fn mailto_of_files_with_text_carries_the_text() {
        let params = ShareParams {
            text: Some("The photo".into()),
            files: vec![ShareFile {
                path: "/tmp/photo.png".into(),
                mime_type: None,
            }],
            ..ShareParams::default()
        };
        assert_eq!(mailto_url(&params).unwrap(), "mailto:?body=The%20photo");
    }

    #[test]
    fn start_share_refusals_answer_err_and_never_call_done() {
        let backend = NativeBackend;
        let refusals = [
            ShareParams {
                mail_to_fallback_enabled: false,
                text: Some("Hello".into()),
                ..ShareParams::default()
            },
            ShareParams {
                files: vec![ShareFile {
                    path: "/tmp/photo.png".into(),
                    mime_type: None,
                }],
                ..ShareParams::default()
            },
        ];
        for params in &refusals {
            let (sent, received) = std::sync::mpsc::channel::<Result<_, ShareError>>();
            let done: ShareDone = Box::new(move |answer| {
                let _ = sent.send(answer);
            });
            let error = backend.start_share(params, done).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::NotAvailable);
            assert!(received.try_recv().is_err());
        }
    }

    #[test]
    fn receiving_is_off_on_linux() {
        let backend = NativeBackend;
        assert!(!run(backend.can_receive()).unwrap());
        assert!(run(backend.get_initial_media()).unwrap().is_empty());
        assert_eq!(
            backend.start_listen(Box::new(|_| {})).unwrap_err().kind(),
            ErrorKind::NotAvailable
        );
    }

    #[test]
    fn in_path_finds_only_real_executables() {
        assert!(in_path("sh"));
        assert!(!in_path("nsh-share-no-such-tool"));
    }
}
