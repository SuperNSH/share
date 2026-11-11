//! `ReceiveSharingIntent` through the fake backend: the cold-start read, one waiter at a time, reset.

use std::{future::Future, sync::Arc};

use nsh_share::{
    ErrorKind, FakeBackend, ReceiveSharingIntent, ShareError, SharedMedia, SharedMediaType,
};

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
    panic!("the listen never started");
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

/// Starts a receive on its own task, and waits until its listener is up.
async fn start_listen(
    intent: &ReceiveSharingIntent,
    fake: &FakeBackend,
) -> tokio::task::JoinHandle<Result<SharedMedia, ShareError>> {
    let call = intent.clone();
    let handle = tokio::spawn(async move { call.receive().await });
    settle(|| fake.listens_started() == 1).await;
    handle
}

#[test]
fn cold_start_send_vs_main() {
    // The real launch-intent read is the Apple and Android bridges'; CI's cross-check compiles them.
    let fake = FakeBackend::new();
    fake.set_initial_media(vec![image("/cache/inbox/photo.png")]);
    let intent = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
    run(async {
        let sent = intent.get_initial_media().await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].path, "/cache/inbox/photo.png");
        assert_eq!(sent[0].kind, SharedMediaType::Image);
        assert_eq!(sent[0].mime_type.as_deref(), Some("image/png"));

        // A MAIN launch or a plain iOS launch starts with nothing, and never an error.
        let plain = ReceiveSharingIntent::with_backend(Arc::new(FakeBackend::new()));
        assert!(plain.get_initial_media().await.unwrap().is_empty());
    });
}

#[test]
fn in_flight_receive_rejected() {
    let fake = FakeBackend::new();
    let intent = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
    run(async {
        let first = start_listen(&intent, &fake).await;
        let second = intent.receive().await.unwrap_err();
        assert_eq!(second.kind(), ErrorKind::InProgress);
        assert_eq!(second.code(), "ERR_SHARE_IN_PROGRESS");
        assert_eq!(fake.listens_started(), 1);

        assert!(fake.deliver_media(Ok(image("/cache/inbox/next.png"))));
        let media = first.await.unwrap().unwrap();
        assert_eq!(media.path, "/cache/inbox/next.png");
        assert_eq!(media.kind, SharedMediaType::Image);
    });
}

#[test]
fn reset_clears_initial_media() {
    let fake = FakeBackend::new();
    fake.set_initial_media(vec![image("/cache/inbox/note.png")]);
    let intent = ReceiveSharingIntent::with_backend(Arc::new(fake.clone()));
    run(async {
        assert_eq!(intent.get_initial_media().await.unwrap().len(), 1);

        // Reset clears what a cold start delivered; a later read starts empty.
        intent.reset();
        assert!(intent.get_initial_media().await.unwrap().is_empty());

        // Reset never resolves a waiting receive; only the backend's answer does.
        let waiting = start_listen(&intent, &fake).await;
        intent.reset();
        assert!(!waiting.is_finished(), "reset must not resolve the wait");
        assert!(fake.deliver_media(Ok(image("/cache/inbox/late.png"))));
        assert_eq!(
            waiting.await.unwrap().unwrap().path,
            "/cache/inbox/late.png"
        );
    });
}

#[test]
fn media_kinds_match_receive_sharing_intent() {
    assert_eq!(SharedMediaType::Image.as_str(), "image");
    assert_eq!(SharedMediaType::Video.as_str(), "video");
    assert_eq!(SharedMediaType::Text.as_str(), "text");
    assert_eq!(SharedMediaType::File.as_str(), "file");
    assert_eq!(SharedMediaType::Url.as_str(), "url");
}
