import { expect, test } from "nsh/test";
import {
  Share,
  memoryBackend,
  type ShareFile,
  type ShareParams,
  type ShareResult,
} from "../src";

/**
 * `ipad_anchor_required` is a unit test in crates/nsh-share/src/platform/apple.rs, which compiles
 * on macOS hosts; `platform_unsupported_reports_not_available` is in
 * crates/nsh-share/src/platform/unsupported.rs, which compiles on every target by design.
 */

/** Lets a delivered answer free the gate, which opens again a microtask later. */
async function delivered(): Promise<void> {
  for (let turn = 0; turn < 4; turn += 1) await Promise.resolve();
}

function success(): ShareResult {
  return { raw: "done", status: "success" };
}

test("cancel_dismissed_not_error", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const call = share.share({ text: "A photo" });
  expect(backend.deliverShare({ raw: "cancelled", status: "dismissed" })).toBe(true);
  // A dismissed sheet is the system's own answer, so the call resolves as a result.
  const result = await call;
  expect(result.status).toBe("dismissed");
});

test("second_share_rejected_in_progress", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const first = share.share({ text: "One" });
  await expect(share.share({ text: "Two" })).rejects.toMatchObject({
    code: "ERR_SHARE_IN_PROGRESS",
  });
  expect(backend.sharesStarted()).toBe(1);
  expect(backend.deliverShare(success())).toBe(true);
  expect((await first).status).toBe("success");
});

test("late_answer_ignored_resolve_once", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const call = share.share({ text: "Once" });
  expect(backend.deliverShare(success())).toBe(true);
  expect((await call).status).toBe("success");

  // A second answer finds no call waiting; nothing resolves again and no counter moves.
  expect(backend.deliverShare(success())).toBe(false);
  expect(backend.sharesStarted()).toBe(1);
  await delivered();

  const next = share.share({ text: "Twice" });
  backend.deliverShare(success());
  expect((await next).status).toBe("success");
});

test("cancel_share_best_effort", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const call = share.share({ text: "Hold" });

  // The cancel reaches the backend seam; it never resolves the call itself.
  backend.cancelShare();
  expect(backend.cancels()).toBe(1);
  expect(backend.pendingShare()).toBe(true);
  await expect(share.share({ text: "Next" })).rejects.toMatchObject({
    code: "ERR_SHARE_IN_PROGRESS",
  });

  // The system's own answer still lands and resolves the held call.
  expect(backend.deliverShare(success())).toBe(true);
  expect((await call).status).toBe("success");
});

test("empty_share_invalid_argument", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  await expect(share.share({})).rejects.toMatchObject({
    code: "ERR_SHARE_INVALID_ARGUMENT",
  });
  expect(backend.sharesStarted()).toBe(0);
});

test("file_uri_rejected_invalid_argument", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  for (const path of ["file:///tmp/report.pdf", "content://media/external/1"]) {
    await expect(share.share({ files: [{ path }] })).rejects.toMatchObject({
      code: "ERR_SHARE_INVALID_ARGUMENT",
    });
  }
  expect(backend.sharesStarted()).toBe(0);
});

test("missing_file_file_not_found", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  // The Rust facade stats paths off-wasm; this side models the bridge's staging failure.
  backend.failNextShare("ERR_SHARE_FILE_NOT_FOUND");
  await expect(share.share({ text: "Staged" })).rejects.toMatchObject({
    code: "ERR_SHARE_FILE_NOT_FOUND",
  });
  expect(backend.sharesStarted()).toBe(0);
});

test("large_file_temp_copy", async () => {
  // The temp copy to a provider URI lives in the Android bridge; CI's cross-check compiles it.
  const backend = memoryBackend();
  const share = new Share({ backend });
  const files: ShareFile[] = [
    { path: "/cache/export/large.bin", mimeType: "application/octet-stream" },
  ];
  const call = share.shareFiles(files, "The export");

  const params = backend.lastParams();
  expect(params?.files).toEqual(files);
  expect(params?.text).toBe("The export");
  expect(params?.files?.[0]?.path.includes("://")).toBe(false);

  backend.deliverShare(success());
  expect((await call).status).toBe("success");
});

test("no_activity_error", async () => {
  // No foreground Activity or window is detected in the Android and Windows bridges; CI compiles them.
  const backend = memoryBackend();
  const share = new Share({ backend });
  backend.failNextShare("ERR_SHARE_NO_ACTIVITY");
  await expect(share.share({ text: "Read this" })).rejects.toMatchObject({
    code: "ERR_SHARE_NO_ACTIVITY",
  });
  expect(backend.sharesStarted()).toBe(0);

  // A failed start frees the gate: the next call shows its sheet.
  const next = share.share({ text: "Again" });
  backend.deliverShare(success());
  expect((await next).status).toBe("success");
});

test("web_share_unavailable_not_available", async () => {
  // The navigator.share check and the AbortError mapping are the web bridge's; CI compiles it.
  const backend = memoryBackend();
  const share = new Share({ backend });
  backend.setAvailable(false);
  expect(await share.isAvailable()).toBe(false);

  backend.failNextShare("ERR_SHARE_NOT_AVAILABLE");
  await expect(share.share({ text: "A link" })).rejects.toMatchObject({
    code: "ERR_SHARE_NOT_AVAILABLE",
  });

  // A user abort lands as a dismissed answer, which is a result, not a rejection.
  const aborting = share.share({ text: "Again" });
  expect(backend.deliverShare({ raw: "AbortError", status: "dismissed" })).toBe(true);
  expect((await aborting).status).toBe("dismissed");
});

test("the convenience calls carry what they are given", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const params: ShareParams = { text: "Plain" };

  const uriCall = share.shareUri("https://nsh.dev", "NSH");
  expect(backend.lastParams()?.uri).toBe("https://nsh.dev");
  expect(backend.lastParams()?.title).toBe("NSH");
  backend.deliverShare(success());
  expect((await uriCall).status).toBe("success");

  const plain = share.share(params);
  expect(backend.lastParams()?.text).toBe("Plain");
  backend.deliverShare(success());
  expect((await plain).status).toBe("success");
});
