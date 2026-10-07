// Desktop mirror of the encrypted version-1 appearance contract. Geometry and seeded
// traits belong to @beans/blobatar; this module never invents shape bands or paths.
export const avatarStates = ["idle", "thinking", "responding", "working", "waiting", "retry", "error"] as const;
export const avatarShapes = ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"] as const;
export const avatarExpressions = ["idle", "happy", "sad", "mad", "surprised", "wink", "sleepy", "smug", "unsure", "scared", "love", "shy", "sick", "thinking"] as const;
export const avatarBackgrounds = ["none", "square", "circle", "squircle"] as const;
export const avatarTones = ["pastel", "pale", "mid", "deep", "bright", "ink"] as const;
export type BotAvatarState = typeof avatarStates[number];
export interface BotAppearance {
  shape?: typeof avatarShapes[number];
  expression?: typeof avatarExpressions[number];
  background?: typeof avatarBackgrounds[number];
  hue?: number;
  tone?: typeof avatarTones[number];
  palette?: { head?: string; eye?: string; bg?: string };
  motion?: boolean;
}
export interface BotLook {
  version: 1;
  base: BotAppearance;
  states?: Partial<Record<BotAvatarState, BotAppearance>>;
}
export type AppearanceSelection = "base" | BotAvatarState;

export interface LookPreview {
  look: BotLook | null | undefined;
  state: BotAvatarState;
}

/** Invalid partial input keeps real last-valid geometry mounted. Base preview excludes
 * the independent Idle override; no geometry or seeded values are invented here. */
export function previewAppearance(look: BotLook | null | undefined, selection: AppearanceSelection, previous: LookPreview): LookPreview {
  if (lookProblem(look)) return previous;
  return selection === "base"
    ? { look: { version: 1, base: look?.base ?? {} }, state: "idle" }
    : { look, state: selection };
}

/** Resolve inheritance only. The package still resolves omitted seeded traits. */
export function appearanceFor(look: BotLook | null | undefined, state: BotAvatarState): BotAppearance {
  const base = look?.base;
  const override = look?.states?.[state];
  const palette = base?.palette || override?.palette ? { ...base?.palette, ...override?.palette } : undefined;
  return { expression: "idle", background: "none", motion: true, ...base, ...override, ...(palette ? { palette } : {}) };
}

/** Immutable editing keeps Cancel and a failed Save independent of the account. */
export function editAppearance(look: BotLook | null | undefined, selection: AppearanceSelection, patch: BotAppearance): BotLook {
  const current = selection === "base" ? look?.base : look?.states?.[selection];
  const next = { ...current, ...patch };
  if (patch.palette) next.palette = { ...current?.palette, ...patch.palette };
  for (const key of Object.keys(next) as (keyof BotAppearance)[]) if (next[key] === undefined) delete next[key];
  if (next.palette) {
    for (const key of ["head", "eye", "bg"] as const) if (next.palette[key] === undefined) delete next.palette[key];
    if (!Object.keys(next.palette).length) delete next.palette;
  }
  return selection === "base"
    ? { ...look, version: 1, base: next }
    : { ...look, version: 1, base: look?.base ?? {}, states: { ...look?.states, [selection]: next } };
}

export function resetAppearance(look: BotLook | null | undefined, selection: AppearanceSelection): BotLook | null {
  if (selection === "base") return null;
  if (!look) return null;
  const states = { ...look.states };
  delete states[selection];
  return { version: 1, base: look.base, ...(Object.keys(states).length ? { states } : {}) };
}

/** Shuffle edits explicit appearance values, never the bot's stable seed/identity. */
export function shuffleAppearance(look: BotLook | null | undefined, selection: AppearanceSelection, random = Math.random): BotLook {
  return editAppearance(look, selection, {
    shape: avatarShapes[Math.floor(random() * avatarShapes.length)],
    hue: Math.floor(random() * 360),
    tone: avatarTones[Math.floor(random() * avatarTones.length)],
  });
}

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const only = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).every((key) => keys.includes(key));
export const lookErrors = {
  invalid: "Invalid avatar appearance.",
  unsupported: "Unsupported avatar appearance. Update the app to edit it.",
} as const;
export function lookProblem(value: unknown): string | undefined {
  if (value == null) return undefined;
  if (!record(value)) return lookErrors.invalid;
  if (value.version !== 1 || !only(value, ["version", "base", "states"])) return lookErrors.unsupported;
  if (!record(value.base)) return lookErrors.invalid;
  const appearance = (item: unknown): string | undefined => {
    if (!record(item)) return lookErrors.invalid;
    if (!only(item, ["shape", "expression", "background", "hue", "tone", "palette", "motion"])) return lookErrors.unsupported;
    for (const [key, choices] of [["shape", avatarShapes], ["expression", avatarExpressions], ["background", avatarBackgrounds], ["tone", avatarTones]] as const) {
      if (item[key] !== undefined && !(choices as readonly unknown[]).includes(item[key])) return typeof item[key] === "string" ? lookErrors.unsupported : lookErrors.invalid;
    }
    if (item.hue !== undefined && (typeof item.hue !== "number" || !Number.isFinite(item.hue) || item.hue < 0 || item.hue >= 360)) return lookErrors.invalid;
    if (item.motion !== undefined && typeof item.motion !== "boolean") return lookErrors.invalid;
    if (item.palette !== undefined) {
      if (!record(item.palette)) return lookErrors.invalid;
      if (!only(item.palette, ["head", "eye", "bg"])) return lookErrors.unsupported;
      if (!Object.values(item.palette).every((color) => typeof color === "string" && /^#[0-9A-F]{6}$/.test(color))) return lookErrors.invalid;
    }
  };
  const baseError = appearance(value.base);
  if (baseError) return baseError;
  if (value.states !== undefined) {
    if (!record(value.states)) return lookErrors.invalid;
    if (!only(value.states, [...avatarStates])) return lookErrors.unsupported;
    for (const state of Object.values(value.states)) {
      const error = appearance(state);
      if (error) return error;
    }
  }
}
