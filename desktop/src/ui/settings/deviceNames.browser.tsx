import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../../l10n";
import { store } from "../../model/store";
import type { WireSnapshot } from "../../model/wire";
import { settingsDeviceID } from "../root";
import { SheetHost } from "../overlay";
import { SettingsPage } from "./panes";

export async function checkDeviceNamingConsumer() {
  const assert = (value: unknown, message: string) => { if (!value) throw new Error(message); };
  const settle = async () => { await Promise.resolve(); flush(); await new Promise((r) => setTimeout(r, 0)); flush(); };
  setLanguage("en");
  Object.defineProperty(store, "isMock", { configurable: true, get: () => false });
  const events = store as unknown as { handle(name: string, data: unknown): void };
  const snapshot = (identity = "account-a"): WireSnapshot => ({
    version: "fixture", identity_id: identity, has_identity: true, is_identity_device: true,
    relay_connected: false, running_chat_ids: [], bots: [], chats: [], devices: ["peer", "other"].map((id) => ({
      id, name: "Original", machine_name: "Machine", model: "PC", os: "linux", os_version: "",
      is_this_device: false, status: "offline", last_seen: 0, machine_key: id, plugins: [], version: "fixture",
    })),
  });
  events.handle("snapshot", snapshot());
  store.setConnected(true);
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let resolve!: (value: unknown) => void;
  let reject!: (error: Error) => void;
  Object.defineProperty(store, "transport", { configurable: true, value: {
    request: (method: string, params: Record<string, unknown>) => {
      calls.push({ method, params });
      return new Promise((yes, no) => { resolve = yes; reject = no; });
    },
  } });
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  settingsDeviceID.set("peer");
  const dispose = render(() => <><SettingsPage pane="device" /><SheetHost /></>, root);
  const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((x) => x.textContent?.trim() === text);
  const dialogs = () => document.querySelectorAll("[role=dialog]").length;
  const open = async () => {
    await settle();
    const rename = button("Rename…");
    assert(rename, "Device pane has no Rename action"); rename!.click(); await settle();
    assert(dialogs() === 1, "Naming prompt did not mount");
    const input = document.querySelector<HTMLInputElement>("[role=dialog] input")!;
    assert(input?.value === "Original" && input.placeholder === "Machine", "Alias and machine names are not distinct");
    return input;
  };
  const submit = async (value: string) => {
    const input = await open(); input.value = value; input.dispatchEvent(new Event("input", { bubbles: true })); await settle();
    button("Rename")!.click(); await settle();
    assert(button("Rename")!.disabled, "Pending alias can be submitted twice");
  };
  try {
    await submit("");
    assert(calls.length === 1 && calls[0]!.method === "device.set_name" && calls[0]!.params.id === "peer" && calls[0]!.params.name === "", `Blank reset did not reach the real store transport: ${JSON.stringify(calls)}`);
    resolve(null); await settle(); assert(dialogs() === 0, "Accepted alias did not close prompt");
    await submit("Office"); reject(new Error("Synthetic refusal")); await settle();
    assert(document.querySelector("[role=alert]")?.textContent?.includes("Synthetic refusal"), "Current refusal is not visible");
    assert(document.querySelector<HTMLInputElement>("[role=dialog] input")?.value === "Office", "Refusal discarded the draft");
    button("Cancel")!.click(); await settle();
    for (const transition of ["account", "reconnect", "target", "cancel"] as const) {
      settingsDeviceID.set("peer"); events.handle("snapshot", snapshot()); store.setConnected(true); await settle();
      const owner = store.deviceNamingOwner("peer");
      await submit("Pending");
      if (transition === "account") { events.handle("snapshot", snapshot("account-b")); events.handle("snapshot", snapshot()); }
      if (transition === "reconnect") { store.setConnected(false); store.setConnected(true); }
      if (transition === "target") settingsDeviceID.set("other");
      if (transition === "cancel") button("Cancel")!.click();
      await settle(); assert(dialogs() === 0, `${transition} retained the prompt`);
      reject(new Error("Late refusal")); await settle();
      assert(!document.querySelector("[role=alert]"), `${transition} published a stale failure`);
      assert(store.device("peer")?.name === "Original", `${transition} changed a replacement view`);
      if (transition === "account" || transition === "reconnect") {
        assert(!owner(), `${transition} restored stale ownership`);
        const before = calls.length; await store.setDeviceName("peer", "Late", owner);
        assert(calls.length === before, `${transition} admitted a stale request`);
      }
    }
    return { reset: "transport verified", refusal: "draft retained", transitions: 4, lateErrors: "suppressed", accountABA: "rejected" };
  } finally { dispose(); root.remove(); }
}
