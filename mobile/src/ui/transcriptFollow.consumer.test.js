// Production ChatScreen callbacks with hook slots and native tags, matching existing phone consumers.
// Native layout, keyboard animation and gesture delivery still require installed-device verification.
import { mock, test } from 'bun:test';
import assert from 'node:assert/strict';
const platform = process.env.SMOKE_PLATFORM || 'ios';
let slots = [], cursor = 0, effects = [], dirty = false;
function slot(make) { const index = cursor++; if (!(index in slots))
    slots[index] = make(); return index; }
const react = {
    useRef: (value) => slots[slot(() => ({ current: value }))],
    useState: (value) => { const i = slot(() => value); return [slots[i], (next) => { const v = typeof next === 'function' ? next(slots[i]) : next; if (!Object.is(v, slots[i])) {
            slots[i] = v;
            dirty = true;
        } }]; },
    useMemo: (fn) => { cursor++; return fn(); },
    useCallback: (fn, deps) => { const i = slot(() => ({ fn, deps })); if (deps.some((v, j) => !Object.is(v, slots[i].deps[j]))) slots[i] = { fn, deps }; return slots[i].fn; },
    useEffect: (fn, deps) => { const i = slot(() => null); if (!slots[i] || deps.some((v, j) => !Object.is(v, slots[i].deps[j])))
        effects.push(() => { slots[i]?.cleanup?.(); slots[i] = { deps, cleanup: fn() }; }); },
    forwardRef: (fn) => fn,
};
mock.module('react', () => react);
const jsx = (type, props) => ({ type, props });
mock.module('react/jsx-runtime', () => ({ jsx, jsxs: jsx, Fragment: 'Fragment' }));
mock.module('react/jsx-dev-runtime', () => ({ jsxDEV: jsx, Fragment: 'Fragment' }));
mock.module('react-native', () => ({ Platform: { OS: platform }, PixelRatio: { get: () => 2 }, Alert: { alert() { } }, View: 'View', Text: 'Text', Pressable: 'Pressable', StyleSheet: { create: (x) => x, absoluteFill: {} } }));
mock.module('expo-router', () => ({ Stack: Object.assign(() => null, { Screen: 'Screen', Title: 'Title', Toolbar: Object.assign(() => null, { Button: 'ToolbarButton' }) }), useLocalSearchParams: () => ({ id: 'chat' }), useRouter: () => ({ push() { } }), useFocusEffect() { } }));
mock.module('expo-router/react-navigation', () => ({ useHeaderHeight: () => 64 }));
mock.module('expo-linear-gradient', () => ({ LinearGradient: 'Gradient' }));
mock.module('react-native-safe-area-context', () => ({ useSafeAreaInsets: () => ({ top: 24, bottom: 8 }) }));
mock.module('react-native-keyboard-controller', () => ({ KeyboardChatScrollView: 'Scroll', KeyboardController: { dismiss() { } }, useKeyboardHandler() { } }));
mock.module('react-native-reanimated', () => ({ default: { View: 'AnimatedView' }, useSharedValue: (value) => slots[slot(() => ({ value }))], useAnimatedStyle: () => ({}), useAnimatedReaction() { }, runOnJS: (x) => x, ZoomIn: { duration() { } }, ZoomOut: { duration() { } } }));
const chat = { id: 'chat', kind: 'dm', bot_ids: [], messages: [{ id: 'quoted', body: { kind: 'text', text: 'old' } }] };
let offset = 1000, height = 1800, scrolls = [];
const list = { scrollToEnd: (x) => scrolls.push(['end', x]), scrollToOffset: (x) => scrolls.push(['offset', x]), scrollToIndex: (x) => scrolls.push(['index', x]), getAbsoluteLastScrollOffset: () => offset, getWindowSize: () => ({ height: 800 }), getChildContainerDimensions: () => ({ height }), getFirstItemOffset: () => 0, getLayout: () => ({ y: 100, isHeightMeasured: true }) };
mock.module('../ui/AvatarFlashList', () => ({ AvatarFlashList: 'List' }));
mock.module('../../modules/beans-core/SoftScrollEdgeView', () => ({ SoftScrollEdgeView: 'Edge' }));
mock.module('../core/engine', () => ({ chatTitle: () => 'Chat', engine: { loadOlder() { }, sendMessage: async () => ({ id: 'sent' }) } }));
mock.module('../core/composerDraft', () => ({ composerDraft: () => ({}), editComposerDraft() { } }));
mock.module('../core/model', () => ({ canBeQuoted: () => true, isLive: () => false }));
const store = (select) => select({ statuses: {}, identityId: 'account', relayUrl: 'relay', deviceId: 'phone', paired: true });
store.subscribe = () => () => { };
mock.module('../core/store', () => ({ useStore: store, useChat: () => chat, useBotMap: () => new Map(), useWorkingBots: () => [], useIsWorking: () => false, useRunningTasks: () => [] }));
mock.module('../i18n', () => ({ t: (s) => s, useLanguage: () => ({ language: 'en' }) }));
for (const [path, exports] of Object.entries({ Avatar: { AvatarCluster: 'Avatar' }, Composer: { Composer: 'Composer', Surface: 'Surface' }, KeyboardFoot: { KeyboardFoot: 'Foot' }, layout: { useWide: () => false }, Symbol: { Symbol: 'Symbol' }, theme: { usePalette: () => ({ background: 'black' }) }, format: { quoteText: () => 'old' }, AnswerSheet: { AnswerSheet: 'Answer' }, transcript: { buildRows: () => [{ type: 'message', key: 'quoted', message: chat.messages[0] }], DayRow: 'Day', MarkerRow: 'Marker', MessageRow: 'Message', quoteAuthorName: () => 'Bot', NoticeRow: 'Notice', PermissionRow: 'Permission', CommandRow: 'Command', StatusRow: 'Status', WorkingRow: 'Working' } }))
    mock.module('../ui/' + path, () => exports);
// Production screen loads after native mocks; static import executes too early.
const { default: Screen } = await import('../../app/(main)/chat/[id]');
let root;
function nodes(node, type) { if (!node || typeof node !== 'object')
    return []; if (Array.isArray(node))
    return node.flatMap(n => nodes(n, type)); return [...(node.type === type ? [node] : []), ...nodes(node.props?.children, type)]; }
function render() { for (let i = 0; i < 10; i++) {
    cursor = 0;
    effects = [];
    dirty = false;
    root = Screen();
    nodes(root, 'List')[0].props.ref.current = list;
    effects.forEach(fn => fn());
    if (!dirty)
        return;
} throw Error('Render loop'); }
const frames = [];
globalThis.requestAnimationFrame = ((fn) => { frames.push(fn); return frames.length; });
test('transcript honors drag, fling, jump and quote reading while content grows', async () => {
render();
const props = () => nodes(root, 'List')[0].props;
const scroll = (distance) => { offset = height + 112 - 800 - distance; props().onScroll({ nativeEvent: { contentOffset: { y: offset }, contentSize: { height }, layoutMeasurement: { height: 800 } } }); };
nodes(root, 'Edge')[0].props.onLayout({ nativeEvent: { layout: { height: 800 } } });
props().onContentSizeChange(400, height);
props().onScrollBeginDrag();
scroll(300);
props().onScrollEndDrag();
await Bun.sleep(65);
scrolls = [];
height += 100;
props().onContentSizeChange(400, height);
assert.equal(scrolls.length, 0, 'reader scrolled up must stay put');
props().onScrollBeginDrag();
scroll(0);
props().onScrollEndDrag();
await Bun.sleep(65);
scrolls = [];
height += 100;
props().onContentSizeChange(400, height);
assert.equal(scrolls.filter(s => s[0] === 'end').length, 1, 'return to end resumes follow');
props().onScrollBeginDrag();
scroll(300);
props().onScrollEndDrag();
props().onMomentumScrollBegin();
await Bun.sleep(65);
scrolls = [];
height += 100;
props().onContentSizeChange(400, height);
assert.equal(scrolls.length, 0, 'fling must suppress follow');
props().onMomentumScrollEnd();
render();
nodes(root, 'Pressable').find(n => n.props.accessibilityLabel === 'Jump to bottom').props.onPress();
scrolls = [];
height += 100;
props().onContentSizeChange(400, height);
assert.equal(scrolls.filter(s => s[0] === 'end').length, 1, 'jump resumes follow');
const row = props().renderItem({ item: { type: 'message', message: chat.messages[0] } });
row.props.onQuotePress('quoted');
assert.equal(scrolls.at(-1)[0], 'index');
scrolls = [];
height += 100;
props().onContentSizeChange(400, height);
assert.equal(scrolls.length, 0, 'quoted message must remain visible while reply streams');
for (const item of slots)
    item?.cleanup?.();
});
