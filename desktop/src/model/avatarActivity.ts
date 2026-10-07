import type { BotAvatarState } from "./botLook";
import type { Message } from "./models";

export interface AvatarActivityInput {
  active: boolean;
  messages: readonly Message[];
  startedAt?: number;
  retrying?: boolean;
  thinking?: boolean;
  errorUntil?: number;
}
const priority: readonly BotAvatarState[] = ["error", "retry", "waiting", "working", "responding", "thinking", "idle"];

/** Only current structured signals count. Status text and finished history never do. */
export function avatarActivity(botID: string, input: AvatarActivityInput, now = Date.now()): BotAvatarState {
  if (input.errorUntil !== undefined && now < input.errorUntil) return "error";
  if (!input.active) return "idle";
  if (input.retrying) return "retry";
  // A new user turn also bounds restored snapshots whose job start is not available.
  let start = input.startedAt ?? 0;
  for (const message of input.messages) if (message.author.kind === "you" && !message.queued) start = Math.max(start, message.createdAt);
  let result: BotAvatarState = input.thinking ? "thinking" : "idle";
  for (const message of input.messages) {
    if (message.createdAt < start || message.author.kind !== "bot" || message.author.botID !== botID) continue;
    const body = message.body;
    let state: BotAvatarState = "idle";
    if (body.kind === "permission" && body.request.decision === "pending") state = "waiting";
    else if (body.kind === "tool" && body.tool.run?.state === "asking") state = "waiting";
    else if (body.kind === "tool" && body.tool.isRunning) state = body.tool.run?.state === "waiting" ? "waiting" : "working";
    else if (body.kind === "text" && message.state.kind === "streaming") state = "responding";
    else if (message.state.kind === "thinking") state = "thinking";
    if (priority.indexOf(state) < priority.indexOf(result)) result = state;
  }
  return result;
}

/** Deterministic across chat iteration order; group members resolve independently. */
export function aggregateAvatarActivity(states: readonly BotAvatarState[]): BotAvatarState {
  return priority.find((state) => states.includes(state)) ?? "idle";
}
