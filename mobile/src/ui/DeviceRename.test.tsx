import { expect, test, vi } from "vitest";
import type { Device } from "../core/model";

  // Follow the existing mobile hook-slot boundary; native presentation needs device proof.
  type Node = { type: unknown; props: { children?: unknown; value?: { value: string }; onPress?: () => void; onClick?: () => void; onDismissRequest?: () => void } };
  type Button = { text: string; onPress?: (value?: string) => void };
  let slots: unknown[] = [], cursor = 0;
  let effects: Array<() => void> = [], cleanups: Array<() => void> = [];
  function slot<T>(make: () => T) { const index = cursor++; if (!(index in slots)) slots[index] = make(); return { index, value: slots[index] as T }; }
  const react = {
    useRef: <T,>(value: T) => slot(() => ({ current: value })).value,
    useState: <T,>(value: T) => { const item = slot(() => value); return [item.value, (next: T) => { slots[item.index] = next; }]; },
    useEffect: (effect: () => () => void) => { const item = slot(() => false); if (!item.value) { slots[item.index] = true; effects.push(() => { cleanups.push(effect()); }); } },
    useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
    useDebugValue: () => {},
    useMemo: <T,>(compute: () => T) => compute(),
    useCallback: <T,>(callback: T) => callback,
  };
  vi.doMock("react", () => ({ ...react, default: react }));
  const jsx = (type: unknown, props: Node["props"]) => ({ type, props });
  vi.doMock("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
  vi.doMock("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
  const platform = { OS: "ios" };
  let buttons: Button[] = [], alerts: string[] = [];
  vi.doMock("react-native", () => ({ Platform: platform, AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) }, Alert: {
    prompt: (_title: string, _message: string, actions: Button[]) => { buttons = actions; },
    alert: (title: string) => { alerts.push(title); },
  } }));
  vi.doMock("@expo/ui/jetpack-compose", () => ({ useNativeState: (value: string) => slot(() => ({ value })).value, Column: "Column", Host: "Host", Text: "Text", TextButton: "Button", OutlinedTextField: Object.assign(() => null, { displayName: "Input", Placeholder: "Placeholder" }), AlertDialog: Object.assign(() => null, { displayName: "Dialog", Title: "Title", Text: "Body", ConfirmButton: "Confirm", DismissButton: "Dismiss" }) }));
  vi.doMock("@expo/ui/jetpack-compose/modifiers", () => ({ fillMaxWidth: () => ({}) }));
  vi.doMock("./forms", () => ({ Row: "Row" }));
  vi.doMock("../i18n", () => ({ t: (value: string) => value }));
  vi.doMock("expo-web-browser", () => ({}));
  vi.doMock("../core/host", () => ({ hostFacts: () => ({}) }));
  vi.doMock("../core/prefs", () => ({ coreHome: () => "unused", loadPrefs: () => ({}), savePrefs: () => {}, pathOf: (value: string) => value, wipePrefs: () => {} }));
  vi.doMock("../core/push", () => ({ clearPushes: () => {}, installPushHandlers: () => {}, registerForPushes: () => {} }));
  const calls: Array<{ method: string; params: unknown }> = [];
  let reply: () => Promise<unknown> = async () => null;
  vi.doMock("../../modules/beans-core", () => ({ request: (method: string, params: unknown) => { calls.push({ method, params }); return reply(); } }));
  const { useStore } = await import("../core/store");
  const { deviceName } = await import("../core/model");
  const { DeviceRename } = await import("./DeviceRename");
  const device: Device = { id: "target", name: "Alias", machine_name: "Original", model: "", os: "android", os_version: "", machine_key: "target", is_this_device: false, status: "offline", last_seen: 0, unknown: false };
  function find(node: unknown, type: unknown): Node | undefined {
    if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
    if (!node || typeof node !== "object" || !("type" in node) || !("props" in node)) return;
    const item = node as Node;
    return item.type === type || (item.type as { displayName?: unknown } | null)?.displayName === type ? item : find(item.props.children, type);
  }
  test.each(["ios", "android"])("%s naming targets paired Devices and revokes stale prompts", async os => {
    slots = []; cursor = 0; effects = []; cleanups = []; calls.length = 0; alerts = []; platform.OS = os; reply = async () => null;
    useStore.setState({ paired: true, identityId: "account", deviceId: "phone", relayUrl: "relay", devices: [device] });
    let root: unknown;
    const render = (target = device) => { cursor = 0; root = DeviceRename({ device: target }); for (const effect of effects.splice(0)) effect(); };
    const open = () => { render(); find(root, "Row")!.props.onPress!(); render(); };
    const submit = async (name: string) => {
      if (os === "ios") buttons[1]!.onPress!(name);
      else { find(root, "Input")!.props.value!.value = name; find(find(root, "Confirm"), "Button")!.props.onClick!(); }
      await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    };
    try {
      expect(deviceName(device)).toBe("Alias");
      render({ ...device, unknown: true }); expect(find(root, "Row")).toBeUndefined();
      open();
      if (os === "ios") buttons[0]!.onPress!(); else find(root, "Dialog")!.props.onDismissRequest!();
      expect(calls).toEqual([]);
      open(); await submit("  " + "😀".repeat(64) + "  ");
      expect(calls).toEqual([{ method: "device.set_name", params: { id: "target", name: "😀".repeat(64) } }]);
      expect(useStore.getState().devices[0]!.name).toBe("Alias");
      open(); await submit(" ");
      expect(calls.at(-1)).toEqual({ method: "device.set_name", params: { id: "target", name: "" } });
      open(); await submit("\uFEFF");
      expect(calls.at(-1)).toEqual({ method: "device.set_name", params: { id: "target", name: "\uFEFF" } });
      const count = calls.length;
      open(); await submit("😀".repeat(65)); expect(calls.length).toBe(count); expect(alerts).toContain("Could not rename Device");
      open();
      if (os === "android") find(root, "Input")!.props.value!.value = "stale";
      const staleSubmit = os === "ios" ? () => buttons[1]!.onPress!("stale") : find(find(root, "Confirm"), "Button")!.props.onClick!;
      useStore.setState({ identityId: "other" }); useStore.setState({ identityId: "account" });
      staleSubmit(); await Promise.resolve(); expect(calls.length).toBe(count);
      open();
      const wrongTarget = os === "ios" ? () => buttons[1]!.onPress!("wrong target") : find(find(root, "Confirm"), "Button")!.props.onClick!;
      render({ ...device, id: "other" });
      wrongTarget(); await Promise.resolve(); expect(calls.length).toBe(count);
      render();
      const { promise, reject } = Promise.withResolvers<unknown>();
      reply = () => promise;
      open(); await submit("pending");
      const before = alerts.length;
      useStore.setState({ relayUrl: "other" }); useStore.setState({ relayUrl: "relay" });
      reject(new Error("old account failure")); await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
      expect(alerts.length).toBe(before);
      const failure = Promise.withResolvers<unknown>();
      reply = () => failure.promise;
      open();
      const dismiss = os === "android" ? find(root, "Dialog")!.props.onDismissRequest : undefined;
      await submit("retry");
      dismiss?.();
      failure.reject(new Error("current failure"));
      await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
      expect(alerts.length).toBe(before + 1);
      open();
      const removed = os === "ios" ? () => buttons[1]!.onPress!("removed") : find(find(root, "Confirm"), "Button")!.props.onClick!;
      const admitted = calls.length;
      useStore.setState({ devices: [] }); useStore.setState({ devices: [device] });
      removed(); await Promise.resolve(); expect(calls.length).toBe(admitted);
      open();
      const changedOs = os === "ios" ? () => buttons[1]!.onPress!("changed OS") : find(find(root, "Confirm"), "Button")!.props.onClick!;
      useStore.setState({ devices: [{ ...device, os: "ios" }] });
      useStore.setState({ devices: [device] });
      changedOs(); await Promise.resolve(); expect(calls.length).toBe(admitted);
    } finally { for (const cleanup of cleanups) cleanup(); }
  });
