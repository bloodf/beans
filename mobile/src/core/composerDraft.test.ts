import { afterEach, expect, test } from "vitest";
import { clearComposerDrafts, composerDraft, editComposerDraft, invalidateComposerSends, removeComposerDraft, submitComposerDraft } from "./composerDraft";

afterEach(clearComposerDrafts);

test("acknowledgement cannot clear a replacement draft with the same key", async () => {
  const original = composerDraft("account", "relay", "chat");
  editComposerDraft(original, { text: "sent" });
  const pending = Promise.withResolvers<boolean>();
  const sent = submitComposerDraft(original, () => pending.promise);
  removeComposerDraft("account", "relay", "chat");
  const replacement = composerDraft("account", "relay", "chat");
  editComposerDraft(replacement, { text: "replacement" });
  pending.resolve(true);
  await sent;
  expect(replacement.text).toBe("replacement");
});

test("a validated acknowledgement settles an unchanged draft after authority revocation", async () => {
  const draft = composerDraft("account", "relay", "chat");
  editComposerDraft(draft, { text: "sent" });
  const pending = Promise.withResolvers<boolean>();
  const sent = submitComposerDraft(draft, () => pending.promise);
  invalidateComposerSends();
  const replacement = composerDraft("other", "relay", "chat");
  editComposerDraft(replacement, { text: "replacement" });
  pending.resolve(true);
  await sent;
  expect(draft.text).toBe("");
  expect(draft.uncertain).toBe(false);
  expect(replacement.text).toBe("replacement");
  let retries = 0;
  await submitComposerDraft(draft, async () => { retries++; return true; });
  expect(retries).toBe(0);
});

test.each([true, false])("revoked completion %s preserves newer edits", async acknowledged => {
  const draft = composerDraft("account", "relay", "chat");
  editComposerDraft(draft, { text: "sent" });
  const pending = Promise.withResolvers<boolean>();
  const sent = submitComposerDraft(draft, () => pending.promise);
  invalidateComposerSends();
  editComposerDraft(draft, { text: "new text" });
  pending.resolve(acknowledged);
  await sent;
  expect(draft.text).toBe("new text");
});

test("revoked failure retains the uncertain original draft", async () => {
  const draft = composerDraft("account", "relay", "chat");
  editComposerDraft(draft, { text: "sent" });
  const pending = Promise.withResolvers<boolean>();
  const sent = submitComposerDraft(draft, () => pending.promise);
  invalidateComposerSends();
  pending.reject(new Error("transport lost"));
  await sent;
  expect(draft.text).toBe("sent");
  expect(draft.uncertain).toBe(true);
});
