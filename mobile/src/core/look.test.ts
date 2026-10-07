import { describe, expect, test } from "bun:test";
import {
  AVATAR_STATES,
  contrastWarnings,
  emptyLook,
  generatedOptions,
  isDefaultLook,
  lookToWire,
  normalizeHex,
  resetState,
  resolveAppearance,
  sameLook,
  sanitizeLook,
  setColor,
  setField,
  shuffleLook,
  type BotLook,
} from "./look";

const look = (base: BotLook["base"], states?: BotLook["states"]): BotLook => ({ version: 1, base, ...(states ? { states } : {}) });

describe("sanitizeLook", () => {
  test("keeps valid fields and drops unknown, out-of-range, and malformed ones", () => {
    const clean = sanitizeLook({
      version: 1,
      base: { shape: "sun", expression: "wink", background: "squircle", hue: 359.5, tone: "ink", palette: { head: "#aabbcc", eye: "nope", bg: "#FFF" }, motion: false, trait: "x", css: "y" },
      states: { error: { expression: "sad" }, retry: {}, bogus: { expression: "happy" } },
      extra: 1,
    });
    expect(clean).toEqual({ version: 1, base: { shape: "sun", expression: "wink", background: "squircle", hue: 359.5, tone: "ink", palette: { head: "#AABBCC" }, motion: false }, states: { error: { expression: "sad" } } });
  });

  test("rejects a hue outside [0, 360), a non-finite hue, and a wrong version", () => {
    expect(sanitizeLook({ version: 1, base: { hue: 360 } })?.base).toEqual({});
    expect(sanitizeLook({ version: 1, base: { hue: -1 } })?.base).toEqual({});
    expect(sanitizeLook({ version: 1, base: { hue: Number.NaN } })?.base).toEqual({});
    expect(sanitizeLook({ version: 2, base: {} })).toBeUndefined();
    expect(sanitizeLook(null)).toBeUndefined();
    expect(sanitizeLook([])).toBeUndefined();
  });

  test("equal looks serialize equally whatever the key order", () => {
    const a = sanitizeLook({ version: 1, base: { tone: "mid", shape: "cloud" } });
    const b = sanitizeLook({ base: { shape: "cloud", tone: "mid" }, version: 1 });
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
  });
});

describe("wire", () => {
  test("a look with nothing set is the seeded default and goes as null", () => {
    expect(isDefaultLook(undefined)).toBe(true);
    expect(isDefaultLook(emptyLook())).toBe(true);
    expect(isDefaultLook(look({}, { error: {} }))).toBe(true);
    expect(lookToWire(emptyLook())).toBeNull();
    expect(lookToWire(look({ shape: "sun" }))).toEqual(look({ shape: "sun" }));
    expect(sameLook(undefined, emptyLook())).toBe(true);
    expect(sameLook(look({ shape: "sun" }), emptyLook())).toBe(false);
  });
});

describe("resolveAppearance", () => {
  test("a missing look leaves shape, hue, and tone to the seed and defaults expression and motion", () => {
    expect(resolveAppearance(undefined, "idle")).toEqual({ expression: "idle", motion: true });
  });

  test("a state overrides the base field by field and merges palette channels", () => {
    const saved = look({ shape: "sun", expression: "happy", palette: { head: "#111111", eye: "#EEEEEE" } }, { error: { expression: "sad", palette: { eye: "#FF0000" } } });
    expect(resolveAppearance(saved, "error")).toEqual({ shape: "sun", expression: "sad", palette: { head: "#111111", eye: "#FF0000" }, motion: true });
    expect(resolveAppearance(saved, "thinking")).toEqual({ shape: "sun", expression: "happy", palette: { head: "#111111", eye: "#EEEEEE" }, motion: true });
  });

  test("a state can turn motion off or back on over the base", () => {
    expect(resolveAppearance(look({ motion: false }, { working: { motion: true } }), "working").motion).toBe(true);
    expect(resolveAppearance(look({ motion: false }, { working: { motion: true } }), "idle").motion).toBe(false);
  });

  test("none maps to a transparent background and tone names to their swatch position", () => {
    expect(generatedOptions(resolveAppearance(look({ background: "none", tone: "pale", hue: 10 }), "idle"))).toEqual({ background: false, tone: 0.28, hue: 10 });
    expect(generatedOptions(resolveAppearance(undefined, "idle"))).toEqual({});
    expect(generatedOptions(resolveAppearance(look({ background: "squircle" }), "idle")).background).toBe("squircle");
  });
});

describe("draft edits", () => {
  test("setField sets and clears, in the base and in a state, without touching the other", () => {
    let draft = setField(emptyLook(), "base", "shape", "capsule");
    draft = setField(draft, "error", "expression", "scared");
    expect(draft).toEqual(look({ shape: "capsule" }, { error: { expression: "scared" } }));
    draft = setField(draft, "error", "expression", undefined);
    expect(draft).toEqual(look({ shape: "capsule" }));
    expect(setField(draft, "base", "shape", undefined)).toEqual(emptyLook());
  });

  test("setColor sets one channel at a time and drops an emptied palette", () => {
    let draft = setColor(emptyLook(), "base", "head", "#102030");
    draft = setColor(draft, "base", "bg", "#FFFFFF");
    expect(draft.base.palette).toEqual({ head: "#102030", bg: "#FFFFFF" });
    draft = setColor(setColor(draft, "base", "head", undefined), "base", "bg", undefined);
    expect(draft).toEqual(emptyLook());
  });

  test("resetState returns one state to inheriting the base and keeps the others", () => {
    const saved = look({ shape: "sun" }, { error: { expression: "sad" }, retry: { expression: "unsure" } });
    expect(resetState(saved, "error")).toEqual(look({ shape: "sun" }, { retry: { expression: "unsure" } }));
    expect(resetState(look({ shape: "sun" }, { error: { expression: "sad" } }), "error")).toEqual(look({ shape: "sun" }));
  });

  test("shuffle draws a new shape, hue, and tone, keeps expression, background, and motion, and drops colors hiding the draw", () => {
    const saved = look(
      { shape: "round", hue: 5, tone: "pale", expression: "happy", background: "circle", motion: false, palette: { head: "#111111" } },
      { error: { expression: "sad", shape: "boxy", hue: 9, palette: { eye: "#000000" } } },
    );
    const values = [0.99, 0.5, 0.0];
    const shuffled = shuffleLook(saved, () => values.shift() ?? 0);
    expect(shuffled.base).toEqual({ shape: "triangle", hue: 180, tone: "pastel", expression: "happy", background: "circle", motion: false });
    expect(shuffled.states).toEqual({ error: { expression: "sad" } });
  });

  test("shuffle never returns an out-of-range pick", () => {
    const shuffled = shuffleLook(emptyLook(), () => 0.9999999);
    expect(shuffled.base.hue).toBeLessThan(360);
    expect(shuffled.base.shape).toBe("triangle");
    expect(shuffled.base.tone).toBe("ink");
  });
});

describe("colors", () => {
  test("normalizeHex accepts what a person types and canonicalizes it", () => {
    expect(normalizeHex(" #aBc123 ")).toBe("#ABC123");
    expect(normalizeHex("aabbcc")).toBe("#AABBCC");
    expect(normalizeHex("#abc")).toBeUndefined();
    expect(normalizeHex("#gggggg")).toBeUndefined();
  });

  test("contrast feedback names the chosen pair that falls short and ignores a generated one", () => {
    expect(contrastWarnings({ palette: { head: "#808080", eye: "#7F7F7F" } })).toEqual(["eye"]);
    expect(contrastWarnings({ palette: { head: "#FFFFFF", eye: "#000000" } })).toEqual([]);
    expect(contrastWarnings({ palette: { eye: "#000000" } })).toEqual([]);
    expect(contrastWarnings({ background: "circle", palette: { head: "#FFFFFF", bg: "#FEFEFE" } })).toEqual(["bg"]);
    // No backdrop drawn: nothing sits behind the body to contrast with.
    expect(contrastWarnings({ background: "none", palette: { head: "#FFFFFF", bg: "#FEFEFE" } })).toEqual([]);
  });
});

test("every state is listed once, idle first", () => {
  expect(AVATAR_STATES[0]).toBe("idle");
  expect(new Set(AVATAR_STATES).size).toBe(7);
});
