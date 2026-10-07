/**
 * Numeric avatar frames: interpolation between two endpoint geometries and one
 * frame of the idle layer on top of it.
 *
 * Every function here is a `"worklet"` and calls only other worklets, so a
 * Reanimated UI-thread callback may call any of them. Nothing parses or emits
 * SVG per frame; `avatarPath` serializes for renderers that need a `d` string.
 * Endpoint geometry comes from `botAvatarGeometry` in `geometry.ts`.
 */
import type { IdleSeeds } from "./vendor/core/animate";
import type { Pose } from "./vendor/core/morph";
import { bobT, breatheT, eyesT, glanceT, idleFrame, rockT, rootT, type Mat } from "./vendor/react-native/worklets";

export type AvatarPoint = [number, number];
/** One cubic Bézier after the previous end point: c1x c1y c2x c2y x y. */
export type AvatarCubic = [number, number, number, number, number, number];
/** A closed contour. */
export interface AvatarCubicPath { start: AvatarPoint; segments: AvatarCubic[] }
export interface AvatarCircle { cx: number; cy: number; r: number }
/** A drawn (unposed) eye: a superellipse with `rot` in degrees. */
export interface AvatarEye { cx: number; cy: number; rx: number; ry: number; n: number; rot: number }
/** SVG `matrix(a b c d e f)`, also `react-native-svg`'s `matrix` prop and CGAffineTransform order. */
export type AvatarMatrix = Mat;

export interface AvatarGeometry {
  version: 1;
  background: { path: AvatarCubicPath; fill: string; opacity: number };
  /** 24 cubics, clockwise on screen, boundary k near angle −90° + 15°·k about the body centre. */
  core: AvatarCubicPath;
  /** 4 cubics: the droplet taper, otherwise collapsed onto the body centre. */
  extra: AvatarCubicPath;
  /** 9 slots; unused slots have r = 0 at the body centre. */
  petals: AvatarCircle[];
  eyes: [AvatarEye, AvatarEye];
  pose: Pose;
  palette: { head: string; eye: string; bg: string };
  motion: boolean;
  seeds: IdleSeeds;
}

export interface AvatarFrame {
  background: { path: AvatarCubicPath; fill: string; opacity: number };
  /** Applies to `core`, `extra` and `petals`. */
  body: AvatarMatrix;
  core: AvatarCubicPath;
  extra: AvatarCubicPath;
  petals: AvatarCircle[];
  head: string;
  eyes: [AvatarCubicPath, AvatarCubicPath];
  /** Full composite per eye, body transform included. */
  eyeMatrices: [AvatarMatrix, AvatarMatrix];
  eye: string;
}

function lerpPath(a: AvatarCubicPath, b: AvatarCubicPath, t: number): AvatarCubicPath {
  "worklet";
  if (a.segments.length !== b.segments.length) throw new Error("avatar paths have different segment counts");
  const segments: AvatarCubic[] = [];
  for (let i = 0; i < b.segments.length; i++) {
    const s = a.segments[i]!;
    const e = b.segments[i]!;
    segments.push([
      s[0] * (1 - t) + e[0] * t, s[1] * (1 - t) + e[1] * t, s[2] * (1 - t) + e[2] * t,
      s[3] * (1 - t) + e[3] * t, s[4] * (1 - t) + e[4] * t, s[5] * (1 - t) + e[5] * t,
    ]);
  }
  return { start: [a.start[0] * (1 - t) + b.start[0] * t, a.start[1] * (1 - t) + b.start[1] * t], segments };
}

/** sRGB byte interpolation, matching upstream `fadeHex` and CSS `transition: fill`. */
function lerpColor(a: string, b: string, t: number): string {
  "worklet";
  if (t <= 0) return a;
  if (t >= 1) return b;
  let out = "#";
  for (let i = 1; i < 7; i += 2) {
    const v = Math.round(parseInt(a.slice(i, i + 2), 16) * (1 - t) + parseInt(b.slice(i, i + 2), 16) * t);
    out += (v < 16 ? "0" : "") + v.toString(16).toUpperCase();
  }
  return out;
}

/**
 * The geometry `t` of the way from `from` to `to`. `t` is clamped to [0, 1];
 * 0 returns `from`'s numbers and 1 returns `to`'s exactly. To retarget an
 * interrupted morph, pass the geometry currently on screen as `from`.
 */
export function interpolateAvatarGeometry(from: AvatarGeometry, to: AvatarGeometry, t: number): AvatarGeometry {
  "worklet";
  const u = t <= 0 ? 0 : t >= 1 ? 1 : t;
  const m = (a: number, b: number) => a * (1 - u) + b * u;
  if (from.petals.length !== to.petals.length) throw new Error("avatar geometries have different petal counts");
  const pose = {} as Pose;
  for (const k in to.pose) {
    const c = k as keyof Pose;
    pose[c] = m(from.pose[c], to.pose[c]);
  }
  const eye = (i: number): AvatarEye => {
    const a = from.eyes[i]!;
    const b = to.eyes[i]!;
    return { cx: m(a.cx, b.cx), cy: m(a.cy, b.cy), rx: m(a.rx, b.rx), ry: m(a.ry, b.ry), n: m(a.n, b.n), rot: m(a.rot, b.rot) };
  };
  return {
    version: 1,
    background: {
      path: lerpPath(from.background.path, to.background.path, u),
      fill: lerpColor(from.background.fill, to.background.fill, u),
      opacity: m(from.background.opacity, to.background.opacity),
    },
    core: lerpPath(from.core, to.core, u),
    extra: lerpPath(from.extra, to.extra, u),
    petals: to.petals.map((p, i) => {
      const q = from.petals[i]!;
      return { cx: m(q.cx, p.cx), cy: m(q.cy, p.cy), r: m(q.r, p.r) };
    }),
    eyes: [eye(0), eye(1)],
    pose,
    palette: {
      head: lerpColor(from.palette.head, to.palette.head, u),
      eye: lerpColor(from.palette.eye, to.palette.eye, u),
      bg: lerpColor(from.palette.bg, to.palette.bg, u),
    },
    motion: u > 0 ? to.motion : from.motion,
    seeds: u > 0 ? to.seeds : from.seeds,
  };
}

/** CSS `cubic-bezier`, transcribed from upstream `ease.ts` because a worklet cannot call that closure. */
function cubicBezier(x: number, x1: number, y1: number, x2: number, y2: number): number {
  "worklet";
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  let t = x;
  for (let i = 0; i < 8; i++) {
    const err = ((ax * t + bx) * t + cx) * t - x;
    if (Math.abs(err) < 1e-5) break;
    const d = (3 * ax * t + 2 * bx) * t + cx;
    if (Math.abs(d) < 1e-6) break;
    t -= err / d;
  }
  return ((ay * t + by) * t + cy) * t;
}

/**
 * Eased morph progress on upstream's two clocks (`MORPH_IN`/`MORPH_OUT` in
 * `vendor/core/ease.ts`): 300 ms toward a pose that moves or tints, 400 ms
 * toward idle.
 */
export function avatarMorphProgress(elapsedMs: number, toward: AvatarGeometry): { t: number; done: boolean } {
  "worklet";
  const p = toward.pose;
  const expressive =
    p.esx !== 1 || p.esy !== 1 || p.tilt !== 0 || p.edy !== 0 || p.edx !== 0 || p.esx2 !== 0 || p.esy2 !== 0 ||
    p.tilt2 !== 0 || p.edy2 !== 0 || p.lock !== 0 || p.heat !== 0 || p.shake !== 0 || p.rock !== 0 || p.bdy !== 0;
  const x = elapsedMs / (expressive ? 300 : 400);
  if (!(x < 1)) return { t: 1, done: true };
  if (x <= 0) return { t: 0, done: false };
  return { t: expressive ? cubicBezier(x, 0.45, 0.05, 0.5, 1) : cubicBezier(x, 0.42, 0, 0.58, 1), done: false };
}

function mul(a: Mat, b: Mat): Mat {
  "worklet";
  return [
    a[0] * b[0] + a[2] * b[1],
    a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3],
    a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4],
    a[1] * b[4] + a[3] * b[5] + a[5],
  ];
}

/**
 * Upstream `poseTransforms` at `rockp: 1`, as a matrix:
 * translate(posed centre) rotate(tilt·wrap + lean·(1 − lock)) scale(x y) rotate(−lean) translate(−centre).
 * The seesaw is composed outside it by `rockT`, as upstream's animated adapter does.
 */
function poseMatrix(e: AvatarEye, p: Pose, i: number): Mat {
  "worklet";
  const wrap = i ? 1 : -1;
  const sel = i ? 1 : 0;
  const r = (((p.tilt + sel * p.tilt2) * wrap + e.rot * (1 - p.lock)) * Math.PI) / 180;
  const q = (-e.rot * Math.PI) / 180;
  const sx = p.esx + sel * p.esx2;
  const sy = p.esy + sel * p.esy2;
  const lin = mul(
    mul([Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0], [sx, 0, 0, sy, 0, 0]),
    [Math.cos(q), Math.sin(q), -Math.sin(q), Math.cos(q), 0, 0],
  );
  const tx = e.cx + p.edx * wrap;
  const ty = e.cy + p.edy + sel * p.edy2;
  return [lin[0], lin[1], lin[2], lin[3], tx - lin[0] * e.cx - lin[2] * e.cy, ty - lin[1] * e.cx - lin[3] * e.cy];
}

/** Upstream `superellipse` as four cubics, rounded to the 0.01 the static SVG emits. */
function eyePath(e: AvatarEye): AvatarCubicPath {
  "worklet";
  const k = Math.min(1, (8 * Math.pow(2, -1 / e.n) - 4) / 3);
  const a = e.rx;
  const b = e.ry;
  const ak = a * k;
  const bk = b * k;
  const pts = [
    a, 0,
    a, bk, ak, b, 0, b,
    -ak, b, -a, bk, -a, 0,
    -a, -bk, -ak, -b, 0, -b,
    ak, -b, a, -bk, a, 0,
  ];
  const t = (e.rot * Math.PI) / 180;
  const cos = Math.cos(t);
  const sin = Math.sin(t);
  const xy: number[] = [];
  for (let i = 0; i < pts.length; i += 2) {
    xy.push(Math.round((e.cx + pts[i]! * cos - pts[i + 1]! * sin) * 100) / 100);
    xy.push(Math.round((e.cy + pts[i]! * sin + pts[i + 1]! * cos) * 100) / 100);
  }
  const segments: AvatarCubic[] = [];
  for (let s = 0; s < 4; s++) {
    const o = 2 + s * 6;
    segments.push([xy[o]!, xy[o + 1]!, xy[o + 2]!, xy[o + 3]!, xy[o + 4]!, xy[o + 5]!]);
  }
  return { start: [xy[0]!, xy[1]!], segments };
}

/**
 * One frame at `timeMs` (any monotonic clock) and amplitude `amp` in [0, 1].
 * `amp` 0 is the static frame: no breathe, bob, glance, blink, tremor or
 * seesaw, matching `botAvatarSVG` at the endpoint. Use it for Reduce Motion,
 * `motion: false` and offscreen avatars.
 */
export function avatarFrame(g: AvatarGeometry, timeMs: number, amp: number): AvatarFrame {
  "worklet";
  const a = amp <= 0 ? 0 : amp >= 1 ? 1 : amp;
  const raw = idleFrame(g.seeds, timeMs, a, g.pose.shake * a);
  const f = { ...raw, rockp: 1 + a * (raw.rockp - 1) };
  const body = mul(rootT(f), mul(breatheT(f), bobT(f, g.pose.bdy)));
  const pair = mul(body, eyesT(f));
  const eyeMatrix = (i: number): Mat => {
    const e = g.eyes[i]!;
    const side = i ? 1 : -1;
    return mul(mul(mul(pair, rockT(f, g.pose, side)), poseMatrix(e, g.pose, i)), glanceT(f, e.cx, e.cy, e.rot, side));
  };
  return {
    background: g.background,
    body,
    core: g.core,
    extra: g.extra,
    petals: g.petals,
    head: g.palette.head,
    eyes: [eyePath(g.eyes[0]), eyePath(g.eyes[1])],
    eyeMatrices: [eyeMatrix(0), eyeMatrix(1)],
    eye: g.palette.eye,
  };
}

/** `M…C…Z` for a cubic path, numbers rounded to `digits` decimals. */
export function avatarPath(p: AvatarCubicPath, digits = 2): string {
  "worklet";
  const k = Math.pow(10, digits);
  const n = (v: number) => {
    const r = Math.round(v * k) / k;
    return r === 0 ? "0" : String(r);
  };
  let d = `M${n(p.start[0])} ${n(p.start[1])}`;
  for (const s of p.segments) d += `C${n(s[0])} ${n(s[1])} ${n(s[2])} ${n(s[3])} ${n(s[4])} ${n(s[5])}`;
  return d + "Z";
}
