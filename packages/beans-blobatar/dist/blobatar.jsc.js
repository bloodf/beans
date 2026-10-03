/* Blobatar 2.7.0; Copyright (c) 2026 Alain; MIT: see ../LICENSE; upstream a7fd546ebede49d0a9fa638945b9e534489782a2. */
(() => {
  // beans-wt/beans/packages/beans-blobatar/vendor/core/color.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/shape.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/hash.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/traits.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/render.ts
  function posed(l, opts, animate) {
    const e = opts.expression;
    if (animate || !e)
      return { l, wrap: "" };
    return e.bake(l, e.p);
  }
  var tinted = (p, e) => e?.tint ? e.tint(p, e.p) : p;
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
      const p = tinted(palette, opts.expression);
      const dim = opts.size ? ` width="${opts.size}" height="${opts.size}"` : "";
      const pose = posed(style.layout(t), opts);
      const body = label(opts) + plate(backdrop(style, opts, p)) + wrap(style.render(pose.l, p), pose.wrap);
      return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"${dim}>${body}</svg>`;
    };
  }

  // beans-wt/beans/packages/beans-blobatar/vendor/core/styles/compose.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/styles/shapes.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/styles/blob.ts
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

  // beans-wt/beans/packages/beans-blobatar/vendor/core/blobatar.ts
  var blobatar = makeBlobatar(style);

  // beans-wt/beans/packages/beans-blobatar/index.ts
  function botAvatarSeed(botID) {
    return botID;
  }
  function blobatar2(botID, options) {
    return blobatar(botAvatarSeed(botID), options);
  }

  // beans-wt/beans/packages/beans-blobatar/jsc.ts
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
  globalThis.blobatar = blobatar2;
})();
