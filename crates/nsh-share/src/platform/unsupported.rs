//! A target with no share backend: nothing is available, and no sheet can be shown. Unlike the
//! bridge modules this one is compiled under `cfg(test)` on every target, so its contract is
//! unit-tested on every runner; `mod.rs` wires it in only where no real backend exists.

use std::future::ready;

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::SharedMedia,
};

pub(crate) struct NativeBackend;

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(Ok::<bool, ShareError>(false)))
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(Ok::<bool, ShareError>(false)))
    }

    fn start_share(&self, _params: &ShareParams, _done: ShareDone) -> Result<(), ShareError> {
        Err(ShareError::new(ErrorKind::NotAvailable))
    }

    fn cancel_share(&self) {}

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        Box::pin(ready(Ok::<Vec<SharedMedia>, ShareError>(Vec::new())))
    }

    fn start_listen(&self, _done: MediaDone) -> Result<(), ShareError> {
        Err(ShareError::new(ErrorKind::NotAvailable))
    }

    fn stop_listen(&self) {}
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use crate::backend::Backend;

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn platform_unsupported_reports_not_available() {
        run(async {
            assert!(!super::NativeBackend.is_available().await.unwrap());
            assert!(!super::NativeBackend.can_receive().await.unwrap());
            assert!(
                super::NativeBackend
                    .get_initial_media()
                    .await
                    .unwrap()
                    .is_empty()
            );
            let shown = super::NativeBackend.start_share(
                &crate::ShareParams::default(),
                Box::new(|_| panic!("no sheet was shown, so done is never called")),
            );
            assert_eq!(shown.unwrap_err().code(), "ERR_SHARE_NOT_AVAILABLE");
            let heard = super::NativeBackend.start_listen(Box::new(|_| {
                panic!("no listener was started, so done is never called")
            }));
            assert_eq!(heard.unwrap_err().code(), "ERR_SHARE_NOT_AVAILABLE");
        });
    }
}
