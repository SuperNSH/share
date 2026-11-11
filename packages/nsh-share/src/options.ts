/** Where a share sheet opens on a tablet or desktop window. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** One file to share. `path` is a plain filesystem path, never a `file://` URI. */
export interface ShareFile {
  path: string;
  /** The file's MIME type, when the app knows it. The platform guesses when left out. */
  mimeType?: string | null;
}

/** What one share sends, with share_plus's names where it has them. */
export interface ShareParams {
  /** The sheet's title. Android uses it; other platforms show their own. */
  title?: string | null;
  /** The text to share. */
  text?: string | null;
  /** The subject of a mail share, where the target is a mail app. */
  subject?: string | null;
  /** The URI to share, such as `https://example.com`. */
  uri?: string | null;
  /** Files staged on disk and sent with the share. */
  files?: ShareFile[];
  /** One name per file, replacing what the path names. */
  fileNameOverrides?: string[];
  /** Where the sheet opens. iPad and desktop only. */
  sharePositionOrigin?: Rect | null;
  /** A preview image for a link share, as a URI. */
  previewThumbnail?: string | null;
  /** Offer a mail-to fallback when no target takes the share. On by default. */
  mailToFallbackEnabled?: boolean;
  /** Offer a download fallback for a shared URI. On by default. */
  downloadFallbackEnabled?: boolean;
}

/** What a share came back with: share_plus's result values. */
export type ShareResultStatus = "success" | "dismissed" | "unavailable";

/** The answer of one share. `raw` is the platform's own result string, when it gives one. */
export interface ShareResult {
  raw: string;
  status: ShareResultStatus;
}

/** A kind of shared media, as `receive_sharing_intent` names it. */
export type SharedMediaType = "image" | "video" | "text" | "file" | "url";

/** One item another app shared into this one. */
export interface SharedMedia {
  /** The staged copy of the media on disk. */
  path: string;
  /** A preview image for a video, when the platform made one. */
  thumbnail?: string | null;
  /** A video or audio item's length, in milliseconds. */
  duration?: number | null;
  kind: SharedMediaType;
  /** The item's MIME type, when the sending app named it. */
  mimeType?: string | null;
  /** The text of a text or URL share. */
  message?: string | null;
}
