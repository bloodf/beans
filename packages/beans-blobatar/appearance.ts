import { contrast, fromHex, type Palette } from "./vendor/core/color";
import {
  happy, idle, love, mad, sad, scared, shy, sick, sleepy, smug, surprised, thinking, unsure, wink,
  type Expression,
} from "./vendor/core/expression";
import { resolve, type BlobatarOptions } from "./vendor/core/render";
import { style } from "./vendor/core/styles/blob";
import type { Layout } from "./vendor/core/styles/compose";

export const BOT_AVATAR_STATES = ["idle", "thinking", "responding", "working", "waiting", "retry", "error"] as const;
export const BOT_SHAPES = ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"] as const;
export const BOT_EXPRESSIONS = [
  "idle", "happy", "sad", "mad", "surprised", "wink", "sleepy", "smug", "unsure", "scared", "love", "shy", "sick", "thinking",
] as const;
export const BOT_BACKGROUNDS = ["none", "square", "circle", "squircle"] as const;
export const BOT_TONES = ["pastel", "pale", "mid", "deep", "bright", "ink"] as const;

export type BotAvatarState = (typeof BOT_AVATAR_STATES)[number];
export type BotShape = (typeof BOT_SHAPES)[number];
export type BotExpression = (typeof BOT_EXPRESSIONS)[number];
export type BotBackground = (typeof BOT_BACKGROUNDS)[number];
export type BotTone = (typeof BOT_TONES)[number];

export interface BotPalette { head?: string; eye?: string; bg?: string }
export interface BotAppearance {
  shape?: BotShape;
  expression?: BotExpression;
  background?: BotBackground;
  hue?: number;
  tone?: BotTone;
  palette?: BotPalette;
  motion?: boolean;
}
export interface BotLook { version: 1; base: BotAppearance; states?: Partial<Record<BotAvatarState, BotAppearance>> }

export interface ResolvedBotAppearance {
  shape: BotShape;
  expression: BotExpression;
  background: BotBackground;
  hue: number;
  tone: BotTone;
  palette: Required<BotPalette>;
  motion: boolean;
}

/**
 * A point inside each shape's band in `vendor/core/styles/blob.ts` (upper edges
 * .22 .48 .6 .7 .79 .86 .915 .95 .98 1). Pinned as the `shape` trait.
 */
const SHAPE_POSITION: Record<BotShape, number> = {
  round: 0.11, organic: 0.35, boxy: 0.54, capsule: 0.65, nub: 0.745,
  cloud: 0.825, droplet: 0.8875, hexagon: 0.9325, sun: 0.965, triangle: 0.99,
};

/** Upper edges of the tone swatches in `vendor/core/color.ts`; each band excludes its edge. */
const TONE_EDGES = [0.2, 0.36, 0.62, 0.8, 0.93, 1] as const;
const TONE_POSITION: Record<BotTone, number> = { pastel: 0.1, pale: 0.28, mid: 0.49, deep: 0.71, bright: 0.865, ink: 0.965 };

const toneName = (v: number): BotTone => BOT_TONES[TONE_EDGES.findIndex(edge => v < edge)] ?? "pastel";

const EXPRESSIONS: Record<BotExpression, Expression> = {
  idle, happy, sad, mad, surprised, wink, sleepy, smug, unsure, scared, love, shy, sick, thinking,
};

const HEX = /^#[0-9A-F]{6}$/;
const APPEARANCE_KEYS = ["shape", "expression", "background", "hue", "tone", "palette", "motion"];

class LookError extends Error {}
const fail = (path: string, message: string): never => {
  throw new LookError(`${path} ${message}`);
};
const isRecord = (v: unknown): v is Record<string, unknown> => {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
};
const onlyKeys = (o: Record<string, unknown>, allowed: readonly string[], path: string) => {
  for (const k of Object.keys(o)) if (!allowed.includes(k)) fail(path, `has unknown field "${k}"`);
};
function oneOf<T extends string>(v: unknown, list: readonly T[], path: string): T {
  if (typeof v !== "string" || !list.includes(v as T)) fail(path, `must be one of ${list.join(", ")}`);
  return v as T;
}

function appearance(v: unknown, path: string): BotAppearance {
  if (!isRecord(v)) return fail(path, "must be an object");
  onlyKeys(v, APPEARANCE_KEYS, path);
  const out: BotAppearance = {};
  if ("shape" in v) out.shape = oneOf(v.shape, BOT_SHAPES, `${path}.shape`);
  if ("expression" in v) out.expression = oneOf(v.expression, BOT_EXPRESSIONS, `${path}.expression`);
  if ("background" in v) out.background = oneOf(v.background, BOT_BACKGROUNDS, `${path}.background`);
  if ("hue" in v) {
    const h = v.hue;
    if (typeof h !== "number" || !Number.isFinite(h) || h < 0 || h >= 360) fail(`${path}.hue`, "must be a finite number in [0, 360)");
    out.hue = h as number;
  }
  if ("tone" in v) out.tone = oneOf(v.tone, BOT_TONES, `${path}.tone`);
  if ("palette" in v) {
    const p = v.palette;
    if (!isRecord(p)) return fail(`${path}.palette`, "must be an object");
    onlyKeys(p, ["head", "eye", "bg"], `${path}.palette`);
    const palette: BotPalette = {};
    for (const k of ["head", "eye", "bg"] as const) {
      if (!(k in p)) continue;
      const c = p[k];
      if (typeof c !== "string" || !HEX.test(c)) fail(`${path}.palette.${k}`, "must be uppercase #RRGGBB");
      palette[k] = c as string;
    }
    out.palette = palette;
  }
  if ("motion" in v) {
    if (typeof v.motion !== "boolean") fail(`${path}.motion`, "must be a boolean");
    out.motion = v.motion as boolean;
  }
  return out;
}

/** Strict structural check of an untrusted look; returns a canonical copy. */
export function validateBotLook(input: unknown): { ok: true; look: BotLook } | { ok: false; error: string } {
  try {
    if (!isRecord(input)) fail("look", "must be an object");
    const v = input as Record<string, unknown>;
    onlyKeys(v, ["version", "base", "states"], "look");
    if (v.version !== 1) fail("look.version", "must be 1");
    const look: BotLook = { version: 1, base: appearance(v.base, "look.base") };
    if ("states" in v) {
      const s = v.states;
      if (!isRecord(s)) return fail("look.states", "must be an object");
      onlyKeys(s, BOT_AVATAR_STATES, "look.states");
      const states: Partial<Record<BotAvatarState, BotAppearance>> = {};
      for (const k of BOT_AVATAR_STATES) if (k in s) states[k] = appearance(s[k], `look.states.${k}`);
      look.states = states;
    }
    return { ok: true, look };
  } catch (e) {
    if (e instanceof LookError) return { ok: false, error: e.message };
    throw e;
  }
}

/** Base with the state's override on top; palette channels merge individually. */
function effective(look: BotLook | null | undefined, state: BotAvatarState | undefined): BotAppearance {
  if (state !== undefined && !BOT_AVATAR_STATES.includes(state)) throw new TypeError(`unknown avatar state "${state}"`);
  if (look == null) return {};
  const checked = validateBotLook(look);
  if (!checked.ok) throw new TypeError(checked.error);
  const { base, states } = checked.look;
  const over = state ? states?.[state] : undefined;
  if (!over) return base;
  const merged: BotAppearance = { ...base, ...over };
  if (base.palette || over.palette) merged.palette = { ...base.palette, ...over.palette };
  return merged;
}

/** Upstream options for an appearance. Only explicit fields become options, so a missing look is the seeded default byte for byte. */
function options(a: BotAppearance): BlobatarOptions {
  const opts: BlobatarOptions = {};
  if (a.shape) opts.traits = { shape: SHAPE_POSITION[a.shape] };
  if (a.hue !== undefined) opts.hue = a.hue;
  if (a.tone) opts.tone = TONE_POSITION[a.tone];
  if (a.palette) opts.palette = { ...a.palette };
  if (a.background && a.background !== "none") opts.background = a.background;
  if (a.expression && a.expression !== "idle") opts.expression = EXPRESSIONS[a.expression];
  return opts;
}

const upper = (p: Palette): Required<BotPalette> => ({
  head: p.head!.toUpperCase(), eye: p.eye!.toUpperCase(), bg: p.bg!.toUpperCase(),
});

/** Everything the SVG and geometry paths share, resolved once. Internal. */
export function prepare(botID: string, look?: BotLook | null, state?: BotAvatarState) {
  const a = effective(look, state);
  const opts = options(a);
  const { t, palette } = resolve(botID, opts);
  const layout = style.layout(t) as Layout;
  const resolved: ResolvedBotAppearance = {
    shape: layout.shape as BotShape,
    expression: a.expression ?? "idle",
    background: a.background ?? "none",
    hue: opts.hue ?? t.num("hue", 0, 360),
    tone: a.tone ?? toneName(t("tone")),
    palette: upper(palette),
    motion: a.motion ?? true,
  };
  return { opts, t, palette, layout, resolved, expression: opts.expression };
}

export function resolveBotAppearance(botID: string, look?: BotLook | null, state?: BotAvatarState): ResolvedBotAppearance {
  return prepare(botID, look, state).resolved;
}

/** WCAG ratios for the editor's contrast warning. Explicit colors are kept regardless. */
export function botAppearanceContrast(a: ResolvedBotAppearance): { eyeOnHead: number; headOnBg: number } {
  const head = fromHex(a.palette.head);
  return {
    eyeOnHead: contrast(fromHex(a.palette.eye), head),
    headOnBg: contrast(head, fromHex(a.palette.bg)),
  };
}
