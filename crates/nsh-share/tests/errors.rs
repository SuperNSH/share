//! Every kind's stable code, and each one reaching the caller through both error paths.

use std::{future::Future, sync::Arc};

use nsh_share::{
    ErrorKind, FakeBackend, Share, ShareError, ShareParams, ShareResult, ShareResultStatus,
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

const ALL_KINDS: [(ErrorKind, &str); 6] = [
    (ErrorKind::NotAvailable, "ERR_SHARE_NOT_AVAILABLE"),
    (ErrorKind::InvalidArgument, "ERR_SHARE_INVALID_ARGUMENT"),
    (ErrorKind::InProgress, "ERR_SHARE_IN_PROGRESS"),
    (ErrorKind::NoActivity, "ERR_SHARE_NO_ACTIVITY"),
    (ErrorKind::FileNotFound, "ERR_SHARE_FILE_NOT_FOUND"),
    (ErrorKind::Backend, "ERR_SHARE_BACKEND"),
];

fn text_params(text: &str) -> ShareParams {
    ShareParams {
        text: Some(text.to_owned()),
        ..ShareParams::default()
    }
}

#[test]
fn every_kind_has_its_code_and_a_message() {
    for (kind, code) in ALL_KINDS {
        assert_eq!(kind.code(), code);
        assert_eq!(kind.to_string(), code, "Display writes the code");
        assert!(!kind.message().is_empty());
        let error = ShareError::new(kind);
        assert_eq!(error.code(), code);
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_platform_status_is_kept_with_its_error() {
    let error = ShareError::new(ErrorKind::Backend).with_status(-7);
    assert_eq!(error.status(), Some(-7));
    assert_eq!(error.kind(), ErrorKind::Backend);
    assert_eq!(ShareError::new(ErrorKind::Backend).status(), None);
}

#[test]
fn every_kind_reaches_the_caller_from_a_failed_start() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        for (kind, code) in ALL_KINDS {
            fake.fail_next_share(ShareError::new(kind));
            let error = share.share(text_params("Report")).await.unwrap_err();
            assert_eq!(error.code(), code, "{code}");
            assert_eq!(error.kind(), kind);
            assert_eq!(fake.shares_started(), 0, "no sheet was shown for {code}");
        }
    });
}

#[test]
fn every_kind_reaches_the_caller_as_the_system_answer() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        for (kind, code) in ALL_KINDS {
            let call = {
                let share = share.clone();
                tokio::spawn(async move { share.share(text_params("Report")).await })
            };
            settle(|| fake.pending_share()).await;
            assert!(fake.deliver_share(Err(ShareError::new(kind))), "{code}");
            assert_eq!(call.await.unwrap().unwrap_err().code(), code, "{code}");
        }
    });
}

#[test]
fn the_system_answer_keeps_its_platform_status() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let call = {
            let share = share.clone();
            tokio::spawn(async move { share.share(text_params("Report")).await })
        };
        settle(|| fake.pending_share()).await;
        assert!(fake.deliver_share(Err(ShareError::new(ErrorKind::Backend).with_status(-100))));
        let error = call.await.unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Backend);
        assert_eq!(error.status(), Some(-100));
    });
}

#[test]
fn an_unavailable_result_is_an_answer_not_an_error() {
    let fake = FakeBackend::new();
    let share = Share::with_backend(Arc::new(fake.clone()));
    run(async {
        let call = {
            let share = share.clone();
            tokio::spawn(async move { share.share(text_params("Report")).await })
        };
        settle(|| fake.pending_share()).await;
        assert!(fake.deliver_share(Ok(ShareResult {
            raw: "unavailable".to_owned(),
            status: ShareResultStatus::Unavailable,
        })));
        assert_eq!(
            call.await.unwrap().unwrap().status,
            ShareResultStatus::Unavailable
        );
    });
}
