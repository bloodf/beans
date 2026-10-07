import { blobatar as upstreamBlobatar, type BlobatarOptions } from "./vendor/core/blobatar";

/** Bot IDs are stable across names, devices, and relay synchronization. */
export function botAvatarSeed(botID: string): string {
  return botID;
}

/** Deterministic, self-contained SVG; no fetch or filesystem access at render time. */
export function blobatar(botID: string, options?: BlobatarOptions): string {
  return upstreamBlobatar(botAvatarSeed(botID), options);
}

export type { BlobatarOptions } from "./vendor/core/blobatar";
export {
  BOT_AVATAR_STATES, BOT_BACKGROUNDS, BOT_EXPRESSIONS, BOT_SHAPES, BOT_TONES,
  botAppearanceContrast, resolveBotAppearance, validateBotLook,
  type BotAppearance, type BotAvatarState, type BotBackground, type BotExpression, type BotLook,
  type BotPalette, type BotShape, type BotTone, type ResolvedBotAppearance,
} from "./appearance";
export { botAvatarGeometry, botAvatarSVG } from "./geometry";
export {
  avatarFrame, avatarMorphProgress, avatarPath, interpolateAvatarGeometry,
  type AvatarCircle, type AvatarCubic, type AvatarCubicPath, type AvatarEye, type AvatarFrame,
  type AvatarGeometry, type AvatarMatrix, type AvatarPoint,
} from "./frame";
export type { IdleSeeds } from "./vendor/core/animate";
export type { Pose } from "./vendor/core/morph";
