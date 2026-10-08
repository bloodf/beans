(() => {
  var __defProp = Object.defineProperty;
  var __returnValue = (v) => v;
  function __exportSetter(name, newValue) {
    this[name] = __returnValue.bind(null, newValue);
  }
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, {
        get: all[name],
        enumerable: true,
        configurable: true,
        set: __exportSetter.bind(all, name)
      });
  };

  // ../packages/beans-blobatar/index.ts
  var exports_beans_blobatar = {};
  __export(exports_beans_blobatar, {
    BOT_AVATAR_STATES: () => BOT_AVATAR_STATES,
    BOT_BACKGROUNDS: () => BOT_BACKGROUNDS,
    BOT_EXPRESSIONS: () => BOT_EXPRESSIONS,
    BOT_SHAPES: () => BOT_SHAPES,
    BOT_TONES: () => BOT_TONES,
    avatarFrame: () => avatarFrame,
    avatarMorphProgress: () => avatarMorphProgress,
    avatarPath: () => avatarPath,
    blobatar: () => blobatar2,
    botAppearanceContrast: () => botAppearanceContrast,
    botAvatarGeometry: () => botAvatarGeometry,
    botAvatarSVG: () => botAvatarSVG,
    botAvatarSeed: () => botAvatarSeed,
    interpolateAvatarGeometry: () => interpolateAvatarGeometry,
    resolveBotAppearance: () => resolveBotAppearance,
    validateBotLook: () => validateBotLook
  });

  // ../packages/beans-blobatar/vendor/core/animate.ts
  function motionSeeds(t) {
    const blink = Math.round(t.num("motion.blink", 3500, 6500));
    const saccade = Math.round(t.num("motion.saccade", 4200, 7600));
    const lookX = t.num("motion.lookX", 1, 2.2);
    const lookY = t.num("motion.lookY", 0.8, 1.7);
    const r2 = (v) => Math.round(v * 100) / 100;
    return {
      phase: Math.round(t.num("motion.phase", 0, 2800)),
      bob: Math.round(t.num("motion.bob", 0, 3400)),
      blink,
      blinkPhase: Math.round(t.num("motion.blinkPhase", 0, blink)),
      saccade,
      saccadePhase: Math.round(t.num("motion.saccadePhase", 0, saccade)),
      lookX: r2(lookX) * (t.bool("motion.lookXFlip") ? -1 : 1),
      lookY: r2(lookY) * (t.bool("motion.lookYFlip") ? -1 : 1),
      lookMX: r2(lookX),
      lookMY: r2(lookY)
    };
  }

  // ../packages/beans-blobatar/vendor/core/color.ts
  function toLinear({ l, c, h }) {
    const r = h * Math.PI / 180;
    const a = c * Math.cos(r);
    const b = c * Math.sin(r);
    const l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    const m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    const s_ = l - 0.0894841775 * a - 1.291485548 * b;
    const L = l_ * l_ * l_;
    const M = m_ * m_ * m_;
    const S = s_ * s_ * s_;
    return [
      4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S,
      -1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S,
      -0.0041960863 * L - 0.7034186147 * M + 1.707614701 * S
    ];
  }
  var inGamut = (rgb) => rgb.every((v) => v >= -0.0001 && v <= 1 + 0.0001);
  function resolve(color) {
    let rgb = toLinear(color);
    if (!inGamut(rgb)) {
      let lo = 0;
      let hi = color.c;
      for (let i = 0;i < 12; i++) {
        const mid = (lo + hi) / 2;
        if (inGamut(toLinear({ ...color, c: mid })))
          lo = mid;
        else
          hi = mid;
      }
      rgb = toLinear({ ...color, c: lo });
    }
    return rgb.map((v) => Math.min(1, Math.max(0, v)));
  }
  function luminance(color) {
    const [r, g, b] = resolve(color);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }
  function contrast(a, b) {
    const x = luminance(a);
    const y = luminance(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  }
  function ensureContrast(fg, bg, min) {
    if (contrast(fg, bg) >= min)
      return fg;
    const lean = fg.l >= bg.l ? 1 : -1;
    for (const dir of [lean, -lean]) {
      const probe = { ...fg };
      for (let i = 0;i < 60; i++) {
        probe.l = Math.min(1, Math.max(0, probe.l + dir * 0.02));
        if (contrast(probe, bg) >= min)
          return probe;
        if (probe.l === 0 || probe.l === 1)
          break;
      }
    }
    const black = { ...fg, l: 0, c: 0 };
    const white = { ...fg, l: 1, c: 0 };
    return contrast(black, bg) >= contrast(white, bg) ? black : white;
  }
  function toHex(color) {
    return "#" + resolve(color).map((v) => {
      const s = v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
      return Math.round(s * 255).toString(16).padStart(2, "0");
    }).join("");
  }
  function fromHex(hex) {
    const n = parseInt(hex.slice(1), 16);
    const [r, g, b] = [n >> 16 & 255, n >> 8 & 255, n & 255].map((v) => {
      const s = v / 255;
      return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    });
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    return {
      l: 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
      c: Math.hypot(A, B),
      h: Math.atan2(B, A) * 180 / Math.PI
    };
  }
  function mix(a, b, t) {
    const rad = (v) => v * Math.PI / 180;
    const ax = a.c * Math.cos(rad(a.h));
    const ay = a.c * Math.sin(rad(a.h));
    const bx = b.c * Math.cos(rad(b.h));
    const by = b.c * Math.sin(rad(b.h));
    const x = ax + (bx - ax) * t;
    const y = ay + (by - ay) * t;
    return {
      l: a.l + (b.l - a.l) * t,
      c: Math.hypot(x, y),
      h: Math.atan2(y, x) * 180 / Math.PI
    };
  }
  var mixHex = (a, b, t) => toHex(mix(fromHex(a), fromHex(b), t));
  var HOT = { h: 27, l: 0.58, pull: 0.6, c: 0.18 };
  var ROSE = { h: 358, l: 0.72, pull: 0.55, c: 0.16 };
  var BLUSH = { h: 12, l: 0.84, pull: 0.4, c: 0.1 };
  var BILE = { h: 142, l: 0.66, pull: 0.6, c: 0.13 };
  var TINT_FLOOR = 4.55;
  function tinted(head, eye, t) {
    const base = fromHex(head);
    const baseEye = fromHex(eye);
    let hotHead = {
      l: base.l + (t.l - base.l) * t.pull,
      c: Math.max(base.c, t.c),
      h: t.h
    };
    hotHead = ensureContrast(hotHead, DARK_SURFACE, SURFACE_FLOOR);
    let hotEye = ensureContrast(baseEye, hotHead, TINT_FLOOR);
    const dir = hotEye.l >= hotHead.l ? 1 : -1;
    const headHex = toHex(hotHead);
    for (let pass = 0;pass < 40; pass++) {
      const eyeHex = toHex(hotEye);
      let worst = Infinity;
      for (let i = 0;i <= 10; i++) {
        const t = i / 10;
        worst = Math.min(worst, contrast(fromHex(mixHex(eye, eyeHex, t)), fromHex(mixHex(head, headHex, t))));
      }
      if (worst >= TINT_FLOOR)
        return [headHex, eyeHex];
      const l = Math.min(1, Math.max(0, hotEye.l + dir * 0.02));
      if (l === hotEye.l)
        return [headHex, eyeHex];
      hotEye = { ...hotEye, l };
    }
    return [headHex, toHex(hotEye)];
  }
  var TONES = [
    [0.2, { l: 0.86, c: 0.085 }],
    [0.36, { l: 0.9, c: 0.028 }],
    [0.62, { l: 0.73, c: 0.135 }],
    [0.8, { l: 0.62, c: 0.165 }],
    [0.93, { l: 0.87, c: 0.16 }],
    [1, { l: 0.34, c: 0.035 }]
  ];
  var toneAt = (v) => TONES.find(([edge]) => v < edge)?.[1] ?? TONES[0][1];
  var DARK_SURFACE = { l: 0.145, c: 0, h: 0 };
  var SURFACE_FLOOR = 1.5;
  var RAMP = (h, tone) => {
    const t = toneAt(tone);
    const head = ensureContrast({ l: t.l, c: t.c, h }, DARK_SURFACE, SURFACE_FLOOR);
    return {
      bg: { l: 0.965, c: 0.01, h },
      head,
      eye: head.l >= 0.5 ? { l: 0.17, c: 0.02, h } : { l: 0.97, c: 0.012, h }
    };
  };
  var FLOORS = [
    ["head", "bg", 1.25],
    ["eye", "head", 4.5]
  ];
  function ramp(hue, enforce = true, tone = 0) {
    const r = RAMP(hue, tone);
    if (enforce) {
      for (const [fg, bg, min] of FLOORS) {
        r[fg] = ensureContrast(r[fg], r[bg], min);
      }
    }
    return r;
  }
  function palette(hue, enforce = true, tone = 0) {
    const r = ramp(hue, enforce, tone);
    const out = {};
    for (const k in r)
      out[k] = toHex(r[k]);
    return out;
  }

  // ../packages/beans-blobatar/vendor/core/shape.ts
  var r2 = (v) => {
    const s = Math.round(v * 100) / 100;
    return Object.is(s, -0) ? "0" : String(s);
  };
  function superellipse({ cx, cy, rx, ry, n = 4, rot = 0 }) {
    const k = Math.min(1, (8 * Math.pow(2, -1 / n) - 4) / 3);
    const a = rx;
    const b = ry;
    const ak = a * k;
    const bk = b * k;
    const pts = [
      [a, 0],
      [a, bk],
      [ak, b],
      [0, b],
      [-ak, b],
      [-a, bk],
      [-a, 0],
      [-a, -bk],
      [-ak, -b],
      [0, -b],
      [ak, -b],
      [a, -bk],
      [a, 0]
    ];
    const t = rot * Math.PI / 180;
    const cos = Math.cos(t);
    const sin = Math.sin(t);
    const at = (i) => {
      const [x, y] = pts[i];
      return `${r2(cx + x * cos - y * sin)} ${r2(cy + x * sin + y * cos)}`;
    };
    let d = `M${at(0)}`;
    for (let i = 1;i < 13; i += 3)
      d += `C${at(i)} ${at(i + 1)} ${at(i + 2)}`;
    return d + "Z";
  }
  function blobPath(cx, cy, rx, ry, radii, rot = 0) {
    const n = radii.length;
    const t0 = rot * Math.PI / 180;
    const p = radii.map((m, i) => {
      const a = t0 + 2 * Math.PI * i / n;
      return [cx + rx * m * Math.cos(a), cy + ry * m * Math.sin(a)];
    });
    const at = (i) => p[(i % n + n) % n];
    let d = `M${r2(at(0)[0])} ${r2(at(0)[1])}`;
    for (let i = 0;i < n; i++) {
      const [x0, y0] = at(i - 1);
      const [x1, y1] = at(i);
      const [x2, y2] = at(i + 1);
      const [x3, y3] = at(i + 2);
      d += `C${r2(x1 + (x2 - x0) / 6)} ${r2(y1 + (y2 - y0) / 6)}` + ` ${r2(x2 - (x3 - x1) / 6)} ${r2(y2 - (y3 - y1) / 6)}` + ` ${r2(x2)} ${r2(y2)}`;
    }
    return d + "Z";
  }
  function polygon({ cx, cy, rx, ry, sides, round = 0.3, rot = 0 }) {
    const k = round > 0 ? round < 1 ? round / 2 : 0.5 : 0;
    const t0 = rot * Math.PI / 180 - Math.PI / 2;
    const v = Array.from({ length: sides }, (_, i) => {
      const a = t0 + 2 * Math.PI * i / sides;
      return [cx + rx * Math.cos(a), cy + ry * Math.sin(a)];
    });
    const at = (i) => v[(i % sides + sides) % sides];
    const cut = (i, j) => {
      const [x0, y0] = at(i);
      const [x1, y1] = at(j);
      return `${r2(x0 + (x1 - x0) * k)} ${r2(y0 + (y1 - y0) * k)}`;
    };
    let d = `M${cut(0, -1)}`;
    for (let i = 0;i < sides; i++) {
      const [x, y] = at(i);
      d += `Q${r2(x)} ${r2(y)} ${cut(i, i + 1)}`;
      if (k < 0.5)
        d += `L${cut(i + 1, i)}`;
    }
    return d + "Z";
  }
  function box(cx, cy, rx, ry) {
    const l = r2(cx - rx);
    const r = r2(cx + rx);
    return `M${l} ${r2(cy - ry)}H${r}V${r2(cy + ry)}H${l}Z`;
  }
  function taper(cx, cy, rx, ry, tip) {
    const t = Math.max(1.05, tip);
    const tx = rx * Math.sqrt(1 - 1 / (t * t));
    const ty = cy - ry / t;
    const apex = cy - t * ry;
    const px = tx * 0.14;
    const py = ty + 0.86 * (apex - ty);
    return `M${r2(cx - tx)} ${r2(ty)}` + `L${r2(cx - px)} ${r2(py)}` + `Q${r2(cx)} ${r2(apex)} ${r2(cx + px)} ${r2(py)}` + `L${r2(cx + tx)} ${r2(ty)}Z`;
  }

  // ../packages/beans-blobatar/vendor/core/hash.ts
  var SEP = 255;
  function feed(h, bytes) {
    for (let i = 0;i < bytes.length; i++) {
      h = Math.imul(h ^ bytes[i], 3432918353);
      h = h << 13 | h >>> 19;
    }
    return h;
  }
  function finalize(h) {
    h = Math.imul(h ^ h >>> 16, 2246822507);
    h = Math.imul(h ^ h >>> 13, 3266489909);
    return (h ^ h >>> 16) >>> 0;
  }
  var utf8 = () => new TextEncoder;
  function normalizeSeed(seed) {
    return seed.normalize("NFC").trim().toLowerCase();
  }
  function seedState(seed, normalize = true) {
    const s = normalize ? normalizeSeed(seed) : seed;
    return feed(1779033703 ^ s.length, utf8().encode(s));
  }
  function stream(state, key) {
    return finalize(feed(feed(state, Uint8Array.of(SEP)), utf8().encode(key))) / 4294967296;
  }

  // ../packages/beans-blobatar/vendor/core/traits.ts
  function traits(seed, normalize = true, overrides) {
    const state = seedState(seed, normalize);
    const t = (key) => {
      const v = overrides?.[key];
      const o = Array.isArray(v) ? v[Math.floor(stream(state, key) * v.length)] : v;
      return o === undefined ? stream(state, key) : o > 0 ? o < 1 ? o : 0.999999 : 0;
    };
    t.num = (key, min, max) => min + t(key) * (max - min);
    t.int = (key, min, max) => min + Math.floor(t(key) * (max - min + 1));
    t.pick = (key, options) => options[Math.floor(t(key) * options.length)];
    t.bool = (key, p = 0.5) => t(key) < p;
    t.jitter = (key, amount) => (t(key) * 2 - 1) * amount;
    return t;
  }

  // ../packages/beans-blobatar/vendor/core/render.ts
  function posed(l, opts, animate) {
    const e = opts.expression;
    if (animate || !e)
      return { l, wrap: "" };
    return e.bake(l, e.p);
  }
  var tinted2 = (p, e) => e?.tint ? e.tint(p, e.p) : p;
  var wrap = (body, t) => t ? `<g transform="${t}">${body}</g>` : body;
  var escape = (s) => s.replace(/[&<>]/g, (c) => c === "&" ? "&amp;" : c === "<" ? "&lt;" : "&gt;");
  function resolve2(seed, opts) {
    const t = traits(seed, opts.normalize ?? true, opts.traits);
    return {
      t,
      palette: {
        ...palette(opts.hue ?? t.num("hue", 0, 360), opts.contrast ?? true, opts.tone ?? t("tone")),
        ...opts.palette
      }
    };
  }
  var label = (opts) => opts.title ? `<title>${escape(opts.title)}</title>` : "";
  function backdrop(style, opts, p) {
    const bg = opts.background ?? style.background;
    if (bg === false)
      return;
    return {
      d: bg === "square" ? "M0 0H100V100H0Z" : superellipse({
        cx: 50,
        cy: 50,
        rx: 50,
        ry: 50,
        n: bg === "circle" ? 2 : 6
      }),
      fill: p.bg
    };
  }
  var plate = (b) => b ? `<path d="${b.d}" fill="${b.fill}"/>` : "";
  function makeBlobatar(style) {
    return (name, opts = {}) => {
      const { t, palette } = resolve2(name, opts);
      const p = tinted2(palette, opts.expression);
      const dim = opts.size ? ` width="${opts.size}" height="${opts.size}"` : "";
      const pose = posed(style.layout(t), opts);
      const body = label(opts) + plate(backdrop(style, opts, p)) + wrap(style.render(pose.l, p), pose.wrap);
      return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"${dim}>${body}</svg>`;
    };
  }

  // ../packages/beans-blobatar/vendor/core/styles/compose.ts
  var faceFit = (t, b, face) => {
    const rx = b.rx;
    const er0 = t.num("eye.rx", 0.075, 0.105) * rx;
    const ratio = t.num("eye.ratio", 1.9, 3.2);
    const scale = t.num("eye.scale", 0.78, 1.24);
    const stretch = t.num("eye.stretch", 0.85, 1.18);
    const clearance = t.num("eye.gap", 0.1, 0.24) * rx;
    const wide = er0 * Math.max(1, scale);
    const tall = er0 * ratio * Math.max(1, scale * stretch);
    const gap0 = wide + rx * 0.03 + clearance;
    const gx = t.jitter("gaze.x", 0.09) * face.rx;
    const gy = t.num("gaze.y", -0.2, 0.08) * face.ry;
    const dy = t.jitter("eye.dy", 0.04) * face.ry;
    const reach = Math.hypot(wide, tall);
    const need = Math.hypot((Math.abs(gx) + gap0 + reach) / face.rx, (Math.abs(gy) + Math.abs(dy) + reach) / face.ry);
    const fit = need > 0.9 ? 0.9 / need : 1;
    const er = er0 * fit;
    const eyeRy = er * ratio;
    const gap = gap0 * fit;
    const room = Math.max(0, Math.min(1, clearance / tall));
    const bound = Math.min(12, Math.asin(room) * 180 / Math.PI);
    const lean = t.num("eye.lean", -1, 1) * bound;
    const lean2 = Math.max(-12, Math.min(12, lean + t.jitter("eye.lean2", 3.5)));
    const cx = face.cx + gx * fit;
    const cy = face.cy + gy * fit;
    return [
      { cx: cx - gap, cy, rx: er, ry: eyeRy, n: t.num("eye.n", 3.5, 6), rot: lean },
      {
        cx: cx + gap,
        cy: cy + dy * fit,
        rx: er * scale,
        ry: eyeRy * scale * stretch,
        n: t.num("eye.n", 3.5, 6),
        rot: lean2
      }
    ];
  };
  function compose(bands, fit) {
    const pick = (v) => (bands.find(([, upTo]) => v < upTo) ?? bands[bands.length - 1])[0];
    function layout(t) {
      const shape = pick(t("shape"));
      const r = t.num("body.r", 31, 38) * shape.core;
      const body = {
        cx: 50 + t.jitter("body.x", 1.5),
        cy: 50 + t.jitter("body.y", 1.5),
        rx: r,
        ry: r * t.num("body.ratio", 0.92, 1.08),
        n: t.num("body.n", 1.9, 2.5),
        rot: 0,
        radii: Array.from({ length: t.int("body.pts", 6, 8) }, (_, i) => 1 + t.jitter(`body.r${i}`, 0.16))
      };
      shape.body?.(t, body);
      const face = shape.face?.(body) ?? body;
      const deco = { petals: [], extra: [] };
      shape.decorate?.(t, body, deco);
      return {
        shape: shape.name,
        draw: shape.path,
        body,
        face,
        petals: deco.petals,
        extra: deco.extra,
        eyes: fit(t, body, face)
      };
    }
    function render(l, p, mo) {
      const r2 = (v) => Math.round(v * 100) / 100;
      const eye = (e, i) => {
        const path = `<path d="${superellipse(e)}"/>`;
        return mo ? `<g class="mo-eye" style="--mo-wrap:${i ? 1 : -1};--mo-lean:${r2(e.rot)};transform-origin:${r2(e.cx)}px ${r2(e.cy)}px">${path}</g>` : path;
      };
      const body = `<g fill="${p.head}">` + l.petals.map((d) => `<circle cx="${r2(d.cx)}" cy="${r2(d.cy)}" r="${r2(d.r)}"/>`).join("") + l.extra.map((d) => `<path d="${d}"/>`).join("") + `<path d="${l.draw ? l.draw(l.body) : superellipse(l.body)}"/>` + `</g>` + `<g fill="${p.eye}"${mo ? ` class="mo-eyes"` : ""}>` + l.eyes.map(eye).join("") + `</g>`;
      return mo ? `<g class="mo-breathe"><g class="mo-bob">${body}</g></g>` : body;
    }
    return { layout, render, background: false };
  }
  function marks(l, p) {
    const r2 = (v) => Math.round(v * 100) / 100;
    const head = p.head;
    return [
      ...l.petals.map((d) => ({ kind: "circle", cx: r2(d.cx), cy: r2(d.cy), r: r2(d.r), fill: head })),
      ...l.extra.map((d) => ({ kind: "path", d, fill: head })),
      { kind: "path", d: l.draw ? l.draw(l.body) : superellipse(l.body), fill: head },
      ...l.eyes.map((e) => ({ kind: "path", d: superellipse(e), fill: p.eye }))
    ];
  }

  // ../packages/beans-blobatar/vendor/core/styles/shapes.ts
  var poly = (b) => polygon(b);
  var spline = (b) => blobPath(b.cx, b.cy, b.rx, b.ry, b.radii, b.rot);
  var shrunk = (k) => (b) => ({
    cx: b.cx,
    cy: b.cy,
    rx: b.rx * k,
    ry: b.ry * k
  });
  var splineFace = (b) => shrunk(Math.min(...b.radii) * 0.95)(b);
  var polyFace = (b) => shrunk(0.84)(b);
  var round = { name: "round", core: 1 };
  var organic = {
    name: "organic",
    core: 0.98,
    path: spline,
    face: splineFace
  };
  var boxy = {
    name: "boxy",
    core: 0.86,
    body: (t, b) => {
      b.n = t.num("body.n", 3.4, 6);
      b.rot = t.num("body.rot", -20, 20);
    }
  };
  var capsule = {
    name: "capsule",
    core: 1.02,
    body: (t, b) => {
      b.ry *= t.num("capsule.squat", 0.55, 0.68);
    },
    face: shrunk(0.94),
    decorate: (_t, b, out) => {
      for (const s of [-1, 1])
        out.petals.push({ cx: b.cx + s * (b.rx - b.ry), cy: b.cy, r: b.ry });
    },
    path: (b) => box(b.cx, b.cy, b.rx - b.ry, b.ry)
  };
  var nub = {
    name: "nub",
    core: 0.88,
    decorate: (t, b, out) => {
      const count = t.int("nub.n", 1, 2);
      for (let i = 0;i < count; i++) {
        const a = t.num(`nub.a${i}`, 0, 2 * Math.PI);
        out.petals.push({
          cx: b.cx + Math.cos(a) * b.rx * 0.88,
          cy: b.cy + Math.sin(a) * b.rx * 0.88,
          r: b.rx * t.num(`nub.r${i}`, 0.24, 0.4)
        });
      }
    }
  };
  var cloud = {
    name: "cloud",
    core: 0.78,
    face: splineFace,
    path: spline,
    decorate: (t, b, out) => {
      const count = t.int("cloud.n", 4, 6);
      for (let i = 0;i < count; i++) {
        const a = Math.PI + Math.PI * (i + 0.5) / count;
        out.petals.push({
          cx: b.cx + Math.cos(a) * b.rx * 0.8,
          cy: b.cy + Math.sin(a) * b.rx * 0.5,
          r: b.rx * t.num(`cloud.r${i}`, 0.44, 0.62)
        });
      }
    }
  };
  var droplet = {
    name: "droplet",
    core: 0.78,
    body: (_t, b) => {
      b.cy += 0.22 * b.ry;
      b.n = 2;
    },
    face: (b) => ({ cx: b.cx, cy: b.cy + b.ry * 0.05, rx: b.rx * 0.88, ry: b.ry * 0.88 }),
    decorate: (t, b, out) => {
      out.extra.push(taper(b.cx, b.cy, b.rx, b.ry, t.num("droplet.tip", 1.4, 1.65)));
    }
  };
  var hexagon = {
    name: "hexagon",
    core: 1.05,
    path: poly,
    face: polyFace,
    body: (t, b) => {
      b.sides = 6;
      b.rot = t.num("body.rot", -12, 12);
      b.round = t.num("poly.round", 0.24, 0.5);
    }
  };
  var sun = {
    name: "sun",
    core: 0.7,
    decorate: (t, b, out) => {
      const count = t.int("sun.n", 6, 9);
      const dist = b.rx * t.num("sun.dist", 1, 1.08);
      const pr = b.rx * t.num("sun.r", 0.2, 0.26);
      const off = t.num("sun.rot", 0, 2 * Math.PI);
      for (let i = 0;i < count; i++) {
        const a = off + 2 * Math.PI * i / count;
        out.petals.push({ cx: b.cx + Math.cos(a) * dist, cy: b.cy + Math.sin(a) * dist, r: pr });
      }
    }
  };
  var triangle = {
    name: "triangle",
    core: 1.15,
    path: poly,
    body: (t, b) => {
      b.sides = 3;
      b.rot = t.num("body.rot", -5, 5);
      b.round = t.num("poly.round", 0.24, 0.5);
    },
    face: (b) => ({ cx: b.cx, cy: b.cy + b.ry * 0.1, rx: b.rx * 0.54, ry: b.ry * 0.36 })
  };

  // ../packages/beans-blobatar/vendor/core/styles/blob.ts
  var BANDS = [
    [round, 0.22],
    [organic, 0.48],
    [boxy, 0.6],
    [capsule, 0.7],
    [nub, 0.79],
    [cloud, 0.86],
    [droplet, 0.915],
    [hexagon, 0.95],
    [sun, 0.98],
    [triangle, 1]
  ];
  var style = compose(BANDS, faceFit);

  // ../packages/beans-blobatar/vendor/core/blobatar.ts
  var blobatar = makeBlobatar(style);

  // ../packages/beans-blobatar/vendor/core/morph.ts
  var IDENT = {
    esx: 1,
    esy: 1,
    tilt: 0,
    edy: 0,
    edx: 0,
    esx2: 0,
    esy2: 0,
    tilt2: 0,
    edy2: 0,
    lock: 0,
    heat: 0,
    shake: 0,
    rock: 0,
    bdy: 0
  };
  var r3 = (v) => String(Math.round(v * 1000) / 1000);
  function bakePose(l, p) {
    return {
      l: {
        ...l,
        eyes: l.eyes.map((e, i) => ({
          ...e,
          cx: e.cx + p.edx * (i ? 1 : -1),
          cy: e.cy + p.edy + (i ? p.edy2 : 0),
          rx: e.rx * (p.esx + (i ? p.esx2 : 0)),
          ry: e.ry * (p.esy + (i ? p.esy2 : 0)),
          rot: e.rot * (1 - p.lock) + (p.tilt + (i ? p.tilt2 : 0)) * (i ? 1 : -1)
        }))
      },
      wrap: p.bdy !== 0 ? `translate(0 ${r3(p.bdy)})` : ""
    };
  }

  // ../packages/beans-blobatar/vendor/core/expression.ts
  function poseVars(p) {
    const out = {};
    for (const k in IDENT) {
      if (k === "heat")
        continue;
      const v = p[k];
      if (v !== IDENT[k])
        out["--mo-" + k] = r3(v);
    }
    return out;
  }
  function tintWith(pal, p, t) {
    const [head, eye] = tinted(pal.head, pal.eye, t);
    return {
      ...pal,
      head: mixHex(pal.head, head, p.heat),
      eye: mixHex(pal.eye, eye, p.heat)
    };
  }
  var heatTint = (pal, p) => tintWith(pal, p, HOT);
  var idle = { p: IDENT, vars: poseVars, bake: bakePose };
  var happy = {
    p: {
      esx: 1.72,
      esy: 0.3,
      tilt: 8,
      edy: -1.5,
      edx: 1.5,
      esx2: 0.08,
      esy2: 0.05,
      tilt2: -16,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: -2.2
    },
    vars: poseVars,
    bake: bakePose
  };
  var sad = {
    p: {
      esx: 0.6,
      esy: 0.56,
      tilt: 26,
      edy: 3.6,
      edx: 1.9,
      esx2: -0.05,
      esy2: -0.07,
      tilt2: -7,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: 2.6
    },
    vars: poseVars,
    bake: bakePose
  };
  var mad = {
    p: {
      esx: 1.85,
      esy: 0.26,
      tilt: -33,
      edy: 0.4,
      edx: 0.6,
      esx2: 0,
      esy2: -0.03,
      tilt2: 5,
      edy2: 0,
      lock: 1,
      heat: 0.62,
      shake: 0.55,
      rock: 0,
      bdy: 0.8
    },
    vars: poseVars,
    bake: bakePose,
    tint: heatTint
  };
  var surprised = {
    p: {
      esx: 1.34,
      esy: 1.2,
      tilt: -6,
      edy: -1.05,
      edx: 0.5,
      esx2: 0.05,
      esy2: 0.07,
      tilt2: 3,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: -1.4
    },
    vars: poseVars,
    bake: bakePose
  };
  var wink = {
    p: {
      esx: 1.32,
      esy: 0.76,
      tilt: 5,
      edy: -0.6,
      edx: 0.8,
      esx2: 0.26,
      esy2: -0.56,
      tilt2: -11,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: -1.1
    },
    vars: poseVars,
    bake: bakePose
  };
  var sleepy = {
    p: {
      esx: 1.14,
      esy: 0.22,
      tilt: 0,
      edy: 2.4,
      edx: 0.3,
      esx2: -0.04,
      esy2: 0.03,
      tilt2: 4,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: 1.2
    },
    vars: poseVars,
    bake: bakePose
  };
  var smug = {
    p: {
      esx: 1.3,
      esy: 0.42,
      tilt: 18,
      edy: -0.5,
      edx: 0.5,
      esx2: 0.06,
      esy2: -0.06,
      tilt2: -36,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: -1
    },
    vars: poseVars,
    bake: bakePose
  };
  var unsure = {
    p: {
      esx: 0.95,
      esy: 1.02,
      tilt: 4,
      edy: -0.2,
      edx: 0.3,
      esx2: 0.24,
      esy2: -0.44,
      tilt2: -18,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0,
      bdy: 0
    },
    vars: poseVars,
    bake: bakePose
  };
  var scared = {
    p: {
      esx: 0.78,
      esy: 0.96,
      tilt: -12,
      edy: -1.5,
      edx: -0.8,
      esx2: -0.04,
      esy2: 0.05,
      tilt2: 4,
      edy2: 0,
      lock: 1,
      heat: 0,
      shake: 0.35,
      rock: 0,
      bdy: -0.6
    },
    vars: poseVars,
    bake: bakePose
  };
  var love = {
    p: {
      esx: 0.86,
      esy: 1.28,
      tilt: -14,
      edy: -0.5,
      edx: -0.35,
      esx2: 0.05,
      esy2: 0.06,
      tilt2: 6,
      edy2: 0,
      lock: 1,
      heat: 0.6,
      shake: 0,
      rock: 0,
      bdy: -1.6
    },
    vars: poseVars,
    bake: bakePose,
    tint: (pal, p) => tintWith(pal, p, ROSE)
  };
  var shy = {
    p: {
      esx: 0.62,
      esy: 0.5,
      tilt: 10,
      edy: 1.4,
      edx: -0.2,
      esx2: -0.05,
      esy2: -0.04,
      tilt2: -8,
      edy2: 0,
      lock: 1,
      heat: 0.55,
      shake: 0,
      rock: 0,
      bdy: 0.9
    },
    vars: poseVars,
    bake: bakePose,
    tint: (pal, p) => tintWith(pal, p, BLUSH)
  };
  var sick = {
    p: {
      esx: 1.25,
      esy: 0.34,
      tilt: 20,
      edy: 1.8,
      edx: 0.8,
      esx2: 0.05,
      esy2: -0.05,
      tilt2: -6,
      edy2: 0,
      lock: 1,
      heat: 0.6,
      shake: 0.18,
      rock: 0,
      bdy: 1.4
    },
    vars: poseVars,
    bake: bakePose,
    tint: (pal, p) => tintWith(pal, p, BILE)
  };
  var thinking = {
    p: {
      esx: 1.15,
      esy: 0.62,
      tilt: 0,
      edy: 4.2,
      edx: 0.4,
      esx2: 0.02,
      esy2: 0.06,
      tilt2: 0,
      edy2: -8.4,
      lock: 1,
      heat: 0,
      shake: 0,
      rock: 0.8,
      bdy: -0.4
    },
    vars: poseVars,
    bake: bakePose
  };

  // ../packages/beans-blobatar/appearance.ts
  var BOT_AVATAR_STATES = ["idle", "thinking", "responding", "working", "waiting", "retry", "error"];
  var BOT_SHAPES = ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"];
  var BOT_EXPRESSIONS = [
    "idle",
    "happy",
    "sad",
    "mad",
    "surprised",
    "wink",
    "sleepy",
    "smug",
    "unsure",
    "scared",
    "love",
    "shy",
    "sick",
    "thinking"
  ];
  var BOT_BACKGROUNDS = ["none", "square", "circle", "squircle"];
  var BOT_TONES = ["pastel", "pale", "mid", "deep", "bright", "ink"];
  var SHAPE_POSITION = {
    round: 0.11,
    organic: 0.35,
    boxy: 0.54,
    capsule: 0.65,
    nub: 0.745,
    cloud: 0.825,
    droplet: 0.8875,
    hexagon: 0.9325,
    sun: 0.965,
    triangle: 0.99
  };
  var TONE_EDGES = [0.2, 0.36, 0.62, 0.8, 0.93, 1];
  var TONE_POSITION = { pastel: 0.1, pale: 0.28, mid: 0.49, deep: 0.71, bright: 0.865, ink: 0.965 };
  var toneName = (v) => BOT_TONES[TONE_EDGES.findIndex((edge) => v < edge)] ?? "pastel";
  var EXPRESSIONS = {
    idle,
    happy,
    sad,
    mad,
    surprised,
    wink,
    sleepy,
    smug,
    unsure,
    scared,
    love,
    shy,
    sick,
    thinking
  };
  var HEX = /^#[0-9A-F]{6}$/;
  var APPEARANCE_KEYS = ["shape", "expression", "background", "hue", "tone", "palette", "motion"];

  class LookError extends Error {
  }
  var fail = (path, message) => {
    throw new LookError(`${path} ${message}`);
  };
  var isRecord = (v) => {
    if (typeof v !== "object" || v === null || Array.isArray(v))
      return false;
    const proto = Object.getPrototypeOf(v);
    return proto === Object.prototype || proto === null;
  };
  var onlyKeys = (o, allowed, path) => {
    for (const k of Object.keys(o))
      if (!allowed.includes(k))
        fail(path, `has unknown field "${k}"`);
  };
  function oneOf(v, list, path) {
    if (typeof v !== "string" || !list.includes(v))
      fail(path, `must be one of ${list.join(", ")}`);
    return v;
  }
  function appearance(v, path) {
    if (!isRecord(v))
      return fail(path, "must be an object");
    onlyKeys(v, APPEARANCE_KEYS, path);
    const out = {};
    if ("shape" in v)
      out.shape = oneOf(v.shape, BOT_SHAPES, `${path}.shape`);
    if ("expression" in v)
      out.expression = oneOf(v.expression, BOT_EXPRESSIONS, `${path}.expression`);
    if ("background" in v)
      out.background = oneOf(v.background, BOT_BACKGROUNDS, `${path}.background`);
    if ("hue" in v) {
      const h = v.hue;
      if (typeof h !== "number" || !Number.isFinite(h) || h < 0 || h >= 360)
        fail(`${path}.hue`, "must be a finite number in [0, 360)");
      out.hue = h;
    }
    if ("tone" in v)
      out.tone = oneOf(v.tone, BOT_TONES, `${path}.tone`);
    if ("palette" in v) {
      const p = v.palette;
      if (!isRecord(p))
        return fail(`${path}.palette`, "must be an object");
      onlyKeys(p, ["head", "eye", "bg"], `${path}.palette`);
      const palette = {};
      for (const k of ["head", "eye", "bg"]) {
        if (!(k in p))
          continue;
        const c = p[k];
        if (typeof c !== "string" || !HEX.test(c))
          fail(`${path}.palette.${k}`, "must be uppercase #RRGGBB");
        palette[k] = c;
      }
      out.palette = palette;
    }
    if ("motion" in v) {
      if (typeof v.motion !== "boolean")
        fail(`${path}.motion`, "must be a boolean");
      out.motion = v.motion;
    }
    return out;
  }
  function validateBotLook(input) {
    try {
      if (!isRecord(input))
        fail("look", "must be an object");
      const v = input;
      onlyKeys(v, ["version", "base", "states"], "look");
      if (v.version !== 1)
        fail("look.version", "must be 1");
      const look = { version: 1, base: appearance(v.base, "look.base") };
      if ("states" in v) {
        const s = v.states;
        if (!isRecord(s))
          return fail("look.states", "must be an object");
        onlyKeys(s, BOT_AVATAR_STATES, "look.states");
        const states = {};
        for (const k of BOT_AVATAR_STATES)
          if (k in s)
            states[k] = appearance(s[k], `look.states.${k}`);
        look.states = states;
      }
      return { ok: true, look };
    } catch (e) {
      if (e instanceof LookError)
        return { ok: false, error: e.message };
      throw e;
    }
  }
  function effective(look, state) {
    if (state !== undefined && !BOT_AVATAR_STATES.includes(state))
      throw new TypeError(`unknown avatar state "${state}"`);
    if (look == null)
      return {};
    const checked = validateBotLook(look);
    if (!checked.ok)
      throw new TypeError(checked.error);
    const { base, states } = checked.look;
    const over = state ? states?.[state] : undefined;
    if (!over)
      return base;
    const merged = { ...base, ...over };
    if (base.palette || over.palette)
      merged.palette = { ...base.palette, ...over.palette };
    return merged;
  }
  function options(a) {
    const opts = {};
    if (a.shape)
      opts.traits = { shape: SHAPE_POSITION[a.shape] };
    if (a.hue !== undefined)
      opts.hue = a.hue;
    if (a.tone)
      opts.tone = TONE_POSITION[a.tone];
    if (a.palette)
      opts.palette = { ...a.palette };
    if (a.background && a.background !== "none")
      opts.background = a.background;
    if (a.expression && a.expression !== "idle")
      opts.expression = EXPRESSIONS[a.expression];
    return opts;
  }
  var upper = (p) => ({
    head: p.head.toUpperCase(),
    eye: p.eye.toUpperCase(),
    bg: p.bg.toUpperCase()
  });
  function prepare(botID, look, state) {
    const a = effective(look, state);
    const opts = options(a);
    const { t, palette } = resolve2(botID, opts);
    const layout = style.layout(t);
    const resolved = {
      shape: layout.shape,
      expression: a.expression ?? "idle",
      background: a.background ?? "none",
      hue: opts.hue ?? t.num("hue", 0, 360),
      tone: a.tone ?? toneName(t("tone")),
      palette: upper(palette),
      motion: a.motion ?? true
    };
    return { opts, t, palette, layout, resolved, expression: opts.expression };
  }
  function resolveBotAppearance(botID, look, state) {
    return prepare(botID, look, state).resolved;
  }
  function botAppearanceContrast(a) {
    const head = fromHex(a.palette.head);
    return {
      eyeOnHead: contrast(fromHex(a.palette.eye), head),
      headOnBg: contrast(head, fromHex(a.palette.bg))
    };
  }
  // ../packages/beans-blobatar/geometry.ts
  function botAvatarSVG(botID, look, state, options = {}) {
    const { opts } = prepare(botID, look, state);
    return blobatar(botID, { ...opts, size: options.size, title: options.title });
  }
  var CORE_SEGMENTS = 24;
  var EXTRA_SEGMENTS = 4;
  var BACKGROUND_SEGMENTS = 8;
  var PETAL_SLOTS = 9;
  function pathCubics(d) {
    const tokens = d.match(/[MCQLHVZ]|-?\d*\.?\d+(?:e[-+]?\d+)?/gi) ?? [];
    const out = [];
    let i = 0;
    let cur = [0, 0];
    let start = [0, 0];
    let cmd = "";
    const num = () => {
      const v = Number(tokens[i++]);
      if (!Number.isFinite(v))
        throw new Error(`malformed path data: ${d}`);
      return v;
    };
    const line = (to) => {
      if (to[0] === cur[0] && to[1] === cur[1])
        return;
      const dx = to[0] - cur[0];
      const dy = to[1] - cur[1];
      out.push([cur, [cur[0] + dx / 3, cur[1] + dy / 3], [cur[0] + 2 * dx / 3, cur[1] + 2 * dy / 3], to]);
      cur = to;
    };
    while (i < tokens.length) {
      if (/^[A-Za-z]$/.test(tokens[i]))
        cmd = tokens[i++];
      switch (cmd) {
        case "M":
          cur = start = [num(), num()];
          cmd = "L";
          break;
        case "C": {
          const c1 = [num(), num()];
          const c2 = [num(), num()];
          const to = [num(), num()];
          out.push([cur, c1, c2, to]);
          cur = to;
          break;
        }
        case "Q": {
          const q = [num(), num()];
          const to = [num(), num()];
          out.push([
            cur,
            [cur[0] + 2 / 3 * (q[0] - cur[0]), cur[1] + 2 / 3 * (q[1] - cur[1])],
            [to[0] + 2 / 3 * (q[0] - to[0]), to[1] + 2 / 3 * (q[1] - to[1])],
            to
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
  var at = (b, t) => {
    const s = 1 - t;
    return [
      s * s * s * b[0][0] + 3 * s * s * t * b[1][0] + 3 * s * t * t * b[2][0] + t * t * t * b[3][0],
      s * s * s * b[0][1] + 3 * s * s * t * b[1][1] + 3 * s * t * t * b[2][1] + t * t * t * b[3][1]
    ];
  };
  function slice(b, t0, t1) {
    const split = (c, t) => {
      const l = (p, q) => [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
      const ab = l(c[0], c[1]);
      const bc = l(c[1], c[2]);
      const cd = l(c[2], c[3]);
      const abc = l(ab, bc);
      const bcd = l(bc, cd);
      const m = l(abc, bcd);
      return [[c[0], ab, abc, m], [m, bcd, cd, c[3]]];
    };
    const tail = t0 > 0 ? split(b, t0)[1] : b;
    if (t1 >= 1)
      return tail;
    return split(tail, (t1 - t0) / (1 - t0))[0];
  }
  function normalizeContour(src, count, centre) {
    if (src.length === 0 || src.length > count)
      throw new Error(`contour has ${src.length} segments; need 1..${count}`);
    let area = 0;
    for (const b of src)
      area += b[0][0] * b[3][1] - b[3][0] * b[0][1];
    const pieces = area >= 0 ? src.slice() : src.slice().reverse().map((b) => [b[3], b[2], b[1], b[0]]);
    const angle = (p) => Math.atan2(p[1] - centre[1], p[0] - centre[0]);
    const sweep = (from, to) => {
      const d = to - from;
      return d - 2 * Math.PI * Math.floor(d / (2 * Math.PI));
    };
    const span = (b) => {
      const a0 = angle(b[0]);
      const s = sweep(a0, angle(b[3]));
      return s > Math.PI && sweep(a0, angle(at(b, 0.5))) > s ? 0 : s;
    };
    while (pieces.length < count) {
      let widest = 0;
      let most = -1;
      pieces.forEach((b, i) => {
        const s = span(b);
        if (s > most + 0.000000000001) {
          most = s;
          widest = i;
        }
      });
      const b = pieces[widest];
      let t = 0.5;
      if (most > 0.000000001) {
        const a0 = angle(b[0]);
        let lo = 0;
        let hi = 1;
        for (let k = 0;k < 60; k++) {
          const mid = (lo + hi) / 2;
          if (sweep(a0, angle(at(b, mid))) < most / 2)
            lo = mid;
          else
            hi = mid;
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
      if (dist < best - 0.000000001) {
        best = dist;
        first = i;
      }
    });
    const ordered = [...pieces.slice(first), ...pieces.slice(0, first)];
    return {
      start: [ordered[0][0][0], ordered[0][0][1]],
      segments: ordered.map((p) => [p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]])
    };
  }
  var collapsed = (count, c) => ({
    start: [c[0], c[1]],
    segments: Array.from({ length: count }, () => [c[0], c[1], c[0], c[1], c[0], c[1]])
  });
  function botAvatarGeometry(botID, look, state) {
    const { opts, t, palette, layout, resolved, expression } = prepare(botID, look, state);
    const p = tinted2(palette, expression);
    const centre = [layout.body.cx, layout.body.cy];
    const drawn = marks(layout, p);
    const petals = [];
    for (const m of drawn)
      if (m.kind === "circle")
        petals.push({ cx: m.cx, cy: m.cy, r: m.r });
    if (petals.length > PETAL_SLOTS)
      throw new Error(`${petals.length} petals exceed ${PETAL_SLOTS} slots`);
    while (petals.length < PETAL_SLOTS)
      petals.push({ cx: centre[0], cy: centre[1], r: 0 });
    const coreD = layout.draw ? layout.draw(layout.body) : superellipse(layout.body);
    const extraD = layout.extra[0];
    if (layout.extra.length > 1)
      throw new Error("more than one extra outline");
    const bg = backdrop(style, opts, p);
    const bgD = bg?.d ?? superellipse({ cx: 50, cy: 50, rx: 50, ry: 50, n: 2 });
    const eye = (i) => {
      const e = layout.eyes[i];
      return { cx: e.cx, cy: e.cy, rx: e.rx, ry: e.ry, n: e.n, rot: e.rot };
    };
    const head = p.head.toUpperCase();
    return {
      version: 1,
      background: {
        path: normalizeContour(pathCubics(bgD), BACKGROUND_SEGMENTS, [50, 50]),
        fill: resolved.palette.bg,
        opacity: bg ? 1 : 0
      },
      core: normalizeContour(pathCubics(coreD), CORE_SEGMENTS, centre),
      extra: extraD ? normalizeContour(pathCubics(extraD), EXTRA_SEGMENTS, centre) : collapsed(EXTRA_SEGMENTS, centre),
      petals,
      eyes: [eye(0), eye(1)],
      pose: { ...expression?.p ?? IDENT },
      palette: { head, eye: p.eye.toUpperCase(), bg: resolved.palette.bg },
      motion: resolved.motion,
      seeds: motionSeeds(t)
    };
  }
  // ../packages/beans-blobatar/vendor/react-native/worklets.ts
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

  // ../packages/beans-blobatar/frame.ts
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

  // ../packages/beans-blobatar/index.ts
  function botAvatarSeed(botID) {
    return botID;
  }
  function blobatar2(botID, options) {
    return blobatar(botAvatarSeed(botID), options);
  }

  // ../packages/beans-blobatar/jsc.ts
  if (typeof globalThis.TextEncoder === "undefined") {
    globalThis.TextEncoder = class {
      encode(text) {
        const bytes = [];
        for (let i = 0;i < text.length; i++) {
          let cp = text.charCodeAt(i);
          if (cp >= 55296 && cp <= 56319) {
            const next = text.charCodeAt(i + 1);
            if (next >= 56320 && next <= 57343) {
              cp = 65536 + (cp - 55296 << 10) + next - 56320;
              i++;
            } else
              cp = 65533;
          } else if (cp >= 56320 && cp <= 57343)
            cp = 65533;
          if (cp < 128)
            bytes.push(cp);
          else if (cp < 2048)
            bytes.push(192 | cp >> 6, 128 | cp & 63);
          else if (cp < 65536)
            bytes.push(224 | cp >> 12, 128 | cp >> 6 & 63, 128 | cp & 63);
          else
            bytes.push(240 | cp >> 18, 128 | cp >> 12 & 63, 128 | cp >> 6 & 63, 128 | cp & 63);
        }
        return Uint8Array.from(bytes);
      }
    };
  }
  Object.assign(globalThis, exports_beans_blobatar);
})();
