import type { Result2, ShareBackend } from "./backend";
import { asShareError, shareError, type ShareError, type ShareErrorCode } from "./errors";
import { rustBackend } from "./rust";
import type { Rect, ShareFile, ShareParams, ShareResult } from "./options";

/** The arguments of `new Share`. */
export interface ShareArgs {
  /** What runs the share. Defaults to the Rust crate's. */
  backend?: ShareBackend;
}

/**
 * Sharing out of the app, with `share_plus`'s names. It runs on the Rust crate unless it is given
 * another backend. One sheet runs at a time, and each call resolves exactly once: a dismissal is
 * an answer, not an error.
 */
export class Share {
  private readonly backend: ShareBackend;
  /** The generation that owns the sheet gate; `null` when no sheet is up. A late answer from an
   * older generation is ignored, so it cannot resolve or unblock a newer call. */
  private owner: number | null = null;
  private nextGeneration = 0;

  constructor(options: ShareArgs = {}) {
    this.backend = options.backend ?? rustBackend();
  }

  async isAvailable(): Promise<boolean> {
    return this.backend.isAvailable();
  }

  /**
   * Shows the share sheet and waits for its answer. Params with no text, no URI and no files are
   * refused, as is a file path that is a URI; whether a listed file exists is the crate's check.
   * A second call while a sheet is up rejects with `ERR_SHARE_IN_PROGRESS`, and only the system's
   * answer opens the gate again. A dismissal resolves with `status: "dismissed"`.
   */
  share(params: ShareParams): Promise<ShareResult> {
    const problem = invalidShare(params);
    if (problem !== undefined) return Promise.reject(shareError(problem));
    if (this.owner !== null) return Promise.reject(shareError("ERR_SHARE_IN_PROGRESS"));
    const generation = this.nextGeneration;
    this.nextGeneration += 1;
    this.owner = generation;
    return new Promise<ShareResult>((resolve, reject) => {
      const done = (result: Result2<ShareResult, ShareError>) => {
        if (this.owner !== generation) return;
        this.owner = null;
        if (result.ok) resolve(result.value);
        else reject(asShareError(result.error));
      };
      try {
        this.backend.startShare(params, done);
      } catch (error: unknown) {
        // A start that throws showed no sheet, so the gate frees and `done` never runs.
        if (this.owner === generation) this.owner = null;
        reject(asShareError(error));
      }
    });
  }

  /** Shares one URI, with an optional title. */
  shareUri(uri: string, title?: string): Promise<ShareResult> {
    return this.share({ uri, title });
  }

  /** Shares files staged on disk, with optional text and a sheet origin. */
  shareFiles(files: ShareFile[], text?: string, origin?: Rect): Promise<ShareResult> {
    return this.share({ files, text, sharePositionOrigin: origin });
  }
}

/** The first rule the params break, or `undefined` when they hold a share. An empty text or URI
 * counts as none, as in the crate; a blank-but-non-empty one does not. */
function invalidShare(params: ShareParams): ShareErrorCode | undefined {
  const hasText = (params.text ?? "") !== "";
  const hasUri = (params.uri ?? "") !== "";
  const files = params.files ?? [];
  if (!hasText && !hasUri && files.length === 0) {
    return "ERR_SHARE_INVALID_ARGUMENT";
  }
  // Whether a listed file exists is the crate's check: TSX has no filesystem to ask. Bridges
  // stage files and turn the paths into provider URIs themselves; a URI here is a mistake.
  for (const file of files) {
    if (file.path.includes("://") || file.path.startsWith("file:")) {
      return "ERR_SHARE_INVALID_ARGUMENT";
    }
  }
  return undefined;
}
