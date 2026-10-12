import { expect, mock, test } from "bun:test";

// Hook slots/native tags follow Composer.test.tsx. Mount the production screen;
// drive its keyboard and layout callbacks, without native layout or IME claims.
type Node = { type: any; props: any };
let slots: any[] = [], cursor = 0;
const slot = (make: () => any) => {
  const index = cursor++;
  if (!(index in slots)) slots[index] = make();
  return slots[index];
};
const react = {
  useCallback: (callback: any) => callback,
  useRef: (initial: any) => slot(() => ({ current: initial })),
  useState: (initial: any) => {
    const item = slot(() => ({ value: initial }));
    return [item.value, (next: any) => { item.value = next; }];
  },
  useMemo: (compute: any) => compute(),
  useEffect: () => {},
  forwardRef: (render: any) => (props: any) => render(props, null),
};
mock.module("react", () => ({ ...react, default: react }));
const jsx = (type: any, props: any) => ({ type, props });
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
mock.module("react-native", () => ({ Platform: { OS: "ios" }, View: "View", Text: "Text", Pressable: "Pressable", Alert: {}, PixelRatio: { get: () => 2 }, StyleSheet: { create: (value: any) => value } }));
mock.module("expo-linear-gradient", () => ({ LinearGradient: "LinearGradient" }));
mock.module("expo-router", () => ({ Stack: { Screen: "Screen", Title: "Title", Toolbar: Object.assign("Toolbar", { Button: "Button" }) }, useLocalSearchParams: () => ({ id: "chat" }), useRouter: () => ({}), useFocusEffect: () => {} }));
mock.module("expo-router/react-navigation", () => ({ useHeaderHeight: () => 100 }));
mock.module("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ top: 0, bottom: 34 }) }));
let keyboard: any;
let prepare: () => number, reactToTarget: (value: number) => void;
const shared: Array<{ value: any; writes: number }> = [];
mock.module("react-native-reanimated", () => ({
  default: { View: "AnimatedView" }, runOnJS: (callback: any) => callback,
  useSharedValue: (initial: any) => slot(() => {
    let value = initial;
    const item = { get value() { return value; }, set value(next) { item.writes++; value = next; }, writes: 0 };
    shared.push(item);
    return item;
  }),
  useAnimatedReaction: (read: any, respond: any) => { prepare = read; reactToTarget = respond; },
  useAnimatedStyle: (compute: any) => compute(), ZoomIn: {}, ZoomOut: {},
}));
mock.module("react-native-keyboard-controller", () => ({ KeyboardChatScrollView: "KeyboardChatScrollView", KeyboardController: {}, useKeyboardHandler: (handlers: any) => { keyboard = handlers; } }));
mock.module("../../modules/beans-core/SoftScrollEdgeView", () => ({ SoftScrollEdgeView: "SoftScrollEdgeView" }));
mock.module("./AvatarFlashList", () => ({ AvatarFlashList: "List" }));
mock.module("../core/engine", () => ({ chatTitle: () => "Chat", engine: {} }));
mock.module("../core/store", () => ({ useChat: () => ({ id: "chat", kind: "group", bot_ids: [], messages: [] }), useBotMap: () => new Map(), useWorkingBots: () => [], useIsWorking: () => false, useRunningTasks: () => [], useStore: (select: any) => select({ statuses: {}, identityId: "account", relayUrl: "relay", deviceId: "phone", paired: true }) }));
mock.module("../i18n", () => ({ t: (text: string) => text, useLanguage: () => ({ language: "en" }) }));
mock.module("./Avatar", () => ({ AvatarCluster: "AvatarCluster" }));
mock.module("./Composer", () => ({ Composer: "Composer", Surface: "Surface" }));
mock.module("./KeyboardFoot", () => ({ KeyboardFoot: "KeyboardFoot" }));
mock.module("./layout", () => ({ useWide: () => false }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ usePalette: () => ({}) }));
mock.module("./format", () => ({ quoteText: (text: string) => text }));
mock.module("./AnswerSheet", () => ({ AnswerSheet: "AnswerSheet" }));
mock.module("./transcript", () => ({ buildRows: () => [], DayRow: "DayRow", MarkerRow: "MarkerRow", MessageRow: "MessageRow", quoteAuthorName: () => "Bot", NoticeRow: "NoticeRow", PermissionRow: "PermissionRow", CommandRow: "CommandRow", StatusRow: "StatusRow", WorkingRow: "WorkingRow" }));
const { default: ChatScreen } = await import("../../app/(main)/chat/[id]");
function find(node: any, predicate: (node: Node) => boolean): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, predicate)).find(Boolean);
  if (!node || typeof node !== "object") return;
  return predicate(node) ? node : find(node.props?.children, predicate);
}

test("production composer publisher preserves finite guess before layout, then compact and expanded measurements", () => {
  slots = []; cursor = 0; shared.length = 0;
  // Passive effects are not needed for geometry; do not start transcript timers or send lifecycle work.
  const root = ChatScreen();
  const list = find(root, node => node.type === "List")!;
  const scroll = list.props.renderScrollComponent({}).props;
  const composer = find(root, node => node.type === "KeyboardFoot" && !!node.props.onLayout)!;
  const target = shared.find(item => item.value === prepare())!;
  const flushReaction = () => reactToTarget(prepare());
  const layout = (height: number) => { composer.props.onLayout({ nativeEvent: { layout: { height } } }); flushReaction(); };
  expect(prepare()).toBe(104);
  keyboard.onStart({ height: 300 });
  keyboard.onEnd({ height: 300 });
  flushReaction();
  expect(prepare()).toBe(104);
  expect(target.writes).toBe(0);
  expect(scroll.extraContentPadding.value).toBe(104);
  expect(Number.isFinite(scroll.blankSpace.value)).toBe(true);

  layout(80); // 80 + 14 gap - 34 safe area = 60 compact, plus 36 guessed growth.
  expect(prepare()).toBe(96);
  expect(scroll.extraContentPadding.value).toBe(96);
  keyboard.onStart({ height: 300 });
  layout(128); // Expanded height publishes 108, but padding waits for keyboard rest.
  expect(prepare()).toBe(108);
  expect(scroll.extraContentPadding.value).toBe(96);
  keyboard.onEnd({ height: 300 });
  flushReaction();
  expect(scroll.extraContentPadding.value).toBe(108);
  keyboard.onStart({ height: 0 });
  layout(80);
  expect(prepare()).toBe(108); // Compact measurement retains learned 48 pt focus growth.
  keyboard.onEnd({ height: 0 });
  flushReaction();
  expect(scroll.extraContentPadding.value).toBe(108);
  layout(140);
  expect(prepare()).toBe(120);
  expect(scroll.extraContentPadding.value).toBe(120);
});
