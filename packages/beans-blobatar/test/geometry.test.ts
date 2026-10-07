import { describe, expect, test } from "bun:test";
import {
  BOT_BACKGROUNDS, BOT_EXPRESSIONS, BOT_SHAPES, avatarFrame, avatarMorphProgress, avatarPath,
  botAvatarGeometry, interpolateAvatarGeometry, type AvatarCubicPath, type AvatarGeometry,
  type AvatarMatrix, type BotLook,
} from "../index";
import { CORE_SEGMENTS, pathCubics } from "../geometry";
import { bakePose } from "../vendor/core/morph";
import { superellipse } from "../vendor/core/shape";
import { _layout, _marks } from "../vendor/core/blobatar";
import { prepare } from "../appearance";

type Pt = [number, number];
const SEEDS = ["6f1c2a90-bot", "lead", "zz-41", "c0ffee", "Ünïcødé", "researcher-7", "b", "9d3e"];

const at = (p0: Pt, c1: Pt, c2: Pt, p1: Pt, t: number): Pt => {
  const s = 1 - t;
  return [
    s * s * s * p0[0] + 3 * s * s * t * c1[0] + 3 * s * t * t * c2[0] + t * t * t * p1[0],
    s * s * s * p0[1] + 3 * s * s * t * c1[1] + 3 * s * t * t * c2[1] + t * t * t * p1[1],
  ];
};
function sample(p: AvatarCubicPath, per = 16, m: AvatarMatrix = [1, 0, 0, 1, 0, 0]): Pt[] {
  const out: Pt[] = [];
  let cur: Pt = p.start;
  for (const s of p.segments) {
    for (let k = 0; k < per; k++) out.push(at(cur, [s[0], s[1]], [s[2], s[3]], [s[4], s[5]], k / per));
    cur = [s[4], s[5]];
  }
  return out.map(([x, y]) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
}
function sampleD(d: string, per = 64): Pt[] {
  const out: Pt[] = [];
  for (const b of pathCubics(d)) for (let k = 0; k <= per; k++) out.push(at(b[0], b[1], b[2], b[3], k / per));
  return out;
}
/** Distance from `p` to the closed polyline `poly`. */
const toPolyline = (p: Pt, poly: Pt[]) => {
  let best = Infinity;
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i]!;
    const b = poly[(i + 1) % poly.length]!;
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const len = dx * dx + dy * dy;
    const t = len > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len)) : 0;
    best = Math.min(best, Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy));
  }
  return best;
};
/** Symmetric Hausdorff distance between two densely sampled outlines. */
const hausdorff = (a: Pt[], b: Pt[]) =>
  Math.max(Math.max(...a.map(p => toPolyline(p, b))), Math.max(...b.map(p => toPolyline(p, a))));

function inside(poly: Pt[], [x, y]: Pt): boolean {
  let c = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i]!;
    const [xj, yj] = poly[j]!;
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) c = !c;
  }
  return c;
}
function selfIntersects(poly: Pt[]): boolean {
  const n = poly.length;
  const cross = (o: Pt, a: Pt, b: Pt) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  for (let i = 0; i < n; i++) {
    const a = poly[i]!;
    const b = poly[(i + 1) % n]!;
    for (let j = i + 2; j < n; j++) {
      if (i === 0 && j === n - 1) continue;
      const c = poly[j]!;
      const d = poly[(j + 1) % n]!;
      const d1 = cross(c, d, a);
      const d2 = cross(c, d, b);
      const d3 = cross(a, b, c);
      const d4 = cross(a, b, d);
      if (((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) && ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9))) return true;
    }
  }
  return false;
}
/** Inside the silhouette: core, taper, or any petal, under the body transform. */
function inSilhouette(g: AvatarGeometry, body: AvatarMatrix, p: Pt): boolean {
  const [x, y] = [p[0] - body[4], p[1] - body[5]];
  if (inside(sample(g.core), [x, y]) || inside(sample(g.extra), [x, y])) return true;
  return g.petals.some(c => Math.hypot(x - c.cx, y - c.cy) <= c.r);
}
const look = (base: BotLook["base"]): BotLook => ({ version: 1, base });

describe("endpoint geometry", () => {
  test("fixed topology for every shape, expression and background", () => {
    for (const shape of BOT_SHAPES) {
      for (const expression of BOT_EXPRESSIONS) {
        for (const background of BOT_BACKGROUNDS) {
          const g = botAvatarGeometry(SEEDS[0]!, look({ shape, expression, background }));
          expect(g.core.segments).toHaveLength(CORE_SEGMENTS);
          expect(g.extra.segments).toHaveLength(4);
          expect(g.background.path.segments).toHaveLength(8);
          expect(g.petals).toHaveLength(9);
          expect(g.background.opacity).toBe(background === "none" ? 0 : 1);
          expect(JSON.parse(JSON.stringify(g))).toEqual(g);
        }
      }
    }
  });

  test("normalized core traces exactly the static silhouette path", () => {
    for (const id of SEEDS) {
      for (const shape of BOT_SHAPES) {
        const { layout } = prepare(id, look({ shape }));
        const d = layout.draw ? layout.draw(layout.body) : superellipse(layout.body);
        const g = botAvatarGeometry(id, look({ shape }));
        expect(hausdorff(sample(g.core, 64), sampleD(d))).toBeLessThan(0.05);
        if (layout.extra[0]) expect(hausdorff(sample(g.extra, 64), sampleD(layout.extra[0]))).toBeLessThan(0.05);
      }
    }
  });

  test("decorations: petals, taper and backdrop match the static marks", () => {
    for (const shape of BOT_SHAPES) {
      for (const background of BOT_BACKGROUNDS) {
        const l = look({ shape, background });
        const { opts } = prepare(SEEDS[1]!, l);
        const m = _marks(SEEDS[1]!, opts);
        const g = botAvatarGeometry(SEEDS[1]!, l);
        const circles = m.marks.filter(k => k.kind === "circle");
        expect(g.petals.filter(p => p.r > 0)).toEqual(circles.map(c => ({ cx: c.cx, cy: c.cy, r: c.r })));
        if (m.bg) expect(hausdorff(sample(g.background.path, 64), sampleD(m.bg.d))).toBeLessThan(0.05);
        else expect(g.background.opacity).toBe(0);
      }
    }
  });

  test("amp 0 frame reproduces the static posed eyes and body offset", () => {
    for (const shape of BOT_SHAPES) {
      for (const expression of BOT_EXPRESSIONS) {
        const l = look({ shape, expression });
        const { opts } = prepare(SEEDS[2]!, l);
        const posed = _layout(SEEDS[2]!, opts);
        const g = botAvatarGeometry(SEEDS[2]!, l);
        const f = avatarFrame(g, 1234.5, 0);
        expect(f.body).toEqual([1, 0, 0, 1, 0, Math.round(g.pose.bdy * 1000) / 1000]);
        posed.eyes.forEach((e, i) => {
          const want = sampleD(superellipse(e)).map(([x, y]): Pt => [x, y + g.pose.bdy]);
          expect(hausdorff(sample(f.eyes[i]!, 64, f.eyeMatrices[i]!), want)).toBeLessThan(0.08);
        });
        expect(f.head).toBe(g.palette.head);
        expect(f.eye).toBe(g.palette.eye);
      }
    }
  });

  test("tinting expressions carry the tinted fills the static SVG paints", () => {
    const g = botAvatarGeometry(SEEDS[0]!, look({ expression: "mad" }));
    const { opts } = prepare(SEEDS[0]!, look({ expression: "mad" }));
    const p = _layout(SEEDS[0]!, opts).palette;
    expect(g.palette.head).toBe(p.head!.toUpperCase());
    expect(g.palette.eye).toBe(p.eye!.toUpperCase());
  });

  test("baked pose and frame composition agree with upstream bakePose", () => {
    const g = botAvatarGeometry(SEEDS[0]!, look({ expression: "wink" }));
    const baked = bakePose({ eyes: g.eyes }, g.pose).l.eyes;
    const f = avatarFrame(g, 0, 0);
    const c = sample(f.eyes[1], 4, f.eyeMatrices[1]).reduce((a, p) => [a[0] + p[0] / 16, a[1] + p[1] / 16], [0, 0]);
    expect(c[0]).toBeCloseTo(baked[1]!.cx, 1);
    expect(c[1]).toBeCloseTo(baked[1]!.cy + g.pose.bdy, 1);
  });
});

describe("morph", () => {
  const geom = (id: string, shape: (typeof BOT_SHAPES)[number], expression: (typeof BOT_EXPRESSIONS)[number] = "idle") =>
    botAvatarGeometry(id, look({ shape, expression }));

  test.each(SEEDS)("all 100 ordered shape pairs (%s): exact endpoints, simple outlines, eyes inside", id => {
    const failures: string[] = [];
    {
      for (const a of BOT_SHAPES) {
        for (const b of BOT_SHAPES) {
          const from = geom(id, a);
          const to = geom(id, b);
          expect(interpolateAvatarGeometry(from, to, 0)).toEqual(from);
          expect(interpolateAvatarGeometry(from, to, 1)).toEqual(to);
          for (const t of [0.1, 0.25, 0.5, 0.75, 0.9]) {
            const g = interpolateAvatarGeometry(from, to, t);
            const core = sample(g.core, 8);
            if (selfIntersects(core)) failures.push(`${id} ${a}->${b} t=${t}: core self-intersects`);
            for (const [x, y] of core) if (x < -2 || x > 102 || y < -2 || y > 102) failures.push(`${id} ${a}->${b} t=${t}: core leaves frame`);
            const f = avatarFrame(g, 0, 0);
            for (let i = 0; i < 2; i++) {
              for (const p of sample(f.eyes[i as 0 | 1], 4, f.eyeMatrices[i as 0 | 1])) {
                if (!inSilhouette(g, f.body, p)) {
                  failures.push(`${id} ${a}->${b} t=${t}: eye ${i} outside silhouette`);
                  break;
                }
              }
            }
          }
        }
      }
    }
    expect(failures).toEqual([]);
  }, 30_000);

  test("expression morph between every pair stays finite and lands exactly", () => {
    const id = SEEDS[0]!;
    for (const a of BOT_EXPRESSIONS) {
      for (const b of BOT_EXPRESSIONS) {
        const from = geom(id, "round", a);
        const to = geom(id, "round", b);
        const mid = interpolateAvatarGeometry(from, to, 0.5);
        for (const v of Object.values(mid.pose)) expect(Number.isFinite(v)).toBe(true);
        expect(interpolateAvatarGeometry(from, to, 1)).toEqual(to);
      }
    }
  });

  test("interrupted retarget starts from what is on screen", () => {
    const a = geom(SEEDS[1]!, "triangle");
    const b = geom(SEEDS[1]!, "sun", "happy");
    const c = geom(SEEDS[1]!, "capsule", "sad");
    const shown = interpolateAvatarGeometry(a, b, 0.4);
    const restart = interpolateAvatarGeometry(shown, c, 0);
    expect(restart).toEqual(shown);
    expect(interpolateAvatarGeometry(shown, c, 1)).toEqual(c);
    const step = interpolateAvatarGeometry(shown, c, 0.01);
    const jump = Math.max(...step.core.segments.flat().map((v, i) => Math.abs(v - shown.core.segments.flat()[i]!)));
    expect(jump).toBeLessThan(2);
  });

  test("palette and background fade numerically", () => {
    const a = botAvatarGeometry(SEEDS[0]!, look({ palette: { head: "#000000" }, background: "none" }));
    const b = botAvatarGeometry(SEEDS[0]!, look({ palette: { head: "#FFFFFF" }, background: "square" }));
    const m = interpolateAvatarGeometry(a, b, 0.5);
    expect(m.palette.head).toBe("#808080");
    expect(m.background.opacity).toBe(0.5);
    expect(interpolateAvatarGeometry(a, b, -1)).toEqual(a);
    expect(interpolateAvatarGeometry(a, b, 2)).toEqual(b);
  });

  test("morph clocks: 300ms toward an expression, 400ms toward idle", () => {
    const happy = geom(SEEDS[0]!, "round", "happy");
    const idle = geom(SEEDS[0]!, "round");
    expect(avatarMorphProgress(0, happy)).toEqual({ t: 0, done: false });
    expect(avatarMorphProgress(299, happy).done).toBe(false);
    expect(avatarMorphProgress(300, happy)).toEqual({ t: 1, done: true });
    expect(avatarMorphProgress(350, idle).done).toBe(false);
    expect(avatarMorphProgress(400, idle)).toEqual({ t: 1, done: true });
    const mid = avatarMorphProgress(200, idle).t;
    expect(mid).toBeGreaterThan(0.4);
    expect(mid).toBeLessThan(0.6);
  });
});

describe("frame", () => {
  test("amp 0 is static across time, including tremor and seesaw poses", () => {
    for (const expression of ["mad", "thinking", "scared", "sick"] as const) {
      const g = botAvatarGeometry(SEEDS[0]!, look({ expression }));
      const ref = avatarFrame(g, 0, 0);
      for (const t of [17, 333, 901, 5000, 123456]) expect(avatarFrame(g, t, 0)).toEqual(ref);
    }
  });

  test("amp 1 moves body and eyes over time and stays bounded", () => {
    const g = botAvatarGeometry(SEEDS[0]!, look({ expression: "thinking" }));
    const frames = Array.from({ length: 200 }, (_, i) => avatarFrame(g, i * 50, 1));
    const bodies = new Set(frames.map(f => f.body.join()));
    const eyes = new Set(frames.map(f => f.eyeMatrices[0].join()));
    expect(bodies.size).toBeGreaterThan(10);
    expect(eyes.size).toBeGreaterThan(10);
    for (const f of frames) {
      expect(Math.abs(f.body[5] - g.pose.bdy)).toBeLessThan(3);
      for (const v of f.eyeMatrices[1]) expect(Number.isFinite(v)).toBe(true);
    }
  });

  test("avatarPath serializes the cubic path", () => {
    const p: AvatarCubicPath = { start: [1, 2.004], segments: [[3, 4, 5, 6, 7, -0.001]] };
    expect(avatarPath(p)).toBe("M1 2C3 4 5 6 7 0Z");
    expect(avatarPath(p, 3)).toBe("M1 2.004C3 4 5 6 7 -0.001Z");
  });
});
