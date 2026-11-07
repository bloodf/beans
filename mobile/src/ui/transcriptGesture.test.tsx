import { expect, test, vi } from "vitest";

// Only unavailable runtime boundaries are replaced. MessageRow and its gesture callbacks run.
type Node = { type: any; props: any };
const jsx = (type: any, props: any): Node => typeof type === "function" ? type(props) : { type, props };
vi.doMock("react", () => ({ useMemo: (fn: any) => fn(), useRef: (value: any) => ({ current: value }), useEffect: () => {}, useState: (value: any) => [value, () => {}] }));
vi.doMock("react/jsx-runtime", () => ({ jsx, jsxs: jsx }));
vi.doMock("react/jsx-dev-runtime", () => ({ jsxDEV: jsx }));
vi.doMock("react-native", () => ({ Alert: {}, Linking: {}, View: "View", Text: "Text", Pressable: "Pressable", ScrollView: "ScrollView", Modal: "Modal", Platform: { OS: "ios" }, StyleSheet: { create: (value: any) => value } }));
vi.doMock("react-native-reanimated", () => ({ default: { View: "AnimatedView" }, useSharedValue: (value: any) => ({ value }), useAnimatedStyle: (fn: any) => fn, withSpring: (value: any) => value, withSequence: () => 1, withTiming: (value: any) => value }));
let haptics = 0;
vi.doMock("expo-haptics", () => ({ ImpactFeedbackStyle: { Light: "light" }, impactAsync: () => { haptics++; return Promise.resolve(); } }));
vi.doMock("expo-clipboard", () => ({}));
vi.doMock("expo-linear-gradient", () => ({ LinearGradient: "Gradient" }));
vi.doMock("../../modules/beans-core/ShimmerView", () => ({ ShimmerView: "Shimmer" }));
vi.doMock("../core/engine", () => ({ engine: new Proxy({}, { get: () => () => { throw new Error("gesture must not send or request"); } }) }));
vi.doMock("../core/store", () => ({ useStore: () => ({}) }));
vi.doMock("../i18n", () => ({ useLanguage: () => {}, t: (value: string) => value, language: "en" }));
vi.doMock("./attachments", () => ({ AttachmentBlock: "Attachments" }));
vi.doMock("./Avatar", () => ({ BotAvatar: "Avatar" }));
vi.doMock("./Markdown", () => ({ Markdown: "Markdown" }));
vi.doMock("./outputs", () => ({ OutputCard: "OutputCard" }));
vi.doMock("./Symbol", () => ({ Symbol: "Symbol" }));
vi.doMock("./layout", () => ({ usePaneWidth: () => 400 }));
vi.doMock("./theme", () => ({ Font: {}, usePalette: () => ({}) }));
vi.doMock("expo-image", () => ({ Image: "Image" }));
vi.doMock("expo-router", () => ({ useRouter: () => ({ push: () => {} }) }));

// Native recognizer boundary: deliver lifecycle events, not a copy of reply logic.
class Pan {
  config: Record<string, any> = {};
  runOnJS(v: boolean) { this.config.js = v; return this; }
  enabled(v: boolean) { this.config.enabled = v; return this; }
  activeOffsetX(v: number | number[]) { this.config.activeX = v; return this; }
  failOffsetX(v: number) { this.config.failX = v; return this; }
  failOffsetY(v: number[]) { this.config.failY = v; return this; }
  onTouchesDown(fn: any) { this.config.down = fn; return this; }
  onUpdate(fn: any) { this.config.update = fn; return this; }
  onEnd(fn: any) { this.config.end = fn; return this; }
  onFinalize(fn: any) { this.config.finalize = fn; return this; }
}
vi.doMock("react-native-gesture-handler", () => ({ Gesture: { Pan: () => new Pan() }, GestureDetector: "Detector" }));
const { MessageRow } = await import("./transcript");
const children = (node: Node): Node[] => [node.props.children].flat().filter(Boolean);
const find = (node: Node, predicate: (node: Node) => boolean): Node | undefined => predicate(node) ? node : children(node).map((child) => typeof child === "object" ? find(child, predicate) : undefined).find(Boolean);

export function gestureConsumerScenario() {
  let replies = 0;
  const row: any = { type: "message", message: { id: "m", chat_id: "c", author: { kind: "you" }, body: { kind: "text", text: "draft target" }, state: { kind: "sent" } }, groupStart: true, groupEnd: true };
  const root = MessageRow({ row, bots: new Map(), isGroup: false, onReply: () => { replies++; } }) as Node;
  const detector = find(root, (node) => node.type === "Detector")!;
  // The hit target must be the bubble, not row whitespace, avatar, quote or send-now.
  const bubble = children(detector)[0];
  expect(bubble.type).toBe("AnimatedView");
  expect(find(bubble, (node) => node.type === "Markdown")).toBeDefined();
  expect(find(bubble, (node) => Array.isArray(node.props.style) && node.props.style.some((style: any) => style?.flexDirection === "row"))).toBeUndefined();
  expect(root.type).toBe("View");
  const pan: Pan = detector.props.gesture;
  const follow = find(root, (node) => node.type === "AnimatedView" && typeof node.props.style === "function" && node.props.style().transform?.[0]?.translateX !== undefined)!;
  const offset = () => follow.props.style().transform[0].translateX;
  const down = (x: number) => {
    let failed = false;
    pan.config.down({ allTouches: [{ absoluteX: x }] }, { fail: () => { failed = true; } });
    return failed;
  };
  // Boundary-first admission: a touch without a detector ancestor belongs to navigation.
  const dragFrom = (target: Node, absoluteX: number) => {
    if (target !== bubble || down(absoluteX)) {
      pan.config.finalize();
      return;
    }
    pan.config.update({ translationX: 80 });
    pan.config.end({}, true);
    pan.config.finalize();
  };
  dragFrom(root, 120);
  dragFrom(bubble, 27);
  expect(replies).toBe(0);
  expect(offset()).toBe(0);
  expect(down(28)).toBe(false);
  pan.config.update({ translationX: 80 });
  expect(offset()).toBeCloseTo(78.4);
  pan.config.end({}, true);
  pan.config.finalize();
  expect(replies).toBe(1);
  expect(offset()).toBe(0);
  // Retreat below threshold; cancellation after crossing (native code scroll wins).
  pan.config.update({ translationX: 60 });
  pan.config.update({ translationX: 40 });
  pan.config.end({}, true);
  pan.config.finalize();
  pan.config.update({ translationX: 60 });
  pan.config.end({}, false);
  pan.config.finalize();
  expect(replies).toBe(1);
  expect(offset()).toBe(0);
  // Native boundary supplies failure/cancellation rather than synthesizing a reply.
  // Apply configured pre-activation limits to vertical and leftward streams.
  const rejectedDrag = (x: number, y: number) => {
    const fails = x < pan.config.failX || y < pan.config.failY[0] || y > pan.config.failY[1];
    if (!fails && x > pan.config.activeX) {
      pan.config.update({ translationX: x });
      pan.config.end({}, true);
    }
    pan.config.finalize();
  };
  rejectedDrag(8, 30);
  rejectedDrag(-80, 0);
  expect(replies).toBe(1);
  expect(offset()).toBe(0);
  const disabled = MessageRow({ row, bots: new Map(), isGroup: false }) as Node;
  expect(find(disabled, (node) => node.type === "Detector")!.props.gesture.config.enabled).toBe(false);
  expect(haptics).toBe(3);
  return { replies, offset: offset(), bubbleOnly: true, edgeExcluded: true, canceledReplyRejected: true };
}

if (process.env.BEANS_GESTURE_SMOKE === "1") console.log(JSON.stringify(gestureConsumerScenario()));
else test("bubble reply accepts right swipe and rejects edge, whitespace, retreat and native cancellation", () => { gestureConsumerScenario(); });
