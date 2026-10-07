// A bot's saved generated look, as the core carries it in `bot.look`. The schema, the enum lists,
// state resolution, and contrast math are @beans/blobatar's; this file reads what the core sent
// without throwing (the package validates strictly) and holds the editor's draft edits. No frame,
// timestamp, or current state is ever stored.

import { BOT_AVATAR_STATES, BOT_BACKGROUNDS, BOT_EXPRESSIONS, BOT_SHAPES, BOT_TONES, type BotAppearance, type BotAvatarState, type BotBackground, type BotExpression, type BotLook, type BotPalette, type BotShape, type BotTone, type ResolvedBotAppearance } from "@beans/blobatar";

export type { BotAppearance, BotAvatarState, BotLook, BotPalette, ResolvedBotAppearance };

/// Hues the editor offers; any finite value in [0, 360) is valid on the wire.
export const HUE_STEPS = [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330] as const;

/// The base, or one activity state.
export type Scope = "base" | BotAvatarState;

// The names the editor, the avatars, and the tests use, over the package's lists.
export const AVATAR_STATES = BOT_AVATAR_STATES;
export const SHAPES = BOT_SHAPES;
export const EXPRESSIONS = BOT_EXPRESSIONS;
export const BACKGROUNDS = BOT_BACKGROUNDS;
export const TONES = BOT_TONES;
export type Shape = BotShape;
export type Expression = BotExpression;
export type Background = BotBackground;
export type Tone = BotTone;

/// The 0–1 swatch position each tone name stands for, as the frozen contract fixes them.
export const TONE_POSITION: Record<Tone, number> = { pastel: 0.1, pale: 0.28, mid: 0.49, deep: 0.71, bright: 0.865, ink: 0.965 };

/// An appearance with the contract's defaults filled in: expression idle, motion on. Shape, hue,
/// and tone stay open because a missing one is the bot-id seeded value (the package's
/// `resolveBotAppearance` fills those from a bot id).
export type ResolvedAppearance = BotAppearance & { expression: Expression; motion: boolean };

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const oneOf = <T extends string>(list: readonly T[], value: unknown): T | undefined => list.find((item) => item === value);

/// `#RRGGBB` upper-cased from what a person types (`#`, case, and surrounding space optional);
/// undefined when it is not six hex digits.
export function normalizeHex(text: string): string | undefined {
  const match = text.trim().match(/^#?([0-9a-f]{6})$/i);
  return match ? `#${match[1].toUpperCase()}` : undefined;
}

function readAppearance(value: unknown): BotAppearance {
  const out: BotAppearance = {};
  if (!isObject(value)) return out;
  const shape = oneOf(BOT_SHAPES, value.shape);
  if (shape) out.shape = shape;
  const expression = oneOf(BOT_EXPRESSIONS, value.expression);
  if (expression) out.expression = expression;
  const background = oneOf(BOT_BACKGROUNDS, value.background);
  if (background) out.background = background;
  if (typeof value.hue === "number" && Number.isFinite(value.hue) && value.hue >= 0 && value.hue < 360) out.hue = value.hue;
  const tone = oneOf(BOT_TONES, value.tone);
  if (tone) out.tone = tone;
  if (isObject(value.palette)) {
    const palette: BotPalette = {};
    for (const channel of ["head", "eye", "bg"] as const) {
      const hex = typeof value.palette[channel] === "string" ? normalizeHex(value.palette[channel] as string) : undefined;
      if (hex) palette[channel] = hex;
    }
    if (Object.keys(palette).length) out.palette = palette;
  }
  if (typeof value.motion === "boolean") out.motion = value.motion;
  return out;
}

/// The one normalizer: reads whatever the core sent (or a draft) into a look the package
/// accepts, dropping fields it does not know or that are out of range, empty overrides, and empty
/// palettes. Undefined for anything that is not a version 1 look. A newer core's look therefore
/// never throws in a render. Keys come out in a fixed order, so equal looks serialize equally.
export function sanitizeLook(value: unknown): BotLook | undefined {
  if (!isObject(value) || value.version !== 1) return undefined;
  const look: BotLook = { version: 1, base: readAppearance(value.base) };
  if (isObject(value.states)) {
    const states: Partial<Record<BotAvatarState, BotAppearance>> = {};
    for (const state of BOT_AVATAR_STATES) {
      const appearance = readAppearance(value.states[state]);
      if (Object.keys(appearance).length) states[state] = appearance;
    }
    if (Object.keys(states).length) look.states = states;
  }
  return look;
}

export function emptyLook(): BotLook {
  return { version: 1, base: {} };
}

/// Nothing customized: the bot-id seeded default.
export function isDefaultLook(look: BotLook | undefined): boolean {
  const clean = sanitizeLook(look);
  return !clean || (Object.keys(clean.base).length === 0 && !clean.states);
}

/// What `bots.update` takes for `look`: null resets generated customization, an object
/// replaces the whole look.
export function lookToWire(look: BotLook | undefined): BotLook | null {
  return isDefaultLook(look) ? null : sanitizeLook(look)!;
}

/// Whether two looks mean the same thing on the wire.
export function sameLook(a: BotLook | undefined, b: BotLook | undefined): boolean {
  return JSON.stringify(lookToWire(a)) === JSON.stringify(lookToWire(b));
}

// MARK: - Draft edits

function appearanceAt(look: BotLook, scope: Scope): BotAppearance {
  return scope === "base" ? look.base : (look.states?.[scope] ?? {});
}

function put(look: BotLook, scope: Scope, appearance: BotAppearance): BotLook {
  const next = scope === "base" ? { ...look, base: appearance } : { ...look, states: { ...look.states, [scope]: appearance } };
  return sanitizeLook(next)!;
}

/// Sets one field of the base or a state; undefined clears it, so the base's value (or, in the
/// base, the seeded default) shows through again.
export function setField<K extends Exclude<keyof BotAppearance, "palette">>(look: BotLook, scope: Scope, key: K, value: BotAppearance[K] | undefined): BotLook {
  const appearance = { ...appearanceAt(look, scope) };
  if (value === undefined) delete appearance[key];
  else appearance[key] = value;
  return put(look, scope, appearance);
}

/// Sets one explicit color (`#RRGGBB`); undefined clears it.
export function setColor(look: BotLook, scope: Scope, channel: keyof BotPalette, hex: string | undefined): BotLook {
  const palette = { ...appearanceAt(look, scope).palette };
  if (hex === undefined) delete palette[channel];
  else palette[channel] = hex;
  return put(look, scope, { ...appearanceAt(look, scope), palette });
}

/// A state back to inheriting the whole base.
export function resetState(look: BotLook, state: BotAvatarState): BotLook {
  const { [state]: _dropped, ...rest } = look.states ?? {};
  return sanitizeLook({ ...look, states: rest })!;
}

/// New shape, hue, and tone in the base, from `random` (default `Math.random`, in [0, 1)).
/// Explicit colors and the states' own shape, hue, tone, and colors go, since they would
/// hide the new draw; expression, background, motion, and the states' other overrides stay.
export function shuffleLook(look: BotLook, random: () => number = Math.random): BotLook {
  const pick = <T,>(list: readonly T[]) => list[Math.min(list.length - 1, Math.floor(random() * list.length))];
  const states: Partial<Record<BotAvatarState, BotAppearance>> = {};
  for (const state of BOT_AVATAR_STATES) {
    const override = look.states?.[state];
    if (!override) continue;
    const { shape: _s, hue: _h, tone: _t, palette: _p, ...kept } = override;
    states[state] = kept;
  }
  const { palette: _palette, ...base } = look.base;
  return sanitizeLook({ ...look, base: { ...base, shape: pick(BOT_SHAPES), hue: Math.floor(random() * 360), tone: pick(BOT_TONES) }, states })!;
}

// MARK: - Contrast feedback

/// The floors the generated palette itself keeps: eyes on the body 4.5:1, body on a drawn
/// background 1.25:1. Explicit colors bypass them, so say which pair falls short.
export const EYE_CONTRAST_FLOOR = 4.5;
export const BACKGROUND_CONTRAST_FLOOR = 1.25;

/// Which pairs of a resolved figure fall under those floors, from the package's ratios
/// (`botAppearanceContrast`), which see generated colors too.
export function resolvedContrastWarnings(resolved: ResolvedBotAppearance, ratios: { eyeOnHead: number; headOnBg: number }): ("eye" | "bg")[] {
  const out: ("eye" | "bg")[] = [];
  if (ratios.eyeOnHead < EYE_CONTRAST_FLOOR) out.push("eye");
  if (resolved.background !== "none" && ratios.headOnBg < BACKGROUND_CONTRAST_FLOOR) out.push("bg");
  return out;
}

/// The appearance a bot shows in `state`: the base with that state's override over it, palette
/// channel by channel, and the contract's defaults for expression and motion.
export function resolveAppearance(look: BotLook | undefined, state: BotAvatarState): ResolvedAppearance {
  const clean = sanitizeLook(look);
  const base = clean?.base ?? {};
  const override = clean?.states?.[state] ?? {};
  const merged: BotAppearance = { ...base, ...override };
  const palette = { ...base.palette, ...override.palette };
  if (Object.keys(palette).length) merged.palette = palette;
  else delete merged.palette;
  return { ...merged, expression: merged.expression ?? "idle", motion: merged.motion ?? true };
}

/// The options the current static renderer takes (background, hue, tone, palette), from a
/// resolved appearance. Shape and expression have no option there.
export interface GeneratedOptions { background?: false | "square" | "circle" | "squircle"; hue?: number; tone?: number; palette?: BotPalette }

export function generatedOptions(appearance: ResolvedAppearance): GeneratedOptions {
  const out: GeneratedOptions = {};
  if (appearance.background) out.background = appearance.background === "none" ? false : appearance.background;
  if (appearance.hue !== undefined) out.hue = appearance.hue;
  if (appearance.tone) out.tone = TONE_POSITION[appearance.tone];
  if (appearance.palette) out.palette = appearance.palette;
  return out;
}

function luminance(hex: string): number {
  const channel = (at: number) => {
    const v = parseInt(hex.slice(at, at + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

/// WCAG contrast ratio of two `#RRGGBB` colors, 1 to 21.
export function contrastRatio(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/// Which of the user's own chosen color pairs fall under the generated palette's floors. A pair
/// is judged only when both colors were chosen, since a generated one is not known here.
export function contrastWarnings(appearance: BotAppearance): ("eye" | "bg")[] {
  const { head, eye, bg } = appearance.palette ?? {};
  const out: ("eye" | "bg")[] = [];
  if (head && eye && contrastRatio(head, eye) < EYE_CONTRAST_FLOOR) out.push("eye");
  if (head && bg && appearance.background && appearance.background !== "none" && contrastRatio(head, bg) < BACKGROUND_CONTRAST_FLOOR) out.push("bg");
  return out;
}
