import type { Result2, ShareBackend } from "./backend";
import { asShareError, shareError, type ShareError } from "./errors";
import { rustBackend } from "./rust";
import type { SharedMedia } from "./options";

/**
 * Inbound sharing, with `receive_sharing_intent`'s names. It runs on the Rust crate unless it is
 * given another backend. One `receive` waits at a time, and only the backend's answer resolves it.
 */
export class ReceiveSharingIntent {
  private readonly backend: ShareBackend;
  /** The cold-start read, taken once; `null` until then. `reset` empties it. */
  private stored: SharedMedia[] | null = null;
  private listening = false;

  constructor(options: { backend?: ShareBackend } = {}) {
    this.backend = options.backend ?? rustBackend();
  }

  /**
   * The media the app was launched with. A plain launch answers an empty list, never an error.
   * The backend is read once; later calls answer the same list until `reset` empties it.
   */
  async getInitialMedia(): Promise<SharedMedia[]> {
    if (this.stored === null) this.stored = [...(await this.backend.getInitialMedia())];
    return [...this.stored];
  }

  /**
   * Waits for the next item another app shares in. A second call while one waits rejects with
   * `ERR_SHARE_IN_PROGRESS`. Only the backend's answer resolves the call: neither `reset` nor a
   * stopped listen ends it.
   */
  receive(): Promise<SharedMedia> {
    if (this.listening) return Promise.reject(shareError("ERR_SHARE_IN_PROGRESS"));
    this.listening = true;
    return new Promise<SharedMedia>((resolve, reject) => {
      let settled = false;
      const done = (result: Result2<SharedMedia, ShareError>) => {
        if (settled) return;
        settled = true;
        this.listening = false;
        if (result.ok) resolve(result.value);
        else reject(asShareError(result.error));
      };
      try {
        this.backend.startListen(done);
      } catch (error: unknown) {
        // A listen that throws registered no waiter, so the gate frees and `done` never runs.
        done({ ok: false, error: asShareError(error) });
      }
    });
  }

  /** Empties the stored initial media; `getInitialMedia` answers empty from now on. */
  reset(): void {
    this.stored = [];
  }
}
