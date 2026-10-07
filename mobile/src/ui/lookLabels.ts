// The editor's names for each look option, in the app's language. Called while rendering (the
// language can change), with literal keys so `bun run l10n` sees every one.

import type { Background, BotAvatarState, Expression, Shape, Tone } from "../core/look";
import { t } from "../i18n";

export function shapeLabels(): Record<Shape, string> {
  return { round: t("Round"), organic: t("Organic"), boxy: t("Boxy"), capsule: t("Capsule"), nub: t("Nub"), cloud: t("Cloud"), droplet: t("Droplet"), hexagon: t("Hexagon"), sun: t("Sun"), triangle: t("Triangle") };
}

export function expressionLabels(): Record<Expression, string> {
  return { idle: t("Calm"), happy: t("Happy"), sad: t("Sad"), mad: t("Mad"), surprised: t("Surprised"), wink: t("Wink"), sleepy: t("Sleepy"), smug: t("Smug"), unsure: t("Unsure"), scared: t("Scared"), love: t("Love"), shy: t("Shy"), sick: t("Sick"), thinking: t("Pensive") };
}

export function backgroundLabels(): Record<Background, string> {
  return { none: t("None"), square: t("Square"), circle: t("Circle"), squircle: t("Squircle") };
}

export function toneLabels(): Record<Tone, string> {
  return { pastel: t("Pastel"), pale: t("Pale"), mid: t("Medium"), deep: t("Deep"), bright: t("Bright"), ink: t("Ink") };
}

export function stateLabels(): Record<BotAvatarState, string> {
  return { idle: t("Idle"), thinking: t("Thinking"), responding: t("Responding"), working: t("Working"), waiting: t("Waiting"), retry: t("Retrying"), error: t("Error") };
}
