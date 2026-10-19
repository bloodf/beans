import { afterEach, beforeEach, describe, expect, jest, mock, test } from "bun:test";
import type { Notification, NotificationBehavior, NotificationResponse } from "expo-notifications";
import { resolveChatBotState } from "./activity";
import type { Bot, Chat, CommandRun, Message, ProviderModel, ProviderStatus } from "./model";

// Exercise the real engine and store, including message/roster ordering, foreground
// transitions, and the working row. Only native bridges are replaced; no account or provider is
// contacted.
type Listener = (frame: { event: string; data: unknown }) => void;
const listeners = new Set<Listener>();
const event: Listener = (frame) => listeners.forEach((listener) => listener(frame));
/// Set to keep the next `bootstrap` answer until the test gives it.
let heldSnapshot: Promise<unknown> | null = null;
let deletion: Promise<unknown> | null = null;
let appState: (status: string) => void;
const reads: string[] = [];
const cleared: string[] = [];
const opened: string[] = [];
let handleNotification: (notification: Notification) => Promise<NotificationBehavior>;
let openNotification: (response: NotificationResponse) => void;
mock.module("react-native", () => ({
  Platform: { OS: "ios" },
  AppState: { currentState: "active", addEventListener: (_: string, listener: typeof appState) => { appState = listener; } },
}));
mock.module("expo-web-browser", () => ({}));
mock.module("./host", () => ({ hostFacts: () => ({ name: "Phone", os: "ios", os_version: "", model: "" }) }));
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-application", () => ({}));
mock.module("expo-device", () => ({ isDevice: true }));
mock.module("expo-router", () => ({ router: { navigate: ({ params }: { params: { id: string } }) => { opened.push(params.id); } } }));
mock.module("expo-notifications", () => ({
  setNotificationHandler: (handler: { handleNotification: typeof handleNotification }) => { handleNotification = handler.handleNotification; },
  addNotificationResponseReceivedListener: (listener: typeof openNotification) => { openNotification = listener; },
  getLastNotificationResponse: () => null,
  getPermissionsAsync: async () => ({ status: "denied" }),
  getPresentedNotificationsAsync: async () => [notification("open", "Confirmation needed: Deploy the app"), notification("other", "Reply failed: Provider unavailable")],
  dismissNotificationAsync: async (id: string) => { cleared.push(id); },
}));
mock.module("../../modules/beans-core", () => ({
  start: () => {}, wake: () => {},
  onEvent: (listener: Listener) => { listeners.add(listener); return () => listeners.delete(listener); },
  request: async (method: string, params: Record<string, any> = {}) => {
    if (method === "chats.delete") return deletion;
    if (method === "chats.mark_read") reads.push(params.chat_id!);
    if (method === "providers.connect_custom") {
      // The core answers with the kind it gave or kept, and every status.
      const kind = params.kind ?? "custom:lab";
      const models = (params.models as string[]).map((id) => ({ id }));
      return { kind, providers: [{ kind, is_connected: true, detail: params.base_url, base_url: params.base_url, name: params.name, api: params.api, models }] };
    }
    if (method === "providers.disconnect") return { providers: [] };
    return method === "bootstrap" ? (heldSnapshot ?? snapshot([])) : null;
  },
}));

function chat(id: string, unread_count = 0): Chat {
  return { id, kind: "dm", bot_ids: ["bot"], is_pinned: false, created_at: 0, messages: [], unread_count };
}

function snapshot(chats: Chat[]) {
  return { has_identity: true, identity_id: "account", this_device_id: "phone", relay_url: null,
    relay_connected: true, devices: [], bots: [], chats, running_turns: [] };
}

const { engine } = await import("./engine");
const { ERROR_DISPLAY_MS, resetStore, runningTasks, TASK_DELAY_MS, useStore } = await import("./store");
const { workingActivity } = await import("../ui/format");
const { composerDraftKey, hasUpdateDrafts, readComposerDraft, writeComposerDraft } = await import("./updateDrafts");
await engine.start();

beforeEach(async () => {
  resetStore();
  appState("active");
  event({ event: "snapshot", data: snapshot([chat("open"), chat("other")]) });
  useStore.setState({ openChatId: "open" });
  await flush();
  reads.length = 0;
  cleared.length = 0;
  opened.length = 0;
});

async function flush() {
  // markRead imports the native bridge asynchronously.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

test("engine deletion and forgotten identity reconcile drafts through store callbacks", async () => {
  const open = composerDraftKey("account", null, "open");
  const other = composerDraftKey("account", null, "other");
  writeComposerDraft(open, { text: "deleted" });
  writeComposerDraft(other, { text: "private" });
  await engine.deleteChat("open");
  expect(readComposerDraft(open).text).toBe("");
  expect(readComposerDraft(other).text).toBe("private");
  await engine.unpair();
  expect(useStore.getState().identityId).toBeNull();
  expect(readComposerDraft(other).text).toBe("");
  expect(hasUpdateDrafts()).toBe(false);
});

test("native removal and identity events discard unreachable drafts", () => {
  const open = composerDraftKey("account", null, "open");
  const other = composerDraftKey("account", null, "other");
  writeComposerDraft(open, { text: "deleted" });
  writeComposerDraft(other, { text: "private" });
  event({ event: "chat.removed", data: { chat_id: "open" } });
  expect(readComposerDraft(open).text).toBe("");
  expect(readComposerDraft(other).text).toBe("private");
  event({ event: "identity.changed", data: { has_identity: false } });
  expect(readComposerDraft(other).text).toBe("");
  expect(hasUpdateDrafts()).toBe(false);
});

test("relay round-trip retains source-keyed drafts and blocker", () => {
  const open = composerDraftKey("account", null, "open");
  writeComposerDraft(open, { text: "unfinished" });
  event({ event: "relay.status", data: { connected: false, url: "next-relay" } });
  event({ event: "relay.status", data: { connected: true, url: null } });
  expect(readComposerDraft(open).text).toBe("unfinished");
  expect(hasUpdateDrafts()).toBe(true);
});


test("rejected deferred deletion retains files and newer edits after snapshot restoration", async () => {
  const key = composerDraftKey("account", null, "open");
  const files = [{ uri: "file:///picked", name: "picked", mime: "text/plain" }];
  writeComposerDraft(key, { text: "before", attachments: files });
  let reject!: (error: Error) => void;
  deletion = new Promise((_, fail) => { reject = fail; });
  const pending = engine.deleteChat("open");
  const rejected = pending.catch(error => error);
  expect(useStore.getState().chats.some(c => c.id === "open")).toBe(true);
  writeComposerDraft(key, { text: "newer" });
  reject(new Error("delete refused"));
  expect((await rejected).message).toBe("delete refused");
  deletion = null;
  event({ event: "snapshot", data: snapshot([chat("open"), chat("other")]) });
  expect(readComposerDraft(key).text).toBe("newer");
  expect(readComposerDraft(key).attachments).toEqual(files);
  expect(hasUpdateDrafts()).toBe(true);
});

test("successful deferred deletion discards only after acknowledgement", async () => {
  const key = composerDraftKey("account", null, "open");
  writeComposerDraft(key, { text: "before" });
  let resolve!: () => void;
  deletion = new Promise<void>(done => { resolve = done; });
  const pending = engine.deleteChat("open");
  expect(readComposerDraft(key).text).toBe("before");
  writeComposerDraft(key, { text: "newer" });
  resolve();
  await pending;
  deletion = null;
  expect(readComposerDraft(key).text).toBe("");
});

test("stale bootstrap null relay followed by held source event retains text and files", async () => {
  event({ event: "relay.status", data: { connected: true, url: "relay" } });
  const key = composerDraftKey("account", "relay", "open");
  const files = [{ uri: "file:///picked", name: "picked", mime: "text/plain" }];
  writeComposerDraft(key, { text: "unfinished", attachments: files });
  let answer!: (value: unknown) => void;
  heldSnapshot = new Promise(resolve => { answer = resolve; });
  const refresh = engine.refreshCustomModels();
  await flush();
  event({ event: "relay.status", data: { connected: true, url: "relay" } });
  answer(snapshot([chat("open"), chat("other")]));
  await refresh;
  heldSnapshot = null;
  expect(useStore.getState().relayUrl).toBe("relay");
  expect(readComposerDraft(key).text).toBe("unfinished");
  expect(readComposerDraft(key).attachments).toEqual(files);
  expect(hasUpdateDrafts()).toBe(true);
});

test("old deletion acknowledgement cannot discard same-key reappearance", async () => {
  const key = composerDraftKey("account", null, "open");
  writeComposerDraft(key, { text: "old" });
  let resolve!: () => void;
  deletion = new Promise<void>(done => { resolve = done; });
  const pending = engine.deleteChat("open");
  event({ event: "chat.removed", data: { chat_id: "open" } });
  event({ event: "snapshot", data: snapshot([chat("open"), chat("other")]) });
  writeComposerDraft(key, { text: "new ownership" });
  resolve();
  await pending;
  deletion = null;
  expect(readComposerDraft(key).text).toBe("new ownership");
  expect(useStore.getState().chats.some(c => c.id === "open")).toBe(true);
});
function reply(id: string, kind: "reply" | "permission" | "failure" = "reply") {
  const message: Message = { id: "reply", chat_id: id, author: { kind: "bot", bot_id: "bot" },
    body: { kind: "text", text: "Done" }, state: { kind: "complete" }, created_at: 1 };
  if (kind === "permission") {
    message.body = { kind: "permission", plugin_id: "computer", plugin_name: "Mac", tool: "bash", summary: "Deploy the app", decision: "pending" };
  } else if (kind === "failure") {
    message.body = { kind: "text", text: "Partial response" };
    message.state = { kind: "failed", error: "Provider unavailable" };
  }
  event({ event: "message.updated", data: { chat_id: id, message } });
  event({ event: "roster.changed", data: { devices: [], bots: [], chats: [chat("open", id === "open" ? 1 : 0), chat("other", id === "other" ? 1 : 0)] } });
}

function notification(chatId: string, body: string): Notification {
  return { date: 1, request: { identifier: chatId, trigger: null, content: {
    title: "Chef", subtitle: null, body, data: { chat_id: chatId }, sound: "default",
  } } };
}

for (const kind of ["permission", "failure"] as const) {
  test(`a background ${kind} stays unread until the user returns`, async () => {
    appState("background");
    reply("open", kind);
    await flush();
    expect(reads).toEqual([]);
    expect(useStore.getState().chats[0].unread_count).toBe(1);
    appState("active");
    await flush();
    expect(reads).toEqual(["open"]);
    expect(cleared).toEqual(["open"]);
  });

  test(`a visible ${kind} is read after its unread count arrives`, async () => {
    reply("open", kind);
    await flush();
    expect(reads).toEqual(["open"]);
    expect(useStore.getState().chats[0].unread_count).toBe(0);
  });

  test(`${kind} pushes show away from the chat and open it when tapped`, async () => {
    const body = kind === "permission" ? "Confirmation needed: Deploy the app" : "Reply failed: Provider unavailable";
    const alert = notification("open", body);
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: false, shouldShowList: false, shouldPlaySound: false });
    appState("background");
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true });
    appState("active");
    useStore.setState({ openChatId: "other" });
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true });
    openNotification({ actionIdentifier: "expo.modules.notifications.actions.DEFAULT", notification: alert });
    expect(opened).toEqual(["open"]);
    expect(alert.request.content.body).toBe(body);
  });
}

test("a visible reply is acknowledged after its unread count arrives", async () => {
  reply("open");
  await flush();
  expect(reads).toEqual(["open"]);
  expect(useStore.getState().chats[0].unread_count).toBe(0);
});

test("a mounted chat cannot mark a background reply read; returning reads it", async () => {
  appState("background");
  reply("open");
  await flush();
  expect(reads).toEqual([]);
  expect(useStore.getState().chats[0].unread_count).toBe(1);
  appState("active");
  await flush();
  expect(reads).toEqual(["open"]);
  expect(cleared).toEqual(["open"]);
});

test("inactive or unfocused UI leaves replies unread", async () => {
  appState("inactive");
  reply("open");
  await flush();
  expect(reads).toEqual([]);
  useStore.setState({ openChatId: null });
  appState("active");
  await flush();
  expect(reads).toEqual([]);
});

test("another chat still notifies while this chat is visible", async () => {
  reply("other");
  await flush();
  expect(reads).toEqual([]);
  expect(useStore.getState().chats[1].unread_count).toBe(1);
});

test("a backlog snapshot is read only while the chat is foregrounded", async () => {
  appState("background");
  event({ event: "snapshot", data: snapshot([chat("open", 2)]) });
  await flush();
  expect(reads).toEqual([]);
  appState("active");
  await flush();
  expect(reads).toEqual(["open"]);
});

test("a turn on the Runner reads as thinking, its command, and its retry", () => {
  const row = () => workingActivity(useStore.getState(), "open");
  const command = (is_running: boolean): Message => ({ id: "call", chat_id: "open", author: { kind: "bot", bot_id: "bot" },
    body: { kind: "tool", name: "bash", summary: is_running ? "Running bash…" : "installed", detail: "", is_running, description: "Install dependencies" },
    state: { kind: is_running ? "streaming" : "complete" }, created_at: 1 });
  event({ event: "job.started", data: { job_id: "job", chat_id: "open", bot_id: "bot" } });
  expect(row()).toBeNull();
  event({ event: "job.thinking", data: { chat_id: "open", bot_id: "bot" } });
  expect(row()).toBe("Thinking…");
  // The bot's next message is what its thinking came to.
  event({ event: "message.added", data: { chat_id: "open", message: command(true) } });
  expect(row()).toBe("Running command: Install dependencies…");
  event({ event: "job.thinking", data: { chat_id: "open", bot_id: "bot" } });
  event({ event: "message.updated", data: { chat_id: "open", message: command(false) } });
  expect(row()).toBe("Thinking…");
  event({ event: "job.retry", data: { chat_id: "open", bot_id: "bot", attempt: 1, max_attempts: 3, delay_ms: 2000, error: "overloaded" } });
  expect(row()).toBe("Retrying (1 of 3) in 2 s…");
  event({ event: "job.finished", data: { job_id: "job", chat_id: "open", bot_id: "bot" } });
  expect(useStore.getState()).toMatchObject({ running: {}, thinking: {}, retries: {} });
});

test("events that arrive while a snapshot is on its way are applied after it", async () => {
  // The relay connects and a reply lands while the core is still building the snapshot the app
  // asked for, so the snapshot is older than both.
  let answer!: (snapshot: unknown) => void;
  heldSnapshot = new Promise((resolve) => { answer = resolve; });
  const paired = engine.pair("beans://pair?v=2&relay=https%3A%2F%2Frelay.example&id=Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs&ek=nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw&n=1qUeEODuTz_A2y5jSZdBqQ", "Phone");
  await flush();
  event({ event: "relay.status", data: { connected: true, update_required: false, url: "https://relay.example" } });
  reply("open");
  answer({ ...snapshot([chat("open"), chat("other")]), relay_connected: false });
  heldSnapshot = null;
  await paired;
  expect(useStore.getState().relayConnected).toBe(true);
  expect(useStore.getState().chats[0].messages.map((m) => m.id)).toEqual(["reply"]);
});

function command(state: CommandRun["state"], session_id?: string, id = "call"): Message {
  return { id, chat_id: "open", author: { kind: "bot", bot_id: "bot" }, state: { kind: "streaming" }, created_at: 1,
    body: { kind: "tool", name: "bash", summary: "Running", detail: "", is_running: true, description: "Install dependencies", run: { command: "bun install", state, session_id } } };
}

const tasks = () => runningTasks(useStore.getState(), "open").map((m) => m.id);

test("a command becomes a running task once it has run in its terminal for a moment", async () => {
  // The call starts, and Auto-review judges the command before a terminal runs it.
  event({ event: "message.added", data: { chat_id: "open", message: command("running") } });
  event({ event: "message.updated", data: { chat_id: "open", message: command("checking") } });
  event({ event: "message.updated", data: { chat_id: "open", message: command("running", "bash-1") } });
  expect(tasks()).toEqual([]);
  await new Promise((resolve) => setTimeout(resolve, TASK_DELAY_MS + 50));
  expect(tasks()).toEqual(["call"]);
  // Waiting at a question is still running.
  event({ event: "message.updated", data: { chat_id: "open", message: command("waiting", "bash-1") } });
  expect(tasks()).toEqual(["call"]);
  event({ event: "message.updated", data: { chat_id: "open", message: command("exited", "bash-1") } });
  expect(tasks()).toEqual([]);
});

test("a quick command never counts, and one running before the phone heard of it counts at once", () => {
  event({ event: "message.added", data: { chat_id: "open", message: command("running") } });
  event({ event: "message.updated", data: { chat_id: "open", message: command("running", "bash-1") } });
  event({ event: "message.updated", data: { chat_id: "open", message: command("exited", "bash-1") } });
  expect(tasks()).toEqual([]);
  expect(useStore.getState().pendingTasks).toEqual({});
  event({ event: "snapshot", data: snapshot([{ ...chat("open"), messages: [command("running", "bash-2", "server")] }, chat("other")]) });
  expect(tasks()).toEqual(["server"]);
});

test("a custom provider is added without a kind and saved with one; the store takes the core's statuses", async () => {
  const kind = await engine.saveCustomProvider({ name: "Lab", api: "responses", baseURL: "http://lab.local:8080/v1", apiKey: "", models: ["llama4", "qwen3:8b"] });
  expect(kind).toBe("custom:lab");
  expect(useStore.getState().providers).toMatchObject([{ kind: "custom:lab", name: "Lab", api: "responses", models: [{ id: "llama4" }, { id: "qwen3:8b" }] }]);
  await engine.saveCustomProvider({ kind: "custom:lab", name: "Lab", api: "messages", baseURL: "http://lab.local:8080", apiKey: "sk-lab", models: [] });
  // Deleting one is a disconnect of its kind, for the whole account.
  await engine.disconnectProvider("custom:lab");
  expect(useStore.getState().providers).toEqual([]);
});

test("custom provider statuses arrive with the roster and stay through one that carries none", () => {
  const lab = { kind: "custom:lab", is_connected: true, detail: "http://lab.local:8080/v1", base_url: "http://lab.local:8080/v1", name: "Lab", api: "chat-completions" as const, models: [{ id: "llama4" }] };
  event({ event: "roster.changed", data: { devices: [], bots: [], chats: [chat("open"), chat("other")], providers: [lab] } });
  expect(useStore.getState().providers).toEqual([lab]);
  event({ event: "roster.changed", data: { devices: [], bots: [], chats: [chat("open"), chat("other")] } });
  expect(useStore.getState().providers).toEqual([lab]);
});

function catalogSnapshot() {
  const bot: Bot = { id: "bot", name: "Catalog bot", description: "", symbol_name: "person", accent: "blue",
    runner_id: "runner", provider: "custom:catalog", model: "chosen-model", thinking: "high", created_at: 0 };
  const providers: ProviderStatus[] = [{ kind: "custom:catalog", is_connected: true, detail: "Connected",
    name: "Catalog", models: [{ id: "chosen-model" }] }];
  const models: ProviderModel[] = [{ provider: "custom:catalog", id: "chosen-model", name: "Chosen", levels: ["high"] }];
  return { ...snapshot([chat("open")]), relay_url: "https://relay.example", bots: [bot], providers, models };
}

test("provider refresh keeps omitted catalogs for the same account and relay but accepts empty lists", async () => {
  const loaded = catalogSnapshot();
  event({ event: "snapshot", data: loaded });
  const { providers, models, ...omitted } = loaded;
  heldSnapshot = Promise.resolve(omitted);
  try {
    await engine.refreshCustomModels();
    expect(useStore.getState().providers).toEqual(providers);
    expect(useStore.getState().models).toEqual(models);
    expect(useStore.getState().bots[0].model).toBe("chosen-model");
    expect(useStore.getState().bots[0].thinking).toBe("high");
    event({ event: "snapshot", data: { ...omitted, providers: [], models: [] } });
    expect(useStore.getState().providers).toEqual([]);
    expect(useStore.getState().models).toEqual([]);
  } finally {
    heldSnapshot = null;
  }
});

test.each([
  ["another account", { identity_id: "another-account" }],
  ["forgotten identity", { has_identity: false, identity_id: null }],
  ["another relay", { relay_url: "https://other-relay.example" }],
])("omitted catalogs do not survive %s", (_name, changed) => {
  const loaded = catalogSnapshot();
  event({ event: "snapshot", data: loaded });
  const { providers: _providers, models: _models, ...omitted } = loaded;
  event({ event: "snapshot", data: { ...omitted, ...changed } });
  expect(useStore.getState().providers).toEqual([]);
  expect(useStore.getState().models).toEqual([]);
});

test("a relay status change cannot make old catalogs belong to the new source", () => {
  const loaded = catalogSnapshot();
  event({ event: "snapshot", data: loaded });
  event({ event: "relay.status", data: { connected: true, url: "https://other-relay.example" } });
  const { providers: _providers, models: _models, ...omitted } = loaded;
  event({ event: "snapshot", data: { ...omitted, relay_url: "https://other-relay.example" } });
  expect(useStore.getState().providers).toEqual([]);
  expect(useStore.getState().models).toEqual([]);
  expect(useStore.getState().bots[0].model).toBe("chosen-model");
});

// Avatar activity: events in, resolver out. The store stamps a failure when it watches the
// transition, keeps a group's retry with the bot it belongs to, and drops state a turn ended.
describe("avatar activity from events", () => {
  const group = (): Chat => ({ ...chat("open"), kind: "group", bot_ids: ["a", "b"] });
  const text = (id: string, author: Message["author"], state: Message["state"] = { kind: "complete" }): Message =>
    ({ id, chat_id: "open", author, body: { kind: "text", text: "x" }, state, created_at: 1 });
  const bot = (id: string): Message["author"] => ({ kind: "bot", bot_id: id });
  const state = (id: string) => resolveChatBotState(useStore.getState(), "open", id);

  test("a retry belongs to its bot: another member's message leaves it, its own message or turn end clears it", () => {
    event({ event: "snapshot", data: snapshot([group()]) });
    event({ event: "job.started", data: { job_id: "ja", chat_id: "open", bot_id: "a" } });
    event({ event: "job.retry", data: { chat_id: "open", bot_id: "a", attempt: 1, max_attempts: 3, delay_ms: 1000, error: "x" } });
    expect(state("a")).toBe("retry");
    event({ event: "message.added", data: { chat_id: "open", message: text("m1", bot("b")) } });
    expect(state("a")).toBe("retry");
    event({ event: "message.added", data: { chat_id: "open", message: text("m2", bot("a")) } });
    expect(useStore.getState().retries).toEqual({});
    event({ event: "job.retry", data: { chat_id: "open", bot_id: "a", attempt: 2, max_attempts: 3, delay_ms: 1000, error: "x" } });
    event({ event: "job.finished", data: { job_id: "ja", chat_id: "open", bot_id: "b" } });
    expect(useStore.getState().retries.open).toBeDefined();
    event({ event: "job.finished", data: { job_id: "ja", chat_id: "open", bot_id: "a" } });
    expect(useStore.getState().retries).toEqual({});
  });

  afterEach(() => jest.useRealTimers());

  test("a failure the phone watches is an error until its entry expires, and the map is left empty", () => {
    jest.useFakeTimers();
    event({ event: "snapshot", data: snapshot([group()]) });
    event({ event: "message.added", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "streaming" }) } });
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "failed", error: "down" }) } });
    expect(useStore.getState().failures.f1).toMatchObject({ chatId: "open", botId: "a" });
    expect(state("a")).toBe("error");
    expect(state("b")).toBe("idle");
    jest.advanceTimersByTime(ERROR_DISPLAY_MS - 1);
    expect(state("a")).toBe("error");
    jest.advanceTimersByTime(1);
    expect(useStore.getState().failures).toEqual({});
    expect(state("a")).toBe("idle");
  });

  test("the same failure arriving again does not restart the clock, and an old timer cannot end a newer failure", () => {
    jest.useFakeTimers();
    event({ event: "snapshot", data: snapshot([group()]) });
    event({ event: "message.added", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "streaming" }) } });
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "failed", error: "down" }) } });
    jest.advanceTimersByTime(3000);
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "failed", error: "down" }) } });
    jest.advanceTimersByTime(ERROR_DISPLAY_MS - 3000);
    expect(useStore.getState().failures).toEqual({});
    // Retried and failed again: a new entry that the first timer is long done with.
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "streaming" }) } });
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "failed", error: "down" }) } });
    jest.advanceTimersByTime(ERROR_DISPLAY_MS - 1);
    expect(state("a")).toBe("error");
    jest.advanceTimersByTime(1);
    expect(state("a")).toBe("idle");
  });

  test("a new user message or the bot's next turn ends the error early, another bot's turn does not", () => {
    jest.useFakeTimers();
    event({ event: "snapshot", data: snapshot([group()]) });
    const fail = () => {
      event({ event: "message.added", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "streaming" }) } });
      event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "failed", error: "down" }) } });
    };
    fail();
    event({ event: "message.added", data: { chat_id: "open", message: text("u1", { kind: "you" }) } });
    expect(state("a")).toBe("idle");
    event({ event: "message.updated", data: { chat_id: "open", message: text("f1", bot("a"), { kind: "streaming" }) } });
    fail();
    event({ event: "job.started", data: { job_id: "jb", chat_id: "open", bot_id: "b" } });
    expect(useStore.getState().failures.f1).toBeDefined();
    event({ event: "job.started", data: { job_id: "ja", chat_id: "open", bot_id: "a" } });
    expect(useStore.getState().failures).toEqual({});
  });

  test("a failure already in a snapshot never reads as an error", () => {
    event({ event: "snapshot", data: snapshot([{ ...group(), messages: [text("old", bot("a"), { kind: "failed", error: "down" })] }]) });
    expect(useStore.getState().failures).toEqual({});
    expect(state("a")).toBe("idle");
  });

  test("an open permission waits until the user writes again, even across a group's interleaved replies", () => {
    event({ event: "snapshot", data: snapshot([group()]) });
    const card: Message = { ...text("p1", bot("a")), body: { kind: "permission", plugin_id: "p", plugin_name: "P", tool: "t", summary: "", decision: "pending" } };
    event({ event: "message.added", data: { chat_id: "open", message: card } });
    event({ event: "message.added", data: { chat_id: "open", message: text("m3", bot("b")) } });
    expect(state("a")).toBe("waiting");
    event({ event: "message.added", data: { chat_id: "open", message: text("u2", { kind: "you" }) } });
    expect(state("a")).toBe("idle");
  });

  test("removing a message or a chat leaves no failure behind", () => {
    jest.useFakeTimers();
    event({ event: "snapshot", data: snapshot([group()]) });
    for (const id of ["f2", "f3"]) {
      event({ event: "message.added", data: { chat_id: "open", message: text(id, bot("a"), { kind: "streaming" }) } });
      event({ event: "message.updated", data: { chat_id: "open", message: text(id, bot("a"), { kind: "failed", error: "down" }) } });
    }
    event({ event: "message.removed", data: { chat_id: "open", message_id: "f2" } });
    expect(Object.keys(useStore.getState().failures)).toEqual(["f3"]);
    event({ event: "chat.removed", data: { chat_id: "open" } });
    expect(useStore.getState().failures).toEqual({});
  });
});
