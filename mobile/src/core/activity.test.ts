import { describe, expect, test } from "bun:test";
import { resolveBotState, resolveChatBotState, type ActivityState } from "./activity";
import type { Body, Chat, Message, MessageState } from "./model";

function message(botId: string, body: Body, state: MessageState = { kind: "complete" }): Message {
  return { id: `${Math.random()}`, chat_id: "c", author: { kind: "bot", bot_id: botId }, body, state, created_at: 1 };
}

const you = (): Message => ({ id: `${Math.random()}`, chat_id: "c", author: { kind: "you" }, body: { kind: "text", text: "go" }, state: { kind: "complete" }, created_at: 1 });
const tool = (is_running: boolean, run?: Extract<Body, { kind: "tool" }>["run"]): Body => ({ kind: "tool", name: "bash", summary: "", detail: "", is_running, run });
const permission = (decision: "pending" | "allowed"): Body => ({ kind: "permission", plugin_id: "p", plugin_name: "P", tool: "t", summary: "", decision });
const failed = (botId: string) => message(botId, { kind: "text", text: "Partial" }, { kind: "failed", error: "down" });

function state(messages: Message[], more: Partial<ActivityState> = {}, kind: Chat["kind"] = "dm"): ActivityState {
  // A reply that failed is one the phone watched fail, unless the test says otherwise.
  const failures = Object.fromEntries(messages.filter((m) => m.state.kind === "failed" && m.author.kind === "bot").map((m) => [m.id, { chatId: "c", botId: m.author.kind === "bot" ? m.author.bot_id : "", at: 1 }]));
  return { running: {}, thinking: {}, retries: {}, failures, chats: [{ id: "c", kind, bot_ids: ["a", "b"], is_pinned: false, created_at: 0, messages, unread_count: 0 }], ...more };
}

const running = { job: { chatId: "c", botId: "a" } };
const at = (s: ActivityState, bot = "a") => resolveChatBotState(s, "c", bot);

describe("one bot in one chat", () => {
  test("a bot with no turn is idle, however busy the chat looks", () => {
    expect(at(state([message("a", tool(true))]))).toBe("idle");
    expect(at(state([], { thinking: { c: "a" } }))).toBe("idle");
    expect(at(state([]), "nobody")).toBe("idle");
  });

  test("a turn reads as thinking, responding, or working from structured signals", () => {
    expect(at(state([], { running }))).toBe("idle");
    expect(at(state([], { running, thinking: { c: "a" } }))).toBe("thinking");
    expect(at(state([message("a", { kind: "text", text: "Hel" }, { kind: "streaming" })], { running }))).toBe("responding");
    expect(at(state([message("a", tool(true))], { running }))).toBe("working");
  });

  test("a finished tool call does not pin the bot to working", () => {
    expect(at(state([message("a", tool(false))], { running }))).toBe("idle");
  });

  test("what the message says never changes the state", () => {
    expect(at(state([message("a", { kind: "text", text: "I am so sorry, error, thinking, retry" })], { running }))).toBe("idle");
  });

  test("a retry in the chat is the retrying bot's, or any bot's when the core names none", () => {
    const retry = { attempt: 1, max_attempts: 3, delay_ms: 2000 };
    expect(at(state([], { running, retries: { c: retry } }))).toBe("retry");
    expect(at(state([], { running, retries: { c: { ...retry, bot_id: "a" } } }))).toBe("retry");
    expect(at(state([], { running, retries: { c: { ...retry, bot_id: "b" } } }))).toBe("idle");
  });

  test("an open permission card or command question is waiting, a decided one is not", () => {
    expect(at(state([message("a", permission("pending"))]))).toBe("waiting");
    expect(at(state([message("a", permission("allowed"))]))).toBe("idle");
    expect(at(state([message("a", tool(true, { command: "sudo", state: "asking" }))], { running }))).toBe("waiting");
    expect(at(state([message("a", tool(true, { command: "x", state: "waiting" }))], { running }))).toBe("waiting");
    expect(at(state([message("a", tool(true, { command: "x", state: "running" }))], { running }))).toBe("working");
    expect(at(state([message("a", permission("pending")), message("a", permission("allowed"))]))).toBe("idle");
  });

  test("a card left open before the user's newer message is stale", () => {
    expect(at(state([message("a", permission("pending")), you()]))).toBe("idle");
    expect(at(state([message("a", tool(true, { command: "x", state: "waiting" })), you()], { running }))).toBe("idle");
  });

  test("a failed reply the phone watched fail is an error until the user or a newer reply moves it on", () => {
    expect(at(state([failed("a")]))).toBe("error");
    expect(at(state([failed("a"), you()]))).toBe("idle");
    expect(at(state([failed("a"), message("a", { kind: "text", text: "ok" })]))).toBe("idle");
    expect(at(state([failed("a")], { running }))).not.toBe("error");
  });

  test("a failure the phone did not watch (history, a snapshot) has no entry and is never an error", () => {
    expect(at(state([failed("a")], { failures: {} }))).toBe("idle");
  });

  test("error outranks retry and waiting", () => {
    expect(at(state([failed("a")], { retries: { c: { attempt: 1, max_attempts: 2, delay_ms: 1 } } }))).toBe("error");
  });
});

describe("group chats", () => {
  test("another member's message after this bot's does not hide this bot's state", () => {
    const group = (messages: Message[], more: Partial<ActivityState> = {}) => state(messages, more, "group");
    expect(at(group([message("a", tool(true)), message("b", { kind: "text", text: "hi" })], { running }))).toBe("working");
    expect(at(group([failed("a"), message("b", { kind: "text", text: "hi" })]))).toBe("error");
    expect(at(group([message("a", permission("pending")), message("b", { kind: "text", text: "hi" })]))).toBe("waiting");
    expect(at(group([failed("a"), message("b", { kind: "text", text: "hi" })]), "b")).toBe("idle");
  });

  test("members resolve separately", () => {
    const s = state([message("a", tool(true))], { running }, "group");
    expect(at(s, "a")).toBe("working");
    expect(at(s, "b")).toBe("idle");
  });
});

describe("one bot across chats", () => {
  test("the highest-priority state in any of its chats wins", () => {
    const chat = (id: string, messages: Message[]): Chat => ({ id, kind: "dm", bot_ids: ["a"], is_pinned: false, created_at: 0, messages, unread_count: 0 });
    const s: ActivityState = {
      running: { j1: { chatId: "c1", botId: "a" }, j2: { chatId: "c2", botId: "a" } },
      thinking: { c1: "a" },
      retries: {},
      failures: {},
      chats: [chat("c1", []), chat("c2", [{ ...message("a", tool(true)), chat_id: "c2" }])],
    };
    expect(resolveBotState(s, "a")).toBe("working");
    expect(resolveBotState(s, "other")).toBe("idle");
  });

  test("a finished or recycled turn leaves nothing behind", () => {
    const busy = state([message("a", tool(true))], { running });
    expect(resolveBotState(busy, "a")).toBe("working");
    expect(resolveBotState({ ...busy, running: {}, chats: [] }, "a")).toBe("idle");
  });
});
