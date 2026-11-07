import { expect, test, vi } from "vitest";

// Mount the production search consumer with real store subscriptions and controlled native replies.
// Hook slots/native tags follow Composer.test.tsx; native layout and navigation need device proof.
type SearchItem = { snippet: string };
type Node = { type: unknown; props: { children?: unknown; data: SearchItem[]; renderItem: (input: { item: SearchItem }) => Node; onPress: () => void; onChangeText: (value: string | { nativeEvent: { text: string } }) => void; hidden?: boolean } };
type HookRecord = { deps?: unknown[]; value?: unknown; cleanup?: () => void };
let slots: unknown[] = [], cursor = 0, dirty = false;
let effects: Array<() => void> = [];
function slot<T>(make: () => T) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = make();
  // Each stable hook position retains the value created by that hook.
  const value = slots[index] as T;
  return { index, value };
}
const react = {
  useCallback: <T,>(fn: T) => fn, useDebugValue: () => {},
  useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
  useState: <T,>(initial: T | (() => T)) => {
    const item = slot(() => typeof initial === "function" ? (initial as () => T)() : initial);
    return [item.value, (next: T | ((previous: T) => T)) => { slots[item.index] = typeof next === "function" ? (next as (previous: T) => T)(slots[item.index] as T) : next; dirty = true; }];
  },
  useMemo: (compute: () => unknown, deps: unknown[]) => {
    const item = slot<HookRecord>(() => ({}));
    const previousDeps = item.value.deps;
    if (!previousDeps || deps.some((value, index) => !Object.is(value, previousDeps[index]))) {
      item.value.deps = deps; item.value.value = compute();
    }
    return item.value.value;
  },
  useRef: <T,>(initial: T) => slot(() => ({ current: initial })).value,
  useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
    const item = slot<HookRecord>(() => ({}));
    const previousDeps = item.value.deps;
    if (!previousDeps || deps.some((value, index) => !Object.is(value, previousDeps[index]))) {
      effects.push(() => {
        item.value.cleanup?.();
        item.value.deps = deps;
        const cleanup = effect();
        item.value.cleanup = typeof cleanup === "function" ? cleanup : undefined;
      });
    }
  },
};
vi.doMock("react", () => ({ ...react, default: react }));
const jsx = (type: unknown, props: Node["props"]) => ({ type, props });
vi.doMock("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
vi.doMock("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
const platform = { OS: "ios" };
const opened: string[] = [];
let searchSurface: "sidebar" | "ios" | "android" = "sidebar";
let wide = false, pathname = "/";
const navigated: string[] = [];
vi.doMock("react-native", () => ({ Platform: platform, View: "View", Text: "Text", TextInput: "TextInput", Pressable: "Pressable", ActivityIndicator: "ActivityIndicator", Alert: {}, AppState: {}, StyleSheet: { create: (x: unknown) => x, hairlineWidth: 1 }, useWindowDimensions: () => ({ width: 400 }) }));
vi.doMock("@expo/ui/community/menu", () => ({ MenuView: "MenuView" }));
vi.doMock("expo-haptics", () => ({}));
vi.doMock("expo-router", () => ({ Link: "Link", Stack: { SearchBar: "SearchBar", Toolbar: Object.assign("Toolbar", { Button: "Button", Menu: "Menu", MenuAction: "MenuAction" }), Title: "Title" }, usePathname: () => pathname, useRouter: () => ({ push: (path: string) => opened.push(path), replace: (path: string) => opened.push(path), navigate: (path: string) => navigated.push(path) }) }));
vi.doMock("expo-router/unstable-native-tabs", () => ({ NativeTabs: Object.assign("NativeTabs", { Trigger: Object.assign("Trigger", { Icon: "Icon", Label: "Label" }) }) }));
vi.doMock("./AvatarFlashList", () => ({ AvatarFlashList: "List" }));
vi.doMock("./Avatar", () => ({ AvatarCluster: "Avatar" }));
vi.doMock("./ChatPeek", () => ({ ChatPeek: "Peek" }));
vi.doMock("./ChatRow", () => ({ ChatRow: "Row" }));
vi.doMock("./layout", () => ({ PaneWidth: "PaneWidth", useSidebarWidth: () => 320, useWide: () => wide }));
vi.doMock("./relay", () => ({ problemTitle: () => "", showRelayProblem: () => {} }));
vi.doMock("./format", () => ({ lastActivity: () => 0, preview: () => "", stamp: () => "" }));
vi.doMock("./SidebarSearch", () => ({ SidebarSearch: "Search", useSidebarSearchInset: () => 0 }));
vi.doMock("./Symbol", () => ({ Symbol: "Symbol" }));
vi.doMock("./theme", () => ({ Font: {}, usePalette: () => ({}) }));
vi.doMock("./navigation", () => ({ AndroidIcons: {} }));
vi.doMock("../i18n", () => ({ t: (x: string) => x, useLanguage: () => ({ language: "en" }) }));
vi.doMock("../core/prefs", () => ({ savePrefs: () => {}, loadPrefs: () => ({}), coreHome: () => "unused", pathOf: (x: string) => x, wipePrefs: () => {} }));
vi.doMock("../core/host", () => ({ hostFacts: () => ({}) }));
vi.doMock("../core/push", () => ({ clearPushes: () => {}, installPushHandlers: () => {}, registerForPushes: () => {} }));
vi.doMock("expo-web-browser", () => ({}));
const replies: Array<(value: unknown) => void> = [];
vi.doMock("../../modules/beans-core", () => ({ request: () => new Promise(resolve => replies.push(resolve)) }));
// These imports exercise production consumers after native and React mocks are installed.
const { useStore, resetStore } = await import("../core/store");
const { ChatsScreen } = await import("./ChatsScreen");
const { default: TabsLayout } = await import("../../app/(main)/(tabs)/_layout");
const timers = new Map<number, () => void>();
let timerID = 0;
function find(node: unknown, type: string): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
  if (!node || typeof node !== "object" || !("type" in node) || !("props" in node)) return;
  // All nodes here come from the test JSX factory, not external input.
  const element = node as Node;
  return element.type === type ? element : find(element.props.children, type);
}
function render(commit = true): Node {
  for (let pass = 0; pass < 10; pass++) {
    cursor = 0; dirty = false; effects = [];
    // The mocked JSX factory returns the Node shape above.
    const root = ChatsScreen(searchSurface === "sidebar" ? { sidebar: true } : { search: true }) as unknown as Node;
    if (!commit) return root;
    for (const effect of effects) effect();
    if (!dirty) return root;
  }
  throw new Error("Search consumer effects did not settle");
}
function startSearch() {
  const root = render();
  if (searchSurface === "ios") find(root, "SearchBar")!.props.onChangeText({ nativeEvent: { text: "needle" } });
  else find(root, searchSurface === "sidebar" ? "Search" : "TextInput")!.props.onChangeText("needle");
  render();
  for (const [id, run] of [...timers]) { timers.delete(id); run(); }
}
const result = (label: string) => ({ chats: [{ chat_id: "same", snippet: label }], messages: [{ chat_id: "same", message_id: "message", snippet: label, created_at: 1 }], files: [{ chat_id: "same", message_id: "file", attachment_id: "attachment", name: label, created_at: 1 }], history_complete: false });
async function flush() { for (let i = 0; i < 8; i++) await Promise.resolve(); }

test.each(["sidebar", "ios", "android"] as const)("%s search retains authority across deferred switches and sticky ABA", async (surface) => {
  searchSurface = surface;
  platform.OS = surface === "android" ? "android" : "ios";
  opened.length = 0;
  const originalTimeout = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  // Fake timers return numeric handles; Bun's native timer type also carries methods.
  const fakeTimeout = ((run: () => void) => { const id = ++timerID; timers.set(id, run); return id; }) as unknown as typeof setTimeout;
  const fakeClear = ((id: number) => { timers.delete(id); }) as unknown as typeof clearTimeout;
  globalThis.setTimeout = fakeTimeout;
  globalThis.clearTimeout = fakeClear;
  const dispose = () => { for (const item of slots) { if (item && typeof item === "object" && "cleanup" in item && typeof item.cleanup === "function") item.cleanup(); } slots = []; timers.clear(); replies.length = 0; };
  const source = { paired: true, identityId: "A", relayUrl: "relay-A", deviceId: "phone-A", relayConnected: true, chats: [{ id: "same", title: "Same chat", kind: "group" as const, bot_ids: [], messages: [], unread_count: 0, is_pinned: false, created_at: 0 }], bots: [] };
  try {
    for (const field of ["identityId", "relayUrl", "deviceId", "paired"] as const) {
      for (const aba of [false, true]) {
        dispose(); resetStore(); useStore.setState(source); startSearch();
        const old = replies.shift()!;
        useStore.setState({ [field]: field === "paired" ? false : "B" });
        if (aba) useStore.setState({ [field]: source[field] });
        old(result("private A")); await flush();
        expect(find(render(false), "List")!.props.data).toEqual([]);
        render();
        for (const [id, run] of [...timers]) { timers.delete(id); run(); }
        if (!aba && field === "paired") continue;
        const current = replies.shift()!;
        expect(current).toBeDefined();
        current(result("current")); await flush();
        expect(find(render(), "List")!.props.data.map(row => row.snippet)).toEqual(["current", "current", "current"]);
        const list = find(render(), "List")!;
        const row = list.props.renderItem({ item: list.props.data[2] });
        row.props.onPress();
        expect(opened.pop()).toBe("/chat/same");
        const queryRoot = render();
        if (surface === "ios") find(queryRoot, "SearchBar")!.props.onChangeText({ nativeEvent: { text: "needle" } });
        else find(queryRoot, surface === "sidebar" ? "Search" : "TextInput")!.props.onChangeText("needle");
        row.props.onPress();
        expect(opened.pop()).toBe("/chat/same");
        if (surface === "ios") {
          find(queryRoot, "SearchBar")!.props.onChangeText({ nativeEvent: { text: "temporary" } });
          find(queryRoot, "SearchBar")!.props.onChangeText({ nativeEvent: { text: "needle" } });
        } else {
          const field = find(queryRoot, surface === "sidebar" ? "Search" : "TextInput")!;
          field.props.onChangeText("temporary");
          field.props.onChangeText("needle");
        }
        row.props.onPress();
        expect(opened).toEqual([]);
        render();
        for (const [id, run] of [...timers]) { timers.delete(id); run(); }
        replies.shift()!(result("restored")); await flush();
        const restoredList = find(render(), "List")!;
        const restoredRow = restoredList.props.renderItem({ item: restoredList.props.data[0] });
        restoredRow.props.onPress();
        expect(opened.pop()).toBe("/chat/same");
        if (surface === "ios") find(queryRoot, "SearchBar")!.props.onChangeText({ nativeEvent: { text: "replacement" } });
        else find(queryRoot, surface === "sidebar" ? "Search" : "TextInput")!.props.onChangeText("replacement");
        // No render or passive effect has run since the field changed.
        row.props.onPress();
        expect(opened).toEqual([]);
        render();
        for (const [id, run] of [...timers]) { timers.delete(id); run(); }
        replies.shift()!(result("replacement")); await flush();
        const replacementList = find(render(), "List")!;
        const replacementRow = replacementList.props.renderItem({ item: replacementList.props.data[0] });
        // Published results must disappear before passive effects run, even after A -> B -> A.
        const owner = useStore.getState()[field];
        useStore.setState({ [field]: field === "paired" ? !owner : "other" });
        useStore.setState({ [field]: owner });
        replacementRow.props.onPress();
        expect(opened).toEqual([]);
        const revoked = render(false);
        expect(find(revoked, "List")!.props.data).toEqual([]);
        expect(find(revoked, "Text")).toBeUndefined();
      }
    }
  } finally { dispose(); globalThis.setTimeout = originalTimeout; globalThis.clearTimeout = originalClear; }
});

test("content changes do not starve search and schedule a follow-up", async () => {
  searchSurface = "sidebar"; platform.OS = "ios";
  const originalTimeout = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  globalThis.setTimeout = ((run: () => void) => { const id = ++timerID; timers.set(id, run); return id; }) as unknown as typeof setTimeout;
  globalThis.clearTimeout = ((id: number) => { timers.delete(id); }) as unknown as typeof clearTimeout;
  const runTimers = () => { for (const [id, run] of [...timers]) { timers.delete(id); run(); } };
  const rows = () => find(render(), "List")!.props.data.map(row => row.snippet);
  try {
    resetStore();
    useStore.setState({ paired: true, identityId: "A", relayUrl: "relay-A", deviceId: "phone-A", chats: [{ id: "same", title: "Before", kind: "group", bot_ids: [], messages: [], unread_count: 0, is_pinned: false, created_at: 0 }], bots: [] });
    const root = render();
    find(root, "Search")!.props.onChangeText("needle"); render();
    for (let i = 0; i < 5; i++) {
      useStore.setState(state => ({ chats: state.chats.map(chat => ({ ...chat, title: `Changed ${i}` })) }));
      render();
    }
    expect(timers.size).toBe(1);
    runTimers();
    expect(replies.length).toBe(1);
    useStore.setState(state => ({ chats: state.chats.map(chat => ({ ...chat, title: "needle chat" })) }));
    useStore.setState(state => ({ bots: [...state.bots, { id: "bot", name: "needle bot" } as never] }));
    useStore.setState(state => ({ chats: state.chats.map(chat => ({ ...chat, messages: [{ id: "m", content: "needle message", attachments: [{ id: "f", name: "needle file" }] } as never] })) }));
    expect(replies.length).toBe(1);
    replies.shift()!(result("first")); await flush();
    expect(rows()).toEqual(["first", "first", "first"]);
    expect(timers.size).toBe(1);
    runTimers();
    expect(replies.length).toBe(1);
    useStore.setState(state => ({ chats: state.chats.map(chat => ({ ...chat, title: "other", messages: [] })) }));
    replies.shift()!(result("updated")); await flush();
    expect(rows()).toEqual(["updated", "updated", "updated"]);
    runTimers();
    replies.shift()!({ chats: [], messages: [], files: [], history_complete: true }); await flush();
    expect(rows()).toEqual([]);
  } finally {
    for (const item of slots) if (item && typeof item === "object" && "cleanup" in item && typeof item.cleanup === "function") item.cleanup();
    slots = []; timers.clear(); replies.length = 0;
    globalThis.setTimeout = originalTimeout; globalThis.clearTimeout = originalClear;
  }
});

test("widening a Devices or Search tab restores the empty chat pane without replacing an open chat", () => {
  for (const path of ["/devices", "/devices/device/runner", "/search", "/chat/current", "/"]) {
    slots = []; cursor = 0; effects = []; navigated.length = 0;
    pathname = path; wide = false;
    // The JSX runtime mock returns the harness Node shape.
    const compact = TabsLayout() as unknown as Node;
    for (const effect of effects) effect();
    expect(compact.props.hidden).toBe(false);
    expect(navigated).toEqual([]);
    cursor = 0; effects = []; wide = true;
    // The same mocked JSX boundary applies after the width transition.
    const expanded = TabsLayout() as unknown as Node;
    for (const effect of effects) effect();
    expect(expanded.props.hidden).toBe(true);
    expect(navigated).toEqual(path.startsWith("/devices") || path === "/search" ? ["/"] : []);
  }
  wide = false; pathname = "/"; slots = [];
});
