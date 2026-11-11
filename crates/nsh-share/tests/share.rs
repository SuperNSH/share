//! `Share` through the fake backend: one sheet at a time, one answer per call, validation.
//!
//! `ipad_anchor_required` is a unit test in src/platform/apple.rs, which compiles on macOS hosts;
//! `platform_unsupported_reports_not_available` is in src/platform/unsupported.rs, which compiles
//! on every target by design.

use std::{future::Future, sync::Arc};

use nsh_share::{
    Backend, ErrorKind, FakeBackend, Share, ShareError, ShareFile, ShareParams, ShareResult,
    ShareResultStatus,
};

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
async fn start_sheet(
    share: &Share,
    fake: &FakeBackend,
    params: ShareParams,
) -> tokio::task::JoinHandle<Result<ShareResult, ShareError>> {
    let call = share.clone();
    let handle = tokio::spawn(async move { call.share(params).await });
    settle(|| fake.pending_share()).await;
    handle
}

fn text_params(text: &str) -> ShareParams {
    ShareParams {
        text: Some(text.to_owned()),
        ..ShareParams::default()
    }
}

fn success(raw: &str) -> Result<ShareResult, ShareError> {
    Ok(ShareResult {
        raw: raw.to_owned(),
        status: ShareResultStatus::Success,
    })
}

/// A real file the facade's existence stat accepts; the bytes stand in for a large payload.
fn staged_file(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("nsh-share-{}-{}.bin", name, std::process::id()));
    std::fs::write(&path, vec![7u8; 64 * 1024]).unwrap();
    path
}

#[test]
fn cancel_dismissed_not_error() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let call = start_sheet(&share, &fake, text_params("A photo")).await;
        assert!(fake.deliver_share(Ok(ShareResult {
            raw: "cancelled".to_owned(),
            status: ShareResultStatus::Dismissed,
        })));
        // A dismissed sheet is the system's own answer, so the call resolves Ok.
        let result = call.await.unwrap().unwrap();
        assert_eq!(result.status, ShareResultStatus::Dismissed);
        assert_eq!(result.status.as_str(), "dismissed");
    });
}

#[test]
fn second_share_rejected_in_progress() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let first = start_sheet(&share, &fake, text_params("One")).await;
        let second = share.share(text_params("Two")).await.unwrap_err();
        assert_eq!(second.kind(), ErrorKind::InProgress);
        assert_eq!(second.code(), "ERR_SHARE_IN_PROGRESS");
        assert_eq!(fake.shares_started(), 1);

        assert!(fake.deliver_share(success("done")));
        let result = first.await.unwrap().unwrap();
        assert_eq!(result.status, ShareResultStatus::Success);
    });
}

#[test]
fn late_answer_ignored_resolve_once() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let call = start_sheet(&share, &fake, text_params("Once")).await;
        assert!(fake.deliver_share(success("done")));
        assert_eq!(
            call.await.unwrap().unwrap().status,
            ShareResultStatus::Success
        );

        // A second answer finds no call waiting; nothing resolves again and no counter moves.
        assert!(!fake.deliver_share(success("again")));
        assert_eq!(fake.shares_started(), 1);

        let next = start_sheet(&share, &fake, text_params("Twice")).await;
        assert!(fake.deliver_share(success("done")));
        assert_eq!(
            next.await.unwrap().unwrap().status,
            ShareResultStatus::Success
        );
    });
}

#[test]
fn cancel_share_best_effort() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let call = start_sheet(&share, &fake, text_params("Hold")).await;

        // The platform's cancel reaches the backend seam; it never resolves the call itself.
        fake.cancel_share();
        assert_eq!(fake.cancels(), 1);
        assert!(fake.pending_share(), "a cancel must not resolve the call");
        let refused = share.share(text_params("Next")).await.unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::InProgress);

        // The system's own answer still lands and resolves the held call.
        assert!(fake.deliver_share(success("done")));
        assert_eq!(
            call.await.unwrap().unwrap().status,
            ShareResultStatus::Success
        );
    });
}

#[test]
fn empty_share_invalid_argument() {
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
fn file_uri_rejected_invalid_argument() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        for path in ["file:///tmp/report.pdf", "content://media/external/1"] {
            let params = ShareParams {
                files: vec![ShareFile {
                    path: path.to_owned(),
                    mime_type: None,
                }],
                ..ShareParams::default()
            };
            let error = share.share(params).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidArgument, "{path}");
        }
        assert_eq!(fake.shares_started(), 0, "no sheet starts for a URI path");
    });
}

#[test]
fn missing_file_file_not_found() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let missing = ShareParams {
            files: vec![ShareFile {
                path: "/nsh-share/no-such-file.bin".to_owned(),
                mime_type: Some("application/octet-stream".to_owned()),
            }],
            ..ShareParams::default()
        };
        let error = share.share(missing).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::FileNotFound);
        assert_eq!(error.code(), "ERR_SHARE_FILE_NOT_FOUND");
        assert_eq!(fake.shares_started(), 0);

        // A bridge that fails staging reports the same code from start_share.
        fake.fail_next_share(ShareError::new(ErrorKind::FileNotFound));
        let error = share.share(text_params("Staged")).await.unwrap_err();
        assert_eq!(error.code(), "ERR_SHARE_FILE_NOT_FOUND");
        assert_eq!(fake.shares_started(), 0);
    });
}

#[test]
fn large_file_temp_copy() {
    // The temp copy to a provider URI lives in the Android bridge; CI's cross-check compiles it.
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    let path = staged_file("large");
    let path_str = path.to_string_lossy().into_owned();
    run(async {
        let call = {
            let share = share.clone();
            let path = path_str.clone();
            tokio::spawn(async move {
                share
                    .share_files(
                        vec![ShareFile {
                            path,
                            mime_type: Some("application/octet-stream".to_owned()),
                        }],
                        Some("The export".to_owned()),
                        None,
                    )
                    .await
            })
        };
        settle(|| fake.pending_share()).await;

        let params = fake.last_params().expect("the sheet carries the params");
        assert_eq!(params.files.len(), 1);
        assert_eq!(params.files[0].path, path_str);
        assert!(!params.files[0].path.contains("://"), "a plain path");
        assert_eq!(
            params.files[0].mime_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(params.text.as_deref(), Some("The export"));

        assert!(fake.deliver_share(success("done")));
        assert_eq!(
            call.await.unwrap().unwrap().status,
            ShareResultStatus::Success
        );
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn no_activity_error() {
    // No foreground Activity or window is detected in the Android and Windows bridges; CI compiles them.
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        fake.fail_next_share(ShareError::new(ErrorKind::NoActivity));
        let error = share.share(text_params("Read this")).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NoActivity);
        assert_eq!(error.code(), "ERR_SHARE_NO_ACTIVITY");
        assert_eq!(fake.shares_started(), 0);

        // A failed start leaves the gate free: the next call shows its sheet.
        let next = start_sheet(&share, &fake, text_params("Again")).await;
        assert!(fake.deliver_share(success("done")));
        assert_eq!(
            next.await.unwrap().unwrap().status,
            ShareResultStatus::Success
        );
    });
}

#[test]
fn web_share_unavailable_not_available() {
    // The navigator.share check and the AbortError mapping are the web bridge's; CI compiles it.
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        fake.set_available(Ok(false));
        assert!(!share.is_available().await.unwrap());

        fake.fail_next_share(ShareError::new(ErrorKind::NotAvailable));
        let error = share.share(text_params("A link")).await.unwrap_err();
        assert_eq!(error.code(), "ERR_SHARE_NOT_AVAILABLE");

        // A user abort lands as a dismissed answer, which is a result, not a rejection.
        let aborting = start_sheet(&share, &fake, text_params("Again")).await;
        assert!(fake.deliver_share(Ok(ShareResult {
            raw: "AbortError".to_owned(),
            status: ShareResultStatus::Dismissed,
        })));
        assert_eq!(
            aborting.await.unwrap().unwrap().status,
            ShareResultStatus::Dismissed
        );
    });
}

#[test]
fn status_values_match_share_plus() {
    assert_eq!(ShareResultStatus::Success.as_str(), "success");
    assert_eq!(ShareResultStatus::Dismissed.as_str(), "dismissed");
    assert_eq!(ShareResultStatus::Unavailable.as_str(), "unavailable");
}
