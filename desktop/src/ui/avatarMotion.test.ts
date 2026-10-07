import { expect, test } from "bun:test";
import { botAvatarGeometry } from "@beans/blobatar";
import { avatarFrame, avatarMorphProgress, interpolateAvatarGeometry } from "@beans/blobatar/frame";
import { AvatarMotion } from "./avatarMotion";

test("an interrupted silhouette transition starts at the displayed geometry rather than its obsolete target", () => {
  const a = botAvatarGeometry("retarget", { version: 1, base: { shape: "round" } });
  const b = botAvatarGeometry("retarget", { version: 1, base: { shape: "sun", expression: "happy" } });
  const c = botAvatarGeometry("retarget", { version: 1, base: { shape: "droplet", expression: "sad" } });
  const motion = new AvatarMotion(a);
  motion.retarget(b, 0, true);
  const midway = interpolateAvatarGeometry(a, b, avatarMorphProgress(150, b).t);
  const shown = motion.frame(150, true);
  expect(shown.core).toEqual(midway.core);
  motion.retarget(c, 150, true);
  expect(motion.frame(150, true).core).toEqual(shown.core);
  expect(motion.frame(1000, true)).toEqual(avatarFrame(c, 1000, 1));
});

test("Reduce Motion, offscreen rendering and appearance motion off snap every geometry/pose/color channel to the target", () => {
  const a = botAvatarGeometry("motion", { version: 1, base: { shape: "cloud", palette: { head: "#123456" } } });
  const b = botAvatarGeometry("motion", { version: 1, base: { shape: "triangle", expression: "mad", background: "square", palette: { head: "#ABCDEF" } } });
  const motion = new AvatarMotion(a);
  motion.retarget(b, 0, true);
  motion.frame(50, true);
  expect(motion.frame(60, false)).toEqual(avatarFrame(b, 60, 0));
  const disabled = botAvatarGeometry("motion", { version: 1, base: { shape: "sun", motion: false } });
  motion.retarget(disabled, 70, true);
  expect(motion.frame(70, true)).toEqual(avatarFrame(disabled, 70, 0));
});
