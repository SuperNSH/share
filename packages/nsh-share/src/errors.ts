/** The codes a share rejection carries. Apps match on `code`. */
export type ShareErrorCode =
  | "ERR_SHARE_NOT_AVAILABLE"
  | "ERR_SHARE_INVALID_ARGUMENT"
  | "ERR_SHARE_IN_PROGRESS"
  | "ERR_SHARE_NO_ACTIVITY"
  | "ERR_SHARE_FILE_NOT_FOUND"
  | "ERR_SHARE_BACKEND";

/** Same wording as the Rust crate's messages. None holds the reason text. */
const MESSAGES: Record<ShareErrorCode, string> = {
  ERR_SHARE_NOT_AVAILABLE: "No share sheet is available on this device.",
  ERR_SHARE_INVALID_ARGUMENT: "Give the share a text, a URI or files, and plain file paths, not URIs.",
  ERR_SHARE_IN_PROGRESS: "Another share sheet is already up. Wait for it to finish.",
  ERR_SHARE_NO_ACTIVITY: "There is no screen to show the share sheet on right now.",
  ERR_SHARE_FILE_NOT_FOUND: "A file the share lists does not exist. Stage it first.",
  ERR_SHARE_BACKEND: "The platform's share service failed.",
};

const KNOWN = new Set<string>(Object.keys(MESSAGES));

/** A rejection of the share API. `code` is the stable name; the message says what to do. */
export class ShareError extends Error {
  readonly code: ShareErrorCode;

  constructor(code: ShareErrorCode) {
    super(MESSAGES[code]);
    this.name = "ShareError";
    this.code = code;
  }
}

/** The error for a known code. */
export function shareError(code: ShareErrorCode): ShareError {
  return new ShareError(code);
}

/**
 * Turns whatever the native side or a host threw into a coded `ShareError`. A host's rejection
 * carries `ERR_NSH_HOST_ERROR`, and its message starts with the code, as
 * `ERR_SHARE_IN_PROGRESS: text`. Anything unknown is a backend failure.
 */
export function asShareError(error: unknown): ShareError {
  if (error instanceof ShareError) return error;
  const raw = (error ?? {}) as { code?: unknown; message?: unknown };
  const code = typeof raw.code === "string" ? raw.code : "";
  const message = typeof raw.message === "string" ? raw.message : "";
  if (code === "ERR_NSH_HOST_ERROR") {
    const named = /^(ERR_SHARE_[A-Z_]+): /.exec(message)?.[1];
    return named !== undefined && KNOWN.has(named)
      ? shareError(named as ShareErrorCode)
      : shareError("ERR_SHARE_BACKEND");
  }
  return KNOWN.has(code) ? shareError(code as ShareErrorCode) : shareError("ERR_SHARE_BACKEND");
}
