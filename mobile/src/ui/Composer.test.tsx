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
function slot(make: () => any) {
  const index = cursor++;
  if (!(index in slots)) slots[index] = make();
  return { index, value: slots[index] };
}
mock.module("react", () => ({
  useState: (initial: any) => {
    const item = slot(() => initial);
    return [item.value, (next: any) => { slots[item.index] = typeof next === "function" ? next(slots[item.index]) : next; }];
  },
  useRef: (initial: any) => slot(() => ({ current: initial })).value,
  useMemo: (compute: () => any) => { cursor++; return compute(); },
  useEffect: () => { cursor++; },
}));
const jsx = (type: Node["type"], props: any): Node => ({ type, props });
mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx }));
mock.module("react-native", () => ({
  Platform: { get OS() { return platform; } },
  View: "View", Pressable: "Pressable", TextInput: "TextInput", Text: "Text", ScrollView: "ScrollView",
  Alert: {}, Linking: {},
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
mock.module("expo-speech-recognition", () => ({ ExpoSpeechRecognitionModule: { requestPermissionsAsync: async () => ({ granted: true }), start: () => {}, stop: () => {}, abort: () => {} }, useSpeechRecognitionEvent: (name: string, handler: any) => { speechEvents.set(name, handler); } }));
mock.module("../core/model", () => ({ fileSize: () => "", MAX_ATTACHMENT_BYTES: 100_000_000, MAX_ATTACHMENTS: 10 }));
mock.module("./avatarVisibility", () => ({ notifyAvatarScroll: () => {} }));
mock.module("./Avatar", () => ({ BotAvatar: "BotAvatar" }));
mock.module("./dictation", () => ({ automaticLanguage: () => "en", languageName: (tag: string) => tag, pickDictationLanguage: () => {}, setDictationLanguage: () => {}, useDictationLanguage: () => ({ language: "en" }), useSupportedLanguages: () => [] }));
mock.module("../i18n", () => ({ t: (text: string) => text, useLanguage: () => {} }));
mock.module("./format", () => ({ joinDictation: (text: string, words: string) => text ? `${text} ${words}` : words }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ Font: { body: 16 }, usePalette: () => ({ fill: "fill", label: "label", cell: "cell" }) }));
mock.module("./navigation", () => ({ AndroidIcons: {} }));
mock.module("../core/store", () => ({ useStore: (select: any) => select({ identityId: "consumer", relayUrl: "relay" }) }));
const { composerDraft, clearComposerDrafts, editComposerDraft, invalidateComposerSends, removeComposerDraft } = await import("../core/composerDraft");
let sendResult: () => Promise<boolean> = async () => true;
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
  cursor = 0;
  root = mount(Composer({ chatId: "chat", members: [], isGroup: false, placeholder: "Message", onSend: () => { sends++; return sendResult(); } }));
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
  clearComposerDrafts();
  slots = []; focusCalls = 0; sends = 0; platform = "ios";
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

  platform = "android"; slots = []; render();
  tap(find(node => node.props.accessibilityLabel === "Attach")!);
  render();
  expect(find(node => node.type === "ModalBottomSheet")).toBeDefined();
  expect(sends).toBe(0);
  return { menuOwnsTouch: true, pillFocusCalls: focusCalls, draftPreserved: true, androidSheetOpened: true, nativePresentationVerified: false };
}
test("production Composer retains pending/refused draft and clears only unchanged acknowledgement", async () => {
  clearComposerDrafts(); slots = []; sends = 0; platform = "ios";
  let acknowledge!: (value: boolean) => void;
  sendResult = () => new Promise(resolve => { acknowledge = resolve; });
  render();
  input().props.onChangeText("submitted"); render();
  const draft = composerDraft("consumer", "relay", "chat");
  editComposerDraft(draft, { attachments: [{ uri: "file:///a", name: "a", mime: "text/plain" }], reply: { messageID: "r", name: "Bot", text: "quote" } });
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  render();
  expect(input().props.value).toBe("submitted");
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  expect(sends).toBe(1);
  // Replacement hook slots are a remount; retained operation is production ownership.
  slots = []; render();
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  expect(sends).toBe(1);
  acknowledge(false); await Promise.resolve(); await Promise.resolve();
  expect(draft.reply?.messageID).toBe("r");
  expect(draft.attachments[0].name).toBe("a");
  render(); tap(find(node => node.props.accessibilityLabel === "Send")!);
  input().props.onChangeText("new edit");
  acknowledge(true); await Promise.resolve(); await Promise.resolve();
  render(); expect(input().props.value).toBe("new edit");
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  acknowledge(true); await Promise.resolve(); await Promise.resolve();
  render(); expect(input().props.value).toBe("");
  expect(draft.reply).toBe(null);
  expect(draft.attachments).toEqual([]);
  tap(find(node => node.props.accessibilityLabel === "Dictate")!);
  await Promise.resolve(); await Promise.resolve(); render();
  speechEvents.get("result")!({ results: [{ transcript: "dictated" }] });
  tap(find(node => node.props.accessibilityLabel === "Send")!);
  speechEvents.get("end")!({});
  speechEvents.get("end")!({});
  render(); expect(input().props.value).toBe("dictated");
  expect(sends).toBe(4);
  acknowledge(false); await Promise.resolve(); await Promise.resolve();
  render(); expect(input().props.value).toBe("dictated");
  render(); tap(find(node => node.props.accessibilityLabel === "Send")!);
  invalidateComposerSends();
  acknowledge(true); await Promise.resolve(); await Promise.resolve();
  const dictatedDraft = composerDraft("consumer", "relay", "chat");
  expect(dictatedDraft.text).toBe("dictated");
  expect(composerDraft("other-account", "relay", "chat").text).toBe("");
  removeComposerDraft("consumer", "relay", "chat");
  const replacement = composerDraft("consumer", "relay", "chat");
  editComposerDraft(replacement, { text: "replacement" });
  editComposerDraft(dictatedDraft, { text: "late callback" });
  expect(replacement.text).toBe("replacement");
  sendResult = async () => true; clearComposerDrafts();
});
if (process.env.BEANS_COMPOSER_TOUCH_SMOKE === "1") {
  console.log(JSON.stringify(checkComposerTouchOwnership()));
} else {
  test("production attachment touch avoids pill focus while ordinary pill taps preserve draft", checkComposerTouchOwnership);
}
