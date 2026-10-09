import { expect, mock, test } from "bun:test";

// Same hook-slot/native-tag boundary as avatarRenderer and Markdown consumer checks.
// This mounts the production Composer, Surface and AttachMenu, not copied handlers.
// Native responder negotiation, SwiftUI presentation, geometry and IME need device proof.
type Node = { type: string | ((props: any) => Node); props: any; parent?: Node };
let platform = "ios";
const glass = process.env.BEANS_COMPOSER_GLASS === "1";
let slots: any[] = [];
let cursor = 0;
let root: Node;
let focusCalls = 0;
let sends = 0;
let route = "chat";
let dirty = false;
let effects: Array<() => void> = [];
const hookReact = {
  useCallback: (callback: any) => callback,
  useDebugValue: () => {},
  useSyncExternalStore: (_subscribe: any, snapshot: any) => snapshot(),
};
function dispose() {
  for (const item of slots) item?.cleanup?.();
  slots = []; effects = []; dirty = false;
}
function hookEffect(effect: () => void | (() => void), dependencies?: any[]) {
  const item = slot(() => ({ dependencies: undefined as any[] | undefined, cleanup: undefined as (() => void) | undefined }));
  if (!dependencies || !item.value.dependencies || dependencies.some((value, index) => !Object.is(value, item.value.dependencies![index]))) {
    effects.push(() => { item.value.cleanup?.(); item.value.dependencies = dependencies; item.value.cleanup = effect() || undefined; });
  }
}
function slot(make: () => any) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = make();
  return { index, value: slots[index] };
}
mock.module("react", () => ({
  ...hookReact,
  useState: (initial: any) => {
    const item = slot(() => initial);
    return [item.value, (next: any) => { slots[item.index] = typeof next === "function" ? next(slots[item.index]) : next; dirty = true; }];
  },
  useRef: (initial: any) => slot(() => ({ current: initial })).value,
  useMemo: (compute: () => any) => { cursor++; return compute(); },
  useEffect: hookEffect,
}));
const jsx = (type: Node["type"], props: any): Node => ({ type, props });
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx }));
mock.module("react-native", () => ({
  Platform: { get OS() { return platform; } },
  View: "View", Pressable: "Pressable", TextInput: "TextInput", Text: "Text", ScrollView: "ScrollView",
  Alert: { alert: () => {} }, Linking: {}, AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) },
  StyleSheet: { create: (value: any) => value, hairlineWidth: 0.5, absoluteFill: {}, flatten: (value: any) => Object.assign({}, ...[value].flat()) },
}));
mock.module("@expo/ui/swift-ui", () => ({ Button: "MenuButton", Host: "SwiftHost", Image: "MenuImage", Menu: "Menu" }));
mock.module("@expo/ui/swift-ui/modifiers", () => ({ background: () => ({}), frame: () => ({}), shapes: { circle: () => ({}) } }));
mock.module("@expo/ui/community/menu", () => ({ MenuView: "MenuView" }));
mock.module("@expo/ui/jetpack-compose", () => ({ Column: "Column", Host: "ComposeHost", Icon: "Icon", ListItem: "ListItem", ModalBottomSheet: "ModalBottomSheet", Text: "ComposeText" }));
mock.module("@expo/ui/jetpack-compose/modifiers", () => ({ clickable: () => ({}), fillMaxWidth: () => ({}), padding: () => ({}) }));
mock.module("expo-document-picker", () => ({}));
mock.module("expo-glass-effect", () => ({ GlassView: "GlassView", isLiquidGlassAvailable: () => glass }));
mock.module("expo-haptics", () => ({ selectionAsync: () => Promise.resolve(), impactAsync: () => Promise.resolve(), ImpactFeedbackStyle: { Light: "light", Medium: "medium" } }));
mock.module("expo-image", () => ({ Image: "Image" }));
mock.module("expo-image-picker", () => ({}));
const speechEvents = new Map<string, (event: any) => void>();
let permission: () => Promise<{ granted: boolean }> = async () => ({ granted: true });
let starts = 0;
let aborts = 0;
let startError = false;
mock.module("expo-speech-recognition", () => ({ ExpoSpeechRecognitionModule: {
  requestPermissionsAsync: () => permission(), start: () => { if (startError) throw new Error("start refused"); starts++; },
  stop: () => {}, abort: () => { aborts++; },
}, useSpeechRecognitionEvent: (name: string, handler: any) => {
  hookEffect(() => { speechEvents.set(name, handler); return () => { if (speechEvents.get(name) === handler) speechEvents.delete(name); }; }, [name, handler]);
} }));
mock.module("./avatarVisibility", () => ({ notifyAvatarScroll: () => {} }));
mock.module("./Avatar", () => ({ BotAvatar: "BotAvatar" }));
mock.module("./dictation", () => ({ automaticLanguage: () => "en", languageName: (tag: string) => tag, pickDictationLanguage: () => {}, setDictationLanguage: () => {}, useDictationLanguage: () => ({ language: "en" }), useSupportedLanguages: () => [] }));
mock.module("../i18n", () => ({ language: "en", t: (text: string) => text, useLanguage: () => ({ language: "en" }) }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ Font: { body: 16 }, usePalette: () => ({ fill: "fill", label: "label", cell: "cell" }) }));
mock.module("./navigation", () => ({ AndroidIcons: {} }));
mock.module("expo-web-browser", () => ({}));
mock.module("../core/host", () => ({ hostFacts: () => ({ name: "Phone", os: "ios", os_version: "", model: "" }) }));
mock.module("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (path: string) => path, wipePrefs: () => {} }));
mock.module("../core/push", () => ({ clearPushes: () => {}, installPushHandlers: () => {}, registerForPushes: async () => {} }));
const frames = new Set<(frame: any) => void>();
let request: (method: string, params: any) => Promise<any> = async () => null;
mock.module("expo-modules-core", () => ({ requireNativeModule: () => ({
  start: () => {}, wake: () => {},
  addListener: (_name: string, listener: any) => { const receive = (frame: any) => listener({ json: JSON.stringify(frame) }); frames.add(receive); return { remove: () => frames.delete(receive) }; },
  request: async (method: string, params: string) => JSON.stringify(await request(method, JSON.parse(params))),
}) }));
const { composerDraft, editComposerDraft } = await import("../core/composerDraft");
const { useStore, resetStore } = await import("../core/store");
const { engine } = await import("../core/engine");
const event = (name: string, data: any) => frames.forEach(listener => listener({ event: name, data }));
const chat = (id: string) => ({ id, kind: "group" as const, bot_ids: [], is_pinned: false, created_at: 0, messages: [], unread_count: 0 });
const snapshot = { has_identity: true, identity_id: "consumer", this_device_id: "phone", relay_url: "relay", relay_connected: true, devices: [], bots: [], chats: [chat("chat"), chat("other")], running_turns: [] };
request = async method => method === "bootstrap" ? { result: snapshot } : { result: null };
await engine.start();
function reset() { dispose(); resetStore(); event("snapshot", snapshot); route = "chat"; starts = 0; aborts = 0; sends = 0; startError = false; permission = async () => ({ granted: true }); }
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); render(); };
const { Composer } = await import("./Composer");

function mount(node: any, parent?: Node): any {
  if (Array.isArray(node)) return node.map(child => mount(child, parent));
  if (!node || typeof node !== "object") return node;
  if (typeof node.type === "function") return mount(node.type(node.props), parent);
  node.parent = parent;
  if (node.type === "TextInput") node.props.ref.current = {
    focus() { focusCalls++; node.props.onFocus(); },
  };
  node.props.children = mount(node.props.children, node);
  return node;
}
function render() {
  for (let pass = 0; pass < 10; pass++) {
    cursor = 0; dirty = false; effects = [];
    root = mount(Composer({ chatId: route, members: [], isGroup: false, placeholder: "Message", onSend: async (text, files, mentions, reply) => {
      sends++; await engine.sendMessage(route, text, files, mentions, reply); return true;
    } }));
    for (const effect of effects) effect();
    if (!dirty) return;
  }
  throw new Error("Consumer effect render did not settle");
}
function find(predicate: (node: Node) => boolean, node: any = root): Node | undefined {
  if (Array.isArray(node)) return node.map(child => find(predicate, child ?? null)).find(Boolean);
  if (!node || typeof node !== "object") return;
  return predicate(node) ? node : node.props.children === undefined ? undefined : find(predicate, node.props.children);
}
function input() { return find(node => node.type === "TextInput")!; }
function pill() { return find(node => node.type === "Pressable" && !!node.props.onPress && node.props.accessible === false)!; }
// Generic host bubbling: deepest start-responder claimant owns this touch;
// otherwise nearest enabled Pressable handles it. No AttachMenu-specific oracle.
function tap(target: Node): Node {
  for (let node: Node | undefined = target; node; node = node.parent) {
    if (node.props.onStartShouldSetResponder?.({ nativeEvent: {} })) return node;
  }
  for (let node: Node | undefined = target; node; node = node.parent) {
    if (node.type === "Pressable" && !node.props.disabled) { node.props.onPress?.(); return node; }
  }
  return target;
}
export function checkComposerTouchOwnership() {
  reset(); focusCalls = 0; platform = "ios";
  render();
  const initialPill = pill();
  const host = find(node => node.type === "SwiftHost")!;
  const owner = tap(host);
  expect(owner).not.toBe(initialPill);
  expect(focusCalls).toBe(0);
  render();
  expect(input().props.value).toBe("");
  expect(pill().props.style).toEqual(initialPill.props.style);
  tap(pill());
  expect(focusCalls).toBe(1);
  render();
  const draft = "未確定 draft\nsecond line";
  input().props.onChangeText(draft);
  input().props.onBlur();
  render();
  const draftPill = pill();
  tap(find(node => node.type === "SwiftHost")!);
  expect(focusCalls).toBe(1);
  render();
  expect(input().props.value).toBe(draft);
  expect(pill().props.style).toEqual(draftPill.props.style);
  tap(pill());
  expect(focusCalls).toBe(2);
  render();
  expect(input().props.value).toBe(draft);
  expect(input().props.accessibilityLabel).toBe("Message");
  expect(sends).toBe(0);

  dispose(); platform = "android"; render();
  tap(find(node => node.props.accessibilityLabel === "Attach")!);
  render();
  expect(find(node => node.type === "ModalBottomSheet")).toBeDefined();
  expect(sends).toBe(0);
  dispose();
  return { menuOwnsTouch: true, pillFocusCalls: focusCalls, draftPreserved: true, androidSheetOpened: true, nativePresentationVerified: false };
}
test("production Engine and Composer preserve omission and validate exact admitted content", async () => {
  reset(); platform = "ios";
  let resolve!: (value: any) => void;
  let reject!: (error: Error) => void;
  request = async method => method === "chats.send" ? new Promise((yes, no) => { resolve = yes; reject = no; }) : null;
  const message = { id: "msg-admitted", chat_id: "chat", author: { kind: "you" }, body: { kind: "text", text: "submitted", attachments: [], mentions: [] }, state: { kind: "complete" }, created_at: 1 };
  render(); input().props.onChangeText("\u0085 submitted \u0085"); render();
  tap(find(node => node.props.accessibilityLabel === "Send")!); render();
  const draft = composerDraft("consumer", "relay", "chat");
  event("roster.changed", { devices: [], bots: [], chats: [chat("other")] });
  expect(draft.text).toBe("\u0085 submitted \u0085");
  event("roster.changed", { devices: [], bots: [], chats: [chat("chat"), chat("other")] });
  dispose(); render(); tap(find(node => node.props.accessibilityLabel === "Send")!);
  expect(sends).toBe(1);
  reject(new Error("response lost")); await flush(); expect(draft.text).toBe("\u0085 submitted \u0085");
  tap(find(node => node.props.accessibilityLabel === "Send")!); resolve({ error: { message: "send refused" } }); await flush();
  expect(draft.text).toBe("\u0085 submitted \u0085");
  for (const bad of [undefined, { ...message, state: undefined }, { ...message, created_at: undefined }, { ...message, body: { kind: "text" } }, { ...message, body: { ...message.body, text: "unrelated" } }, { ...message, chat_id: "other" }, { ...message, author: { kind: "bot", bot_id: "b" } }, { ...message, body: { ...message.body, reply_to: { message_id: "unexpected" } } }]) {
    tap(find(node => node.props.accessibilityLabel === "Send")!); resolve({ result: { message: bad } }); await flush();
    expect(draft.text).toBe("\u0085 submitted \u0085");
    expect(useStore.getState().chats.find(chat => chat.id === "chat")!.messages).toEqual([]);
  }
  tap(find(node => node.props.accessibilityLabel === "Send")!); resolve({ result: { message } }); await flush();
  expect(input().props.value).toBe("");
  expect(useStore.getState().chats.find(chat => chat.id === "chat")!.messages[0].id).toBe("msg-admitted");
  editComposerDraft(draft, { text: "submitted" }); render();
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  input().props.onChangeText("newer edit"); resolve({ result: { message } }); await flush();
  expect(draft.text).toBe("newer edit");
  editComposerDraft(draft, { text: "files", attachments: [{ uri: "file:///picked", name: "picked.txt", mime: "text/plain" }], reply: { messageID: "quoted", name: "Bot", text: "quote" } }); render();
  const withFiles = { ...message, id: "msg-files", body: { kind: "text", text: "files", attachments: [{ id: "att-abc123", name: "picked.txt", mime: "text/plain", size: 7 }], reply_to: { message_id: "quoted", author: { kind: "bot", bot_id: "bot" }, text: "quote" } } };
  for (const bad of [{ ...withFiles, body: { ...withFiles.body, attachments: [] } }, { ...withFiles, body: { ...withFiles.body, reply_to: undefined } }, { ...withFiles, body: { ...withFiles.body, reply_to: { ...withFiles.body.reply_to, message_id: "wrong" } } }, { ...withFiles, body: { ...withFiles.body, attachments: [{ ...withFiles.body.attachments[0], name: "other.txt" }] } }]) {
    tap(find(node => node.props.accessibilityLabel === "Send")!); resolve({ result: { message: bad } }); await flush();
    expect(draft.reply?.messageID).toBe("quoted"); expect(draft.attachments[0].name).toBe("picked.txt");
  }
  tap(find(node => node.props.accessibilityLabel === "Send")!); resolve({ result: { message: withFiles } }); await flush();
  expect(draft.reply).toBe(null); expect(draft.attachments).toEqual([]);
  editComposerDraft(draft, { text: "submitted" }); render();
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  useStore.setState({ deviceId: "replacement" }); useStore.setState({ deviceId: "phone" });
  resolve({ result: { message: { ...message, id: "msg-stale" } } }); await flush();
  expect(draft.text).toBe("submitted");
  expect(useStore.getState().chats.find(chat => chat.id === "chat")!.messages.some(message => message.id === "msg-stale")).toBe(false);
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  event("message.added", { message: { ...message, id: "msg-event" } });
  reject(new Error("response lost after event")); await flush(); expect(draft.text).toBe("submitted");
  event("roster.changed", { devices: [], bots: [], chats: [chat("other")] });
  expect(draft.text).toBe("submitted");
  event("roster.changed", { devices: [], bots: [], chats: [chat("chat"), chat("other")] });
  expect(composerDraft("consumer", "relay", "chat")).toBe(draft);
  editComposerDraft(draft, { text: "kept" });
  event("chat.removed", { chat_id: "chat" }); expect(draft.invalidated).toBe(true);
  resetStore(); expect(composerDraft("consumer", "relay", "other").text).toBe(""); dispose();
});

test("effectful Composer revokes permission ABA and resets recording on owner change and errors", async () => {
  reset(); platform = "ios"; render();
  let grant!: (value: { granted: boolean }) => void;
  permission = () => new Promise(resolve => { grant = resolve; });
  for (const transition of [() => { useStore.setState({ deviceId: "replacement" }); useStore.setState({ deviceId: "phone" }); }, () => { useStore.setState({ paired: false }); useStore.setState({ paired: true }); }, () => { useStore.setState({ identityId: "B", relayUrl: "B" }); render(); useStore.setState({ identityId: "consumer", relayUrl: "relay" }); render(); }, () => { route = "other"; render(); route = "chat"; render(); }, () => { dispose(); render(); }]) {
    tap(find(node => node.props.accessibilityLabel === "Dictate")!);
    transition(); grant({ granted: true }); await flush(); expect(starts).toBe(0);
  }
  permission = async () => { throw new Error("permission refused"); };
  tap(find(node => node.props.accessibilityLabel === "Dictate")!); await flush();
  expect(composerDraft("consumer", "relay", "chat").permission).toBe(null);
  permission = async () => ({ granted: true }); startError = true;
  tap(find(node => node.props.accessibilityLabel === "Dictate")!); await flush();
  expect(composerDraft("consumer", "relay", "chat").recording).toBe(null);
  startError = false; tap(find(node => node.props.accessibilityLabel === "Dictate")!); await flush();
  speechEvents.get("result")!({ results: [{ transcript: "heard" }] });
  route = "other"; render(); expect(aborts).toBe(1);
  expect(find(node => node.props.accessibilityLabel === "Dictate")).toBeDefined();
  expect(composerDraft("consumer", "relay", "chat").text).toBe("heard");
  tap(find(node => node.props.accessibilityLabel === "Dictate")!); await flush(); expect(starts).toBe(2);
  speechEvents.get("result")!({ results: [{ transcript: "new recording" }] });
  let refuse!: (error: Error) => void;
  request = async method => method === "chats.send" ? new Promise((_resolve, reject) => { refuse = reject; }) : { result: null };
  render(); tap(find(node => node.props.accessibilityLabel === "Send")!);
  const end = speechEvents.get("end")!;
  end({}); end({}); await flush();
  expect(sends).toBe(1);
  expect(composerDraft("consumer", "relay", "other").text).toBe("new recording");
  refuse(new Error("send refused")); await flush();
  expect(composerDraft("consumer", "relay", "other").text).toBe("new recording");
  route = "chat"; render(); expect(input().props.value).toBe("heard");
  dispose(); resetStore();
});
if (process.env.BEANS_COMPOSER_TOUCH_SMOKE === "1") {
  console.log(JSON.stringify(checkComposerTouchOwnership()));
} else {
  test("production attachment touch avoids pill focus while ordinary pill taps preserve draft", checkComposerTouchOwnership);
}
