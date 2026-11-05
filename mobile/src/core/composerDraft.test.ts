import { afterEach, expect, test } from "bun:test";
import type { PickedFile } from "./engine";
import { clearComposerDrafts, composerDraft, discardPastedImages, editComposerDraft, releaseComposerDraft, removeComposerDraft, setPastedImageCleanup, submitComposerDraft } from "./composerDraft";

const pasted = (uri = "file:///cache/Pasted/image.png"): PickedFile => ({ uri, name: "Pasted Image.png", mime: "image/png", pasted: true });
const draft = (chat = "chat") => composerDraft("identity", "relay", chat);
const removed: string[] = [];
function start() {
  removed.length = 0;
  setPastedImageCleanup(uris => { removed.push(...uris); });
  const value = draft();
  editComposerDraft(value, { attachments: [pasted()] });
  return value;
}
function deferred() {
  let resolve!: (value: boolean) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<boolean>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
afterEach(() => { clearComposerDrafts(); setPastedImageCleanup(() => {}); });

test("removing a paste deletes it, but never photo or document picker sources", () => {
  const value = start();
  const picked = { ...pasted("file:///photos/original.png"), pasted: undefined };
  editComposerDraft(value, { attachments: [...value.attachments, picked] });
  editComposerDraft(value, { attachments: [] });
  expect(removed).toEqual([pasted().uri]);
  discardPastedImages([picked]);
  expect(removed).toHaveLength(1);
});

test("rejected or late paste results are removed, accepted ones remain owned", () => {
  start();
  const rejected = pasted("file:///cache/Pasted/rejected.png");
  discardPastedImages([pasted(), rejected, rejected]);
  expect(removed).toEqual([rejected.uri]);
});

test("unmount and navigation retain a draft and its pasted images", () => {
  const value = start();
  releaseComposerDraft(value);
  draft("other");
  expect(draft()).toBe(value);
  expect(removed).toEqual([]);
});

test("a shared paste survives until its last draft releases it", () => {
  const first = start();
  editComposerDraft(draft("other"), { attachments: [pasted()] });
  editComposerDraft(first, { attachments: [] });
  expect(removed).toEqual([]);
  removeComposerDraft("identity", "relay", "other");
  expect(removed).toEqual([pasted().uri]);
});

test("acknowledged sends delete the paste only after the core has finished", async () => {
  const value = start();
  const send = deferred();
  const pending = submitComposerDraft(value, () => send.promise);
  expect(removed).toEqual([]);
  send.resolve(true); await pending;
  expect(value.attachments).toEqual([]);
  expect(removed).toEqual([pasted().uri]);
});

test("failed and uncertain sends preserve attachments for retry", async () => {
  const value = start();
  for (const send of [async () => false, async () => { throw new Error("response lost"); }]) {
    await submitComposerDraft(value, send);
    expect(value.uncertain).toBe(true);
    expect(value.attachments).toEqual([pasted()]);
    expect(removed).toEqual([]);
  }
  await submitComposerDraft(value, async () => true);
  expect(removed).toEqual([pasted().uri]);
});

test("editing or removing an attachment during send pins its submitted file", async () => {
  const value = start();
  const send = deferred();
  let submitted: PickedFile[] = [];
  const pending = submitComposerDraft(value, (_text, files) => { submitted = files; return send.promise; });
  const newer = pasted("file:///cache/Pasted/new.png");
  editComposerDraft(value, { text: "new draft", attachments: [newer] });
  expect(submitted).toEqual([pasted()]);
  expect(removed).toEqual([]);
  send.resolve(true); await pending;
  expect(value.attachments).toEqual([newer]);
  expect(value.text).toBe("new draft");
  expect(removed).toEqual([pasted().uri]);
});

test("a newer draft retaining the sent image keeps it after acknowledgement", async () => {
  const value = start();
  const send = deferred();
  const pending = submitComposerDraft(value, () => send.promise);
  editComposerDraft(value, { text: "newer" });
  send.resolve(true); await pending;
  expect(value.attachments).toEqual([pasted()]);
  expect(removed).toEqual([]);
});

test("account reset and chat removal defer deletion until an in-flight send settles", async () => {
  for (const discard of [clearComposerDrafts, () => removeComposerDraft("identity", "relay", "chat")]) {
    for (const success of [true, false]) {
      const value = start();
      const send = deferred();
      const pending = submitComposerDraft(value, () => send.promise);
      discard();
      expect(value.invalidated).toBe(true);
      expect(removed).toEqual([]);
      if (success) send.resolve(true); else send.reject(new Error("rejected"));
      await pending;
      expect(removed).toEqual([pasted().uri]);
    }
  }
});

test("concurrent sends keep a shared file until both settle", async () => {
  const first = start();
  const second = draft("other");
  editComposerDraft(second, { attachments: [pasted()] });
  const a = deferred(), b = deferred();
  const pendingA = submitComposerDraft(first, () => a.promise);
  const pendingB = submitComposerDraft(second, () => b.promise);
  clearComposerDrafts();
  a.resolve(true); await pendingA;
  expect(removed).toEqual([]);
  b.resolve(true); await pendingB;
  expect(removed).toEqual([pasted().uri]);
});

test("cleanup errors never change a successful send into a failed one", async () => {
  for (const cleanup of [() => { throw new Error("unavailable"); }, async () => { throw new Error("unavailable"); }]) {
    const value = start();
    setPastedImageCleanup(cleanup);
    await submitComposerDraft(value, async () => true);
    expect(value.attachments).toEqual([]);
    expect(value.uncertain).toBe(false);
  }
});
