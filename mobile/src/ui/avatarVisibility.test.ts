import { expect, test } from "bun:test";
import { AvatarViewability, avatarIntersectsViewport, notifyAvatarScroll, subscribeAvatarScroll } from "./avatarVisibility";

// A recycled/draw-distance cell stays mounted. Key-based viewability (not mount state or index)
// is what lets its renderer's callback run, and losing viewability must notify the subscriber.
test("viewability removes old and recycled row keys without waking unchanged rows", () => {
  const tracker = new AvatarViewability();
  const changes: [boolean, boolean][] = [];
  const unsubscribe = tracker.subscribe(() => changes.push([tracker.has("old"), tracker.has("new")]));
  expect(tracker.has("old")).toBe(false);
  tracker.update(["old"]);
  tracker.update(["old"]);
  tracker.update(["new"]);
  tracker.update([]);
  expect(changes).toEqual([[true, false], [false, true], [false, false]]);
  unsubscribe();
  tracker.update(["old"]);
  expect(changes).toEqual([[true, false], [false, true], [false, false]]);
});

test("native measurement stops fully offscreen avatars on every edge, including zero-area and edge contact", () => {
  expect(avatarIntersectsViewport(20, 20, 40, 40, 400, 800)).toBe(true);
  expect(avatarIntersectsViewport(-20, 20, 40, 40, 400, 800)).toBe(true);
  expect(avatarIntersectsViewport(-40, 20, 40, 40, 400, 800)).toBe(false);
  expect(avatarIntersectsViewport(400, 20, 40, 40, 400, 800)).toBe(false);
  expect(avatarIntersectsViewport(20, -40, 40, 40, 400, 800)).toBe(false);
  expect(avatarIntersectsViewport(20, 800, 40, 40, 400, 800)).toBe(false);
  expect(avatarIntersectsViewport(20, 20, 0, 40, 400, 800)).toBe(false);
});

test("scroll visibility subscriptions stop requesting native measurements after unmount", () => {
  let checks = 0;
  const unsubscribe = subscribeAvatarScroll(() => { ++checks; });
  notifyAvatarScroll();
  expect(checks).toBe(1);
  unsubscribe();
  notifyAvatarScroll();
  expect(checks).toBe(1);
});
