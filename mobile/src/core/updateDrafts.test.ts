import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { Bot, Chat } from "./model";
import { composerDraftKey, hasUpdateDrafts, readComposerDraft, writeComposerDraft } from "./updateDrafts";
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

for (const removal of ["local/event", "roster", "snapshot"] as const) {
  test(`${removal} removal discards only deleted chat and rejects late callbacks`, () => {
    writeComposerDraft(key, { text: "deleted", attachments: [file], mentions: [bot], listening: true });
    writeComposerDraft(other, { text: "keep" });
    if (removal === "local/event") removeChat("chat");
    else if (removal === "roster") applyRoster({ devices: [], bots: [], chats: [chat("other")] });
    else replaceSnapshot(snapshot(["other"]));
    expect(useStore.getState().chats.map(c => c.id)).toEqual(["other"]);
    expect(readComposerDraft(key)).toEqual({ text: "", attachments: [], mentions: [], listening: false });
    writeComposerDraft(key, { text: "late transcript", attachments: [file] });
    expect(readComposerDraft(key).text).toBe("");
    expect(readComposerDraft(other).text).toBe("keep");
    expect(hasUpdateDrafts()).toBe(true);
    removeChat("other");
    expect(hasUpdateDrafts()).toBe(false);
  });
}

for (const transition of ["reset", "unpaired snapshot", "account", "relay snapshot", "relay status"] as const) {
  test(`${transition} discards unreachable ownership without carrying content into next source`, () => {
    writeComposerDraft(key, { text: "private", attachments: [file], mentions: [bot], listening: true });
    if (transition === "reset") resetStore();
    else if (transition === "unpaired snapshot") replaceSnapshot({ ...snapshot([]), has_identity: false, identity_id: null });
    else if (transition === "account") replaceSnapshot(snapshot(undefined, "next"));
    else if (transition === "relay snapshot") replaceSnapshot(snapshot(undefined, "account", "next-relay"));
    else setRelayStatus({ url: "next-relay", connected: false });
    writeComposerDraft(key, { text: "late" });
    expect(readComposerDraft(key)).toEqual({ text: "", attachments: [], mentions: [], listening: false });
    expect(hasUpdateDrafts()).toBe(false);
    replaceSnapshot(snapshot(undefined, "next"));
    const next = composerDraftKey("next", "relay", "chat");
    writeComposerDraft(next, { text: "new account" });
    writeComposerDraft(key, { text: "old callback" });
    expect(readComposerDraft(next).text).toBe("new account");
    expect(readComposerDraft(key).text).toBe("");
  });
}

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
