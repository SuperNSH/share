import {
  NativeShare,
  type NativeRect,
  type NativeShareFile,
  type NativeShareParams,
  type NativeShareResult,
} from "nsh-share/native";
import type { Result2, ShareBackend } from "./backend";
import { asShareError, type ShareError } from "./errors";
import type { ShareParams, ShareResult, ShareResultStatus, SharedMedia } from "./options";

/**
 * The Rust crate's sharing (`nsh-share/native`). The app or package must list the crate under
 * `"rust"` in `nsh.json`; importing this module without it fails the bundle.
 */
export function rustBackend(): ShareBackend {
  return new RustBackend();
}

/** Only the fields the crate reads; unset ones keep the crate's defaults. */
function nativeParams(params: ShareParams): NativeShareParams {
  const result: NativeShareParams = {};
  if (params.title != null) result.title = params.title;
  if (params.text != null) result.text = params.text;
  if (params.subject != null) result.subject = params.subject;
  if (params.uri != null) result.uri = params.uri;
  if (params.files !== undefined) result.files = nativeFiles(params.files);
  if (params.fileNameOverrides !== undefined) result.fileNameOverrides = params.fileNameOverrides;
  if (params.sharePositionOrigin != null) result.sharePositionOrigin = nativeRect(params.sharePositionOrigin);
  if (params.previewThumbnail != null) result.previewThumbnail = params.previewThumbnail;
  if (params.mailToFallbackEnabled !== undefined) result.mailToFallbackEnabled = params.mailToFallbackEnabled;
  if (params.downloadFallbackEnabled !== undefined) result.downloadFallbackEnabled = params.downloadFallbackEnabled;
  return result;
}

function nativeFiles(files: ShareParams["files"]): NativeShareFile[] {
  return (files ?? []).map((file) => ({ path: file.path, mimeType: file.mimeType ?? null }));
}

function nativeRect(rect: NonNullable<ShareParams["sharePositionOrigin"]>): NativeRect {
  return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
}

function asShareResult(result: NativeShareResult): ShareResult {
  return { raw: result.raw, status: result.status as ShareResultStatus };
}

class RustBackend implements ShareBackend {
  private readonly native = new NativeShare();

  isAvailable(): Promise<boolean> {
    return this.settle(() => this.native.isAvailable());
  }

  canReceive(): Promise<boolean> {
    // The crate exposes no receive-capability query; apps that share in use `receive` directly.
    return Promise.resolve(false);
  }

  startShare(params: ShareParams, done: (result: Result2<ShareResult, ShareError>) => void): void {
    this.native.share(nativeParams(params)).then(
      (result) => done({ ok: true, value: asShareResult(result) }),
      (error: unknown) => done({ ok: false, error: asShareError(error) }),
    );
  }

  cancelShare(): void {
    // The crate's facade exposes no cancel; the sheet's own answer ends every call.
  }

  getInitialMedia(): Promise<SharedMedia[]> {
    return this.settle(async () => (await this.native.getInitialMedia()) as SharedMedia[]);
  }

  startListen(done: (result: Result2<SharedMedia, ShareError>) => void): void {
    this.native.receive().then(
      (media) => done({ ok: true, value: media as SharedMedia }),
      (error: unknown) => done({ ok: false, error: asShareError(error) }),
    );
  }

  stopListen(): void {
    // The crate's facade exposes no stop; a waiting receive ends when the system answers.
  }

  private async settle<T>(operation: () => Promise<T>): Promise<T> {
    try {
      return await operation();
    } catch (error) {
      throw asShareError(error);
    }
  }
}
