// Browser consumer check: Settings › Secrets (#560) lists a Runner's secrets by name and length,
// adds one typed in (its value sent once, never shown), deletes one after a confirmation, and
// revokes what was allowed always. The store's Runner calls are stubbed; no CLI is used.
import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../../l10n";
import type { Device } from "../../model/models";
import type { WireSecrets } from "../../model/secrets";
import { store } from "../../model/store";
import { SheetHost } from "../overlay";
import { SecretsSettings } from "./secretsPane";

export async function checkSecretsPane(): Promise<Record<string, string>> {
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  document.body.append(root);
  const typed = "tok-TYPED-IN-VALUE-0042";
  let kept: WireSecrets = {
    secrets: [{ name: "STRIPE_KEY", length: 35, source: "typed in Beans on Studio", note: "billing", saved_by: "", updated_at: 1 }],
    grants: [
      { kind: "tool", tool: "create_charge", secrets: ["STRIPE_KEY"], granted_at: 1_763_000_000 },
      { kind: "command", folder: "/example/user/shop", command: "vercel env add STRIPE_KEY {{secret:STRIPE_KEY}}", secrets: ["STRIPE_KEY"], granted_at: 1_763_000_000 },
    ],
  };
  const calls: string[] = [];
  const saved = { listSecrets: store.listSecrets, setSecret: store.setSecret, deleteSecret: store.deleteSecret, revokeSecretGrants: store.revokeSecretGrants };
  store.listSecrets = async () => kept;
  store.setSecret = async (_runner, name, value, note) => {
    calls.push(`set ${name} ${value === typed ? "<typed>" : "<other>"} ${note}`);
    kept = { ...kept, secrets: [...kept.secrets, { name, length: value.length, source: "typed in Beans on Studio", note, saved_by: "", updated_at: 2 }] };
  };
  store.deleteSecret = async (_runner, name) => {
    calls.push(`delete ${name}`);
    kept = { secrets: kept.secrets.filter((s) => s.name !== name), grants: kept.grants.filter((g) => !g.secrets.includes(name)) };
  };
  store.revokeSecretGrants = async (_runner, name) => {
    calls.push(`revoke ${name ?? "all"}`);
    kept = { ...kept, grants: name ? kept.grants.filter((g) => !g.secrets.includes(name)) : [] };
  };
  const device = { id: "runner", name: "Studio", os: "linux", plugins: [] } as unknown as Device;
  const dispose = render(() => (<><SecretsSettings device={device} /><SheetHost /></>), root);
  const settle = async () => { for (let i = 0; i < 4; i++) { await Promise.resolve(); flush(); await new Promise((r) => setTimeout(r, 0)); flush(); } };
  const row = (label: string) => [...root.querySelectorAll<HTMLElement>(".action-row")].find((r) => r.dataset.label === label);
  const button = (element: HTMLElement | undefined | null, title: string) => [...(element?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((b) => b.textContent?.trim() === title);
  const dialog = () => document.querySelector<HTMLElement>("[role=dialog]");
  const type = (input: HTMLInputElement, text: string) => { input.value = text; input.dispatchEvent(new Event("input", { bubbles: true })); };
  try {
    await settle();
    assert(row("STRIPE_KEY")?.textContent?.includes("35 characters · typed in Beans on Studio · billing"), `The secret's row: ${row("STRIPE_KEY")?.textContent}`);
    assert(row("create_charge receives STRIPE_KEY"), "The tool grant is not listed");
    assert(row("$ vercel env add STRIPE_KEY {{secret:STRIPE_KEY}}")?.textContent?.includes("in /example/user/shop with STRIPE_KEY"), "The command grant is not listed with its folder");

    // Add one typed in: the value goes to the Runner once and is never shown.
    button(row("Add a secret"), "Add…")!.click();
    await settle();
    const inputs = [...dialog()!.querySelectorAll<HTMLInputElement>("input")];
    assert(inputs[1]?.type === "password", "The value field shows what is typed");
    type(inputs[0]!, "bad name");
    await settle();
    assert(dialog()!.textContent?.includes("Use letters, digits and _"), "A bad name is not explained");
    type(inputs[0]!, "NEW_TOKEN");
    type(inputs[1]!, typed);
    type(inputs[2]!, "deploys");
    await settle();
    button(dialog(), "Save")!.click();
    await settle();
    assert(calls.includes("set NEW_TOKEN <typed> deploys"), `Saving did not reach the Runner: ${calls.join(", ")}`);
    assert(!dialog(), "The sheet did not close");
    assert(row("NEW_TOKEN"), "The new secret is not listed");
    assert(!document.body.textContent?.includes(typed), "The typed value is shown");

    // Revoke one grant, then delete a secret after its confirmation.
    button(row("create_charge receives STRIPE_KEY"), "Revoke")!.click();
    await settle();
    assert(calls.includes("revoke STRIPE_KEY"), `Revoking did not reach the Runner: ${calls.join(", ")}`);
    button(row("NEW_TOKEN"), "Delete…")!.click();
    await settle();
    assert(dialog()?.textContent?.includes("Delete “NEW_TOKEN”?"), "Deleting does not ask first");
    dialog()!.querySelector<HTMLElement>(".alert")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    assert(calls.includes("delete NEW_TOKEN") && !row("NEW_TOKEN"), `Deleting did not apply: ${calls.join(", ")}`);
    return { list: "names, lengths, sources, grants", add: "value sent once, never shown", revoke: "per secret", delete: "confirmed" };
  } finally {
    Object.assign(store, saved);
    dispose();
    root.remove();
  }
}
