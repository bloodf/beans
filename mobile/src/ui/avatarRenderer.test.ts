import { describe, expect, mock, test } from "bun:test";
import * as React from "react";
import type { ReactElement, ReactNode } from "react";
import { botAppearanceContrast, botAvatarGeometry, resolveBotAppearance } from "@beans/blobatar";
import { avatarFrame, avatarPath, type AvatarGeometry } from "@beans/blobatar/frame";
import { createAvatarClock, type AvatarVisibility } from "./avatarClock";
import { avatarGeometry } from "./avatarGeometry";
import { displayedGeometry, frameAt } from "./avatarMotion";
import { resolvedContrastWarnings } from "../core/look";

// Bun has no native/UI runtime. This executes the actual AvatarFigure hooks, effects, callback
// and animated-prop worklets against a counted native host; shared geometry/math are NOT mocked.
// Device rasterization and worklet thread scheduling are separately unverified.
const originalReact = { ...React };
interface Slot { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void }
interface Host {
  slots: Slot[];
  cursor: number;
  effects: (() => void)[];
  reactions: (() => void)[];
  active: boolean;
  registered: number;
  paints: number;
  tick?: (info: { timestamp: number }) => void;
}
let host: Host;
function freshHost(): Host { return { slots: [], cursor: 0, effects: [], reactions: [], active: false, registered: 0, paints: 0 }; }
function slot(): Slot {
  const index = host.cursor++;
  return host.slots[index] ?? (host.slots[index] = {});
}
function changed(previous: readonly unknown[] | undefined, next: readonly unknown[] | undefined) {
  return !previous || !next || previous.length !== next.length || next.some((value, i) => value !== previous[i]);
}
function useMemo<T>(make: () => T, deps: readonly unknown[]): T {
  const item = slot();
  if (changed(item.deps, deps)) { item.value = make(); item.deps = deps; }
  return item.value as T; // This slot is initialized only by the generic factory above.
}
function useRef<T>(value: T) { return useMemo(() => ({ current: value }), []); }
function useEffect(effect: () => void | (() => void), deps?: readonly unknown[]) {
  const item = slot();
  if (!changed(item.deps, deps)) return;
  item.deps = deps;
  host.effects.push(() => { item.cleanup?.(); item.cleanup = effect() ?? undefined; });
}
mock.module("react", () => ({ ...originalReact, useMemo, useRef, useEffect, useCallback: (callback: unknown, deps: readonly unknown[]) => useMemo(() => callback, deps) }));
mock.module("react-native-svg", () => ({ default: "Svg", G: "G", Path: "Path", Circle: "Circle" }));
mock.module("react-native-reanimated", () => ({
  default: { createAnimatedComponent: (component: string) => component },
  createAnimatedPropAdapter: (adapt: (props: Record<string, unknown>) => void) => adapt,
  processColor: (hex: string) => (0xff000000 | parseInt(hex.slice(1), 16)) >>> 0,
  useSharedValue: <T,>(initial: T) => useMemo(() => {
    let current = initial;
    return { get value() { return current; }, set value(next: T) { current = next; for (const reaction of host.reactions) reaction(); } };
  }, []),
  useAnimatedReaction: (select: () => unknown, react: (next: unknown) => void) => {
    const item = slot();
    if (item.value) return;
    let previous: unknown;
    const run = () => { const next = select(); if (next !== previous) { previous = next; react(next); } };
    item.value = run;
    host.reactions.push(run);
    run();
  },
  useAnimatedProps: (make: () => Record<string, unknown>, _deps?: unknown, adapter?: (props: Record<string, unknown>) => void) => {
    slot();
    return { read() { const result = make(); adapter?.(result); return result; } };
  },
  useFrameCallback: (tick: (info: { timestamp: number }) => void, autostart: boolean) => {
    const captured = host;
    const callback = useMemo(() => ({ callbackId: -1, setActive(active: boolean) { captured.active = active; } }), []);
    captured.tick = tick;
    useEffect(() => {
      ++captured.registered;
      callback.callbackId = 1;
      captured.active = autostart;
      return () => { --captured.registered; captured.active = false; callback.callbackId = -1; };
    }, []);
    return callback;
  },
}));
// Native modules must be mocked before loading this component (a module-loading boundary test).
const { AvatarFigure } = await import("./AvatarFigure");

const visible: AvatarVisibility = { appActive: true, focused: true, reduceMotion: false, visible: true, motion: true };
const round = botAvatarGeometry("renderer-bot", { version: 1, base: { shape: "round", expression: "idle" } });
const sun = botAvatarGeometry("renderer-bot", { version: 1, base: { shape: "sun", expression: "happy", background: "square", palette: { head: "#DD1122" } } });
function render(geometry: AvatarGeometry, visibility = visible): ReactElement {
  host.cursor = 0;
  const tree = AvatarFigure({ geometry, visibility, size: 72 });
  const pending = host.effects.splice(0);
  for (const effect of pending) effect();
  return tree;
}
function advance(timestamp: number) {
  if (!host.active) return;
  ++host.paints;
  host.tick?.({ timestamp });
}
function nativeProps(tree: ReactNode, type: string): Record<string, unknown>[] {
  if (!React.isValidElement<{ children?: ReactNode; animatedProps?: { read(): Record<string, unknown> } }>(tree)) return [];
  // Render the actual Petal/Eye children too. They use only the animated-prop hooks above.
  if (typeof tree.type === "function") {
    const component = tree.type as (props: unknown) => ReactNode;
    return nativeProps(component(tree.props), type);
  }
  const own = tree.type === type && tree.props.animatedProps ? [tree.props.animatedProps.read()] : [];
  return [...own, ...React.Children.toArray(tree.props.children).flatMap((child) => nativeProps(child, type))];
}
function unmount() { for (const item of host.slots) item.cleanup?.(); }

describe("actual AvatarFigure callback and native props", () => {
  test.each(["appActive", "focused", "visible", "motion", "reduceMotion"] as const)("%s deactivates the registered callback and snaps to the full target", (gate) => {
    host = freshHost();
    render(round);
    advance(1000);
    const paused = { ...visible, [gate]: gate === "reduceMotion" };
    const tree = render(sun, paused);
    expect(host.active).toBe(false);
    const count = host.paints;
    advance(1300);
    advance(10000);
    expect(host.paints).toBe(count);
    const paths = nativeProps(tree, "Path");
    expect(paths[0].opacity).toBe(1);
    expect(paths[2].d).toBe(avatarPath(avatarFrame(sun, 0, 0).core));
    expect(paths[2].fill).toEqual({ type: 0, payload: 0xffdd1122 });
    expect(nativeProps(tree, "G")[0].matrix).toEqual(avatarFrame(sun, 0, 0).body);
    const groups = nativeProps(tree, "G");
    const still = avatarFrame(sun, 0, 0);
    expect(groups[1].matrix).toEqual(still.eyeMatrices[0]);
    expect(groups[2].matrix).toEqual(still.eyeMatrices[1]);
    expect(paths[3].d).toBe(avatarPath(still.eyes[0]));
    expect(paths[4].d).toBe(avatarPath(still.eyes[1]));
    expect(nativeProps(tree, "Circle").map((props) => ({ cx: props.cx, cy: props.cy, r: props.r }))).toEqual(still.petals);
    unmount();
    expect(host.registered).toBe(0);
    expect(host.active).toBe(false);
  });

  test("idle frames update native matrices; targets morph and interrupted retargets keep the last painted contour", () => {
    host = freshHost();
    let tree = render(round);
    advance(1000);
    const first = nativeProps(tree, "G")[0].matrix;
    advance(1800);
    expect(nativeProps(tree, "G")[0].matrix).not.toEqual(first);
    tree = render(sun);
    advance(1900);
    advance(2020);
    const painted = nativeProps(tree, "Path")[2].d;
    expect(painted).not.toBe(avatarPath(round.core));
    expect(painted).not.toBe(avatarPath(sun.core));
    tree = render(round);
    // No paint yet: a second retarget must not advance or overwrite the displayed geometry.
    tree = render(sun);
    advance(2040);
    expect(nativeProps(tree, "Path")[2].d).toBe(painted);
    advance(2500);
    expect(nativeProps(tree, "Path")[2].d).toBe(avatarPath(sun.core));
    unmount();
    expect(host.registered).toBe(0);
  });

  test("motion off from mount starts no active callback, including on a shape change", () => {
    host = freshHost();
    render(round, { ...visible, motion: false });
    advance(1000);
    render(sun, { ...visible, motion: false });
    advance(2000);
    expect(host.paints).toBe(0);
    expect(host.active).toBe(false);
    unmount();
    expect(host.registered).toBe(0);
  });
});

test("clock updates are idempotent and disposal prevents later activation", () => {
  const calls: boolean[] = [];
  const clock = createAvatarClock((active) => calls.push(active));
  clock.update(visible);
  clock.update(visible);
  clock.update({ ...visible, motion: false });
  clock.update(visible);
  clock.dispose();
  clock.update(visible);
  clock.dispose();
  expect(calls).toEqual([true, false, true, false]);
});

test("geometry cache keeps bot identity and appearance/state independent", () => {
  const look = { version: 1 as const, base: { shape: "sun" as const }, states: { error: { shape: "triangle" as const } } };
  const a = avatarGeometry("cache-a", look, "idle");
  expect(avatarGeometry("cache-a", structuredClone(look), "idle")).toBe(a);
  expect(avatarGeometry("cache-a", look, "error").core).not.toEqual(a.core);
  expect(avatarGeometry("cache-b", look, "idle").seeds).not.toEqual(a.seeds);
});

test("amp zero eliminates every ambient transform regardless of elapsed time", () => {
  const mad = botAvatarGeometry("shake", { version: 1, base: { expression: "mad" } });
  expect(frameAt(mad, mad, 0, 1000, 0)).toEqual(frameAt(mad, mad, 0, 100000, 0));
  const part = displayedGeometry(round, sun, 120);
  expect(displayedGeometry(part, round, 0)).toEqual(part);
  expect(displayedGeometry(part, round, 400)).toBe(round);
});

test("shared resolved contrast sees chosen-versus-generated pairs and honors transparent backgrounds", () => {
  const base = { version: 1 as const, base: { palette: { head: "#808080", eye: "#7F7F7F" } } };
  let appearance = resolveBotAppearance("contrast", base);
  expect(resolvedContrastWarnings(appearance, botAppearanceContrast(appearance))).toEqual(["eye"]);
  appearance = resolveBotAppearance("contrast", { version: 1, base: { background: "circle", palette: { head: "#FFFFFF", eye: "#000000", bg: "#FEFEFE" } } });
  expect(resolvedContrastWarnings(appearance, botAppearanceContrast(appearance))).toEqual(["bg"]);
  appearance = { ...appearance, background: "none" };
  expect(resolvedContrastWarnings(appearance, botAppearanceContrast(appearance))).toEqual([]);
});
