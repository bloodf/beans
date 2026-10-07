import { afterEach, beforeAll, beforeEach, expect, test } from "bun:test";
import { AvatarClock } from "./avatarClock";
import type { AvatarScheduler } from "./avatarClock";
import type { watchAvatarVisibility } from "./avatarVisibility";

let watch: typeof watchAvatarVisibility;
const names = ["window", "document", "location", "IntersectionObserver"] as const;
const originals = new Map<string, PropertyDescriptor | undefined>();
for (const name of names) originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
let intersection: (entries: IntersectionObserverEntry[]) => void;
let focused = true, reduced = false, visible = true;
let fakeWindow: EventTarget, fakeDocument: EventTarget, fakeMedia: EventTarget;
const cleanups: (() => void)[] = [];

beforeAll(async () => {
  Object.defineProperty(globalThis, "location", { configurable: true, value: { search: "?platform=linux" } });
  // Import after initializing location: the real browser host reads it at module load.
  watch = (await import("./avatarVisibility")).watchAvatarVisibility;
  const original = originals.get("location");
  if (original) Object.defineProperty(globalThis, "location", original);
  else Reflect.deleteProperty(globalThis, "location");
});
beforeEach(() => {
  focused = visible = true; reduced = false;
  fakeMedia = new EventTarget();
  Object.defineProperty(fakeMedia, "matches", { get: () => reduced });
  fakeWindow = Object.assign(new EventTarget(), { matchMedia: () => fakeMedia });
  fakeDocument = Object.assign(new EventTarget(), { hasFocus: () => focused });
  Object.defineProperty(fakeDocument, "visibilityState", { get: () => visible ? "visible" : "hidden" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: fakeWindow });
  Object.defineProperty(globalThis, "document", { configurable: true, value: fakeDocument });
  Object.defineProperty(globalThis, "IntersectionObserver", { configurable: true, value: class {
    constructor(callback: typeof intersection) { intersection = callback; }
    observe() {} unobserve() {} disconnect() {}
  } });
});
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  for (const name of names) {
    const original = originals.get(name);
    if (original) Object.defineProperty(globalThis, name, original);
    else Reflect.deleteProperty(globalThis, name);
  }
});

function fixture() {
  let next = 1, callbacks = 0;
  const scheduled = new Map<number, FrameRequestCallback>();
  const scheduler: AvatarScheduler = { request: (callback) => { const id = next++; scheduled.set(id, callback); return id; }, cancel: (id) => { scheduled.delete(id); } };
  const clock = new AvatarClock(scheduler);
  const element = {} as Element;
  let stop: (() => void) | undefined;
  const cleanup = watch(element, (onScreen, reduce) => {
    if (onScreen && !reduce) stop ??= clock.subscribe(() => { callbacks++; });
    else { stop?.(); stop = undefined; }
  });
  cleanups.push(() => { stop?.(); cleanup(); });
  const intersect = (onScreen: boolean) => intersection([{ target: element, isIntersecting: onScreen, intersectionRatio: onScreen ? 1 : 0 } as IntersectionObserverEntry]);
  const tick = () => {
    const pending = [...scheduled]; scheduled.clear();
    for (const [, callback] of pending) callback(16);
  };
  return { scheduled, intersect, tick, calls: () => callbacks };
}

test("offscreen, hidden documents, background windows and Reduce Motion each leave zero live frame callbacks", () => {
  const f = fixture();
  expect(f.scheduled.size).toBe(0);
  f.intersect(true); f.tick();
  expect(f.calls()).toBe(1);
  f.intersect(false); f.tick();
  expect(f.scheduled.size).toBe(0); expect(f.calls()).toBe(1);
  f.intersect(true);
  visible = false; fakeDocument.dispatchEvent(new Event("visibilitychange")); f.tick();
  expect(f.scheduled.size).toBe(0); expect(f.calls()).toBe(1);
  visible = true; fakeDocument.dispatchEvent(new Event("visibilitychange"));
  focused = false; fakeWindow.dispatchEvent(new Event("blur")); f.tick();
  expect(f.scheduled.size).toBe(0); expect(f.calls()).toBe(1);
  focused = true; fakeWindow.dispatchEvent(new Event("focus"));
  reduced = true; fakeMedia.dispatchEvent(new Event("change")); f.tick();
  expect(f.scheduled.size).toBe(0); expect(f.calls()).toBe(1);
  reduced = false; fakeMedia.dispatchEvent(new Event("change")); f.tick();
  expect(f.calls()).toBe(2);
});
