import { expect, test } from "bun:test";
import { appearanceFor, editAppearance, resetAppearance, shuffleAppearance, lookProblem, previewAppearance, type BotLook } from "./botLook";
import { avatarActivity, aggregateAvatarActivity } from "./avatarActivity";
import { toMessage, type WireBody } from "./wire";

const message = (body: WireBody, state = "complete", botID = "b", at = 20) => toMessage({ id: `m-${at}`, chat_id: "c", author: { kind: "bot", bot_id: botID }, body, state: { kind: state }, created_at: at });

test("state appearance inherits individual palette channels and motion without changing the saved draft", () => {
  const look: BotLook = { version: 1, base: { shape: "cloud", motion: false, palette: { head: "#AABBCC", eye: "#112233" } }, states: { working: { expression: "happy", palette: { head: "#FFFFFF" } } } };
  expect(appearanceFor(look, "working")).toEqual({ shape: "cloud", motion: false, expression: "happy", background: "none", palette: { head: "#FFFFFF", eye: "#112233" } });
  const draft = editAppearance(look, "working", { background: "square" });
  expect(look.states?.working?.background).toBeUndefined();
  expect(appearanceFor(resetAppearance(draft, "working"), "working")).toEqual(appearanceFor(look, "idle"));
  expect(resetAppearance(draft, "base")).toBeNull();
});

test("draft clearing inherits one color and keeps the other overrides; shuffle never changes identity or saved look", () => {
  const look: BotLook = { version: 1, base: { palette: { eye: "#000000" } }, states: { retry: { palette: { head: "#FFFFFF", bg: "#ABCDEF" } } } };
  const edited = editAppearance(look, "retry", { palette: { head: undefined } });
  expect(appearanceFor(edited, "retry").palette).toEqual({ eye: "#000000", bg: "#ABCDEF" });
  const shuffled = shuffleAppearance(look, "retry", () => 0.99);
  expect(shuffled.states?.retry).toMatchObject({ shape: "triangle", hue: 356, tone: "ink" });
  expect(shuffled.base).toEqual(look.base);
  expect(look.states?.retry?.shape).toBeUndefined();
});

test("look validation rejects non-finite/out-of-range hues, unknown enums, extra persisted data and noncanonical colors", () => {
  expect(lookProblem({ version: 1, base: { tone: "ink", background: "none", hue: 359.9 } })).toBeUndefined();
  for (const hue of [NaN, Infinity, -1, 360]) expect(lookProblem({ version: 1, base: { hue } })).toBeDefined();
  for (const base of [{ tone: "dark" }, { palette: { head: "#aabbcc" } }, { palette: { body: "#ABCDEF" } }, { svg: "<svg/>" }, { motion: "yes" }]) expect(lookProblem({ version: 1, base })).toBeDefined();
  expect(lookProblem({ version: 2, base: {} })).toBeDefined();
});

test("activity precedence is error, retry, permission, live tool, streamed text, thinking, idle", () => {
  const messages = [message({ kind: "text", text: "" }, "thinking"), message({ kind: "text", text: "reply" }, "streaming"), message({ kind: "tool", name: "bash", summary: "x", is_running: true }), message({ kind: "permission", plugin_id: "computer", plugin_name: "Computer", tool: "bash", summary: "x", decision: "pending" })];
  const input = { active: true, messages, startedAt: 10_000, thinking: true, retrying: true, errorUntil: 50_000 };
  expect(avatarActivity("b", input, 30_000)).toBe("error");
  expect(avatarActivity("b", { ...input, errorUntil: undefined }, 30_000)).toBe("retry");
  expect(avatarActivity("b", { ...input, errorUntil: undefined, retrying: false }, 30_000)).toBe("waiting");
  expect(avatarActivity("b", { active: true, messages: messages.slice(0, 3) }, 30_000)).toBe("working");
  expect(avatarActivity("b", { active: true, messages: messages.slice(0, 2) }, 30_000)).toBe("responding");
  expect(avatarActivity("b", { active: true, messages: messages.slice(0, 1) }, 30_000)).toBe("thinking");
  expect(avatarActivity("b", { active: true, messages: [] }, 30_000)).toBe("idle");
});

test("finished/disconnected jobs, another bot and historical messages cannot hold activity; terminal error expires", () => {
  const stale = [message({ kind: "tool", name: "bash", summary: "x", is_running: true }), message({ kind: "text", text: "bad" }, "failed")];
  expect(avatarActivity("b", { active: false, messages: stale, retrying: true, thinking: true }, 30_000)).toBe("idle");
  expect(avatarActivity("b", { active: true, startedAt: 25_000, messages: stale }, 30_000)).toBe("idle");
  expect(avatarActivity("other", { active: true, messages: stale }, 30_000)).toBe("idle");
  expect(avatarActivity("b", { active: false, messages: [], errorUntil: 30_000 }, 30_000)).toBe("idle");
  expect(avatarActivity("b", { active: true, messages: [message({ kind: "text", text: "bad" }, "failed")], errorUntil: 30_000 }, 31_000)).toBe("idle");
  expect(aggregateAvatarActivity(["thinking", "working", "waiting"])).toBe("waiting");
  expect(aggregateAvatarActivity(["waiting", "working", "thinking"])).toBe("waiting");
});

test("a new user turn bounds restored activity while completed cards and model output never infer emotion", () => {
  const user = { ...message({ kind: "text", text: "next turn" }, "complete", "b", 25), author: { kind: "you" as const } };
  const old = message({ kind: "tool", name: "bash", is_running: true });
  expect(avatarActivity("b", { active: true, messages: [old, user] })).toBe("idle");
  const complete = [message({ kind: "permission", decision: "allowed" }), message({ kind: "tool", is_running: false }), message({ kind: "text", text: "I am angry and working" })];
  expect(avatarActivity("b", { active: true, messages: complete })).toBe("idle");
  const waiting = message({ kind: "tool", is_running: true, run: { state: "waiting", session_id: "terminal" } });
  expect(avatarActivity("b", { active: true, messages: [waiting], thinking: true })).toBe("waiting");
});

test("explicit low-contrast colors remain valid and unchanged", () => {
  const look: BotLook = { version: 1, base: { palette: { head: "#AAAAAA", eye: "#AAAAAA" } } };
  expect(lookProblem(look)).toBeUndefined();
  expect(look.base.palette).toEqual({ head: "#AAAAAA", eye: "#AAAAAA" });
});

test("base and multiple activity drafts remain independent through edits, reset and validation", () => {
  const saved: BotLook = { version: 1, base: { shape: "round", motion: false, palette: { eye: "#111111" } }, states: { error: { expression: "sad" } } };
  const base = editAppearance(saved, "base", { shape: "hexagon", expression: "smug", background: "square", hue: 42.5, tone: "deep", palette: { head: "#ABCDEF", bg: "#FFFFFF" }, motion: true });
  const working = editAppearance(base, "working", { shape: "cloud", expression: "thinking", background: "none", hue: 240, tone: "ink", palette: { head: "#123456" }, motion: false });
  const waiting = editAppearance(working, "waiting", { expression: "unsure" });
  expect(appearanceFor(waiting, "working")).toEqual({ shape: "cloud", expression: "thinking", background: "none", hue: 240, tone: "ink", palette: { head: "#123456", eye: "#111111", bg: "#FFFFFF" }, motion: false });
  expect(appearanceFor(waiting, "waiting").shape).toBe("hexagon");
  expect(appearanceFor(waiting, "error").expression).toBe("sad");
  const reset = resetAppearance(waiting, "working");
  expect(appearanceFor(reset, "working")).toEqual(appearanceFor(base, "idle"));
  expect(reset?.states?.waiting?.expression).toBe("unsure");
  expect(saved.base).toEqual({ shape: "round", motion: false, palette: { eye: "#111111" } });
  const invalid = editAppearance(waiting, "working", { palette: { head: "#12" } });
  expect(lookProblem(invalid)).toBeDefined();
  expect(lookProblem(editAppearance(invalid, "working", { palette: { head: "#123456" } }))).toBeUndefined();
});

test("invalid drafts keep the last valid preview, and base preview never includes Idle overrides", () => {
  const saved: BotLook = { version: 1, base: { shape: "cloud", palette: { head: "#123456" } }, states: { idle: { shape: "sun" }, working: { expression: "thinking" } } };
  const initial = { look: saved, state: "idle" as const };
  const base = previewAppearance(saved, "base", initial);
  expect(appearanceFor(base.look, base.state).shape).toBe("cloud");
  const invalid = editAppearance(saved, "working", { palette: { head: "#" } });
  expect(previewAppearance(invalid, "working", base)).toBe(base);
  expect(saved.base.palette?.head).toBe("#123456");
  const valid = editAppearance(invalid, "working", { palette: { head: "#ABCDEF" } });
  const working = previewAppearance(valid, "working", base);
  expect(appearanceFor(working.look, working.state)).toMatchObject({ shape: "cloud", expression: "thinking", palette: { head: "#ABCDEF" } });
});

test("an unanswered asking command is waiting, while checking and running tools are working", () => {
  for (const isRunning of [true, false]) {
    const asking = message({ kind: "tool", name: "bash", is_running: isRunning, run: { state: "asking", command: "echo approved", reason: "Needs approval" } });
    expect(avatarActivity("b", { active: true, messages: [asking] })).toBe("waiting");
    expect(avatarActivity("b", { active: false, messages: [asking] })).toBe("idle");
  }
  for (const state of ["checking", "running"]) {
    const tool = message({ kind: "tool", name: "bash", is_running: true, run: { state, command: "echo approved" } });
    expect(avatarActivity("b", { active: true, messages: [tool] })).toBe("working");
  }
});
