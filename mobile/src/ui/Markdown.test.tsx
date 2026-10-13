import { expect, mock, test } from "bun:test";

// Replace only the unavailable runtime boundaries; both production adapters run below.
let platform = "ios";
let fontScale = 1;
const calls: [string, number, number, number][] = [];
const failure = new Error("measurement failed");
let fail = false;
const native: { measure?: (text: string, width: number, body: number, code: number) => { width: number; height: number } } = {};
const measure = (text: string, width: number, body: number, code: number) => {
  calls.push([text, width, body, code]);
  if (fail) throw failure;
  // Distinct opaque native dimensions: no measurement algorithm is copied into this test.
  return { width: 173.25, height: 91.5 };
};
native.measure = measure;
mock.module("expo-modules-core", () => ({ requireNativeModule: () => native, requireNativeViewManager: () => "MarkdownView" }));
mock.module("react", () => ({ useMemo: (compute: () => unknown) => compute() }));
mock.module("react/jsx-runtime", () => ({ jsx: (type: unknown, props: unknown) => ({ type, props }), jsxs: (type: unknown, props: unknown) => ({ type, props }) }));
mock.module("react-native", () => ({
  Platform: { get OS() { return platform; } },
  useWindowDimensions: () => ({ width: 400, height: 800, scale: 2, fontScale }),
  processColor: (color: unknown) => color,
  PixelRatio: { getFontScale: () => fontScale },
}));
mock.module("./theme", () => ({ Font: { message: 16, code: 14 }, usePalette: () => ({ link: "link" }) }));
const { Markdown } = await import("./Markdown");
const { measureMarkdown } = await import("../../modules/beans-core/MarkdownView");
const render = (text: string, maxWidth = 280, size = 16) => Markdown({ text, maxWidth, size, color: "text" }).props;

test("production Markdown measurement preserves dimensions, exact inputs, scaling, bounded reuse and errors", () => {
  const first = render("hello");
  expect(first.style).toEqual({ width: 173.25, height: 91.5 });
  expect(calls.at(-1)).toEqual([first.markdown, first.maxWidth, first.fontSize, first.codeFontSize]);
  const count = calls.length;
  expect(render("hello").style).toBe(first.style);
  expect(calls.length).toBe(count);
  render("hello ");
  render("hello", 279.5);
  render("hello", 280, 17);
  measureMarkdown("hello", 280, 16, 15);
  expect(calls.length).toBe(count + 4);

  fontScale = 1.5;
  const scaled = render("hello");
  expect(scaled.fontSize).toBe(24);
  expect(scaled.codeFontSize).toBe(21);
  expect(calls.at(-1)).toEqual([scaled.markdown, scaled.maxWidth, scaled.fontSize, scaled.codeFontSize]);
  fontScale = 1;
  expect(render("hello").style).toBe(first.style);

  // Fill capacity, refresh one entry, then distinguish LRU eviction from FIFO.
  const oldest = render("evict-oldest").style;
  for (let i = 0; i < 127; i++) render(`entry-${i}`);
  render("entry-0");
  render("overflow");
  const beforeHot = calls.length;
  render("entry-0");
  expect(calls.length).toBe(beforeHot);
  expect(render("evict-oldest").style).not.toBe(oldest);

  const long = "x".repeat(16_385);
  const beforeLong = calls.length;
  render(long);
  render(long);
  expect(calls.length).toBe(beforeLong + 2);
  const boundary = "x".repeat(16_384);
  const boundarySize = render(boundary).style;
  const beforeBoundary = calls.length;
  expect(render(boundary).style).toBe(boundarySize);
  expect(calls.length).toBe(beforeBoundary);

  fail = true;
  expect(() => render("error-path")).toThrow(failure);
  expect(() => render("error-path")).toThrow(failure);
  fail = false;
  expect(render("error-path").style).toEqual(first.style);

  platform = "android";
  delete native.measure;
  fontScale = 2;
  const android = render("android-native-layout");
  expect(android.style).toBeUndefined();
  expect(android.fontSize).toBe(16);
  expect(android.codeFontSize).toBe(14);
  native.measure = measure;
  const beforeRetry = calls.length;
  const androidMeasured = render("android-native-layout");
  expect(calls.length).toBe(beforeRetry + 1);
  const beforeScaleChange = calls.length;
  fontScale = 1.5;
  const androidScaled = render("android-native-layout");
  expect(androidScaled.fontSize).toBe(16);
  expect(androidScaled.codeFontSize).toBe(14);
  expect(calls.length).toBe(beforeScaleChange + 1);
  fontScale = 2;
  expect(render("android-native-layout").style).toBe(androidMeasured.style);
});
