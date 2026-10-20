import { expect, mock, test } from "bun:test";

// Mount the production search consumer with real store subscriptions and controlled native replies.
// Hook slots/native tags follow Composer.test.tsx; native layout and navigation need device proof.
type Node = { type: any; props: any };
let slots: any[] = [], cursor = 0, dirty = false;
let effects: Array<() => void> = [];
function slot(make: () => any) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = make();
  return { index, value: slots[index] };
}
const react = {
  useCallback: (fn: any) => fn, useDebugValue: () => {},
  useSyncExternalStore: (_subscribe: any, snapshot: any) => snapshot(),
  useState: (initial: any) => {
    const item = slot(() => typeof initial === "function" ? initial() : initial);
    return [item.value, (next: any) => { slots[item.index] = typeof next === "function" ? next(slots[item.index]) : next; dirty = true; }];
  },
  useMemo: (compute: any, deps: any[]) => {
    const item = slot(() => ({ deps: undefined, value: undefined }));
    if (!item.value.deps || deps.some((value, index) => !Object.is(value, item.value.deps[index]))) {
      item.value.deps = deps; item.value.value = compute();
    }
    return item.value.value;
  },
  useRef: (initial: any) => slot(() => ({ current: initial })).value,
  useEffect: (effect: any, deps: any[]) => {
    const item = slot(() => ({ deps: undefined, cleanup: undefined }));
    if (!item.value.deps || deps.some((value, index) => !Object.is(value, item.value.deps[index]))) {
      effects.push(() => { item.value.cleanup?.(); item.value.deps = deps; item.value.cleanup = effect(); });
    }
  },
};
mock.module("react", () => ({ ...react, default: react }));
const jsx = (type: any, props: any) => ({ type, props });
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
const platform = { OS: "ios" };
const opened: string[] = [];
let searchSurface: "sidebar" | "ios" | "android" = "sidebar";
let wide = false, pathname = "/";
const navigated: string[] = [];
mock.module("react-native", () => ({ Platform: platform, View: "View", Text: "Text", TextInput: "TextInput", Pressable: "Pressable", ActivityIndicator: "ActivityIndicator", Alert: {}, AppState: {}, StyleSheet: { create: (x: unknown) => x, hairlineWidth: 1 }, useWindowDimensions: () => ({ width: 400 }) }));
mock.module("@expo/ui/community/menu", () => ({ MenuView: "MenuView" }));
mock.module("expo-haptics", () => ({}));
mock.module("expo-router", () => ({ Link: "Link", Stack: { SearchBar: "SearchBar", Toolbar: Object.assign("Toolbar", { Button: "Button", Menu: "Menu", MenuAction: "MenuAction" }), Title: "Title" }, usePathname: () => pathname, useRouter: () => ({ push: (path: string) => opened.push(path), replace: (path: string) => opened.push(path), navigate: (path: string) => navigated.push(path) }) }));
mock.module("expo-router/unstable-native-tabs", () => ({ NativeTabs: Object.assign("NativeTabs", { Trigger: Object.assign("Trigger", { Icon: "Icon", Label: "Label" }) }) }));
mock.module("./AvatarFlashList", () => ({ AvatarFlashList: "List" }));
mock.module("./Avatar", () => ({ AvatarCluster: "Avatar" }));
mock.module("./ChatPeek", () => ({ ChatPeek: "Peek" }));
mock.module("./ChatRow", () => ({ ChatRow: "Row" }));
mock.module("./layout", () => ({ PaneWidth: "PaneWidth", useSidebarWidth: () => 320, useWide: () => wide }));
mock.module("./relay", () => ({ problemTitle: () => "", showRelayProblem: () => {} }));
mock.module("./format", () => ({ lastActivity: () => 0, preview: () => "", stamp: () => "" }));
mock.module("./SidebarSearch", () => ({ SidebarSearch: "Search", useSidebarSearchInset: () => 0 }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ Font: {}, usePalette: () => ({}) }));
mock.module("./navigation", () => ({ AndroidIcons: {} }));
mock.module("../i18n", () => ({ t: (x: string) => x, useLanguage: () => ({ language: "en" }) }));
mock.module("../core/prefs", () => ({ savePrefs: () => {}, loadPrefs: () => ({}), coreHome: () => "unused", pathOf: (x: string) => x, wipePrefs: () => {} }));
mock.module("../core/host", () => ({ hostFacts: () => ({}) }));
mock.module("../core/push", () => ({ clearPushes: () => {}, installPushHandlers: () => {}, registerForPushes: () => {} }));
mock.module("expo-web-browser", () => ({}));
const replies: Array<(value: any) => void> = [];
mock.module("../../modules/beans-core", () => ({ request: () => new Promise(resolve => replies.push(resolve)) }));
// These imports exercise production consumers after native and React mocks are installed.
const { useStore, resetStore } = await import("../core/store");
const { ChatsScreen } = await import("./ChatsScreen");
const { default: TabsLayout } = await import("../../app/(main)/(tabs)/_layout");
const timers = new Map<number, () => void>();
let timerID = 0;
function find(node: any, type: string): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
  if (!node || typeof node !== "object") return;
  return node.type === type ? node : find(node.props?.children, type);
}
function render(commit = true): Node {
  for (let pass = 0; pass < 10; pass++) {
    cursor = 0; dirty = false; effects = [];
    const root = ChatsScreen(searchSurface === "sidebar" ? { sidebar: true } : { search: true });
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
  globalThis.setTimeout = ((run: () => void) => { const id = ++timerID; timers.set(id, run); return id; }) as any;
  globalThis.clearTimeout = ((id: number) => { timers.delete(id); }) as any;
  const dispose = () => { for (const item of slots) item?.cleanup?.(); slots = []; timers.clear(); replies.length = 0; };
  const source = { paired: true, identityId: "A", relayUrl: "relay-A", deviceId: "phone-A", relayConnected: true, chats: [{ id: "same", title: "Same chat", kind: "group", bot_ids: [], messages: [], unread_count: 0 }], bots: [] };
  try {
    for (const field of ["identityId", "relayUrl", "deviceId", "paired"] as const) {
      for (const aba of [false, true]) {
        dispose(); resetStore(); useStore.setState(source as any); startSearch();
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
        expect(find(render(), "List")!.props.data.map((row: any) => row.snippet)).toEqual(["current", "current", "current"]);
        const list = find(render(), "List")!;
        const row = list.props.renderItem({ item: list.props.data[2] });
        row.props.onPress();
        expect(opened.pop()).toBe("/chat/same");
        const queryRoot = render();
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

test("widening a Devices or Search tab restores the empty chat pane without replacing an open chat", () => {
  for (const path of ["/devices", "/devices/device/runner", "/search", "/chat/current", "/"]) {
    slots = []; cursor = 0; effects = []; navigated.length = 0;
    pathname = path; wide = false;
    const compact = TabsLayout();
    for (const effect of effects) effect();
    expect(compact.props.hidden).toBe(false);
    expect(navigated).toEqual([]);
    cursor = 0; effects = []; wide = true;
    const expanded = TabsLayout();
    for (const effect of effects) effect();
    expect(expanded.props.hidden).toBe(true);
    expect(navigated).toEqual(path.startsWith("/devices") || path === "/search" ? ["/"] : []);
  }
  wide = false; pathname = "/"; slots = [];
});
