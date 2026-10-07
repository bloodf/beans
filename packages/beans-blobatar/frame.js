// vendor/react-native/worklets.ts
function solve(x, x1, y1, x2, y2) {
  "worklet";
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  let t = x;
  for (let i = 0;i < 8; i++) {
    const err = ((ax * t + bx) * t + cx) * t - x;
    if (Math.abs(err) < 0.00001)
      break;
    const d = (3 * ax * t + 2 * bx) * t + cx;
    if (Math.abs(d) < 0.000001)
      break;
    t -= err / d;
  }
  return ((ay * t + by) * t + cy) * t;
}
var easeInOut = (x) => {
  "worklet";
  return solve(x, 0.42, 0, 0.58, 1);
};
var easeIn = (x) => {
  "worklet";
  return solve(x, 0.42, 0, 1, 1);
};
var easeOutW = (x) => {
  "worklet";
  return solve(x, 0, 0, 0.58, 1);
};
var SACCADE = [
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
  [1, 0, 0]
];
var WRAP = [
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
  [1, 0, 0, 0, 0]
];
var SHAKE = [
  [0, 0.62, -0.34],
  [0.25, -0.7, 0.22],
  [0.5, 0.38, 0.66],
  [0.75, -0.44, -0.6],
  [1, 0.62, -0.34]
];
function stops(u, table, col) {
  "worklet";
  for (let i = table.length - 1;i >= 0; i--) {
    const row = table[i];
    if (u < row[0])
      continue;
    const next = table[i + 1];
    if (!next)
      return row[col];
    const span = next[0] - row[0];
    return span <= 0 ? row[col] : row[col] + (next[col] - row[col]) * ((u - row[0]) / span);
  }
  return table[0][col];
}
function idleFrame(s, t, amp, shake) {
  "worklet";
  const cyc = (phase, period) => {
    const u = (t + phase) / period;
    return u - Math.floor(u);
  };
  const alt = (phase, period) => {
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
  const blink = b < 0.972 ? 1 : b < 0.986 ? 1 - 0.92 * amp * easeIn((b - 0.972) / 0.014) : 1 - 0.92 * amp * (1 - easeOutW((b - 0.986) / 0.014));
  return {
    shake: [stops(sh, SHAKE, 1) * shake, stops(sh, SHAKE, 2) * shake],
    breathe: [1 + 0.022 * amp * breathe, 1 - 0.018 * amp * breathe],
    bob: -1.1 * amp * bob,
    saccade: [
      stops(sac, SACCADE, 1) * s.lookX * amp,
      stops(sac, SACCADE, 2) * s.lookY * amp
    ],
    rockp,
    blink,
    wrap: {
      mx: stops(sac, WRAP, 1) * s.lookMX * amp,
      side: stops(sac, WRAP, 2) * s.lookX * amp,
      sy: stops(sac, WRAP, 3) * s.lookMY * amp,
      rot: stops(sac, WRAP, 4) * s.lookX * s.lookY * amp
    }
  };
}
function n3(v) {
  "worklet";
  return Math.round(v * 1000) / 1000;
}
function rootT(f) {
  "worklet";
  return [1, 0, 0, 1, n3(f.shake[0]), n3(f.shake[1])];
}
function breatheT(f) {
  "worklet";
  const sx = n3(f.breathe[0]);
  const sy = n3(f.breathe[1]);
  return [sx, 0, 0, sy, 50 - 50 * sx, 50 - 50 * sy];
}
function bobT(f, bdy) {
  "worklet";
  return [1, 0, 0, 1, 0, n3(bdy + f.bob)];
}
function eyesT(f) {
  "worklet";
  return [1, 0, 0, 1, n3(f.saccade[0]), n3(f.saccade[1])];
}
function rockT(f, p, side) {
  "worklet";
  return [1, 0, 0, 1, 0, n3(p.edy2 * p.rock * side * (f.rockp - 1) / 2)];
}
function glanceT(f, cx, cy, lean, side) {
  "worklet";
  const sx = n3(1 + f.wrap.mx + f.wrap.side * side);
  const sy = n3(1 + f.wrap.sy);
  const bl = n3(f.blink);
  const r = n3(f.wrap.rot * side) * Math.PI / 180;
  const cr = Math.cos(r);
  const sr = Math.sin(r);
  const q = n3(lean) * Math.PI / 180;
  const cq = Math.cos(q);
  const sq = Math.sin(q);
  const k = sq * cq * (1 - bl);
  const ba = cq * cq + sq * sq * bl;
  const bd = sq * sq + cq * cq * bl;
  const a1 = cr * sx;
  const b1 = sr * sx;
  const c1 = -sr * sy;
  const d1 = cr * sy;
  const a = a1 * ba + c1 * k;
  const b = b1 * ba + d1 * k;
  const c = a1 * k + c1 * bd;
  const d = b1 * k + d1 * bd;
  const ix = n3(-cx);
  const iy = n3(-cy);
  return [a, b, c, d, n3(cx) + a * ix + c * iy, n3(cy) + b * ix + d * iy];
}

// frame.ts
function lerpPath(a, b, t) {
  "worklet";
  if (a.segments.length !== b.segments.length)
    throw new Error("avatar paths have different segment counts");
  const segments = [];
  for (let i = 0;i < b.segments.length; i++) {
    const s = a.segments[i];
    const e = b.segments[i];
    segments.push([
      s[0] * (1 - t) + e[0] * t,
      s[1] * (1 - t) + e[1] * t,
      s[2] * (1 - t) + e[2] * t,
      s[3] * (1 - t) + e[3] * t,
      s[4] * (1 - t) + e[4] * t,
      s[5] * (1 - t) + e[5] * t
    ]);
  }
  return { start: [a.start[0] * (1 - t) + b.start[0] * t, a.start[1] * (1 - t) + b.start[1] * t], segments };
}
function lerpColor(a, b, t) {
  "worklet";
  if (t <= 0)
    return a;
  if (t >= 1)
    return b;
  let out = "#";
  for (let i = 1;i < 7; i += 2) {
    const v = Math.round(parseInt(a.slice(i, i + 2), 16) * (1 - t) + parseInt(b.slice(i, i + 2), 16) * t);
    out += (v < 16 ? "0" : "") + v.toString(16).toUpperCase();
  }
  return out;
}
function interpolateAvatarGeometry(from, to, t) {
  "worklet";
  const u = t <= 0 ? 0 : t >= 1 ? 1 : t;
  const m = (a, b) => a * (1 - u) + b * u;
  if (from.petals.length !== to.petals.length)
    throw new Error("avatar geometries have different petal counts");
  const pose = {};
  for (const k in to.pose) {
    const c = k;
    pose[c] = m(from.pose[c], to.pose[c]);
  }
  const eye = (i) => {
    const a = from.eyes[i];
    const b = to.eyes[i];
    return { cx: m(a.cx, b.cx), cy: m(a.cy, b.cy), rx: m(a.rx, b.rx), ry: m(a.ry, b.ry), n: m(a.n, b.n), rot: m(a.rot, b.rot) };
  };
  return {
    version: 1,
    background: {
      path: lerpPath(from.background.path, to.background.path, u),
      fill: lerpColor(from.background.fill, to.background.fill, u),
      opacity: m(from.background.opacity, to.background.opacity)
    },
    core: lerpPath(from.core, to.core, u),
    extra: lerpPath(from.extra, to.extra, u),
    petals: to.petals.map((p, i) => {
      const q = from.petals[i];
      return { cx: m(q.cx, p.cx), cy: m(q.cy, p.cy), r: m(q.r, p.r) };
    }),
    eyes: [eye(0), eye(1)],
    pose,
    palette: {
      head: lerpColor(from.palette.head, to.palette.head, u),
      eye: lerpColor(from.palette.eye, to.palette.eye, u),
      bg: lerpColor(from.palette.bg, to.palette.bg, u)
    },
    motion: u > 0 ? to.motion : from.motion,
    seeds: u > 0 ? to.seeds : from.seeds
  };
}
function cubicBezier(x, x1, y1, x2, y2) {
  "worklet";
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  let t = x;
  for (let i = 0;i < 8; i++) {
    const err = ((ax * t + bx) * t + cx) * t - x;
    if (Math.abs(err) < 0.00001)
      break;
    const d = (3 * ax * t + 2 * bx) * t + cx;
    if (Math.abs(d) < 0.000001)
      break;
    t -= err / d;
  }
  return ((ay * t + by) * t + cy) * t;
}
function avatarMorphProgress(elapsedMs, toward) {
  "worklet";
  const p = toward.pose;
  const expressive = p.esx !== 1 || p.esy !== 1 || p.tilt !== 0 || p.edy !== 0 || p.edx !== 0 || p.esx2 !== 0 || p.esy2 !== 0 || p.tilt2 !== 0 || p.edy2 !== 0 || p.lock !== 0 || p.heat !== 0 || p.shake !== 0 || p.rock !== 0 || p.bdy !== 0;
  const x = elapsedMs / (expressive ? 300 : 400);
  if (!(x < 1))
    return { t: 1, done: true };
  if (x <= 0)
    return { t: 0, done: false };
  return { t: expressive ? cubicBezier(x, 0.45, 0.05, 0.5, 1) : cubicBezier(x, 0.42, 0, 0.58, 1), done: false };
}
function mul(a, b) {
  "worklet";
  return [
    a[0] * b[0] + a[2] * b[1],
    a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3],
    a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4],
    a[1] * b[4] + a[3] * b[5] + a[5]
  ];
}
function poseMatrix(e, p, i) {
  "worklet";
  const wrap = i ? 1 : -1;
  const sel = i ? 1 : 0;
  const r = ((p.tilt + sel * p.tilt2) * wrap + e.rot * (1 - p.lock)) * Math.PI / 180;
  const q = -e.rot * Math.PI / 180;
  const sx = p.esx + sel * p.esx2;
  const sy = p.esy + sel * p.esy2;
  const lin = mul(mul([Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0], [sx, 0, 0, sy, 0, 0]), [Math.cos(q), Math.sin(q), -Math.sin(q), Math.cos(q), 0, 0]);
  const tx = e.cx + p.edx * wrap;
  const ty = e.cy + p.edy + sel * p.edy2;
  return [lin[0], lin[1], lin[2], lin[3], tx - lin[0] * e.cx - lin[2] * e.cy, ty - lin[1] * e.cx - lin[3] * e.cy];
}
function eyePath(e) {
  "worklet";
  const k = Math.min(1, (8 * Math.pow(2, -1 / e.n) - 4) / 3);
  const a = e.rx;
  const b = e.ry;
  const ak = a * k;
  const bk = b * k;
  const pts = [
    a,
    0,
    a,
    bk,
    ak,
    b,
    0,
    b,
    -ak,
    b,
    -a,
    bk,
    -a,
    0,
    -a,
    -bk,
    -ak,
    -b,
    0,
    -b,
    ak,
    -b,
    a,
    -bk,
    a,
    0
  ];
  const t = e.rot * Math.PI / 180;
  const cos = Math.cos(t);
  const sin = Math.sin(t);
  const xy = [];
  for (let i = 0;i < pts.length; i += 2) {
    xy.push(Math.round((e.cx + pts[i] * cos - pts[i + 1] * sin) * 100) / 100);
    xy.push(Math.round((e.cy + pts[i] * sin + pts[i + 1] * cos) * 100) / 100);
  }
  const segments = [];
  for (let s = 0;s < 4; s++) {
    const o = 2 + s * 6;
    segments.push([xy[o], xy[o + 1], xy[o + 2], xy[o + 3], xy[o + 4], xy[o + 5]]);
  }
  return { start: [xy[0], xy[1]], segments };
}
function avatarFrame(g, timeMs, amp) {
  "worklet";
  const a = amp <= 0 ? 0 : amp >= 1 ? 1 : amp;
  const raw = idleFrame(g.seeds, timeMs, a, g.pose.shake * a);
  const f = { ...raw, rockp: 1 + a * (raw.rockp - 1) };
  const body = mul(rootT(f), mul(breatheT(f), bobT(f, g.pose.bdy)));
  const pair = mul(body, eyesT(f));
  const eyeMatrix = (i) => {
    const e = g.eyes[i];
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
    eye: g.palette.eye
  };
}
function avatarPath(p, digits = 2) {
  "worklet";
  const k = Math.pow(10, digits);
  const n = (v) => {
    const r = Math.round(v * k) / k;
    return r === 0 ? "0" : String(r);
  };
  let d = `M${n(p.start[0])} ${n(p.start[1])}`;
  for (const s of p.segments)
    d += `C${n(s[0])} ${n(s[1])} ${n(s[2])} ${n(s[3])} ${n(s[4])} ${n(s[5])}`;
  return d + "Z";
}
export {
  avatarFrame,
  avatarMorphProgress,
  avatarPath,
  interpolateAvatarGeometry
};
