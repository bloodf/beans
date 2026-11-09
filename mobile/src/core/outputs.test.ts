import { expect, test, vi } from "vitest";
import type { Chat, Message, Output } from "./model";

vi.doMock("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
vi.doMock("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
vi.doMock("react-native", () => ({
  Alert: {},
  Linking: {},
  View: "View",
  Text: "Text",
  Pressable: "Pressable",
  ScrollView: "ScrollView",
  Modal: "Modal",
  Platform: { OS: "ios" },
  StyleSheet: { create: <T,>(value: T) => value, hairlineWidth: 1, absoluteFill: {} },
}));
vi.doMock("react-native-reanimated", () => ({
  default: { View: "AnimatedView" },
  useSharedValue: <T,>(value: T) => ({ value }),
  useAnimatedStyle: <T,>(fn: () => T) => fn,
  withSpring: <T,>(value: T) => value,
  withSequence: () => 1,
  withTiming: <T,>(value: T) => value,
}));
vi.doMock("expo-haptics", () => ({ ImpactFeedbackStyle: { Light: "light" }, impactAsync: () => Promise.resolve() }));
vi.doMock("expo-clipboard", () => ({ setStringAsync: vi.fn() }));
vi.doMock("expo-linear-gradient", () => ({ LinearGradient: "Gradient" }));
vi.doMock("expo-image", () => ({ Image: "Image" }));
vi.doMock("expo-router", () => ({ useRouter: () => ({ push: () => {} }) }));
vi.doMock("../../modules/beans-core/ShimmerView", () => ({ ShimmerView: "Shimmer" }));
vi.doMock("./engine", () => ({ engine: { fetchFile: () => { throw new Error("output helper tests must not fetch files"); } } }));
vi.doMock("../i18n", () => ({ useLanguage: () => {}, t: (value: string) => value, language: "en", tc: (value: string) => value }));
vi.doMock("../ui/attachments", () => ({ AttachmentBlock: "Attachments" }));
vi.doMock("../ui/Avatar", () => ({ BotAvatar: "Avatar", ContactAvatar: "ContactAvatar" }));
vi.doMock("../ui/Markdown", () => ({ Markdown: "Markdown" }));
vi.doMock("../ui/Symbol", () => ({ Symbol: "Symbol" }));
vi.doMock("../ui/layout", () => ({ usePaneWidth: () => 400 }));
vi.doMock("../ui/theme", () => ({ Font: { message: 16, small: 13, caption: 12 }, usePalette: () => ({ dark: false }), accentColor: () => "#000" }));
vi.doMock("react-native-gesture-handler", () => ({ Gesture: { Pan: () => ({ runOnJS() { return this; }, enabled() { return this; }, activeOffsetX() { return this; }, failOffsetX() { return this; }, failOffsetY() { return this; }, onTouchesDown() { return this; }, onUpdate() { return this; }, onEnd() { return this; }, onFinalize() { return this; } }) }, GestureDetector: "Detector" }));

// These cases import UI modules but never render native views or fetch files.
// Unexpected native actions fail instead of accepting a vacuous async mock.
const nativeAction = () => { throw new Error("output helper tests must not invoke native actions"); };
vi.doMock("expo-file-system", () => ({ File: class { constructor() { nativeAction(); } } }));
vi.doMock("expo-sharing", () => ({ shareAsync: nativeAction }));
vi.doMock("expo-web-browser", () => ({ openBrowserAsync: nativeAction }));
vi.doMock("../../modules/beans-core", () => ({ canPreviewFiles: false, previewFile: nativeAction }));
vi.doMock("../ui/Pressable", () => ({ Pressable: "Pressable" }));
vi.doMock("../ui/AgentLogo", () => ({ AgentLogo: "AgentLogo" }));
vi.doMock("../ui/forms", () => ({ Row: "Row" }));
vi.doMock("../ui/alert", () => ({ alert: nativeAction }));

const { documentUrl, evidenceStatus, groupOutputs, outputKind, outputSymbol, outputTypeName } = await import("./model");
const { OUTPUT_CARD_OPENS_DETAIL } = await import("../ui/outputs");
const { buildRows, isOutputCard } = await import("../ui/transcript");
const { markFile, markFileError, removeChat, removeMessage, useStore } = await import("./store");

function version(id: string, n: number, minutesAgo: number, output: Partial<Output> = {}): Message {
  return {
    id: `${id}-${n}`,
    chat_id: "chat",
    author: { kind: "bot", bot_id: "bot" },
    body: { kind: "text", text: "" },
    state: { kind: "complete" },
    created_at: 1_760_000_000 - minutesAgo * 60,
    output: { id, name: `${id}.txt`, mime: "text/plain", bot_id: "bot", version: n, ...output },
  };
}

test("versions group under their output, newest first, the latest published output first", () => {
  const plain: Message = { ...version("x", 1, 0), output: undefined };
  const series = groupOutputs([version("report", 1, 30), version("chart", 1, 20), version("report", 2, 5), plain, version("report", 2, 5)]);
  expect(series.map((s) => s.id)).toEqual(["report", "chart"]);
  expect(series[0].versions.map((m) => m.output!.version)).toEqual([2, 1]);
});

test("a check reads as one word, and only an https link opens", () => {
  expect(evidenceStatus({ kind: "test_result", summary: "", status: "unverified" })).toBe("Not verified");
  expect(outputSymbol({ id: "o", name: "n", mime: "image/png", bot_id: "b", version: 1 })).toBe("photo");
  expect(documentUrl({ id: "o", name: "n", mime: "text/html", bot_id: "b", version: 1, url: "https://docs.example.com/a" })).toBe("https://docs.example.com/a");
  for (const url of ["http://example.com", "javascript:alert(1)", "https://user:secret@example.com", "file:///etc/passwd"]) {
    expect(documentUrl({ id: "o", name: "n", mime: "text/html", bot_id: "b", version: 1, url })).toBeUndefined();
  }
});

test("a failed fetch is kept until the bytes land, and listed outputs follow removals", () => {
  markFileError("att", "the relay no longer has it");
  expect(useStore.getState().fileErrors.att).toBe("the relay no longer has it");
  markFile("att", "file:///a");
  expect(useStore.getState().fileErrors.att).toBeUndefined();

  useStore.setState({ outputs: { chat: [version("report", 1, 3), version("report", 2, 1)] } });
  removeMessage("chat", "report-1");
  expect(useStore.getState().outputs.chat.map((m) => m.id)).toEqual(["report-2"]);
  removeChat("chat");
  expect(useStore.getState().outputs.chat).toBeUndefined();
});

test("an output says what kind of file it is, for its card's icon and type", () => {
  const output = (name: string, mime: string, url?: string): Output => ({ id: "o", name, mime, bot_id: "b", version: 1, url });
  expect(outputKind(output("mygo-site/index.html", "text/html"))).toBe("code");
  expect(outputSymbol(output("mygo-site/index.html", "text/html"))).toBe("chevron.left.forwardslash.chevron.right");
  expect(outputTypeName(output("mygo-site/index.html", "text/html"))).toBe("HTML");
  expect(outputKind(output("preview-full.png", "image/png"))).toBe("image");
  expect(outputKind(output("results.csv", "text/csv"))).toBe("table");
  expect(outputKind(output("build.tar.gz", "application/gzip"))).toBe("archive");
  expect(outputKind(output("site.v2/notes", "text/plain"))).toBe("text");
  expect(outputTypeName(output("site.v2/notes", "text/plain"))).toBe("PLAIN");
  expect(outputTypeName(output("blob", "application/octet-stream"))).toBe("");
  expect(outputKind(output("Report", "text/html", "https://www.example.com/r"))).toBe("link");
  expect(outputTypeName(output("Report", "text/html", "https://www.example.com/r"))).toBe("example.com");
});
test("output cards open the delivered detail route", () => {
  expect(OUTPUT_CARD_OPENS_DETAIL).toBe(true);
});

test("back-to-back outputs from one bot join into one compact list", () => {
  const first = version("a", 1, 3, { name: "a.html", mime: "text/html" });
  const second = version("b", 1, 2, { name: "b.png", mime: "image/png" });
  const third = version("c", 1, 1, { name: "c.csv", mime: "text/csv" });
  const chat: Chat = {
    id: "chat",
    kind: "dm",
    bot_ids: ["bot"],
    is_pinned: false,
    created_at: 1,
    unread_count: 0,
    messages: [first, second, third],
  };
  expect(isOutputCard(undefined)).toBe(false);
  expect(isOutputCard(first)).toBe(true);
  expect(isOutputCard({ ...first, author: { kind: "you" } })).toBe(false);
  const rows = buildRows(chat, new Map(), [], false, null).filter((row) => row.type === "message");
  expect(rows).toHaveLength(3);
  expect(rows.map((row) => [row.stackAbove, row.stackBelow])).toEqual([
    [false, true],
    [true, true],
    [true, false],
  ]);
});
