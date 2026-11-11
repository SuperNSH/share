//! The share sheet as TSX sees it: `NativeShare`, a class whose async members reject with a `code`
//! on failure. Results are plain values; the sheet and the inbound channel stay on the Rust side.

use nsh_core::prelude::*;

use crate::{ReceiveSharingIntent, Rect, Share, ShareError, ShareFile, ShareParams, SharedMedia};

/// The params of one share, from TSX. Unset fields keep the defaults.
#[derive(JsObject, Clone, Debug, Default)]
pub struct NativeShareParams {
    /// The title the sheet shows where it takes one.
    pub title: Option<String>,
    /// The text body of the share.
    pub text: Option<String>,
    /// The email subject, where the chosen target takes one.
    pub subject: Option<String>,
    /// A URI to share, such as `https://…` or `mailto:…`.
    pub uri: Option<String>,
    /// The files to share, by path. Never a URI; stage the file first.
    pub files: Option<Vec<NativeShareFile>>,
    /// A name per file, renaming it in the sheet.
    pub file_name_overrides: Option<Vec<String>>,
    /// Where a popover sheet anchors, in screen coordinates.
    pub share_position_origin: Option<NativeRect>,
    /// A preview thumbnail, as a URI the platform can load.
    pub preview_thumbnail: Option<String>,
    /// On a target without mail, fall back to a plain share. On unless set.
    pub mail_to_fallback_enabled: Option<bool>,
    /// On a target without downloads, fall back to a plain share. On unless set.
    pub download_fallback_enabled: Option<bool>,
}

/// One file of a share, from TSX.
#[derive(JsObject, Clone, Debug, Default)]
pub struct NativeShareFile {
    /// The file's path on disk.
    pub path: String,
    /// The file's MIME type, when the caller knows it better than the extension does.
    pub mime_type: Option<String>,
}

/// A rectangle in screen coordinates, from TSX.
#[derive(JsObject, Clone, Debug, Default)]
pub struct NativeRect {
    /// The left edge.
    pub x: Option<f64>,
    /// The top edge.
    pub y: Option<f64>,
    /// How wide the anchor is.
    pub width: Option<f64>,
    /// How tall the anchor is.
    pub height: Option<f64>,
}

/// What the system answered about one sheet, for TSX.
#[derive(JsObject, Clone, Debug, Default)]
pub struct NativeShareResult {
    /// The platform's own answer, raw: the activity type chosen, the target package, or empty.
    pub raw: String,
    /// `"success"`, `"dismissed"` or `"unavailable"`.
    pub status: String,
}

/// One inbound shared item, for TSX.
#[derive(JsObject, Clone, Debug, Default)]
pub struct NativeSharedMedia {
    /// Where the platform staged the item, as a plain filesystem path.
    pub path: String,
    /// A thumbnail of the item, for images and videos.
    pub thumbnail: Option<String>,
    /// How long the item plays, for audio and video.
    pub duration: Option<u64>,
    /// `"image"`, `"video"`, `"text"`, `"file"` or `"url"`.
    pub kind: String,
    /// The item's MIME type, when the sender named one.
    pub mime_type: Option<String>,
    /// The text the sender wrote, when the share came with one.
    pub message: Option<String>,
}

/// A share sheet handle. Create one and keep it; every handle shares the platform's sheet gate.
#[class]
pub struct NativeShare {
    share: Share,
    receive: ReceiveSharingIntent,
}

#[class]
impl NativeShare {
    /// The platform's share sheet and inbound channel.
    pub fn new() -> NativeShare {
        NativeShare {
            share: Share::new(),
            receive: ReceiveSharingIntent::new(),
        }
    }

    /// Whether the platform can show a share sheet here.
    pub async fn is_available(this: Instance<Self>) -> Result<bool, JsError> {
        let share = this.with(|native| native.share.clone());
        share.is_available().await.map_err(into_js)
    }

    /// Shows the sheet with `params`. Resolves when the system answers; a dismissed sheet is a
    /// success with status `"dismissed"`.
    pub async fn share(
        this: Instance<Self>,
        params: Option<NativeShareParams>,
    ) -> Result<NativeShareResult, JsError> {
        let share = this.with(|native| native.share.clone());
        let params = to_share_params(params.unwrap_or_default());
        let answer = share.share(params).await.map_err(into_js)?;
        Ok(native_result(answer))
    }

    /// Shares one URI, with an optional title.
    pub async fn share_uri(
        this: Instance<Self>,
        uri: String,
        title: Option<String>,
    ) -> Result<NativeShareResult, JsError> {
        let share = this.with(|native| native.share.clone());
        let answer = share.share_uri(uri, title).await.map_err(into_js)?;
        Ok(native_result(answer))
    }

    /// Shares files, with optional text and an anchor for popover sheets.
    pub async fn share_files(
        this: Instance<Self>,
        files: Vec<NativeShareFile>,
        text: Option<String>,
        origin: Option<NativeRect>,
    ) -> Result<NativeShareResult, JsError> {
        let share = this.with(|native| native.share.clone());
        let answer = share
            .share_files(
                files.into_iter().map(share_file).collect(),
                text,
                native_rect(origin),
            )
            .await
            .map_err(into_js)?;
        Ok(native_result(answer))
    }

    /// The items the app was launched with. A plain launch answers an empty list, never a
    /// rejection.
    pub async fn get_initial_media(
        this: Instance<Self>,
    ) -> Result<Vec<NativeSharedMedia>, JsError> {
        let receive = this.with(|native| native.receive.clone());
        let media = receive.get_initial_media().await.map_err(into_js)?;
        Ok(media.into_iter().map(native_media).collect())
    }

    /// Waits for the next item another app shares in. One waiter at a time.
    pub async fn receive(this: Instance<Self>) -> Result<NativeSharedMedia, JsError> {
        let receive = this.with(|native| native.receive.clone());
        let media = receive.receive().await.map_err(into_js)?;
        Ok(native_media(media))
    }

    /// Clears the stored launch media, so a later `getInitialMedia` answers empty.
    pub fn reset(&self) {
        self.receive.reset();
    }
}

impl Default for NativeShare {
    fn default() -> Self {
        Self::new()
    }
}

/// The error a rejection carries: its message, and the `code` apps match on.
fn into_js(error: ShareError) -> JsError {
    JsError::Error(error.message().to_owned()).with_code(error.code())
}

fn share_file(file: NativeShareFile) -> ShareFile {
    ShareFile {
        path: file.path,
        mime_type: file.mime_type,
    }
}

/// A rect anchors a popover only whole; any missing coordinate drops it.
fn native_rect(rect: Option<NativeRect>) -> Option<Rect> {
    let rect = rect?;
    Some(Rect {
        x: rect.x?,
        y: rect.y?,
        width: rect.width?,
        height: rect.height?,
    })
}

fn native_result(result: crate::share::ShareResult) -> NativeShareResult {
    NativeShareResult {
        raw: result.raw,
        status: result.status.as_str().to_owned(),
    }
}

fn native_media(media: SharedMedia) -> NativeSharedMedia {
    NativeSharedMedia {
        path: media.path,
        thumbnail: media.thumbnail,
        duration: media.duration,
        kind: media.kind.as_str().to_owned(),
        mime_type: media.mime_type,
        message: media.message,
    }
}

/// Turns the TSX params into the crate's, leaving unset fields at their defaults.
fn to_share_params(params: NativeShareParams) -> ShareParams {
    ShareParams {
        title: params.title,
        text: params.text,
        subject: params.subject,
        uri: params.uri,
        files: params
            .files
            .map(|files| files.into_iter().map(share_file).collect())
            .unwrap_or_default(),
        file_name_overrides: params.file_name_overrides.unwrap_or_default(),
        share_position_origin: native_rect(params.share_position_origin),
        preview_thumbnail: params.preview_thumbnail,
        mail_to_fallback_enabled: params.mail_to_fallback_enabled.unwrap_or(true),
        download_fallback_enabled: params.download_fallback_enabled.unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{ErrorKind, ShareResultStatus, SharedMediaType};

    #[test]
    fn unset_params_keep_the_defaults_with_the_fallbacks_on() {
        let params = to_share_params(NativeShareParams::default());
        assert_eq!(params, ShareParams::default());
        assert!(params.mail_to_fallback_enabled);
        assert!(params.download_fallback_enabled);
    }

    #[test]
    fn set_params_carry_through() {
        let params = to_share_params(NativeShareParams {
            title: Some("Look".to_owned()),
            text: Some("A photo".to_owned()),
            subject: Some("Friday".to_owned()),
            uri: Some("https://nsh.dev".to_owned()),
            files: Some(vec![NativeShareFile {
                path: "/tmp/a.png".to_owned(),
                mime_type: Some("image/png".to_owned()),
            }]),
            file_name_overrides: Some(vec!["friday.png".to_owned()]),
            share_position_origin: Some(NativeRect {
                x: Some(1.0),
                y: Some(2.0),
                width: Some(3.0),
                height: Some(4.0),
            }),
            preview_thumbnail: Some("https://nsh.dev/a.png".to_owned()),
            mail_to_fallback_enabled: Some(false),
            download_fallback_enabled: None,
        });
        assert_eq!(params.title.as_deref(), Some("Look"));
        assert_eq!(params.files.len(), 1);
        assert_eq!(params.files[0].mime_type.as_deref(), Some("image/png"));
        assert_eq!(params.file_name_overrides, vec!["friday.png".to_owned()]);
        assert_eq!(
            params.share_position_origin,
            Some(Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0
            })
        );
        assert!(!params.mail_to_fallback_enabled);
        assert!(params.download_fallback_enabled);
    }

    #[test]
    fn a_partial_rect_is_dropped_whole() {
        let whole = NativeRect {
            x: Some(1.0),
            y: Some(2.0),
            width: Some(3.0),
            height: Some(4.0),
        };
        assert!(native_rect(Some(whole.clone())).is_some());
        let partial = NativeRect {
            height: None,
            ..whole
        };
        assert!(native_rect(Some(partial)).is_none());
        assert!(native_rect(None).is_none());
    }

    #[test]
    fn an_error_rejects_with_its_stable_code() {
        let error = into_js(ShareError::new(ErrorKind::InProgress));
        assert_eq!(error.code(), Some("ERR_SHARE_IN_PROGRESS"));
        assert_eq!(error.message(), ErrorKind::InProgress.message());
    }

    #[test]
    fn a_result_carries_its_status_name() {
        let result = native_result(crate::share::ShareResult {
            raw: "com.apple.UIKit.activity.Mail".to_owned(),
            status: ShareResultStatus::Success,
        });
        assert_eq!(result.status, "success");
        assert_eq!(result.raw, "com.apple.UIKit.activity.Mail");
    }

    #[test]
    fn every_media_kind_has_the_name_tsx_uses() {
        assert_eq!(SharedMediaType::Image.as_str(), "image");
        assert_eq!(SharedMediaType::Video.as_str(), "video");
        assert_eq!(SharedMediaType::Text.as_str(), "text");
        assert_eq!(SharedMediaType::File.as_str(), "file");
        assert_eq!(SharedMediaType::Url.as_str(), "url");
    }
}
