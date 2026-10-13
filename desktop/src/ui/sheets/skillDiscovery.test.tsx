// Run checkSkillDiscoveryConsumer through the installed Solid Vite plugin; synthetic transport proves UI only.
import { render } from "@solidjs/web";
import { flush } from "solid-js";
import { store } from "../../model/store";
import { SkillDiscoverySheet } from "./skillDiscovery";
import { PageMenuHost } from "../menu";
import { toDevice } from "../../model/wire";

export async function checkSkillDiscoveryConsumer() {
  const root = document.createElement("div"); document.body.append(root);
  const saved = { devices: store.devices, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected };
  const internals = store as unknown as { transport: unknown; emit: (event: { kind: "identityChanged" | "rosterChanged" | "snapshotReplaced" }) => void };
  const transport = internals.transport;
  let calls = 0, finish: (value: unknown) => void = () => {};
  store.devices = [toDevice({ id: "synthetic-runner", name: "Synthetic Linux", os: "linux", model: "Fixture", os_version: "fixture", is_this_device: false, status: "online", last_seen: 1, machine_key: "synthetic-public-key", plugins: [] })];
  store.identityID = "synthetic-account"; store.hasIdentity = true; store.isConnected = true;
  internals.transport = { request: (method: string) => { if (method !== "skills.discovery") return Promise.reject(new Error("Unrelated app request outside consumer fixture")); calls++; return new Promise((resolve) => { finish = resolve; }); } };
  const dispose = render(() => <><SkillDiscoverySheet dismiss={() => {}} /><PageMenuHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  try {
    flush(); await settle(); assert(calls === 0, "Opening sheet scanned automatically");
    // Use existing popup's native option menu, not component internals.
    const runner = [...root.querySelectorAll("button")].find((b) => b.textContent?.includes("Select a Runner"))!;
    runner.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); flush(); await settle();
    const option = [...document.querySelectorAll("[role=menuitem], [role=option], button")].find((b) => b.textContent === "Synthetic Linux");
    assert(option, "Runner option absent"); (option as HTMLElement).click(); await settle();
    const input = root.querySelector("input")!;
    input.value = "/synthetic-selected"; input.dispatchEvent(new Event("input", { bubbles: true })); flush();
    assert(root.textContent?.includes("Absolute path on remote Runner"), "Remote path label absent");
    const scan = () => { [...root.querySelectorAll("button")].find((b) => b.textContent === "Scan")!.click(); flush(); };
    scan(); assert(calls === 1, "Explicit Scan did not send exactly one request");
    finish({ version: 1, status: "unsupported", skills: [], diagnostics: [{ path: null, code: "secure_discovery_unavailable" }] });
    await settle(); assert(root.textContent?.includes("Secure discovery unavailable"), "Unsupported state absent");
    scan(); store.identityID = "replacement-account"; internals.emit({ kind: "identityChanged" }); flush();
    finish({ version: 1, status: "scanned", skills: [{ path: "SKILL.md", name: "PRIVATE_OLD_ACCOUNT", description: null, license: null }], diagnostics: [] });
    await settle(); assert(!root.textContent?.includes("PRIVATE_OLD_ACCOUNT"), "Late old-account metadata rendered");
    assert(calls === 2, "Expected exactly two explicit discovery requests");
    const selected = store.devices[0]!;
    const metadata = { path: "SKILL.md", name: "CURRENT_METADATA", description: "Selected metadata", license: null };
    const valid = { version: 1, status: "scanned", skills: [metadata], diagnostics: [] };
    const malformed: unknown[] = [null, [], { ...valid, version: 2 }, { ...valid, status: "raw error" }, { ...valid, body: "PRIVATE" },
      { ...valid, skills: Array(257).fill(metadata) }, { ...valid, diagnostics: Array(4097).fill({ path: null, code: "path_not_inspected" }) },
      { ...valid, skills: [null] }, { ...valid, skills: [{ ...metadata, path: null }] }, { ...valid, skills: [{ ...metadata, path: "/private" }] },
      ...["../private", "a//b", "C:private", "a\\\\b", "a\u0085b", "a".repeat(4097)].map((path) => ({ ...valid, skills: [{ ...metadata, path }] })),
      { ...valid, skills: [{ ...metadata, name: {} }] }, { ...valid, skills: [{ ...metadata, name: "名".repeat(86) }] },
      { ...valid, skills: [{ ...metadata, description: "x".repeat(2049) }] }, { ...valid, skills: [{ ...metadata, license: "\n" }] },
      { ...valid, skills: [{ ...metadata, hook: "PRIVATE" }] }, { ...valid, diagnostics: [{ path: null, code: "raw error" }] },
      { ...valid, diagnostics: [{ path: {}, code: "path_not_inspected" }] }, { ...valid, diagnostics: [null] }, { ...valid, status: "unsupported" }];
    for (const reply of malformed) {
      const request = store.discoverSkills(selected.id, "/synthetic-selected").then(() => false, () => true);
      finish(reply); assert(await request, "Malformed reply published by production adapter");
    }
    scan(); finish(valid); await settle(); assert(root.textContent?.includes("CURRENT_METADATA"), "Valid metadata absent");
    internals.emit({ kind: "rosterChanged" }); flush(); assert(root.textContent?.includes("CURRENT_METADATA"), "Unrelated roster cleared result");
    store.devices = [{ ...selected, machineKey: "replacement-key" }]; internals.emit({ kind: "rosterChanged" }); flush();
    assert(!root.textContent?.includes("CURRENT_METADATA"), "Key change retained existing result");
    store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); flush();
    for (const transition of ["removal", "key"] as const) {
      const request = store.discoverSkills(selected.id, "/synthetic-selected").then(() => false, () => true);
      store.devices = transition === "removal" ? [] : [{ ...selected, machineKey: "temporary-key" }];
      internals.emit({ kind: "rosterChanged" }); flush();
      store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); flush();
      finish(valid); assert(await request, "Production adapter accepted Runner ABA");
      scan();
      store.devices = transition === "removal" ? [] : [{ ...selected, machineKey: "temporary-key" }];
      internals.emit({ kind: "rosterChanged" }); flush();
      store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); flush();
      finish({ ...valid, skills: [{ ...metadata, name: "PRIVATE_ABA" }] }); await settle();
      assert(!root.textContent?.includes("PRIVATE_ABA"), "Runner ABA metadata rendered");
    }
    return { mounted: true, automaticRequests: 0, malformedRejected: malformed.length, runnerRemovalABA: "fenced", runnerKeyABA: "fenced", existingResult: "cleared", unrelatedRoster: "retained", unsupported: "visible", lateAccountMetadata: "fenced" };
  } finally { dispose(); root.remove(); Object.assign(store, saved); internals.transport = transport; }
}
