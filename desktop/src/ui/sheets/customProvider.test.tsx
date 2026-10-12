// Run checkCustomProviderRefresh through the installed Solid Vite plugin; the synthetic transport
// stands in for the CLI, so no Runner, account or provider call is made. Mounts the real sheet.
import { render } from "@solidjs/web";
import { flush } from "solid-js";
import { presentCustomProvider } from "./customProvider";
import { PageMenuHost } from "../menu";
import { SheetHost } from "../overlay";
import { setLanguage } from "../../l10n";
import { store } from "../../model/store";
import { toDevice } from "../../model/wire";
import type { ProviderCredential } from "../../model/models";

const provider = (models: ProviderCredential["models"]): ProviderCredential => ({
  kind: "custom:lab",
  isConnected: true,
  detail: "sk-…4f2c · http://127.0.0.1:11434/v1",
  name: "Lab",
  api: "chat-completions",
  baseURL: "http://127.0.0.1:11434/v1",
  models,
});

const saved = provider([{ id: "qwen3", levels: ["low"] }]);

export async function checkCustomProviderRefresh() {
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  setLanguage("en");
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  const internals = store as unknown as {
    transport: unknown;
    emit: (event: { kind: "identityChanged" | "rosterChanged" }) => void;
  };
  const realTransport = internals.transport;
  const real = { devices: store.devices, providers: store.providers, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected };
  const runner = toDevice({ id: "runner-1", name: "Studio", os: "macos", model: "Fixture", os_version: "fixture", is_this_device: true, status: "online", last_seen: 1, machine_key: "runner-key", plugins: [] });
  store.devices = [runner];
  store.providers = [saved];
  store.identityID = "account-1";
  store.hasIdentity = true;
  store.isConnected = true;
  let reply: (value: unknown) => void = () => {};
  let calls = 0;
  let failWith: unknown = null;
  internals.transport = {
    request: (method: string) => {
      if (method === "providers.api_key") return Promise.resolve({ api_key: "sk-saved" });
      if (method === "providers.refresh") {
        calls++;
        const failure = failWith;
        return new Promise((resolve, reject) => {
          reply = (value) => (failure ? reject(failure) : resolve(value));
        });
      }
      return Promise.reject(new Error(`Unrelated request outside the refresh fixture: ${method}`));
    },
  };
  const dispose = render(() => <><SheetHost /><PageMenuHost /></>, root);
  const settle = async () => { await Promise.resolve(); flush(); await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const button = (text: string) => [...root.querySelectorAll("button")].find((each) => each.textContent?.trim() === text) as HTMLButtonElement | undefined;
  const text = () => root.textContent ?? "";
  const type = (label: string, value: string) => {
    const field = [...root.querySelectorAll<HTMLElement>("[aria-label]")].find((each) => each.getAttribute("aria-label") === label)!;
    field.querySelector("input")!.value = value;
    field.querySelector("input")!.dispatchEvent(new Event("input", { bubbles: true }));
    flush();
  };
  try {
    await presentCustomProvider("custom:lab");
    await settle();
    assert(button("Refresh Models"), "Refresh Models absent on an edited provider");

    // Unsaved edits to the source: the Runner would check the saved provider, so Refresh waits for Save.
    type("API base URL", "http://127.0.0.1:1234/v1");
    await settle();
    assert(button("Refresh Models")!.disabled, "Refresh stayed available with an unsaved base URL");
    assert(text().includes("Save these changes first"), "Save-first note absent");
    type("API base URL", "http://127.0.0.1:11434/v1");
    await settle();
    assert(!button("Refresh Models")!.disabled, "Refresh stayed disabled after the edit was undone");

    // An unrelated roster change must not settle the refresh, and neither must an account swap.
    button("Refresh Models")!.click();
    await settle();
    assert(calls === 1, "Refresh sent something other than exactly one providers.refresh");
    internals.emit({ kind: "rosterChanged" });
    await settle();
    assert(text().includes("Loading models…"), "An unrelated roster event settled the refresh");
    store.identityID = "account-2";
    internals.emit({ kind: "identityChanged" });
    store.providers = [provider([{ id: "qwen3", levels: ["low"] }, { id: "gemma3", levels: ["low"] }])];
    internals.emit({ kind: "rosterChanged" });
    reply({ updated: 1 });
    await settle();
    assert(text().includes("has not reached this Device yet"), "A provider from another account settled the refresh");

    // Same account, same Runner, the refreshed catalog arrives on the roster: same ids, new metadata.
    store.identityID = "account-1";
    button("Refresh Models")!.click();
    await settle();
    reply({ updated: 1 });
    await settle();
    internals.emit({ kind: "rosterChanged" });
    await settle();
    store.providers = [provider([{ id: "qwen3", levels: ["low", "high"], contextWindow: 32768 }, { id: "gemma3", levels: ["low"] }])];
    internals.emit({ kind: "rosterChanged" });
    await settle();
    assert(text().includes("Models updated"), "The refreshed catalog did not settle the refresh");
    assert(text().includes("gemma3"), "A model the refreshed catalog added is missing from the sheet");

    // A reply that promises nothing changes settles as unchanged without waiting for an event.
    button("Refresh Models")!.click();
    await settle();
    reply({ updated: 0 });
    await settle();
    assert(text().includes("Models unchanged"), "An unchanged reply did not settle the refresh");

    // A failure leaves the sheet idle with the reason, and no silent second attempt.
    failWith = new Error("Runner refused the catalog");
    button("Refresh Models")!.click();
    await settle();
    reply(undefined);
    await settle();
    assert(text().includes("Runner refused the catalog"), "The refresh failure did not reach the status line");
    assert(!button("Refresh Models")!.disabled, "The sheet stayed busy after a failed refresh");
    assert(calls === 4, `Expected exactly four refresh requests, saw ${calls}`);
  } finally {
    dispose();
    internals.transport = realTransport;
    Object.assign(store, real);
  }
}