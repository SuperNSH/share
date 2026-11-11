//! The web: `navigator.share` in a secure context, with the download fallback share_plus plays
//! when the API level will not take the files. Nothing arrives from outside the page, so
//! receiving answers empty.

use std::future::ready;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{File, FilePropertyBag, HtmlAnchorElement, Navigator, Response, ShareData, Window};

use crate::{
    backend::{Backend, BackendFuture, MediaDone, ShareDone},
    error::{ErrorKind, ShareError},
    options::ShareParams,
    receive::SharedMedia,
    share::{ShareResult, ShareResultStatus},
};

pub(crate) struct NativeBackend;

/// Whether `navigator.share` exists here. `Reflect::get` reads the property without calling it,
/// so a browser without the API answers instead of throwing.
fn has_share(navigator: &Navigator) -> bool {
    js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("share"))
        .map(|value| !value.is_undefined())
        .unwrap_or(false)
}

/// Whether this page may show a sheet: `navigator.share` exists, and the context is secure.
fn sheet_available(window: &Window) -> bool {
    has_share(&window.navigator()) && window.is_secure_context()
}

/// Whether this navigator's API level takes files: `canShare` must exist and accept the data.
/// Reading the property first avoids calling a method an older browser lacks, which would throw;
/// a browser without `canShare` gets the fallback, as it does in share_plus.
fn takes_files(navigator: &Navigator, data: &ShareData) -> bool {
    let has_can_share = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("canShare"))
        .map(|value| !value.is_undefined())
        .unwrap_or(false);
    has_can_share && navigator.can_share_with_data(data)
}

/// The DOMException name a rejection carries, empty when the browser gave none.
fn rejection_name(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default()
}

/// The name a file carries in the sheet: the caller's override, else the last path segment.
fn file_name(params: &ShareParams, index: usize, path: &str) -> String {
    params
        .file_name_overrides
        .get(index)
        .cloned()
        .unwrap_or_else(|| {
            path.rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())
                .unwrap_or(path)
                .to_owned()
        })
}

/// share_plus's fallback answer: the files left through a download, not a sheet, so the call
/// still resolves exactly once, as unavailable.
fn fallback_result() -> ShareResult {
    ShareResult {
        raw: String::new(),
        status: ShareResultStatus::Unavailable,
    }
}

/// Clicks a hidden anchor that saves its href as a download. Appended first because Firefox
/// will not click a detached anchor, and removed in the same turn, so it never paints.
fn click_anchor(window: &Window, href: &str, download: &str) {
    let Some(document) = window.document() else {
        return;
    };
    let Ok(element) = document.create_element("a") else {
        return;
    };
    let Ok(anchor) = element.dyn_into::<HtmlAnchorElement>() else {
        return;
    };
    anchor.set_href(href);
    anchor.set_download(download);
    let attached = document
        .body()
        .is_some_and(|body| body.append_child(&anchor).is_ok());
    anchor.click();
    if attached {
        if let Some(body) = document.body() {
            let _ = body.remove_child(&anchor);
        }
    }
}

/// share_plus's fallback when the sheet will not take the files: each staged path saves as a
/// download while that flag is set; with it off, the answer is `NotAvailable`.
fn download_files(window: &Window, params: &ShareParams) -> Result<ShareResult, ShareError> {
    if !params.download_fallback_enabled {
        return Err(ShareError::new(ErrorKind::NotAvailable));
    }
    for (index, file) in params.files.iter().enumerate() {
        // A staged web path is already a URL the browser can save; no bytes are read here.
        click_anchor(window, &file.path, &file_name(params, index, &file.path));
    }
    Ok(fallback_result())
}

/// Fetches one staged path and wraps the bytes in a File the sheet can take. A path the page
/// cannot fetch answers `None`; the caller then falls back to saving the path itself.
async fn stage_file(path: &str, name: &str, mime: &str) -> Option<File> {
    let window = web_sys::window()?;
    let response = JsFuture::from(window.fetch_with_str(path))
        .await
        .ok()?
        .dyn_into::<Response>()
        .ok()?;
    let promise = response.array_buffer().ok()?;
    let buffer = JsFuture::from(promise).await.ok()?;
    let bag = FilePropertyBag::new();
    bag.set_type(mime);
    // web-sys takes fileBits as one JS value, so the buffer rides in a one-element array.
    let bits = js_sys::Array::of1(&buffer);
    File::new_with_buffer_source_sequence_and_options(&JsValue::from(bits), name, &bag).ok()
}

/// Stages every file for the sheet as one JS array, or `None` when any path cannot be fetched.
async fn stage_files(params: &ShareParams) -> Option<js_sys::Array> {
    let staged = js_sys::Array::new();
    for (index, file) in params.files.iter().enumerate() {
        let name = file_name(params, index, &file.path);
        let mime = file
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        staged.push(&JsValue::from(stage_file(&file.path, &name, mime).await?));
    }
    Some(staged)
}

/// Awaits the sheet's one answer: shared, dismissed by the user, or failed.
async fn await_sheet(navigator: &Navigator, data: ShareData) -> Result<ShareResult, ShareError> {
    match JsFuture::from(navigator.share_with_data(&data)).await {
        Ok(_) => Ok(ShareResult {
            raw: String::new(),
            status: ShareResultStatus::Success,
        }),
        Err(error) => match rejection_name(&error).as_str() {
            // The user closed the sheet: a dismissal, which the facade reports as an Ok answer.
            "AbortError" => Ok(ShareResult {
                raw: String::new(),
                status: ShareResultStatus::Dismissed,
            }),
            // Every other refusal is the backend's own failure. ShareError's status is a number
            // in the fixed API, so the browser's message cannot ride in it.
            _ => Err(ShareError::new(ErrorKind::Backend)),
        },
    }
}

/// Builds the ShareData, gates files on the API level, and runs the sheet.
async fn run_share(window: &Window, params: &ShareParams) -> Result<ShareResult, ShareError> {
    let navigator = window.navigator();
    let data = ShareData::new();
    // share_plus titles the sheet with the subject first and the title second.
    if let Some(title) = params.subject.as_deref().or(params.title.as_deref()) {
        data.set_title(title);
    }
    if let Some(text) = params.text.as_deref() {
        data.set_text(text);
    }
    if let Some(uri) = params.uri.as_deref() {
        data.set_url(uri);
    }
    if params.files.is_empty() {
        return await_sheet(&navigator, data).await;
    }
    // A path the page cannot fetch cannot ride in the sheet; save it instead.
    let Some(files) = stage_files(params).await else {
        return download_files(window, params);
    };
    data.set_files(&JsValue::from(files));
    if takes_files(&navigator, &data) {
        await_sheet(&navigator, data).await
    } else {
        // The API level refuses files: save them, the flag-gated way share_plus does.
        download_files(window, params)
    }
}

impl Backend for NativeBackend {
    fn is_available(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        Box::pin(ready(Ok::<bool, ShareError>(
            web_sys::window().is_some_and(|window| sheet_available(&window)),
        )))
    }

    fn can_receive(&self) -> BackendFuture<'_, Result<bool, ShareError>> {
        // The web has no inbound share surface: nothing arrives from outside the page.
        Box::pin(ready(Ok::<bool, ShareError>(false)))
    }

    fn start_share(&self, params: &ShareParams, done: ShareDone) -> Result<(), ShareError> {
        let Some(window) = web_sys::window() else {
            // wasm outside a browser, such as a worker, has no sheet and no DOM to fall back on.
            return Err(ShareError::new(ErrorKind::NotAvailable));
        };
        if !sheet_available(&window) {
            // No navigator.share, or an insecure context: no sheet exists, so done stays uncalled.
            return Err(ShareError::new(ErrorKind::NotAvailable));
        }
        let params = params.clone();
        spawn_local(async move {
            done(run_share(&window, &params).await);
        });
        Ok(())
    }

    fn cancel_share(&self) {
        // The browser owns the sheet and offers no programmatic dismiss; the user's own cancel
        // still arrives, as the AbortError that maps to Dismissed.
    }

    fn get_initial_media(&self) -> BackendFuture<'_, Result<Vec<SharedMedia>, ShareError>> {
        Box::pin(ready(Ok::<Vec<SharedMedia>, ShareError>(Vec::new())))
    }

    fn start_listen(&self, _done: MediaDone) -> Result<(), ShareError> {
        // Nothing can ever arrive on the web, so a listener could only hang; refusing keeps the
        // call resolved exactly once.
        Err(ShareError::new(ErrorKind::NotAvailable))
    }

    fn stop_listen(&self) {}
}
