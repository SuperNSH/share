import type { Result2, ShareBackend } from "./backend";
import { shareError, type ShareError, type ShareErrorCode } from "./errors";
import type { ShareParams, ShareResult, SharedMedia } from "./options";

/** The answers a memory backend starts with. */
export interface MemoryShareConfig {
  available?: boolean;
  canReceive?: boolean;
  initialMedia?: SharedMedia[];
}

/** A backend the test drives: it answers sheets and listeners, and counts what was asked. */
export interface MemoryShare extends ShareBackend {
  /** Makes the next `startShare` throw `code`, as a platform does when it cannot show a sheet. */
  failNextShare(code: ShareErrorCode): void;
  /** Whether a sheet waits for its answer. */
  pendingShare(): boolean;
  /** Answers the waiting sheet. Returns `false` when none waits. */
  deliverShare(answer: ShareResult | ShareErrorCode): boolean;
  /** Answers the waiting listener. Returns `false` when none waits. */
  deliverMedia(answer: SharedMedia | ShareErrorCode): boolean;
  setInitialMedia(media: SharedMedia[]): void;
  sharesStarted(): number;
  cancels(): number;
  listensStarted(): number;
  lastParams(): ShareParams | undefined;
  setAvailable(value: boolean): void;
  setCanReceive(value: boolean): void;
}

/** A memory backend, for tests and for apps that run without a share sheet. */
export function memoryBackend(config: MemoryShareConfig = {}): MemoryShare {
  let available = config.available ?? true;
  let canReceive = config.canReceive ?? false;
  let initial = config.initialMedia ? [...config.initialMedia] : [];
  let nextStartError: ShareErrorCode | null = null;
  let waitingShare: ((result: Result2<ShareResult, ShareError>) => void) | null = null;
  let waitingMedia: ((result: Result2<SharedMedia, ShareError>) => void) | null = null;
  let shares = 0;
  let cancels = 0;
  let listens = 0;
  let last: ShareParams | undefined;

  return {
    isAvailable: async () => available,
    canReceive: async () => canReceive,
    startShare: (params, done) => {
      last = params;
      if (nextStartError !== null) {
        const code = nextStartError;
        nextStartError = null;
        throw shareError(code);
      }
      shares += 1;
      waitingShare = done;
    },
    cancelShare: () => {
      // Like the platform, a cancel does not answer the sheet; only the system's answer does.
      cancels += 1;
    },
    getInitialMedia: async () => [...initial],
    startListen: (done) => {
      listens += 1;
      waitingMedia = done;
    },
    stopListen: () => {
      // Stopping never resolves a waiting listener; deliverMedia still answers it.
    },
    failNextShare: (code) => {
      nextStartError = code;
    },
    pendingShare: () => waitingShare !== null,
    deliverShare: (answer) => {
      const done = waitingShare;
      waitingShare = null;
      done?.(typeof answer === "string" ? { ok: false, error: shareError(answer) } : { ok: true, value: answer });
      return done !== null;
    },
    deliverMedia: (answer) => {
      const done = waitingMedia;
      waitingMedia = null;
      done?.(typeof answer === "string" ? { ok: false, error: shareError(answer) } : { ok: true, value: answer });
      return done !== null;
    },
    setInitialMedia: (media) => {
      initial = [...media];
    },
    sharesStarted: () => shares,
    cancels: () => cancels,
    listensStarted: () => listens,
    lastParams: () => last,
    setAvailable: (value) => {
      available = value;
    },
    setCanReceive: (value) => {
      canReceive = value;
    },
  };
}
