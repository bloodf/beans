import { expect, mock, test } from "bun:test";

// Drive the hook's actual external-store seam, not a second implementation of its cache.
let scheme: "light" | "dark" | null = "light";
const platform = { OS: "android" };
let nativeCalls = 0;
let generation = 0;
let failColor = false;
const appState = { currentState: "active" };
const listeners = new Set<(state: string) => void>();
const consumers: { render: () => void; stop?: () => void }[] = [];
let rendering: (typeof consumers)[number];
mock.module("react", () => ({
  useSyncExternalStore: (subscribe: (notify: () => void) => () => void, snapshot: () => number) => {
    if (!rendering.stop) rendering.stop = subscribe(rendering.render);
    return snapshot();
  },
}));
mock.module("react-native", () => ({
  Platform: platform,
  PlatformColor: (name: string) => { ++nativeCalls; return { semantic: name }; },
  useColorScheme: () => scheme,
  AppState: Object.assign(appState, {
    addEventListener: (_: string, listener: (state: string) => void) => {
      listeners.add(listener);
      return { remove: () => listeners.delete(listener) };
    },
  }),
}));
mock.module("expo-router", () => ({ Color: { android: { dynamic: new Proxy({}, {
  get: (_, role) => {
    ++nativeCalls;
    if (failColor) throw new Error("native color unavailable");
    return `${generation}:${String(role)}`;
  },
}) } } }));

function state(value: string) {
  appState.currentState = value;
  for (const listener of listeners) listener(value);
}

test("production palettes reuse semantic colors and refresh every mounted Android consumer on foreground", async () => {
  let { usePalette } = await import("./theme");
  function mount() {
    let palette: ReturnType<typeof usePalette>;
    const consumer = { render: () => { rendering = consumer; palette = usePalette(); }, stop: undefined as (() => void) | undefined };
    consumers.push(consumer);
    consumer.render();
    return { read: () => palette!, render: consumer.render, stop: () => consumer.stop!() };
  }
  const first = mount();
  const light = first.read();
  const calls = nativeCalls;
  const second = mount();
  first.render();
  expect(second.read()).toBe(light);
  expect(nativeCalls).toBe(calls);
  expect(light.label).toBe("0:onSurface");
  expect(light.userBubble).toBe("0:primary");
  expect(listeners.size).toBe(1);
  scheme = "dark";
  first.render(); second.render();
  const dark = first.read();
  expect(dark.dark).toBe(true);
  expect(second.read()).toBe(dark);
  expect(dark).not.toBe(light);
  scheme = null;
  first.render(); second.render();
  expect(first.read()).toBe(light);
  const beforeForeground = nativeCalls;
  state("active"); state("inactive"); state("background");
  expect(nativeCalls).toBe(beforeForeground);
  ++generation;
  state("active");
  expect(first.read()).not.toBe(light);
  expect(second.read()).toBe(first.read());
  expect(first.read().label).toBe("1:onSurface");
  const refreshedCalls = nativeCalls;
  state("active");
  expect(nativeCalls).toBe(refreshedCalls);
  scheme = "dark";
  first.render(); second.render();
  expect(first.read()).not.toBe(dark);
  expect(first.read().label).toBe("1:onSurface");
  first.stop();
  expect(listeners.size).toBe(1);
  second.stop();
  expect(listeners.size).toBe(0);
  state("background"); ++generation; state("active");
  const remounted = mount();
  expect(remounted.read().label).toBe("2:onSurface");
  remounted.stop();
  failColor = true;
  expect(() => mount()).toThrow("native color unavailable");
  consumers.at(-1)!.stop!();
  failColor = false;
  const recovered = mount();
  expect(recovered.read().label).toBe("2:onSurface");
  recovered.stop();

  platform.OS = "ios";
  scheme = "light";
  const iosModule = "./theme.ts?ios";
  ({ usePalette } = await import(iosModule));
  const ios = mount();
  const iosLight = ios.read();
  const iosCalls = nativeCalls;
  ios.render();
  expect(ios.read()).toBe(iosLight);
  expect(nativeCalls).toBe(iosCalls);
  expect(iosLight.label).toEqual({ semantic: "label" });
  expect(iosLight.tint).toEqual({ semantic: "systemBlue" });
  scheme = "dark";
  ios.render();
  expect(ios.read().codeFade).toEqual(["rgb(20,20,22)", "rgba(20,20,22,0)"]);
  scheme = "light";
  ios.render();
  expect(ios.read()).toBe(iosLight);
  state("background"); state("active");
  expect(ios.read()).toBe(iosLight);
  expect(listeners.size).toBe(0);
  ios.stop();
});
