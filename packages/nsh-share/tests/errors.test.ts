import { expect, test } from "nsh/test";
import { Share, asShareError, memoryBackend, shareError, type ShareErrorCode } from "../src";

const CODES: ShareErrorCode[] = [
  "ERR_SHARE_NOT_AVAILABLE",
  "ERR_SHARE_INVALID_ARGUMENT",
  "ERR_SHARE_IN_PROGRESS",
  "ERR_SHARE_NO_ACTIVITY",
  "ERR_SHARE_FILE_NOT_FOUND",
  "ERR_SHARE_BACKEND",
];

test("every code is constructible with its message", () => {
  for (const code of CODES) {
    const error = shareError(code);
    expect(error.code).toBe(code);
    expect(error.name).toBe("ShareError");
    expect(error.message).not.toBe("");
  }
});

test("every code reaches the caller from a failed start", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  for (const code of CODES) {
    backend.failNextShare(code);
    await expect(share.share({ text: "Report" })).rejects.toMatchObject({ code });
    expect(backend.sharesStarted()).toBe(0);
  }
});

test("every code reaches the caller as the system answer", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  for (const code of CODES) {
    const call = share.share({ text: "Report" });
    expect(backend.deliverShare(code)).toBe(true);
    await expect(call).rejects.toMatchObject({ code });
  }
});

test("an unavailable result is an answer, not an error", async () => {
  const backend = memoryBackend();
  const share = new Share({ backend });
  const call = share.share({ text: "Report" });
  backend.deliverShare({ raw: "unavailable", status: "unavailable" });
  expect((await call).status).toBe("unavailable");
});

test("a native rejection keeps its code, and an unknown one is a backend failure", () => {
  expect(asShareError({ code: "ERR_SHARE_FILE_NOT_FOUND", message: "x" }).code).toBe(
    "ERR_SHARE_FILE_NOT_FOUND",
  );
  expect(asShareError({ code: "ERR_NSH_HOST_ERROR", message: "ERR_SHARE_NO_ACTIVITY: text" }).code).toBe(
    "ERR_SHARE_NO_ACTIVITY",
  );
  expect(asShareError({ code: "ERR_NSH_HOST_ERROR", message: "ERR_SHARE_NOT_A_CODE: text" }).code).toBe(
    "ERR_SHARE_BACKEND",
  );
  expect(asShareError(new Error("boom")).code).toBe("ERR_SHARE_BACKEND");
});
