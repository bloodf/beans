// What a bot's avatar is doing, from the structured signals the store already mirrors: turns in
// flight, thinking, retries, and the chat's own messages. Never from localized status text or
// message content. Pure, so the avatars and the tests share it.
//
// A newer message from the user starts a new turn, so nothing the bot did before it counts:
// a stale permission card or failed reply cannot pin the state. Other bots' messages in a group
// are skipped, since each member resolves from its own latest message.
//
// A reply the phone watched fail is an `error` while its entry in `StoreState.failures` exists;
// the store removes the entry when `ERROR_DISPLAY_MS` is up, a newer message or the bot's next
// turn removes it sooner, and a failure in loaded history or a snapshot never has one. So the
// resolver reads no clock and a subscribed avatar re-renders exactly when the error ends.

import type { BotAvatarState } from "./look";
import type { Message } from "./model";
import type { StoreState } from "./store";

export type ActivityState = Pick<StoreState, "running" | "thinking" | "retries" | "chats" | "failures">;

/// Higher wins when one bot works in several chats: error, retry, waiting, working, responding,
/// thinking, idle.
const RANK: Record<BotAvatarState, number> = { idle: 0, thinking: 1, responding: 2, working: 3, waiting: 4, retry: 5, error: 6 };

/// The bot's latest message since the user last wrote, other bots' skipped.
function latestOwn(messages: Message[], botId: string): Message | undefined {
  for (let i = messages.length - 1; i >= 0; i--) {
    const author = messages[i].author;
    if (author.kind === "you") return undefined;
    if (author.kind === "bot" && author.bot_id === botId) return messages[i];
  }
  return undefined;
}

/// The bot waits on the user: its message since then is an open permission card or command question.
function waitsOnUser(message: Message | undefined): boolean {
  const body = message?.body;
  if (body?.kind === "permission") return body.decision === "pending";
  return body?.kind === "tool" && (body.run?.state === "asking" || body.run?.state === "waiting");
}

/// One bot in one chat.
export function resolveChatBotState(s: ActivityState, chatId: string, botId: string): BotAvatarState {
  const chat = s.chats.find((c) => c.id === chatId);
  if (!chat) return "idle";
  const running = Object.values(s.running).some((r) => r.chatId === chatId && r.botId === botId);
  const own = latestOwn(chat.messages, botId);
  if (!running && own && s.failures[own.id] && own.state.kind === "failed") return "error";
  const retry = s.retries[chatId];
  if (running && retry && (!retry.bot_id || retry.bot_id === botId)) return "retry";
  if (waitsOnUser(own)) return "waiting";
  if (!running) return "idle";
  if (own?.body.kind === "tool" && own.body.is_running) return "working";
  if (own?.body.kind === "text" && own.state.kind === "streaming") return "responding";
  if (s.thinking[chatId] === botId || own?.state.kind === "thinking") return "thinking";
  return "idle";
}

/// One bot across every chat it is in: the highest-priority state any of them shows.
export function resolveBotState(s: ActivityState, botId: string): BotAvatarState {
  let best: BotAvatarState = "idle";
  for (const chat of s.chats) {
    if (!chat.bot_ids.includes(botId)) continue;
    const state = resolveChatBotState(s, chat.id, botId);
    if (RANK[state] > RANK[best]) best = state;
  }
  return best;
}
