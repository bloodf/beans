// Run through the installed Solid Vite plugin. Synthetic transport proves mounted consumer only.
import { render } from "@solidjs/web";
import { flush } from "solid-js";
import { store } from "../../model/store";
import { SkillDiscoverySheet } from "./skillDiscovery";
import { PageMenuHost } from "../menu";
import { toDevice } from "../../model/wire";

export async function checkSkillLibraryConsumer() {
  const root = document.createElement("div"); document.body.append(root);
  const saved = { devices: store.devices, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected };
  const internals = store as unknown as { transport: unknown; emit: (event: { kind: "identityChanged" | "rosterChanged" | "snapshotReplaced" }) => void };
  const transport = internals.transport;
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let finish: (value: unknown) => void = () => {};
  const selected = toDevice({ id: "synthetic-runner", name: "Synthetic Linux", os: "linux", model: "Fixture", os_version: "fixture", is_this_device: false, status: "online", last_seen: 1, machine_key: "synthetic-key", plugins: [] });
  store.devices = [selected]; store.identityID = "synthetic-account"; store.hasIdentity = true; store.isConnected = true;
  internals.transport = { request: (method: string, params: Record<string, unknown>) => { calls.push({ method, params }); return new Promise((resolve) => { finish = resolve; }); } };
  const dispose = render(() => <><SkillDiscoverySheet dismiss={() => {}} /><PageMenuHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  const button = (text: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === text);
  const click = (text: string) => { const b = button(text); assert(b && !b.disabled, `Missing/enabled button: ${text}`); b!.click(); flush(); };
  const summary = { name: "FIXTURE_COPY", description: "Inactive", license: null, findings: ["Resources remain inactive"], file_count: 2, total_bytes: 123 };
  const token = "a".repeat(32), id = "b".repeat(32);
  const preview = { version: 1, runner_id: selected.id, token, expires_in_seconds: 300, source: "/synthetic-selected", destination: "/synthetic-owned/skill-library", summary };
  try {
    flush(); await settle(); assert(calls.length === 0, "Mount automatically requested import or preview");
    const chooser = button("Select a Runner")!;
    chooser.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); flush(); await settle();
    const option = [...document.querySelectorAll("[role=menuitem], [role=option], button")].find((b) => b.textContent === "Synthetic Linux");
    assert(option, "Runner option absent"); (option as HTMLElement).click(); await settle();
    const input = root.querySelector("input")!;
    input.value = "/synthetic-selected"; input.dispatchEvent(new Event("input", { bubbles: true })); flush();
    assert(calls.length === 0, "Selection automatically requested copy");
    click("Preview copy"); assert(calls.at(-1)?.method === "skills.library.preview", "Preview used wrong API");
    finish(preview); await settle();
    assert(root.textContent?.includes(preview.source) && root.textContent?.includes(preview.destination) && root.textContent?.includes("Unknown license"), "Confirmation omitted source/destination/license");
    assert(calls.length === 1 && button("Confirm import"), "Preview automatically imported");
    click("Cancel import"); assert(!button("Confirm import") && calls.length === 1, "Cancel imported");
    click("Preview copy"); finish(preview); await settle(); click("Confirm import");
    const importedRequest = calls.at(-1)!;
    assert(importedRequest.method === "skills.library.import" && importedRequest.params.token === token && importedRequest.params.confirmed === true && !("root" in importedRequest.params) && !("destination" in importedRequest.params), "Import not opaque explicit confirmation");
    finish({ version: 1, runner_id: selected.id, managed_id: id, summary, durability_warning: false }); await settle();
    assert(root.textContent?.includes(id), "Managed ID not rendered");
    const beforeRemoval = calls.length; click("Remove managed copy…");
    assert(calls.length === beforeRemoval && root.textContent?.includes("Original source remains untouched"), "Removal was automatic or omitted source safety");
    click("Cancel removal"); assert(calls.length === beforeRemoval && !button("Confirm removal"), "Cancel removed copy");
    click("Remove managed copy…"); click("Confirm removal");
    assert(calls.at(-1)?.method === "skills.library.uninstall" && calls.at(-1)?.params.managed_id === id && calls.at(-1)?.params.confirmed === true, "Removal not exact ID confirmation");
    finish({ version: 1, runner_id: selected.id, managed_id: id, cleanup_warning: false, durability_warning: false }); await settle();
    assert(!root.textContent?.includes(id), "Removed copy retained");
    for (const transition of ["removal", "key", "account"] as const) {
      click("Preview copy");
      if (transition === "account") { store.identityID = "other-account"; internals.emit({ kind: "identityChanged" }); store.identityID = "synthetic-account"; internals.emit({ kind: "identityChanged" }); }
      else { store.devices = transition === "removal" ? [] : [{ ...selected, machineKey: "temporary-key" }]; internals.emit({ kind: "rosterChanged" }); store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); }
      flush(); finish({ ...preview, summary: { ...summary, name: "PRIVATE_ABA" } }); await settle();
      assert(!root.textContent?.includes("PRIVATE_ABA") && !button("Confirm import"), `Late ${transition} ABA preview rendered`);
    }
    click("Preview copy"); finish(preview); await settle();
    internals.emit({ kind: "rosterChanged" }); flush(); assert(button("Confirm import"), "Unrelated roster invalidated approval");
    store.devices = []; internals.emit({ kind: "rosterChanged" }); flush();
    store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); flush();
    assert(!button("Confirm import"), "Existing confirmation survived Runner ABA");
    click("Preview copy"); finish(preview); await settle(); click("Confirm import");
    store.identityID = "replacement-account"; internals.emit({ kind: "identityChanged" });
    store.identityID = "synthetic-account"; internals.emit({ kind: "identityChanged" }); flush();
    finish({ version: 1, runner_id: selected.id, managed_id: id, summary, durability_warning: false }); await settle();
    assert(!root.textContent?.includes(id) && !button("Remove managed copy…"), "Late old-account import published managed state");
    click("Preview copy"); finish(preview); await settle(); click("Confirm import");
    finish({ version: 1, runner_id: selected.id, managed_id: id, summary, durability_warning: false }); await settle();
    click("Remove managed copy…"); click("Confirm removal");
    store.devices = [{ ...selected, machineKey: "temporary-key" }]; internals.emit({ kind: "rosterChanged" });
    store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); flush();
    finish({ version: 1, runner_id: selected.id, managed_id: id, cleanup_warning: true, durability_warning: true }); await settle();
    assert(!root.textContent?.includes(id) && !root.textContent?.includes("durability or private cleanup needs attention"), "Late Runner-ABA removal published state or warnings");
    const invalid = [null, { ...preview, destination: "relative" }, { ...preview, token: "../source" }, { ...preview, body: "PRIVATE" }, { ...preview, summary: { ...summary, file_count: 257 } }, { ...preview, summary: { ...summary, license: "\n" } }];
    for (const reply of invalid) {
      const pending = store.previewSkillLibrary(selected.id, preview.source).then(() => false, () => true);
      finish(reply); assert(await pending, "Malformed preview accepted");
    }
    return { mounted: true, automaticRequests: 0, importConfirmation: "explicit", removalConfirmation: "exact-managed-ID", cancellation: "no-effect", accountABA: "fenced", runnerRemovalABA: "fenced", runnerKeyABA: "fenced", lateImportAccount: "fenced", lateRemovalRunnerABA: "fenced", existingApproval: "cleared", unrelatedRoster: "retained", malformedRejected: invalid.length };
  } finally { dispose(); root.remove(); Object.assign(store, saved); internals.transport = transport; }
}
