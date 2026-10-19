import { expect, mock, test } from "bun:test";
import type { Chat, ChatUsage } from "../core/model";

// Exercise the real Details page and store reducer without native layout or network calls.
const jsx = (type: unknown, props: any) => ({ type, props });
const react = {
  useState: (value: unknown) => [value, () => {}], useEffect: () => {},
  useMemo: (compute: () => unknown) => compute(), useRef: (value: unknown) => ({ current: value }),
  useCallback: (fn: unknown) => fn, useDebugValue: () => {},
  useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
};
mock.module("react", () => ({ ...react, default: react }));
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
mock.module("react-native", () => ({
  Alert: {}, Platform: { OS: "ios" }, Pressable: "Pressable", ScrollView: "ScrollView",
  Switch: "Switch", Text: "Text", View: "View", StyleSheet: { create: (value: unknown) => value },
}));
mock.module("expo-router", () => ({ Stack: { Screen: "Screen" }, useLocalSearchParams: () => ({ id: "chat" }), useRouter: () => ({}) }));
mock.module("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en" }] }));
mock.module("../core/engine", () => ({ chatTitle: (chat: Chat) => chat.title, engine: {} }));
mock.module("./avatarVisibility", () => ({ notifyAvatarScroll: () => {} }));
mock.module("./Avatar", () => ({ AvatarCluster: "AvatarCluster", BotAvatar: "BotAvatar" }));
mock.module("./forms", () => ({ Section: "Section", Row: "Row", CheckRow: "CheckRow", FieldRow: "FieldRow", ToggleRow: "ToggleRow" }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ usePalette: () => ({}) }));
mock.module("./navigation", () => ({ CloseToolbar: "CloseToolbar" }));
const { useStore, setChatUsage } = await import("../core/store");
const { setAppLanguage } = await import("../i18n");
const { default: ChatInfoScreen } = await import("../../app/chat-info/[id]");

function rows(node: any): any[] {
  if (Array.isArray(node)) return node.flatMap(rows);
  if (!node || typeof node !== "object") return [];
  return node.type === "Row" ? [node.props] : rows(node.props?.children);
}
const usage: ChatUsage = {
  context_tokens: 128_000, context_window: 1_000_000, input_tokens: 200_000,
  output_tokens: 1_000, cache_read_tokens: 30_000, cost_usd: 0.42, cost_known: true,
  turns: 18, model: "wire-model", updated_at: 1,
};

test("Details consumes stored usage totals without confirming unknown or partial cost", () => {
  const chat: Chat = { id: "chat", kind: "group", title: "Chat", bot_ids: [], is_pinned: false, created_at: 1, messages: [], unread_count: 0 };
  useStore.setState({ chats: [chat], bots: [] });
  expect(rows(ChatInfoScreen()).some(row => row.title === "Spent")).toBe(false);
  setChatUsage("chat", usage);
  const rendered = () => rows(ChatInfoScreen());
  const spent = () => rendered().find(row => row.title === "Spent")?.subtitle;
  expect(rendered().find(row => row.title === "Context")?.subtitle).toBe("128,000 / 1,000,000 · 13%");
  expect(spent()).toBe("$0.42 · 18 turns");
  setChatUsage("chat", { ...usage, cost_known: true, cost_usd: 0, turns: 1 });
  expect(spent()).toBe("$0.00 · 1 turn");
  for (const cost_known of [false, null, undefined]) {
    setChatUsage("chat", { ...usage, cost_known, cost_usd: 0 });
    expect(spent()).toBe("Unknown cost · 18 turns");
    setChatUsage("chat", { ...usage, cost_known });
    expect(spent()).toBe("$0.42 known subtotal · total unknown · 18 turns");
  }
  setChatUsage("chat", { ...usage, context_window: 0, cost_usd: 0.001 });
  expect(rendered().find(row => row.title === "Context")?.subtitle).toBe("128,000 tokens");
  expect(spent()).toBe("<$0.01 · 18 turns");
  setAppLanguage("zh");
  setChatUsage("chat", { ...usage, cost_known: false, cost_usd: 0 });
  expect(rendered().find(row => row.title === "已花费")?.subtitle).toBe("费用未知 · 18 轮");
  setAppLanguage("en");
  useStore.setState({ chats: [] });
  expect(ChatInfoScreen()).toBeNull();
});
