import { describe, expect, test } from "bun:test";
import {
  BOT_AVATAR_STATES, BOT_SHAPES, BOT_TONES, blobatar, botAppearanceContrast, botAvatarSVG,
  resolveBotAppearance, validateBotLook, type BotLook,
} from "../index";

const ID = "6f1c2a90-bot";

describe("validateBotLook", () => {
  test("accepts a full canonical look and returns a copy", () => {
    const look: BotLook = {
      version: 1,
      base: {
        shape: "sun", expression: "happy", background: "squircle", hue: 359.5, tone: "ink",
        palette: { head: "#AABBCC", eye: "#000000", bg: "#FFFFFF" }, motion: false,
      },
      states: { error: { expression: "mad", palette: { head: "#FF0000" } }, thinking: {} },
    };
    const r = validateBotLook(look);
    expect(r).toEqual({ ok: true, look });
    if (r.ok) expect(r.look).not.toBe(look);
  });

  test.each([
    ["non-object", 3],
    ["array", []],
    ["wrong version", { version: 2, base: {} }],
    ["missing base", { version: 1 }],
    ["unknown top field", { version: 1, base: {}, extra: 1 }],
    ["unknown appearance field", { version: 1, base: { traits: {} } }],
    ["unknown state", { version: 1, base: {}, states: { sleeping: {} } }],
    ["unknown shape", { version: 1, base: { shape: "star" } }],
    ["unknown expression", { version: 1, base: { expression: "angry" } }],
    ["unknown background", { version: 1, base: { background: "rounded" } }],
    ["unknown tone", { version: 1, base: { tone: "dark" } }],
    ["hue 360", { version: 1, base: { hue: 360 } }],
    ["negative hue", { version: 1, base: { hue: -1 } }],
    ["NaN hue", { version: 1, base: { hue: Number.NaN } }],
    ["infinite hue", { version: 1, base: { hue: Infinity } }],
    ["string hue", { version: 1, base: { hue: "10" } }],
    ["lowercase hex", { version: 1, base: { palette: { head: "#aabbcc" } } }],
    ["short hex", { version: 1, base: { palette: { eye: "#ABC" } } }],
    ["css color", { version: 1, base: { palette: { bg: "red" } } }],
    ["unknown palette key", { version: 1, base: { palette: { cheek: "#FF0000" } } }],
    ["null field", { version: 1, base: { shape: null } }],
    ["null palette", { version: 1, base: { palette: null } }],
    ["non-boolean motion", { version: 1, base: { motion: 1 } }],
    ["null states", { version: 1, base: {}, states: null }],
    ["function", { version: 1, base: { shape: () => "round" } }],
  ])("rejects %s", (_name, input) => {
    const r = validateBotLook(input);
    expect(r.ok).toBe(false);
  });
});

describe("resolveBotAppearance", () => {
  test("missing look keeps the seeded default: transparent, idle, motion on", () => {
    const r = resolveBotAppearance(ID);
    expect(r.background).toBe("none");
    expect(r.expression).toBe("idle");
    expect(r.motion).toBe(true);
    expect(BOT_SHAPES).toContain(r.shape);
    expect(BOT_TONES).toContain(r.tone);
    expect(r.palette.head).toMatch(/^#[0-9A-F]{6}$/);
    expect(resolveBotAppearance(ID, null)).toEqual(r);
    expect(resolveBotAppearance(ID, { version: 1, base: {} })).toEqual(r);
  });

  test("every shape and tone resolves to itself", () => {
    for (const shape of BOT_SHAPES) expect(resolveBotAppearance(ID, { version: 1, base: { shape } }).shape).toBe(shape);
    for (const tone of BOT_TONES) expect(resolveBotAppearance(ID, { version: 1, base: { tone } }).tone).toBe(tone);
  });

  test("state overrides inherit omitted fields and deep-merge palette", () => {
    const look: BotLook = {
      version: 1,
      base: { shape: "boxy", hue: 120, palette: { head: "#112233", eye: "#FFFFFF" }, motion: false },
      states: { error: { expression: "mad", palette: { eye: "#EEEEEE" } } },
    };
    const err = resolveBotAppearance(ID, look, "error");
    expect(err).toMatchObject({ shape: "boxy", hue: 120, expression: "mad", motion: false });
    expect(err.palette.head).toBe("#112233");
    expect(err.palette.eye).toBe("#EEEEEE");
    const idle = resolveBotAppearance(ID, look, "idle");
    expect(idle.expression).toBe("idle");
    expect(idle.palette.eye).toBe("#FFFFFF");
    for (const s of BOT_AVATAR_STATES) expect(() => resolveBotAppearance(ID, look, s)).not.toThrow();
  });

  test("throws on an invalid look or state", () => {
    expect(() => resolveBotAppearance(ID, { version: 1, base: { hue: 400 } } as BotLook)).toThrow(TypeError);
    expect(() => resolveBotAppearance(ID, undefined, "sleeping" as never)).toThrow(TypeError);
  });

  test("seed is the bot id: edits never change identity traits they did not touch", () => {
    const a = resolveBotAppearance(ID);
    const b = resolveBotAppearance(ID, { version: 1, base: { expression: "happy", background: "circle" } });
    expect(b.shape).toBe(a.shape);
    expect(b.hue).toBe(a.hue);
    expect(b.palette).toEqual(a.palette);
  });

  test("contrast reports WCAG ratios", () => {
    const c = botAppearanceContrast(resolveBotAppearance(ID, {
      version: 1, base: { palette: { head: "#FFFFFF", eye: "#000000", bg: "#FFFFFF" } },
    }));
    expect(c.eyeOnHead).toBeCloseTo(21, 1);
    expect(c.headOnBg).toBeCloseTo(1, 5);
  });
});

describe("botAvatarSVG", () => {
  test("no look is byte-identical to the pinned portrait", () => {
    for (const id of [ID, "a", "lead-bot", "Ünïcødé"]) {
      expect(botAvatarSVG(id)).toBe(blobatar(id));
      expect(botAvatarSVG(id, null, "idle")).toBe(blobatar(id));
    }
  });

  test("background none is transparent; others draw a plate", () => {
    expect(botAvatarSVG(ID, { version: 1, base: { background: "none" } })).toBe(blobatar(ID));
    for (const background of ["square", "circle", "squircle"] as const) {
      expect(botAvatarSVG(ID, { version: 1, base: { background } })).toBe(blobatar(ID, { background }));
    }
  });

  test("size and title pass through", () => {
    const svg = botAvatarSVG(ID, null, undefined, { size: 48, title: "A <bot>" });
    expect(svg).toContain(`width="48"`);
    expect(svg).toContain("<title>A &lt;bot&gt;</title>");
  });
});
