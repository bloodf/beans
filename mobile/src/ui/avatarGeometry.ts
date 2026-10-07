// Endpoint geometry for a bot's look in one state, from the shared package, cached by bot id +
// the canonical look + state (the package leaves caching to the caller). A look from the core
// is read through `sanitizeLook` first, so a newer core's fields can never make the strict
// package throw during a render.

import { botAvatarGeometry, type AvatarGeometry } from "@beans/blobatar";
import { isDefaultLook, sanitizeLook, type BotAvatarState, type BotLook } from "../core/look";

/// Distinct (bot, look, state) endpoints kept; a roster has a few dozen bots at seven states.
const CACHE_MAX = 512;
const cache = new Map<string, AvatarGeometry>();

export function avatarGeometry(botId: string, look: BotLook | undefined, state: BotAvatarState): AvatarGeometry {
  const clean = isDefaultLook(look) ? null : sanitizeLook(look)!;
  const key = `${botId}\u0000${state}\u0000${JSON.stringify(clean)}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const made = botAvatarGeometry(botId, clean, state);
  // Oldest first out: a Map iterates in insertion order.
  if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value!);
  cache.set(key, made);
  return made;
}
