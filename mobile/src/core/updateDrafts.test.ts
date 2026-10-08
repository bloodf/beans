import { expect, test } from "bun:test";
import { composerDraftKey, hasUpdateDrafts, readComposerDraft, writeComposerDraft } from "./updateDrafts";
import { UpdateController } from "./updateController";

test("remounted composer recovers content under stable ownership and can release installation blocker", async () => {
  const key = composerDraftKey("account", "relay", "chat");
  const other = composerDraftKey("account", "relay", "other-chat");
  const file = { uri: "file:///draft.txt", name: "draft.txt", mime: "text/plain" };
  writeComposerDraft(key, { text: "unfinished", attachments: [file] });
  writeComposerDraft(other, { text: "another draft" });
  // Replacement mount uses the same route identity, not the old component's object.
  const recovered = readComposerDraft(composerDraftKey("account", "relay", "chat"));
  expect(recovered.text).toBe("unfinished");
  expect(recovered.attachments).toEqual([file]);
  expect(readComposerDraft(composerDraftKey("different-account", "relay", "chat")).text).toBe("");
  let installs = 0;
  const controller = new UpdateController({ check: async () => ({ id: "fresh", version: "1.2.3", notes: "" }), install: async () => { installs++; }, cancel: () => {} });
  await controller.check();
  await expect(controller.install("fresh", hasUpdateDrafts())).rejects.toThrow("Save or send drafts");
  writeComposerDraft(key, { text: "", attachments: [] });
  expect(hasUpdateDrafts()).toBe(true);
  writeComposerDraft(other, { text: "" });
  expect(hasUpdateDrafts()).toBe(false);
  await controller.install("fresh", hasUpdateDrafts());
  expect(installs).toBe(1);
});
