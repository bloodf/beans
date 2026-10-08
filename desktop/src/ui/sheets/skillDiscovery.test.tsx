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
  const internals = store as unknown as { transport: unknown; emit: (event: { kind: "identityChanged" }) => void };
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
    return { mounted: true, automaticRequests: 0, explicitRequests: calls, unsupported: "visible", lateAccountMetadata: "fenced" };
  } finally { dispose(); root.remove(); Object.assign(store, saved); internals.transport = transport; }
}
