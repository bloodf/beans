import { prepare, type BotAvatarState, type BotLook } from "./appearance";
import type { AvatarCircle, AvatarCubic, AvatarCubicPath, AvatarEye, AvatarGeometry, AvatarPoint } from "./frame";
import { motionSeeds } from "./vendor/core/animate";
import { blobatar } from "./vendor/core/blobatar";
import { IDENT } from "./vendor/core/morph";
import { backdrop, tinted } from "./vendor/core/render";
import { superellipse } from "./vendor/core/shape";
import { style } from "./vendor/core/styles/blob";
import { marks } from "./vendor/core/styles/compose";

/** Static SVG from the pinned generator. No look is byte-identical to `blobatar(botID)`. */
export function botAvatarSVG(
  botID: string,
  look?: BotLook | null,
  state?: BotAvatarState,
  options: { size?: number; title?: string } = {},
): string {
  const { opts } = prepare(botID, look, state);
  return blobatar(botID, { ...opts, size: options.size, title: options.title });
}

export const CORE_SEGMENTS = 24;
export const EXTRA_SEGMENTS = 4;
export const BACKGROUND_SEGMENTS = 8;
/** The most petals any silhouette draws: `sun` with `sun.n` up to 9. */
export const PETAL_SLOTS = 9;

type Pt = [number, number];
type Bez = [Pt, Pt, Pt, Pt];

/**
 * Reads the absolute `M C Q L H V Z` path data `vendor/core/shape.ts` emits as
 * cubics, exactly: a quadratic is degree-elevated and a line gets controls at
 * its thirds. Runs once per endpoint; frames never see path strings.
 */
export function pathCubics(d: string): Bez[] {
  const tokens = d.match(/[MCQLHVZ]|-?\d*\.?\d+(?:e[-+]?\d+)?/gi) ?? [];
  const out: Bez[] = [];
  let i = 0;
  let cur: Pt = [0, 0];
  let start: Pt = [0, 0];
  let cmd = "";
  const num = () => {
    const v = Number(tokens[i++]);
    if (!Number.isFinite(v)) throw new Error(`malformed path data: ${d}`);
    return v;
  };
  const line = (to: Pt) => {
    if (to[0] === cur[0] && to[1] === cur[1]) return;
    const dx = to[0] - cur[0];
    const dy = to[1] - cur[1];
    out.push([cur, [cur[0] + dx / 3, cur[1] + dy / 3], [cur[0] + (2 * dx) / 3, cur[1] + (2 * dy) / 3], to]);
    cur = to;
  };
  while (i < tokens.length) {
    if (/^[A-Za-z]$/.test(tokens[i]!)) cmd = tokens[i++]!;
    switch (cmd) {
      case "M":
        cur = start = [num(), num()];
        cmd = "L";
        break;
      case "C": {
        const c1: Pt = [num(), num()];
        const c2: Pt = [num(), num()];
        const to: Pt = [num(), num()];
        out.push([cur, c1, c2, to]);
        cur = to;
        break;
      }
      case "Q": {
        const q: Pt = [num(), num()];
        const to: Pt = [num(), num()];
        out.push([
          cur,
          [cur[0] + (2 / 3) * (q[0] - cur[0]), cur[1] + (2 / 3) * (q[1] - cur[1])],
          [to[0] + (2 / 3) * (q[0] - to[0]), to[1] + (2 / 3) * (q[1] - to[1])],
          to,
        ]);
        cur = to;
        break;
      }
      case "L":
        line([num(), num()]);
        break;
      case "H":
        line([num(), cur[1]]);
        break;
      case "V":
        line([cur[0], num()]);
        break;
      case "Z":
        line(start);
        cmd = "";
        break;
      default:
        throw new Error(`unsupported path command "${cmd}" in ${d}`);
    }
  }
  return out;
}

const at = (b: Bez, t: number): Pt => {
  const s = 1 - t;
  return [
    s * s * s * b[0][0] + 3 * s * s * t * b[1][0] + 3 * s * t * t * b[2][0] + t * t * t * b[3][0],
    s * s * s * b[0][1] + 3 * s * s * t * b[1][1] + 3 * s * t * t * b[2][1] + t * t * t * b[3][1],
  ];
};

/** The part of `b` between parameters `t0` and `t1`, by de Casteljau: the same curve, not an approximation. */
function slice(b: Bez, t0: number, t1: number): Bez {
  const split = (c: Bez, t: number): [Bez, Bez] => {
    const l = (p: Pt, q: Pt): Pt => [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
    const ab = l(c[0], c[1]);
    const bc = l(c[1], c[2]);
    const cd = l(c[2], c[3]);
    const abc = l(ab, bc);
    const bcd = l(bc, cd);
    const m = l(abc, bcd);
    return [[c[0], ab, abc, m], [m, bcd, cd, c[3]]];
  };
  const tail = t0 > 0 ? split(b, t0)[1] : b;
  if (t1 >= 1) return tail;
  return split(tail, (t1 - t0) / (1 - t0))[0];
}

/**
 * Exactly `count` cubics tracing the same closed outline, clockwise on screen,
 * starting from the piece whose start lies nearest straight up from `centre`.
 *
 * Every source segment boundary stays a boundary, so corners survive and the
 * outline is the source curve itself, cut by de Casteljau rather than refitted.
 * The remaining cuts go one at a time to whichever piece sweeps the widest
 * angle about `centre`, splitting it at half its sweep. Two silhouettes
 * normalized this way have piece k pointing roughly the same way, which is what
 * makes a pointwise interpolation between them a morph rather than a scramble.
 */
export function normalizeContour(src: Bez[], count: number, centre: Pt): AvatarCubicPath {
  if (src.length === 0 || src.length > count) throw new Error(`contour has ${src.length} segments; need 1..${count}`);
  let area = 0;
  for (const b of src) area += b[0][0] * b[3][1] - b[3][0] * b[0][1];
  // Positive shoelace area in y-down coordinates is clockwise on screen.
  const pieces: Bez[] =
    area >= 0 ? src.slice() : src.slice().reverse().map((b): Bez => [b[3], b[2], b[1], b[0]]);

  const angle = (p: Pt) => Math.atan2(p[1] - centre[1], p[0] - centre[0]);
  const sweep = (from: number, to: number) => {
    const d = to - from;
    return d - 2 * Math.PI * Math.floor(d / (2 * Math.PI));
  };
  // Sweep through the midpoint, so a short segment reads as short rather than
  // as a full turn when rounding puts its end a hair behind its start.
  const span = (b: Bez) => {
    const a0 = angle(b[0]);
    const s = sweep(a0, angle(b[3]));
    return s > Math.PI && sweep(a0, angle(at(b, 0.5))) > s ? 0 : s;
  };
  while (pieces.length < count) {
    let widest = 0;
    let most = -1;
    pieces.forEach((b, i) => {
      const s = span(b);
      if (s > most + 1e-12) {
        most = s;
        widest = i;
      }
    });
    const b = pieces[widest]!;
    let t = 0.5;
    if (most > 1e-9) {
      const a0 = angle(b[0]);
      let lo = 0;
      let hi = 1;
      for (let k = 0; k < 60; k++) {
        const mid = (lo + hi) / 2;
        if (sweep(a0, angle(at(b, mid))) < most / 2) lo = mid;
        else hi = mid;
      }
      t = (lo + hi) / 2;
    }
    pieces.splice(widest, 1, slice(b, 0, t), slice(b, t, 1));
  }

  let first = 0;
  let best = Infinity;
  pieces.forEach((p, i) => {
    const s = sweep(-Math.PI / 2, angle(p[0]));
    const dist = Math.min(s, 2 * Math.PI - s);
    if (dist < best - 1e-9) {
      best = dist;
      first = i;
    }
  });
  const ordered = [...pieces.slice(first), ...pieces.slice(0, first)];
  return {
    start: [ordered[0]![0][0], ordered[0]![0][1]],
    segments: ordered.map((p): AvatarCubic => [p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]]),
  };
}

const collapsed = (count: number, c: Pt): AvatarCubicPath => ({
  start: [c[0], c[1]],
  segments: Array.from({ length: count }, (): AvatarCubic => [c[0], c[1], c[0], c[1], c[0], c[1]]),
});

/** The endpoint geometry `botAvatarSVG` draws, as numbers a renderer can morph. */
export function botAvatarGeometry(botID: string, look?: BotLook | null, state?: BotAvatarState): AvatarGeometry {
  const { opts, t, palette, layout, resolved, expression } = prepare(botID, look, state);
  const p = tinted(palette, expression);
  const centre: Pt = [layout.body.cx, layout.body.cy];
  const drawn = marks(layout, p);

  const petals: AvatarCircle[] = [];
  for (const m of drawn) if (m.kind === "circle") petals.push({ cx: m.cx, cy: m.cy, r: m.r });
  if (petals.length > PETAL_SLOTS) throw new Error(`${petals.length} petals exceed ${PETAL_SLOTS} slots`);
  while (petals.length < PETAL_SLOTS) petals.push({ cx: centre[0], cy: centre[1], r: 0 });

  const coreD = layout.draw ? layout.draw(layout.body) : superellipse(layout.body);
  const extraD = layout.extra[0];
  if (layout.extra.length > 1) throw new Error("more than one extra outline");

  const bg = backdrop(style, opts, p);
  const bgD = bg?.d ?? superellipse({ cx: 50, cy: 50, rx: 50, ry: 50, n: 2 });

  const eye = (i: number): AvatarEye => {
    const e = layout.eyes[i]!;
    return { cx: e.cx, cy: e.cy, rx: e.rx, ry: e.ry, n: e.n, rot: e.rot };
  };
  const head = p.head!.toUpperCase();
  return {
    version: 1,
    background: {
      path: normalizeContour(pathCubics(bgD), BACKGROUND_SEGMENTS, [50, 50]),
      fill: resolved.palette.bg,
      opacity: bg ? 1 : 0,
    },
    core: normalizeContour(pathCubics(coreD), CORE_SEGMENTS, centre),
    extra: extraD ? normalizeContour(pathCubics(extraD), EXTRA_SEGMENTS, centre) : collapsed(EXTRA_SEGMENTS, centre),
    petals,
    eyes: [eye(0), eye(1)],
    pose: { ...(expression?.p ?? IDENT) },
    palette: { head, eye: p.eye!.toUpperCase(), bg: resolved.palette.bg },
    motion: resolved.motion,
    seeds: motionSeeds(t),
  };
}

export type { AvatarPoint };
