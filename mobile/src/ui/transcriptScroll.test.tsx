import { expect, mock, test } from "bun:test";
import type { Chat, Message } from "../core/model";
import { fileURLToPath } from "node:url";

// Module mocks live only in a dedicated Bun host, never in the full-suite process.
if (process.env.BEANS_TRANSCRIPT_SCROLL_HOST === "1") {
  installTranscriptScenario();
} else {
  test("transcript scroll consumer in an isolated Bun host", () => {
    const child = Bun.spawnSync([process.execPath, "test", "--isolate", fileURLToPath(import.meta.url)], {
      env: { ...process.env, BEANS_TRANSCRIPT_SCROLL_HOST: "1" },
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(child.exitCode, child.stdout.toString() + child.stderr.toString()).toBe(0);
  });
}

function installTranscriptScenario() {
// No renderer is installed in mobile; retain its existing hook-slot/native-tag boundary.
// These scenarios exercise production callbacks, not native layout or React scheduling.

type ScrollEvent = { nativeEvent: { contentOffset: { y: number }; contentSize: { height: number }; layoutMeasurement: { height: number } } };
type Props = {
  blankSpace?: { value: number };
  renderScrollComponent: (props: Record<string, never>, ref: null) => Node;
  children?: unknown;
  data?: Array<{ key: string }>;
  ref?: { current: unknown };
  onLayout: (event: { nativeEvent: { layout: { height: number } } }) => void;
  onContentSizeChange: (width: number, height: number) => void;
  onScroll: (event: ScrollEvent) => void;
  onScrollBeginDrag: () => void;
  onScrollEndDrag: () => void;
  onMomentumScrollBegin: () => void;
  onMomentumScrollEnd: () => void;
  onScrollToTop: () => void;
  onPress: () => void;
  onSend: (text: string, files: [], mentions: []) => Promise<boolean>;
  accessibilityLabel?: string;
};
type Node = { type: unknown; props: Props };
let slots: unknown[] = [], cursor = 0;
const effects: Array<() => void> = [];
function slot<T>(initial: () => T) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = initial();
  // Stable hook positions own their original in-process values.
  return { index, value: slots[index] as T };
}
const react = {
  useRef: <T,>(value: T) => slot(() => ({ current: value })).value,
  useState: <T,>(initial: T) => {
    const state = slot(() => initial);
    return [state.value, (value: T | ((previous: T) => T)) => {
      // React owns the value at this stable hook position.
      const previous = slots[state.index] as T;
      slots[state.index] = typeof value === "function" ? (value as (previous: T) => T)(previous) : value;
    }];
  },
  useMemo: <T,>(compute: () => T, deps: unknown[]) => {
    const state = slot<{ deps?: unknown[]; value?: T }>(() => ({})).value;
    const previous = state.deps;
    if (!previous || deps.some((value, index) => !Object.is(value, previous[index]))) {
      state.deps = deps;
      state.value = compute();
    }
    // The first call always computes before returning.
    return state.value as T;
  },
  useCallback: <T,>(callback: T, deps: unknown[]): T => react.useMemo(() => callback, deps),
  useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
    const state = slot<{ deps?: unknown[]; cleanup?: () => void }>(() => ({})).value;
    if (!state.deps || deps.some((value, index) => !Object.is(value, state.deps![index]))) {
      effects.push(() => { state.cleanup?.(); state.deps = deps; state.cleanup = effect() || undefined; });
    }
  },
  forwardRef: (component: unknown) => component,
  useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
  useDebugValue: () => {},
};
mock.module("react", () => ({ ...react, default: react }));
const scrolls: Array<{ kind: string; offset?: number }> = [];
let offset = 0, tail = 300;
let renderedRows: Array<{ key: string }> = [];
const list = {
  scrollToOffset: ({ offset: target }: { offset: number }) => { scrolls.push({ kind: "offset", offset: target }); },
  scrollToEnd: () => { scrolls.push({ kind: "end" }); },
  getAbsoluteLastScrollOffset: () => offset,
  getWindowSize: () => ({ height: 800 }),
  getChildContainerDimensions: () => ({ height: tail }),
  getFirstItemOffset: () => 0,
  getLayout: (index: number) => renderedRows[index]?.key === message.id ? { y: 0, isHeightMeasured: true } : undefined,
};
const jsx = (type: unknown, props: Props): Node => {
  if (type === "List" && props.ref) {
    props.ref.current = list;
    renderedRows = props.data ?? [];
  }
  return { type, props };
};
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
const platform = { OS: "android" };
mock.module("react-native", () => ({ Platform: platform, View: "View", Text: "Text", Pressable: "Pressable", Alert: { alert: () => {} }, PixelRatio: { get: () => 1 }, StyleSheet: { create: (value: unknown) => value, hairlineWidth: 1 } }));
mock.module("expo-router", () => ({ Stack: Object.assign("Stack", { Screen: "Screen", Title: "Title", Toolbar: Object.assign("Toolbar", { Button: "Button" }) }), useLocalSearchParams: () => ({ id: "chat" }), useRouter: () => ({}), useFocusEffect: () => {} }));
mock.module("expo-router/react-navigation", () => ({ useHeaderHeight: () => 64 }));
mock.module("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ top: 24, bottom: 0 }) }));
mock.module("react-native-keyboard-controller", () => ({ KeyboardChatScrollView: "Scroll", KeyboardController: { dismiss: () => {} }, useKeyboardHandler: () => {} }));
mock.module("react-native-reanimated", () => ({ default: { View: "AnimatedView" }, useSharedValue: (value: number) => slot(() => ({ value })).value, useAnimatedStyle: () => ({}), useAnimatedReaction: () => {}, runOnJS: (callback: unknown) => callback, ZoomIn: { duration: () => ({}) }, ZoomOut: { duration: () => ({}) } }));
mock.module("expo-linear-gradient", () => ({ LinearGradient: "Gradient" }));
mock.module("../../modules/beans-core/SoftScrollEdgeView", () => ({ SoftScrollEdgeView: "Edge" }));
mock.module("./AvatarFlashList", () => ({ AvatarFlashList: "List" }));
mock.module("./Avatar", () => ({ AvatarCluster: "Avatar" }));
mock.module("./Composer", () => ({ Composer: "Composer", Surface: "Surface" }));
mock.module("./KeyboardFoot", () => ({ KeyboardFoot: "Foot" }));
mock.module("./layout", () => ({ useWide: () => false }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ usePalette: () => ({}) }));
mock.module("./format", () => ({ quoteText: (text: string) => text }));
mock.module("./AnswerSheet", () => ({ AnswerSheet: "Answer" }));
mock.module("../i18n", () => ({ t: (text: string) => text, useLanguage: () => ({ language: "en" }) }));
const message: Message = { id: "sent", chat_id: "chat", author: { kind: "you" }, body: { kind: "text", text: "sent" }, state: { kind: "complete" }, created_at: 0 };
let chat: Chat = { id: "chat", kind: "dm", bot_ids: [], messages: [message], unread_count: 0, is_pinned: false, created_at: 0 };
mock.module("../core/prefs", () => ({ savePrefs: () => {}, loadPrefs: () => ({}), coreHome: () => "unused", pathOf: (value: string) => value, wipePrefs: () => {} }));
const { useStore }: typeof import("../core/store") = require("../core/store");
const { composerDraft, editComposerDraft, submitComposerDraft }: typeof import("../core/composerDraft") = require("../core/composerDraft");
let sendReply: (() => Promise<Message>) | undefined;
mock.module("../core/engine", () => ({ chatTitle: () => "Chat", engine: { sendMessage: async () => {
  if (sendReply) return sendReply();
  chat = { ...chat, messages: [...chat.messages, message] };
  useStore.setState({ chats: [chat] });
  return message;
}, loadOlder: async () => {} } }));
mock.module("./transcript", () => ({ buildRows: (current: Chat) => current.messages.map(row => ({ key: row.id, type: "message", message: row })), quoteAuthorName: () => "You", DayRow: "Day", MarkerRow: "Marker", MessageRow: "Message", NoticeRow: "Notice", PermissionRow: "Permission", CommandRow: "Command", StatusRow: "Status", WorkingRow: "Working" }));
// Native mocks must be installed before this production consumer loads.
const { default: ChatScreen }: typeof import("../../app/(main)/chat/[id]") = require("../../app/(main)/chat/[id]");
function find(node: unknown, type: unknown): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
  if (!node || typeof node !== "object" || !("type" in node) || !("props" in node)) return;
  // The JSX factory above supplies every node in this tree.
  const element = node as Node;
  return element.type === type ? element : find(element.props.children, type);
}

test.each(["ios", "android"])("%s transcript yields streaming scroll to reader and anchors acknowledged sends", async os => {
  platform.OS = os; slots = []; cursor = 0; effects.length = 0; scrolls.length = 0; tail = 300; offset = 0; sendReply = undefined;
  useStore.setState({ identityId: "account", relayUrl: "relay", deviceId: "phone", paired: true, chats: [chat] });
  chat.messages = [{ ...message, id: "earlier" }];
  renderedRows = [];
  const originalTimeout = globalThis.setTimeout, originalClear = globalThis.clearTimeout, originalFrame = globalThis.requestAnimationFrame;
  const timers = new Map<number, () => void>(); let timerId = 0;
  // Native timer handles are replaced only within this deterministic consumer scenario.
  const timeout = ((callback: () => void) => { const id = ++timerId; timers.set(id, callback); return id; }) as unknown as typeof setTimeout;
  const clear = ((id: number) => { timers.delete(id); }) as unknown as typeof clearTimeout;
  globalThis.setTimeout = timeout; globalThis.clearTimeout = clear;
  globalThis.requestAnimationFrame = callback => { callback(0); return 0; };
  try {
    let root: unknown;
    const render = () => { cursor = 0; root = ChatScreen(); for (const effect of effects.splice(0)) effect(); };
    render();
    const edge = find(root, "Edge")!.props;
    const events = find(root, "List")!.props;
    edge.onLayout({ nativeEvent: { layout: { height: 800 } } });
    events.onContentSizeChange(400, 1200);
    events.onScrollBeginDrag(); scrolls.length = 0;
    const scroll = (y: number) => { offset = y; events.onScroll({ nativeEvent: { contentOffset: { y }, contentSize: { height: 1200 }, layoutMeasurement: { height: 800 } } }); };
    scroll(0); events.onContentSizeChange(400, 1300);
    expect(scrolls).toEqual([]);
    events.onScrollEndDrag(); events.onMomentumScrollBegin(); scroll(80);
    events.onContentSizeChange(400, 1400); expect(scrolls).toEqual([]);
    events.onMomentumScrollEnd(); events.onContentSizeChange(400, 1500); expect(scrolls).toEqual([]);
    // Growth before release revokes the old near-end sample.
    events.onScrollBeginDrag(); scroll(804); events.onContentSizeChange(400, 1600); expect(scrolls).toEqual([]);
    events.onScrollEndDrag(); for (const callback of [...timers.values()]) callback(); timers.clear();
    events.onContentSizeChange(400, 1700); expect(scrolls).toEqual([]);
    // Momentum completion also uses current geometry, without another scroll sample.
    events.onScrollBeginDrag(); scroll(1004); events.onScrollEndDrag(); events.onMomentumScrollBegin();
    events.onContentSizeChange(400, 1800); events.onMomentumScrollEnd();
    events.onContentSizeChange(400, 1900); expect(scrolls).toEqual([]);
    // A final release within 24 points still resumes following.
    events.onScrollBeginDrag(); scroll(1190); events.onScrollEndDrag();
    for (const callback of [...timers.values()]) callback(); timers.clear();
    events.onContentSizeChange(400, 2000); expect(scrolls.pop()?.kind).toBe("end");
    events.onScrollToTop(); scrolls.length = 0; events.onContentSizeChange(400, 1800); expect(scrolls).toEqual([]);
    render();
    expect(await find(root, "Composer")!.props.onSend("sent", [], [])).toBe(true);
    // The acknowledgement adds the sent row; only the next render exposes its layout.
    expect(renderedRows.some(row => row.key === message.id)).toBe(false);
    render();
    expect(renderedRows.some(row => row.key === message.id)).toBe(true);
    find(root, "List")!.props.onContentSizeChange(400, 1801);
    render(); scrolls.length = 0;
    // Observe the inset handed to the real scroll consumer, not the harness's hook slots.
    const scrollView = find(root, "List")!.props.renderScrollComponent({}, null);
    expect(scrollView.type).toBe("Scroll");
    expect(scrollView.props.blankSpace?.value).toBe(os === "android" ? 412 : 436);
    const currentEvents = find(root, "List")!.props;
    currentEvents.onScrollBeginDrag(); scrolls.length = 0; tail = 350;
    currentEvents.onContentSizeChange(400, 1850);
    expect(scrolls).toEqual([]);
    // A smaller anchored inset waits for the iOS row commit; Android publishes immediately.
    if (os === "ios") {
      expect(scrollView.props.blankSpace?.value).toBe(436);
      for (const callback of [...timers.values()]) callback();
      timers.clear();
    }
    expect(scrollView.props.blankSpace?.value).toBe(os === "android" ? 362 : 386);
    // Revocation clears a pending send's anchor gate synchronously, with the chat unchanged.
    for (const field of ["identityId", "deviceId", "relayUrl"] as const) {
      const pending = Promise.withResolvers<Message>();
      sendReply = () => pending.promise;
      render();
      const retainedSend = find(root, "Composer")!.props.onSend;
      const origin = composerDraft(useStore.getState().identityId, useStore.getState().relayUrl, "chat");
      editComposerDraft(origin, { text: "pending" });
      const sent = submitComposerDraft(origin, text => retainedSend(text, [], []));
      const original = useStore.getState()[field];
      useStore.setState({ [field]: "replacement" });
      useStore.setState({ [field]: original });
      expect(await retainedSend("revoked before render", [], [])).toBe(false);
      useStore.setState({ [field]: "replacement" });
      render();
      expect(await retainedSend("retained", [], [])).toBe(false);
      const state = useStore.getState();
      const draft = composerDraft(state.identityId, state.relayUrl, "chat");
      editComposerDraft(draft, { text: "new presentation draft" });
      const fresh = find(root, "List")!.props;
      fresh.onScrollBeginDrag(); offset = 10000; fresh.onScrollEndDrag();
      for (const callback of [...timers.values()]) callback(); timers.clear();
      scrolls.length = 0;
      fresh.onContentSizeChange(400, 2100); expect(scrolls.pop()?.kind).toBe("end");
      if (field === "deviceId") pending.reject(new Error("revoked failure"));
      else pending.resolve(message);
      await sent;
      expect(draft.text).toBe("new presentation draft");
      expect(origin.text).toBe(origin === draft ? "new presentation draft" : "pending");
      scrolls.length = 0;
      fresh.onContentSizeChange(400, 2200); expect(scrolls.pop()?.kind).toBe("end");
      useStore.setState({ [field]: original }); render();
    }
  } finally {
    for (const value of slots) {
      if (value && typeof value === "object" && "cleanup" in value && typeof value.cleanup === "function") value.cleanup();
    }
    globalThis.setTimeout = originalTimeout; globalThis.clearTimeout = originalClear; globalThis.requestAnimationFrame = originalFrame;
  }
});
}
