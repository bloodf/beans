import { afterAll, afterEach, beforeAll, beforeEach, expect, spyOn, test } from "bun:test";
import type { AppStore } from "./store";
import type { BotLook } from "./botLook";
import type { FileInfo, CLIState } from "../host";
import type { WireBot, WireMessage, WireSnapshot } from "./wire";

// Use the real host module with its browser defaults, not a global module mock.
// Only import-time location and per-test timer globals are supplied and restored.
let Store: typeof AppStore;
const locationDescriptor = Object.getOwnPropertyDescriptor(globalThis, "location");
const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
interface EventFixture { handle(name: string, data: unknown): void; cliStateChanged(state: CLIState): void }
const stores: AppStore[] = [];
const callbacks = new Map<number, { run: () => void; due: number }>();
let now = 10_000, nextTimer = 1, nextMessage = 1;
let timerSpy: { mockRestore(): void }, clearSpy: { mockRestore(): void }, dateSpy: { mockRestore(): void };

beforeAll(async () => {
  Object.defineProperty(globalThis, "location", { configurable: true, value: { search: "?platform=linux" } });
  // Static import cannot precede the browser-only host's location initialization.
  Store = (await import("./store")).AppStore;
  if (locationDescriptor) Object.defineProperty(globalThis, "location", locationDescriptor);
  else Reflect.deleteProperty(globalThis, "location");
});
beforeEach(() => {
  now = 10_000;
  dateSpy = spyOn(Date, "now").mockImplementation(() => now);
  timerSpy = spyOn(globalThis, "setTimeout").mockImplementation(((run: () => void, delay: number) => {
    const id = nextTimer++;
    callbacks.set(id, { run, due: now + delay });
    return id;
  }) as unknown as typeof setTimeout);
  clearSpy = spyOn(globalThis, "clearTimeout").mockImplementation(((id: number) => { callbacks.delete(id); }) as typeof clearTimeout);
  Object.defineProperty(globalThis, "window", { configurable: true, value: { setTimeout: globalThis.setTimeout } });
});
afterEach(() => {
  for (const store of stores.splice(0)) store.setConnected(false);
  callbacks.clear();
  timerSpy.mockRestore(); clearSpy.mockRestore(); dateSpy.mockRestore();
  if (windowDescriptor) Object.defineProperty(globalThis, "window", windowDescriptor);
  else Reflect.deleteProperty(globalThis, "window");
});
afterAll(() => {
  if (locationDescriptor) Object.defineProperty(globalThis, "location", locationDescriptor);
  else Reflect.deleteProperty(globalThis, "location");
});

const bot: WireBot = { id: "b", name: "Bot", description: "", symbol_name: "sparkles", accent: "blue", runner_id: "runner", provider: "deepseek", created_at: 1 };
const old: WireMessage = { id: "historical", chat_id: "c", author: { kind: "bot", bot_id: "b" }, body: { kind: "text", text: "old reply" }, state: { kind: "complete" }, created_at: 1 };
const job = { job_id: "j", chat_id: "c", bot_id: "b" };
function snapshot(running = false): WireSnapshot {
  return { version: "fixture", has_identity: true, is_identity_device: true, relay_connected: false, devices: [], bots: [bot], chats: [{ id: "c", kind: "dm", bot_ids: ["b"], is_pinned: false, created_at: 1, messages: [old] }], running_chat_ids: running ? ["c"] : [], running_turns: running ? [job] : [] };
}
function fixture(data = snapshot()) {
  const store = new Store();
  const events = store as unknown as EventFixture;
  const requests: { method: string; params: Record<string, unknown> }[] = [];
  Object.defineProperty(store, "transport", { writable: true, value: { request: async (method: string, params: Record<string, unknown>) => { requests.push({ method, params }); return {}; } } });
  events.handle("snapshot", data);
  store.setConnected(true);
  stores.push(store);
  return { store, events, requests };
}
function advance(ms: number) {
  now += ms;
  for (const [id, timer] of [...callbacks]) if (timer.due <= now) { callbacks.delete(id); timer.run(); }
}
function failCurrent(events: EventFixture) {
  events.handle("job.started", job);
  events.handle("message.added", { chat_id: "c", message: { ...old, id: `current-${nextMessage++}`, state: { kind: "failed", error: "failure" } } });
  events.handle("job.finished", job);
}

test("restored turns do not treat updates to loaded historical replies as current failures", () => {
  const restored = fixture(snapshot(true));
  restored.events.handle("message.updated", { chat_id: "c", message: { ...old, state: { kind: "failed", error: "historical failure" } } });
  expect(restored.store.botAvatarState("b", "c")).toBe("idle");
  expect(callbacks.size).toBe(0);
});

test("pending sends do not treat updates to loaded historical replies as current failures", () => {
  const pending = fixture();
  pending.store.send("next turn", [], [], "c");
  pending.events.handle("message.updated", { chat_id: "c", message: { ...old, state: { kind: "failed", error: "historical failure" } } });
  expect(pending.store.botAvatarState("b", "c")).toBe("idle");
  advance(4000);
  expect(pending.store.isWorking("b")).toBe(false);
});

test("terminal error expires once and a new turn clears it without a later stale timer event", () => {
  const { store, events } = fixture();
  failCurrent(events);
  expect(store.botAvatarState("b", "c")).toBe("error");
  expect(callbacks.size).toBe(1);
  advance(4999);
  expect(store.botAvatarState("b", "c")).toBe("error");
  advance(1);
  expect(store.botAvatarState("b", "c")).toBe("idle");
  expect(callbacks.size).toBe(0);
  failCurrent(events);
  events.handle("job.started", { ...job, job_id: "new" });
  expect(store.botAvatarState("b", "c")).toBe("idle");
  expect(callbacks.size).toBe(0);
});

for (const cleanup of ["disconnect", "snapshot", "chat-event", "chat-delete", "bot-delete"] as const) {
  test(`${cleanup} clears avatar errors, retries, thinking and error timers`, () => {
    const data = snapshot();
    if (cleanup === "chat-delete") data.chats[0]!.kind = "group";
    const { store, events } = fixture(data);
    failCurrent(events);
    events.handle("job.started", { ...job, bot_id: "other", job_id: "other" });
    events.handle("job.retry", { ...job, bot_id: "other", attempt: 1, max_attempts: 3, delay_ms: 1000 });
    events.handle("job.thinking", { chat_id: "c", bot_id: "other" });
    if (cleanup === "disconnect") events.cliStateChanged({ connection: "disconnected", launcher: { kind: "idle" }, starting: false });
    else if (cleanup === "snapshot") events.handle("snapshot", snapshot());
    else if (cleanup === "chat-event") events.handle("chat.removed", { chat_id: "c" });
    else store.deleteChat("c");
    expect(store.botAvatarState("b", "c")).toBe("idle");
    expect(store.retryNote("c")).toBeUndefined();
    expect(store.isThinking("other", "c")).toBe(false);
    expect(callbacks.size).toBe(0);
    advance(10_000);
    expect(store.botAvatarState("b", "c")).toBe("idle");
  });
}

const look: BotLook = { version: 1, base: { shape: "cloud", tone: "ink" }, states: { working: { expression: "happy" } } };
const photo: FileInfo = { path: "photo.png", name: "photo.png", mime: "image/png", size: 4, isFile: true, url: "file:///photo.png" };

test("omitted, null and complete appearance updates preserve the independently omitted photo/look", async () => {
  const data = snapshot();
  data.bots[0] = { ...bot, look, avatar: { id: "saved-photo", name: "saved.png", mime: "image/png", size: 4 } };
  const { store, requests } = fixture(data);
  await store.saveBotLook("b", undefined, null);
  expect(requests[0]?.params).toEqual({ id: "b", avatar: null });
  expect(store.bot("b")?.look).toEqual(look);
  expect(store.bot("b")?.avatar).toBeUndefined();
  await store.saveBotLook("b", undefined, photo);
  expect(Object.hasOwn(requests[1]!.params, "look")).toBe(false);
  expect(store.bot("b")?.look).toEqual(look);
  const savedPhoto = store.bot("b")?.avatar;
  await store.saveBotLook("b", null, undefined);
  expect(requests[2]?.params).toEqual({ id: "b", look: null });
  expect(store.bot("b")?.look).toBeUndefined();
  expect(store.bot("b")?.avatar).toEqual(savedPhoto);
  await store.saveBotLook("b", look, undefined);
  expect(requests[3]?.params).toEqual({ id: "b", look });
  expect(store.bot("b")?.look).toEqual(look);
  expect(store.bot("b")?.avatar).toEqual(savedPhoto);
  await store.saveBotLook("b", null, null);
  expect(requests[4]?.params).toEqual({ id: "b", look: null, avatar: null });
  expect(store.bot("b")?.look).toBeUndefined();
  expect(store.bot("b")?.avatar).toBeUndefined();
});

test("a rejected combined appearance/photo update does not mutate the account", async () => {
  const data = snapshot();
  data.bots[0] = { ...bot, look, avatar: { id: "saved-photo", name: "saved.png", mime: "image/png", size: 4 } };
  const { store } = fixture(data);
  const before = structuredClone(store.bots);
  let calls = 0;
  Object.defineProperty(store, "transport", { value: { request: async () => { calls++; throw new Error("request rejected"); } } });
  await expect(store.saveBotLook("b", { version: 1, base: { shape: "triangle" } }, photo)).rejects.toThrow();
  expect(store.bots).toEqual(before);
  expect((Object.getOwnPropertyDescriptor(store, "attachmentFiles")!.value as Map<string, unknown>).size).toBe(0);
  expect(calls).toBe(1);
});

test("future appearance fields are preserved on read and rejected before any update request", async () => {
  const future = { version: 1, base: { shape: "cloud", futureTrait: true } } as unknown as BotLook;
  const data = snapshot(); data.bots[0] = { ...bot, look: future };
  const { store, requests } = fixture(data);
  const before = structuredClone(store.bots);
  await expect(store.saveBotLook("b", future, null)).rejects.toThrow();
  expect(requests).toEqual([]);
  expect(store.bots).toEqual(before);
});

test("removing a group member clears only that bot's chat activity and terminal timer", () => {
  const data = snapshot();
  data.bots.push({ ...bot, id: "other" });
  data.chats[0] = { ...data.chats[0]!, kind: "group", bot_ids: ["b", "other"] };
  const { store, events } = fixture(data);
  events.handle("job.started", job);
  events.handle("message.added", { chat_id: "c", message: { ...old, id: "new-failure", state: { kind: "failed", error: "failure" } } });
  events.handle("job.retry", { ...job, attempt: 1, max_attempts: 3, delay_ms: 1000 });
  events.handle("job.thinking", { chat_id: "c", bot_id: "b" });
  expect(store.botAvatarState("b", "c")).toBe("error");
  store.removeBot("b", "c");
  expect(store.chat("c")?.botIDs).toEqual(["other"]);
  expect(store.botAvatarState("b", "c")).toBe("idle");
  expect(store.retryNote("c")).toBeUndefined();
  expect(store.isThinking("b", "c")).toBe(false);
  expect(callbacks.size).toBe(0);
});

test("another bot's message or finish does not clear the active bot's retry state", () => {
  const data = snapshot();
  data.bots.push({ ...bot, id: "other" });
  data.chats[0] = { ...data.chats[0]!, kind: "group", bot_ids: ["b", "other"] };
  const { store, events } = fixture(data);
  events.handle("job.started", job);
  const otherJob = { ...job, job_id: "other-job", bot_id: "other" };
  events.handle("job.started", otherJob);
  events.handle("job.retry", { ...job, attempt: 1, max_attempts: 3, delay_ms: 1000 });
  events.handle("message.added", { chat_id: "c", message: { ...old, id: "other-reply", author: { kind: "bot", bot_id: "other" } } });
  expect(store.botAvatarState("b", "c")).toBe("retry");
  expect(store.botAvatarState("other", "c")).toBe("idle");
  events.handle("job.finished", otherJob);
  expect(store.botAvatarState("b", "c")).toBe("retry");
  events.handle("job.finished", job);
  expect(store.botAvatarState("b", "c")).toBe("idle");
});

test("deleting a DM bot clears its retry state in surviving groups", () => {
  const data = snapshot();
  data.bots.push({ ...bot, id: "other" });
  data.chats.push({ ...data.chats[0]!, id: "g", kind: "group", bot_ids: ["b", "other"], messages: [] });
  const { store, events } = fixture(data);
  const groupJob = { ...job, chat_id: "g" };
  events.handle("job.started", groupJob);
  events.handle("job.retry", { ...groupJob, attempt: 1, max_attempts: 3, delay_ms: 1000 });
  store.deleteChat("c");
  expect(store.chat("g")?.botIDs).toEqual(["other"]);
  expect(store.retryNote("g")).toBeUndefined();
  expect(store.botAvatarState("b", "g")).toBe("idle");
});

test("an appearance object replaces the complete generated draft without merging obsolete state overrides", async () => {
  const data = snapshot();
  data.bots[0] = { ...bot, look, avatar: { id: "saved-photo", name: "saved.png", mime: "image/png", size: 4 } };
  const { store } = fixture(data);
  const replacement: BotLook = { version: 1, base: { expression: "happy" } };
  await store.saveBotLook("b", replacement, undefined);
  expect(store.bot("b")?.look).toEqual(replacement);
  expect(store.bot("b")?.look?.states).toBeUndefined();
  expect(store.bot("b")?.avatar?.id).toBe("saved-photo");
});
