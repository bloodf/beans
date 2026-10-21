// Mounted Settings consumer check; synthetic RPC transport, no CLI or live home.
import { render } from "@solidjs/web";
import { flush } from "solid-js";
import { store } from "../../model/store";
import { toDevice } from "../../model/wire";
import { settingsDeviceID } from "../root";
import { SheetHost } from "../overlay";
import { SettingsPage } from "./panes";

export async function checkDeviceNamingConsumer() {
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  const saved = { devices: store.devices, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected, selection: settingsDeviceID.get() };
  const internal = store as unknown as { transport: unknown; emit(event: { kind: "identityChanged" | "rosterChanged" }): void };
  const transport = internal.transport;
  const calls: { params: Record<string, unknown>; resolve(value: unknown): void; reject(error: Error): void }[] = [];
  internal.transport = { request(method: string, params: Record<string, unknown>) {
    if (method !== "device.set_name") throw new Error(`Unexpected naming fixture RPC: ${method}`);
    return new Promise((resolve, reject) => calls.push({ params, resolve, reject }));
  } };
  const wire = (id: string, name: string) => ({ id, name, machine_name: `Machine ${id}`, model: "Fixture", os: "linux", os_version: "fixture", machine_key: `key-${id}`, is_this_device: false, status: "offline", last_seen: 1 });
  store.devices = [toDevice(wire("a", "Office")), toDevice(wire("b", "Travel"))];
  store.identityID = "naming-account-a"; store.hasIdentity = true; store.isConnected = true;
  settingsDeviceID.set("a");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><SettingsPage pane="device" /><SheetHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
  const button = (scope: ParentNode, title: string) => {
    const result = [...scope.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.trim() === title);
    if (!result) throw new Error(`Missing mounted button: ${title}`);
    return result;
  };
  const field = () => {
    const input = dialog()?.querySelector<HTMLInputElement>('input[aria-label="Name"]');
    if (!input) throw new Error("Actual Device naming field is absent");
    return input;
  };
  const open = async () => { button(root, "Rename…").click(); await settle(); assert(dialog()?.getAttribute("aria-labelledby") || dialog()?.getAttribute("aria-label"), "Naming dialog must have an accessible name"); };
  const type = (name: string) => { field().value = name; field().dispatchEvent(new Event("input", { bubbles: true })); flush(); };
  const confirm = async () => { button(dialog()!, "Rename").click(); await settle(); };
  const publish = (name: string) => { store.devices = [toDevice(wire("a", name)), toDevice(wire("b", "Travel"))]; internal.emit({ kind: "rosterChanged" }); };
  try {
    await settle();
    await open();
    assert(field().value === "Office" && field().placeholder === "Machine a", "Prompt must separate effective and original names");
    type("Studio"); await confirm();
    assert(calls.length === 1 && calls[0]!.params.id === "a" && calls[0]!.params.name === "Studio", "Rename must submit the selected offline Device explicitly");
    assert(store.device("a")?.name === "Office", "Pending RPC must not optimistically replace account names");
    publish("Studio"); calls[0]!.resolve(null); await settle();
    assert(!dialog() && root.querySelector(".device-header-name")?.textContent === "Studio", "Authoritative rename must render and close prompt");
    assert(store.device("a")?.machineName === "Machine a", "Alias must not replace original machine name");

    await open(); type(""); await confirm();
    assert(calls.length === 2 && calls[1]!.params.id === "a" && calls[1]!.params.name === "", "Confirmed blank must send reset, not cancel");
    publish("Machine a"); calls[1]!.resolve(null); await settle();
    assert(!dialog() && root.querySelector(".device-header-name")?.textContent === "Machine a", "Reset must render authoritative machine name");
    await open(); type("Cancelled"); button(dialog()!, "Cancel").click(); await settle();
    assert(!dialog() && calls.length === 2, "Cancel must dismiss without RPC");

    await open(); type("Wrong target"); settingsDeviceID.set("b"); await settle();
    assert(!dialog() && calls.length === 2, "Selection change must revoke unsubmitted prompt");
    await open(); assert(field().value === "Travel", "New target must open its own name");
    type("Old account"); store.identityID = "naming-account-b"; internal.emit({ kind: "identityChanged" }); await settle();
    assert(!dialog() && calls.length === 2, "Account change must revoke unsubmitted prompt");

    await open(); type("Pending old account"); await confirm();
    assert(calls.length === 3 && calls[2]!.params.id === "b", "New account prompt must use its selected target");
    store.identityID = "naming-account-c"; internal.emit({ kind: "identityChanged" }); await settle();
    await open(); type("Current account draft"); calls[2]!.reject(new Error("PRIVATE_STALE_ACCOUNT_ERROR")); await settle();
    assert(field().value === "Current account draft" && !dialog()?.textContent?.includes("PRIVATE_STALE_ACCOUNT_ERROR"), "Old account error must not affect current prompt");
    button(dialog()!, "Cancel").click(); await settle();

    await open(); type("Pending old target"); await confirm();
    assert(calls.length === 4 && calls[3]!.params.id === "b", "Pending target request must retain explicit id");
    settingsDeviceID.set("a"); await settle(); await open(); type("Current target draft");
    calls[3]!.resolve(null); await settle();
    assert(field().value === "Current target draft", "Old target success must not dismiss or rewrite new prompt");
    button(dialog()!, "Cancel").click(); await settle();
    return { mountedSettings: true, offlineRename: "explicit-id", blankReset: "submitted", cancel: "no-rpc", accountSwitch: "fenced", targetSwitch: "fenced", staleError: "suppressed", staleSuccess: "fenced" };
  } finally {
    dispose(); root.remove(); internal.transport = transport;
    store.devices = saved.devices; store.identityID = saved.identityID; store.hasIdentity = saved.hasIdentity; store.isConnected = saved.isConnected;
    settingsDeviceID.set(saved.selection);
  }
}
