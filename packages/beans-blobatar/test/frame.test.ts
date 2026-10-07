import { describe, expect, test } from "bun:test";
import { BOT_EXPRESSIONS, avatarFrame, botAvatarGeometry, type AvatarMatrix } from "../index";
import { idleAt, idleTransforms } from "../vendor/core/idle";
import { idleFrame } from "../vendor/react-native/worklets";

/** Test-only oracle: an SVG transform list as a matrix. */
function parse(list: string): AvatarMatrix {
  let m: AvatarMatrix = [1, 0, 0, 1, 0, 0];
  const mul = (a: AvatarMatrix, b: AvatarMatrix): AvatarMatrix => [
    a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
  ];
  for (const [, op, args] of list.matchAll(/(\w+)\(([^)]*)\)/g)) {
    const n = args!.trim().split(/[\s,]+/).map(Number);
    if (op === "translate") m = mul(m, [1, 0, 0, 1, n[0]!, n[1] ?? 0]);
    else if (op === "scale") m = mul(m, [n[0]!, 0, 0, n[1] ?? n[0]!, 0, 0]);
    else if (op === "rotate") {
      const r = (n[0]! * Math.PI) / 180;
      m = mul(m, [Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0]);
    } else throw new Error(op);
  }
  return m;
}
const close = (a: number[], b: number[], eps: number) => a.every((v, i) => Math.abs(v - b[i]!) < eps);

describe("avatarFrame against upstream idleTransforms", () => {
  test("amp 1 composes the same tree as the pinned idle layer", () => {
    for (const id of ["lead", "6f1c2a90-bot", "zz-41"]) {
      for (const expression of BOT_EXPRESSIONS) {
        const g = botAvatarGeometry(id, { version: 1, base: { expression } });
        for (const t of [0, 140, 777, 1450, 2999, 4100, 6200, 9001]) {
          const f = idleAt(g.seeds, t, 1, g.pose.shake);
          const want = idleTransforms({ eyes: g.eyes }, g.pose, f);
          const body = parse(`${want.root} ${want.breathe} ${want.bob}`);
          const got = avatarFrame(g, t, 1);
          expect(close(got.body, body, 2e-3)).toBe(true);
          for (const i of [0, 1] as const) {
            const eye = parse(`${want.root} ${want.breathe} ${want.bob} ${want.eyes} ${want.eye[i]} ${want.glance[i]}`);
            expect(close(got.eyeMatrices[i], eye, 2e-3)).toBe(true);
          }
        }
      }
    }
  });

  test("vendored worklet idleFrame is a faithful transcription of idleAt", () => {
    const g = botAvatarGeometry("lead");
    for (let t = 0; t < 20000; t += 37) {
      for (const amp of [0, 0.3, 1]) expect(idleFrame(g.seeds, t, amp, 0.4)).toEqual(idleAt(g.seeds, t, amp, 0.4));
    }
  });

  test("amp ramps the ambient layer continuously", () => {
    const g = botAvatarGeometry("lead", { version: 1, base: { expression: "mad" } });
    const a = avatarFrame(g, 1000, 0.5);
    const lo = avatarFrame(g, 1000, 0);
    const hi = avatarFrame(g, 1000, 1);
    expect(a.body[5]).toBeGreaterThanOrEqual(Math.min(lo.body[5], hi.body[5]) - 1e-9);
    expect(a.body[5]).toBeLessThanOrEqual(Math.max(lo.body[5], hi.body[5]) + 1e-9);
    expect(avatarFrame(g, 1000, -3)).toEqual(lo);
    expect(avatarFrame(g, 1000, 7)).toEqual(hi);
  });
});
