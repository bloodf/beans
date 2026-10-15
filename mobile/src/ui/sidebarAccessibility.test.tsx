import { expect, mock, test } from "bun:test";
import type { Chat } from "../core/model";

// Mount the production components through the existing native-tag fixture pattern.
// Native focus order and VoiceOver/TalkBack announcements still require device proof.
type Node = { type: any; props: any };
const jsx = (type: any, props: any): Node => typeof type === "function" ? type(props) : { type, props };
mock.module("react", () => ({
  memo: (component: any) => component,
  useRef: (value: any) => ({ current: value }),
  useEffect: () => {},
  useSyncExternalStore: (_subscribe: any, snapshot: any) => snapshot(),
}));
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx }));
mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx }));
mock.module("react-native", () => ({
  Pressable: "Pressable", Text: "Text", View: "View", TextInput: "TextInput",
  StyleSheet: { create: (value: any) => value },
}));
mock.module("react-native-reanimated", () => ({
  default: { View: "AnimatedView" }, Easing: { linear: "linear" },
  useSharedValue: (value: any) => ({ value }), useAnimatedStyle: (compute: any) => compute,
  withRepeat: (value: any) => value, withTiming: (value: any) => value,
}));
mock.module("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ bottom: 0 }) }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en" }] }));
mock.module("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
mock.module("./Avatar", () => ({ AvatarCluster: "Avatar" }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ Font: { body: 16 }, usePalette: () => ({}) }));
mock.module("./Composer", () => ({ Surface: "Surface" }));
mock.module("./KeyboardFoot", () => ({ KeyboardFoot: "KeyboardFoot" }));
const { ChatRow } = await import("./ChatRow");
const { SidebarSearch } = await import("./SidebarSearch");
const { setAppLanguage, t } = await import("../i18n");
const { lastActivity, preview, stamp } = await import("./format");
function find(node: any, type: string): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
  if (!node || typeof node !== "object") return;
  return node.type === type ? node : find(node.props?.children, type);
}
const chat: Chat = {
  id: "chat", kind: "group", bot_ids: [], is_pinned: false, created_at: 1, unread_count: 1000,
  messages: [{ id: "message", chat_id: "chat", author: { kind: "you" }, body: { kind: "text", text: "Latest message" }, state: { kind: "sent" }, created_at: 2 }],
};

test("mounted chat rows expose one localized button label and preserve selection, unread and responding semantics", () => {
  try {
    for (const language of ["en", "zh"] as const) {
      setAppLanguage(language);
      for (const selected of [undefined, false, true]) {
        for (const responding of [false, true]) {
          let presses = 0, longPresses = 0;
          const row = ChatRow({ chat, bots: new Map(), title: "Team chat", working: true, responding, selected,
            onPress: () => { presses++; }, onLongPress: () => { longPresses++; } }) as Node;
          expect(row.type).toBe("Pressable");
          expect(row.props.accessible).toBe(true);
          expect(row.props.accessibilityRole).toBe("button");
          expect(row.props.accessibilityState).toEqual(selected === undefined ? undefined : { selected });
          expect(row.props.accessibilityLabel).toBe([
            "Team chat", stamp(new Date(lastActivity(chat) * 1000)), responding ? t("Working…") : preview(chat, new Map()), t("{count} unread", { count: 1000 }),
          ].join(", "));
          expect(row.props.accessibilityLabel.split(t("{count} unread", { count: 1000 }))).toHaveLength(2);
          expect(find(row, "Avatar")!.props.working).toBe(true);
          expect(find(row, "Symbol")?.props.name).toBe(selected === undefined ? "chevron.right" : undefined);
          row.props.onPress(); row.props.onLongPress();
          expect([presses, longPresses]).toEqual([1, 1]);
          // The badge remains visually capped, but no longer owns a second spoken label.
          const badge = find(row, "View")!.props.children[1].props.children[1];
          expect(badge.props.children.props.children).toBe("999+");
          expect(badge.props.accessibilityLabel).toBeUndefined();
        }
      }
      const read = ChatRow({ chat: { ...chat, unread_count: 0, is_pinned: true }, bots: new Map(), title: "Team chat", working: false, responding: false, onPress: () => {} }) as Node;
      expect(read.props.accessibilityLabel).toContain(t("Pinned"));
      expect(read.props.accessibilityLabel).not.toContain(t("{count} unread", { count: 0 }));
    }
  } finally { setAppLanguage("en"); }
});

test("mounted sidebar search retains its localized name with empty and nonempty input and preserves clear", () => {
  try {
    for (const language of ["en", "zh"] as const) {
      setAppLanguage(language);
      for (const value of ["", "needle"]) {
        const changes: string[] = [];
        const root = SidebarSearch({ value, placeholder: t("Search"), onChangeText: text => changes.push(text) }) as Node;
        const input = find(root, "TextInput")!;
        expect(input.props.accessibilityLabel).toBe(t("Search"));
        expect(input.props.placeholder).toBe(t("Search"));
        expect(input.props.value).toBeUndefined();
        input.props.onChangeText("typed");
        const clear = find(root, "Pressable");
        expect(!!clear).toBe(!!value);
        if (clear) {
          expect(clear.props.accessibilityRole).toBe("button");
          expect(clear.props.accessibilityLabel).toBe(t("Clear"));
          clear.props.onPress();
        }
        expect(changes).toEqual(value ? ["typed", ""] : ["typed"]);
      }
    }
  } finally { setAppLanguage("en"); }
});
