import { expect, test } from "nsh/test";
import { ReceiveSharingIntent, memoryBackend, type SharedMedia } from "../src";

/** Lets a settled promise's continuations run, so an unresolved one stays provably pending. */
async function delivered(): Promise<void> {
  for (let turn = 0; turn < 4; turn += 1) await Promise.resolve();
}

function image(path: string): SharedMedia {
  return { path, kind: "image", mimeType: "image/png" };
}

test("cold_start_send_vs_main", async () => {
  // The real launch-intent read is the Apple and Android bridges'; CI's cross-check compiles them.
  const backend = memoryBackend({ initialMedia: [image("/cache/inbox/photo.png")] });
  const intent = new ReceiveSharingIntent({ backend });
  const sent = await intent.getInitialMedia();
  expect(sent).toEqual([image("/cache/inbox/photo.png")]);

  // A MAIN launch or a plain iOS launch starts with nothing, and never an error.
  const plain = new ReceiveSharingIntent({ backend: memoryBackend() });
  expect(await plain.getInitialMedia()).toEqual([]);
});

test("in_flight_receive_rejected", async () => {
  const backend = memoryBackend();
  const intent = new ReceiveSharingIntent({ backend });
  const first = intent.receive();
  await expect(intent.receive()).rejects.toMatchObject({
    code: "ERR_SHARE_IN_PROGRESS",
  });
  expect(backend.listensStarted()).toBe(1);

  expect(backend.deliverMedia(image("/cache/inbox/next.png"))).toBe(true);
  expect(await first).toEqual(image("/cache/inbox/next.png"));
});

test("reset_clears_initial_media", async () => {
  const backend = memoryBackend({ initialMedia: [image("/cache/inbox/note.png")] });
  const intent = new ReceiveSharingIntent({ backend });
  expect((await intent.getInitialMedia()).length).toBe(1);

  // Reset empties what a cold start delivered; a later read starts from nothing.
  intent.reset();
  expect(await intent.getInitialMedia()).toEqual([]);

  // Reset never resolves a waiting receive; only the backend's answer does.
  const waiting = intent.receive();
  let settled = false;
  void waiting.then(
    () => {
      settled = true;
    },
    () => {
      settled = true;
    },
  );
  intent.reset();
  await delivered();
  expect(settled).toBe(false);

  expect(backend.deliverMedia(image("/cache/inbox/late.png"))).toBe(true);
  expect(await waiting).toEqual(image("/cache/inbox/late.png"));
});

test("a media error reaches the caller with its code", async () => {
  const backend = memoryBackend();
  const intent = new ReceiveSharingIntent({ backend });
  const call = intent.receive();
  expect(backend.deliverMedia("ERR_SHARE_BACKEND")).toBe(true);
  await expect(call).rejects.toMatchObject({ code: "ERR_SHARE_BACKEND" });

  // The answer freed the waiter: the next receive listens again.
  const next = intent.receive();
  expect(backend.listensStarted()).toBe(2);
  backend.deliverMedia(image("/cache/inbox/after.png"));
  expect(await next).toEqual(image("/cache/inbox/after.png"));
});
