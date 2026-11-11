import type { ShareError } from "./errors";
import type { ShareParams, ShareResult, SharedMedia } from "./options";

/** A result handed to a `done` callback: `Ok` as `{ ok: true }`, `Err` as `{ ok: false }`. */
export type Result2<T, E> = { ok: true; value: T } | { ok: false; error: E };

/** What a `Share` and a `ReceiveSharingIntent` run on: the Rust crate, or a memory backend in tests. */
export interface ShareBackend {
  isAvailable(): Promise<boolean>;
  canReceive(): Promise<boolean>;
  /** Starts the sheet and returns at once. `done` is called exactly once, when the system answers.
   * A throw means no sheet was shown, and `done` is not called. */
  startShare(params: ShareParams, done: (result: Result2<ShareResult, ShareError>) => void): void;
  /** Asks the system to dismiss the sheet. Best-effort: the sheet's own answer still arrives. */
  cancelShare(): void;
  /** The media the app was launched with. A plain launch answers an empty list, never an error. */
  getInitialMedia(): Promise<SharedMedia[]>;
  /** Starts listening and returns at once. `done` is called exactly once, with the next inbound item. */
  startListen(done: (result: Result2<SharedMedia, ShareError>) => void): void;
  stopListen(): void;
}
