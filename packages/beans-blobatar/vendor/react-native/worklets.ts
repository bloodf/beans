/**
 * The idle layer again, on the UI thread.
 *
 * ## This file is a copy, and that is the thing to know about it
 *
 * `blobatar/idle` is the original. Everything here is transcribed from it,
 * because a worklet cannot call it: `react-native-worklets` runs a function on
 * the UI thread by capturing its source and its closed-over values, and a
 * function imported from another module is neither. So the loops that draw a
 * blobatar sixty times a second have to exist twice, which is exactly the
 * failure ADR-0009 is written to prevent everywhere else in this repo.
 *
 * It is here anyway, for a reason that is about the product rather than about
 * the code: blobatar's main use is a sidebar of agents, all animating at once,
 * and a React render per blobatar per frame on the JS thread is the cost that
 * makes that stutter. Worklets are the only way to move it off, and shipping
 * them is the only way for a *library* to offer them.
 *
 * ## What keeps the copy honest
 *
 * `packages/harness/test/react-native-worklets.test.ts` runs both versions over
 * a wide sweep of seeds, times and amplitudes and asserts they agree exactly.
 * That is not as good as having one implementation, and it is much better than
 * a comment asking people to keep two in step: a transcription error is a
 * failing test rather than a device somebody has to notice.
 *
 * The worklets are plain functions with a directive, so that test can call them
 * on the JS thread like any other code. Nothing here needs a device to check.
 *
 * ## What is deliberately NOT copied
 *
 * The pose composition. `poseTransforms` stays in core and runs in JavaScript,
 * because the pose only changes when the consumer changes the expression, and a
 * morph is a one-shot 300ms transition rather than a loop. It is also by far
 * the subtlest arithmetic in the library, the place two shipped bugs lived, and
 * the thing `probe-compose.ts` measures against a real browser. Copying that
 * into a worklet to save eighteen renders would trade the risk in the wrong
 * direction.
 *
 * What the UI thread does instead is compose *around* it: the seesaw is an
 * extra translate outside the pose's own, which commutes with it exactly
 * because the pose's outermost term is a translate too.
 */

import type { IdleFrame, IdleSeeds } from "../core/idle";
import type { Pose } from "../core/internal";

/** `bezier`, from `src/ease.ts` in core, inlined because a worklet must be. */
function solve(x: number, x1: number, y1: number, x2: number, y2: number): number {
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

const easeInOut = (x: number) => {
  "worklet";
  return solve(x, 0.42, 0, 0.58, 1);
};
const easeIn = (x: number) => {
  "worklet";
  return solve(x, 0.42, 0, 1, 1);
};
const easeOutW = (x: number) => {
  "worklet";
  return solve(x, 0, 0, 0.58, 1);
};

/** The keyframe tables, transcribed from `src/idle.ts`. */
const SACCADE = [
  [0, 0, 0],
  [0.15, 0, 0],
  [0.165, -0.8, -0.9],
  [0.31, -0.8, -0.9],
  [0.325, 1, 0.1],
  [0.47, 1, 0.1],
  [0.485, -0.15, 0.85],
  [0.63, -0.15, 0.85],
  [0.645, 0.75, -0.8],
  [0.79, 0.75, -0.8],
  [0.805, -1, -0.15],
  [0.985, -1, -0.15],
  [1, 0, 0],
];

const WRAP = [
  [0, 0, 0, 0, 0],
  [0.15, 0, 0, 0, 0],
  [0.165, -0.0176, 0.008, -0.027, 0.648],
  [0.31, -0.0176, 0.008, -0.027, 0.648],
  [0.325, -0.022, -0.01, -0.003, 0.09],
  [0.47, -0.022, -0.01, -0.003, 0.09],
  [0.485, -0.0033, 0.0015, -0.0255, -0.115],
  [0.63, -0.0033, 0.0015, -0.0255, -0.115],
  [0.645, -0.0165, -0.0075, -0.024, -0.54],
  [0.79, -0.0165, -0.0075, -0.024, -0.54],
  [0.805, -0.022, 0.01, -0.0045, 0.135],
  [0.985, -0.022, 0.01, -0.0045, 0.135],
  [1, 0, 0, 0, 0],
];

const SHAKE = [
  [0, 0.62, -0.34],
  [0.25, -0.7, 0.22],
  [0.5, 0.38, 0.66],
  [0.75, -0.44, -0.6],
  [1, 0.62, -0.34],
];

function stops(u: number, table: number[][], col: number): number {
  "worklet";
  for (let i = table.length - 1; i >= 0; i--) {
    const row = table[i]!;
    if (u < row[0]!) continue;
    const next = table[i + 1];
    if (!next) return row[col]!;
    const span = next[0]! - row[0]!;
    return span <= 0
      ? row[col]!
      : row[col]! + (next[col]! - row[col]!) * ((u - row[0]!) / span);
  }
  return table[0]![col]!;
}

/**
 * `idleAt`, on the UI thread. See `src/idle.ts` for what any of it means; this
 * is a transcription and carries none of the reasoning, on purpose, so there is
 * one place to read it and one place to change it first.
 */
export function idleFrame(
  s: IdleSeeds,
  t: number,
  amp: number,
  shake: number,
): IdleFrame {
  "worklet";
  const cyc = (phase: number, period: number) => {
    const u = (t + phase) / period;
    return u - Math.floor(u);
  };
  const alt = (phase: number, period: number) => {
    const u = (t + phase) / period;
    const n = Math.floor(u);
    const f = u - n;
    return n % 2 ? 1 - f : f;
  };

  const breathe = easeInOut(alt(s.phase, 2800));
  const bob = easeInOut(alt(s.bob, 3400));
  const sac = cyc(s.saccadePhase, s.saccade);
  const sh = cyc(0, 112);

  const r = cyc(0, 900);
  const rockp = r < 0.5 ? 1 - 2 * easeInOut(r * 2) : -1 + 2 * easeInOut(r * 2 - 1);

  const b = cyc(s.blinkPhase, s.blink);
  const blink =
    b < 0.972
      ? 1
      : b < 0.986
        ? 1 - 0.92 * amp * easeIn((b - 0.972) / 0.014)
        : 1 - 0.92 * amp * (1 - easeOutW((b - 0.986) / 0.014));

  return {
    shake: [stops(sh, SHAKE, 1) * shake, stops(sh, SHAKE, 2) * shake],
    breathe: [1 + 0.022 * amp * breathe, 1 - 0.018 * amp * breathe],
    bob: -1.1 * amp * bob,
    saccade: [
      stops(sac, SACCADE, 1) * s.lookX * amp,
      stops(sac, SACCADE, 2) * s.lookY * amp,
    ],
    rockp,
    blink,
    wrap: {
      mx: stops(sac, WRAP, 1) * s.lookMX * amp,
      side: stops(sac, WRAP, 2) * s.lookX * amp,
      sy: stops(sac, WRAP, 3) * s.lookMY * amp,
      rot: stops(sac, WRAP, 4) * s.lookX * s.lookY * amp,
    },
  };
}

/**
 * ## Why these return a matrix and not a transform string
 *
 * Every other renderer in this repo writes `transform="translate(...)"`, and so
 * did this file, and on a device it did nothing at all.
 *
 * `react-native-svg` parses `transform` in **JavaScript**: `G.render` runs the
 * string through `extractTransform` and hands the native view a `matrix` prop,
 * which is the only transform `RNSVGGroup` actually has. That parse happens
 * during a React render. A worklet does not render: `updateProps` writes the
 * props it is given straight onto the shadow node from the UI thread, so a
 * `transform` string arrives at a native component that has no such prop and is
 * dropped. Nothing throws, nothing warns, and the blobatar renders its first
 * frame and then sits perfectly still while every loop below runs correctly
 * sixty times a second into nowhere.
 *
 * So the loops compose the matrix themselves and write the prop the native view
 * really has. `packages/harness/test/react-native-worklets.test.ts` still
 * checks them against the strings core composes, by parsing those strings: the
 * transform being described has not changed, only the spelling that survives
 * the trip across the thread.
 *
 * The pose transform in `poseTransforms` stays a string, and correctly so. It
 * is rendered by React on the JS thread like any other prop, which is exactly
 * the path that does the parsing.
 */

/** An affine, in `react-native-svg`'s `matrix` order: `[a, b, c, d, e, f]`. */
export type Mat = [number, number, number, number, number, number];

/**
 * `r3`, as a number.
 *
 * The rounding is kept even though nothing is being printed any more, because
 * the point of it was never the string: core rounds every transform component
 * to three decimals before the browser sees it, and a UI thread that composed
 * from unrounded values would be drawing a subtly different picture from every
 * other renderer. The harness compares the two, so it would also fail.
 */
function n3(v: number): number {
  "worklet";
  return Math.round(v * 1000) / 1000;
}

/** `.mo-root`: the tremor. */
export function rootT(f: IdleFrame): Mat {
  "worklet";
  return [1, 0, 0, 1, n3(f.shake[0]), n3(f.shake[1])];
}

/** `.mo-breathe`, about the viewBox centre. */
export function breatheT(f: IdleFrame): Mat {
  "worklet";
  const sx = n3(f.breathe[0]);
  const sy = n3(f.breathe[1]);
  // `translate(50 50) scale(sx sy) translate(-50 -50)`, multiplied out.
  return [sx, 0, 0, sy, 50 - 50 * sx, 50 - 50 * sy];
}

/** `.mo-bob`, carrying the pose's own lift as well. */
export function bobT(f: IdleFrame, bdy: number): Mat {
  "worklet";
  return [1, 0, 0, 1, 0, n3(bdy + f.bob)];
}

/** `.mo-eyes`: the glance, which belongs to the pair. */
export function eyesT(f: IdleFrame): Mat {
  "worklet";
  return [1, 0, 0, 1, n3(f.saccade[0]), n3(f.saccade[1])];
}

/**
 * The seesaw, as an offset *outside* the pose's own transform rather than
 * folded into it.
 *
 * `poseTransforms` is computed in JavaScript at `rockp: 1`, which is the
 * extreme the still renderer bakes. What the loop adds is the difference
 * between that and the current phase, which works out to
 * `edy2 · rock · side · (rockp − 1) / 2` and is a pure vertical translate. It
 * composes exactly, because the pose transform's outermost term is a translate
 * too and translations commute with each other.
 *
 * That is the whole reason the pose composition did not need copying into a
 * worklet, and it is worth not undoing.
 */
export function rockT(f: IdleFrame, p: Pose, side: number): Mat {
  "worklet";
  return [1, 0, 0, 1, 0, n3((p.edy2 * p.rock * side * (f.rockp - 1)) / 2)];
}

/**
 * `.mo-eye > *`: the blink and the glance's foreshortening, both about the
 * eye's own drawn centre.
 *
 * Two scales bracketed by two different rotations, and neither bracket is
 * optional. The blink closes the capsule across its own width, so it is
 * bracketed by the seeded lean. The foreshortening is a screen-space effect, so
 * it is not.
 *
 * The seven terms are multiplied out here rather than composed through a pair
 * of matrix helpers, and that is a size decision rather than a taste one. A
 * worklet carries its own source across the bridge, so a helper two worklets
 * call is not shared the way a function normally is. It is serialized into
 * each of them. `mul` and `rot` cost more in this bundle than the algebra they
 * were hiding, and `@blobatar/react-native/animated` has a 10 kB budget it was
 * already sitting at 99% of.
 *
 * The lean bracket collapses first: `rotate(l) · scale(1 bl) · rotate(-l)` is
 * symmetric, and works out to `[cos²l + sin²l·bl, k, k, sin²l + cos²l·bl]` with
 * `k = sin l · cos l · (1 − bl)`. The outer rotation and the foreshortening
 * scale multiply into it, and the two translates fall out as the offset that
 * keeps the whole thing about the eye's own centre.
 */
export function glanceT(
  f: IdleFrame,
  cx: number,
  cy: number,
  lean: number,
  side: number,
): Mat {
  "worklet";
  const sx = n3(1 + f.wrap.mx + f.wrap.side * side);
  const sy = n3(1 + f.wrap.sy);
  const bl = n3(f.blink);

  const r = (n3(f.wrap.rot * side) * Math.PI) / 180;
  const cr = Math.cos(r);
  const sr = Math.sin(r);

  const q = (n3(lean) * Math.PI) / 180;
  const cq = Math.cos(q);
  const sq = Math.sin(q);
  const k = sq * cq * (1 - bl);
  const ba = cq * cq + sq * sq * bl;
  const bd = sq * sq + cq * cq * bl;

  // `rotate(rot) · scale(sx sy)`, then that against the bracket above.
  const a1 = cr * sx;
  const b1 = sr * sx;
  const c1 = -sr * sy;
  const d1 = cr * sy;
  const a = a1 * ba + c1 * k;
  const b = b1 * ba + d1 * k;
  const c = a1 * k + c1 * bd;
  const d = b1 * k + d1 * bd;

  // Rounded the way the printed form rounded them, which is not the same as
  // negating the rounded centre: `r3(-cx)` is what core writes.
  const ix = n3(-cx);
  const iy = n3(-cy);
  return [a, b, c, d, n3(cx) + a * ix + c * iy, n3(cy) + b * ix + d * iy];
}
