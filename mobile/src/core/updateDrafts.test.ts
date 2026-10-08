import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { Bot, Chat } from "./model";
import { composerDraftGeneration, composerDraftKey, discardComposerDraft, hasUpdateDrafts, readComposerDraft, retainedComposerDrafts, writeComposerDraft } from "./updateDrafts";
import { UpdateController } from "./updateController";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
const { applyRoster, removeChat, replaceSnapshot, resetStore, setRelayStatus, useStore } = await import("./store");
const chat = (id: string): Chat => ({ id, kind: "dm", bot_ids: [], is_pinned: false, created_at: 0, messages: [], unread_count: 0 });
const snapshot = (ids = ["chat", "other"], identity = "account", relay = "relay") => ({ has_identity: true, identity_id: identity, relay_url: relay, this_device_id: "phone", relay_connected: true, devices: [], bots: [], chats: ids.map(chat), running_turns: [] });
const key = composerDraftKey("account", "relay", "chat");
const other = composerDraftKey("account", "relay", "other");
const file = { uri: "file:///draft.txt", name: "draft.txt", mime: "text/plain" };
const bot = { id: "bot", name: "Bot" } as Bot;
beforeEach(() => { resetStore(); replaceSnapshot(snapshot()); });
afterEach(() => resetStore());

test("navigation and ordinary reconnect preserve content and installation veto", async () => {
  writeComposerDraft(key, { text: "@Bot unfinished", attachments: [file], mentions: [bot] });
  useStore.setState({ openChatId: "other" });
  setRelayStatus({ connected: false });
  replaceSnapshot({ ...snapshot(), relay_connected: false });
  applyRoster({ devices: [], bots: [], chats: snapshot().chats });
  setRelayStatus({ url: "relay", connected: true });
  useStore.setState({ openChatId: "chat" });
  expect(readComposerDraft(key)).toEqual({ text: "@Bot unfinished", attachments: [file], mentions: [bot], listening: false });
  let installs = 0;
  const controller = new UpdateController({ check: async () => ({ id: "fresh", version: "1.2.3", notes: "" }), install: async () => { installs++; }, cancel: () => {} });
  await controller.check();
  await expect(controller.install("fresh", hasUpdateDrafts())).rejects.toThrow("Save or send drafts");
  removeChat("chat");
  await controller.install("fresh", hasUpdateDrafts());
  expect(installs).toBe(1);
});

test("explicit removal fences old callbacks after same-key reappearance", () => {
  const generation = composerDraftGeneration(key);
  writeComposerDraft(key, { text: "deleted", attachments: [file] });
  writeComposerDraft(other, { text: "keep" });
  removeChat("chat");
  replaceSnapshot(snapshot());
  writeComposerDraft(key, { text: "new" });
  writeComposerDraft(key, { text: "old callback", attachments: [file] }, generation);
  expect(readComposerDraft(key).text).toBe("new");
  expect(readComposerDraft(key).attachments).toEqual([]);
  expect(readComposerDraft(other).text).toBe("keep");
});

for (const observation of ["roster", "snapshot", "unpaired snapshot", "account", "null relay"] as const) {
  test(`${observation} retains source-keyed recovery and blocker until explicit discard`, () => {
    writeComposerDraft(key, { text: "private", attachments: [file] });
    if (observation === "roster") applyRoster({ devices: [], bots: [], chats: [] });
    else if (observation === "snapshot") replaceSnapshot(snapshot([]));
    else if (observation === "unpaired snapshot") replaceSnapshot({ ...snapshot([]), has_identity: false, identity_id: null });
    else if (observation === "account") replaceSnapshot(snapshot(undefined, "next"));
    else setRelayStatus({ url: null, connected: false });
    expect(readComposerDraft(key).text).toBe("private");
    expect(readComposerDraft(key).attachments).toEqual([file]);
    expect(retainedComposerDrafts().map(([key]) => key)).toContain(key);
    expect(hasUpdateDrafts()).toBe(true);
    replaceSnapshot(snapshot());
    expect(readComposerDraft(key).text).toBe("private");
    discardComposerDraft(key);
    expect(hasUpdateDrafts()).toBe(false);
  });
}

test("authoritative forget fences callbacks across account restoration", () => {
  const generation = composerDraftGeneration(key);
  writeComposerDraft(key, { text: "private", attachments: [file] });
  resetStore();
  replaceSnapshot(snapshot());
  writeComposerDraft(key, { text: "old callback" }, generation);
  expect(readComposerDraft(key).text).toBe("");
  expect(hasUpdateDrafts()).toBe(false);
});

test("manual text clear releases stale mentions but preserves files and other drafts", () => {
  writeComposerDraft(key, { text: "@Bot ", mentions: [bot], attachments: [file] });
  writeComposerDraft(other, { text: "@Bot keep", mentions: [bot] });
  writeComposerDraft(key, { text: "" });
  expect(readComposerDraft(key)).toEqual({ text: "", mentions: [], attachments: [file], listening: false });
  expect(readComposerDraft(other).mentions).toEqual([bot]);
  writeComposerDraft(key, { attachments: [] });
  expect(hasUpdateDrafts()).toBe(true);
  writeComposerDraft(other, { text: "" });
  expect(readComposerDraft(other).mentions).toEqual([]);
  expect(hasUpdateDrafts()).toBe(false);
});
