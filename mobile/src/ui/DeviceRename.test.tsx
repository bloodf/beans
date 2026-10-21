import { expect, mock, test } from "bun:test";
import { fileURLToPath } from "node:url";
import type { Device } from "../core/model";

if (process.env.BEANS_DEVICE_RENAME_HOST === "1") install();
else test("Device naming consumer in an isolated Bun host", () => {
  const child = Bun.spawnSync([process.execPath, "test", "--isolate", fileURLToPath(import.meta.url)], {
    env: { ...process.env, BEANS_DEVICE_RENAME_HOST: "1" }, stdout: "pipe", stderr: "pipe",
  });
  expect(child.exitCode, child.stdout.toString() + child.stderr.toString()).toBe(0);
});

function install() {
  // Follow the existing mobile hook-slot boundary; native presentation needs device proof.
  type Node = { type: unknown; props: { children?: unknown; value?: { value: string }; onValueChange?: (value: string) => void; onPress?: () => void; onClick?: () => void; onDismissRequest?: () => void } };
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
  };
  mock.module("react", () => ({ ...react, default: react }));
  const jsx = (type: unknown, props: Node["props"]) => typeof type === "function" ? type(props) : { type, props };
  mock.module("react/jsx-runtime", () => ({ jsx, jsxs: jsx, Fragment: "Fragment" }));
  mock.module("react/jsx-dev-runtime", () => ({ jsxDEV: jsx, Fragment: "Fragment" }));
  const platform = { OS: "ios" };
  let buttons: Button[] = [], alerts: string[] = [];
  mock.module("react-native", () => ({ Platform: platform, AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) }, Alert: {
    prompt: (_title: string, _message: string, actions: Button[]) => { buttons = actions; },
    alert: (title: string) => { alerts.push(title); },
  } }));
  mock.module("@expo/ui", () => ({ useNativeState: (value: string) => ({ value }) }));
  mock.module("@expo/ui/jetpack-compose", () => ({ Column: "Column", Host: "Host", Text: "Text", TextButton: "Button", OutlinedTextField: Object.assign("Input", { Placeholder: "Placeholder" }), AlertDialog: Object.assign("Dialog", { Title: "Title", Text: "Body", ConfirmButton: "Confirm", DismissButton: "Dismiss" }) }));
  mock.module("@expo/ui/jetpack-compose/modifiers", () => ({ fillMaxWidth: () => ({}) }));
  mock.module("./forms", () => ({ Row: "Row" }));
  mock.module("../i18n", () => ({ t: (value: string) => value }));
  mock.module("expo-web-browser", () => ({}));
  mock.module("../core/host", () => ({ hostFacts: () => ({}) }));
  mock.module("../core/prefs", () => ({ coreHome: () => "unused", loadPrefs: () => ({}), savePrefs: () => {}, pathOf: (value: string) => value, wipePrefs: () => {} }));
  mock.module("../core/push", () => ({ clearPushes: () => {}, installPushHandlers: () => {}, registerForPushes: () => {} }));
  const calls: Array<{ method: string; params: unknown }> = [];
  let reply: () => Promise<unknown> = async () => null;
  mock.module("../../modules/beans-core", () => ({ request: (method: string, params: unknown) => { calls.push({ method, params }); return reply(); } }));
  const { useStore }: typeof import("../core/store") = require("../core/store");
  const { deviceName }: typeof import("../core/model") = require("../core/model");
  const { DeviceRename }: typeof import("./DeviceRename") = require("./DeviceRename");
  const device: Device = { id: "target", name: "Alias", machine_name: "Original", model: "", os: "android", os_version: "", machine_key: "target", is_this_device: false, status: "offline", last_seen: 0 };
  function find(node: unknown, type: unknown): Node | undefined {
    if (Array.isArray(node)) return node.map(child => find(child, type)).find(Boolean);
    if (!node || typeof node !== "object" || !("type" in node) || !("props" in node)) return;
    const item = node as Node;
    return item.type === type ? item : find(item.props.children, type);
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
      render();
      open();
      if (os === "android") expect(find(root, "Input")!.props.value!.value).toBe(device.name);
      if (os === "ios") buttons[0]!.onPress!(); else find(root, "Dialog")!.props.onDismissRequest!();
      open();
      if (os === "ios") buttons[1]!.onPress!(device.name);
      else find(find(root, "Confirm"), "Button")!.props.onClick!();
      await Promise.resolve(); expect(calls).toEqual([]);
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
    } finally { for (const cleanup of cleanups) cleanup(); }
  });
}
