//! What a share carries: text and URIs, the files, and where a popover sheet anchors.

/// A rectangle in the screen's coordinates, where an iPad popover or a desktop sheet anchors.
#[derive(PartialEq, Debug, Clone, Copy, Default)]
pub struct Rect {
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
    /// How wide the anchor is.
    pub width: f64,
    /// How tall the anchor is.
    pub height: f64,
}

/// One file of a share. `path` is a plain filesystem path; bridges turn it into the provider URI
/// their platform needs, so a `file://` or `content://` URI here is refused.
#[derive(PartialEq, Debug, Clone, Default)]
pub struct ShareFile {
    /// The file's path on disk.
    pub path: String,
    /// The file's MIME type, when the caller knows it better than the extension does.
    pub mime_type: Option<String>,
}

/// What one share call carries, after `share_plus`'s `ShareParams`.
///
/// A share needs a `text`, a `uri` or `files`; the rest is optional. Both fallbacks default to on.
#[derive(PartialEq, Debug, Clone)]
pub struct ShareParams {
    /// The title the sheet shows where it takes one.
    pub title: Option<String>,
    /// The text body of the share.
    pub text: Option<String>,
    /// The email subject, where the chosen target takes one.
    pub subject: Option<String>,
    /// A URI to share, such as `https://…` or `mailto:…`.
    pub uri: Option<String>,
    /// The files to share, by path.
    pub files: Vec<ShareFile>,
    /// A name per file, renaming it in the sheet; shortens or pads to `files`.
    pub file_name_overrides: Vec<String>,
    /// Where a popover sheet anchors, in screen coordinates.
    pub share_position_origin: Option<Rect>,
    /// A preview thumbnail, as a URI the platform can load.
    pub preview_thumbnail: Option<String>,
    /// On a target without mail, fall back to sharing the text as a plain share.
    pub mail_to_fallback_enabled: bool,
    /// On a target without downloads, fall back to saving through a plain share.
    pub download_fallback_enabled: bool,
}

impl Default for ShareParams {
    /// Everything off or empty, except the two fallbacks, which share_plus leaves on.
    fn default() -> Self {
        Self {
            title: None,
            text: None,
            subject: None,
            uri: None,
            files: Vec::new(),
            file_name_overrides: Vec::new(),
            share_position_origin: None,
            preview_thumbnail: None,
            mail_to_fallback_enabled: true,
            download_fallback_enabled: true,
        }
    }
}
