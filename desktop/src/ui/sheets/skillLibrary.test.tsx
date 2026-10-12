// Run through the installed Solid Vite plugin. Synthetic transport proves mounted consumer only.
import { render } from "@solidjs/web";
import { flush } from "solid-js";
import { L, setLanguage } from "../../l10n";
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

export async function checkSkillKeepBothConsumer() {
  const root = document.createElement("div"); document.body.append(root);
  const saved = { devices: store.devices, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected };
  const internals = store as unknown as { transport: unknown; emit: (event: { kind: "identityChanged" | "rosterChanged" | "snapshotReplaced" }) => void };
  const transport = internals.transport;
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let finish: (value: unknown) => void = () => {};
  let reject: (error: unknown) => void = () => {};
  const selected = toDevice({ id: "synthetic-runner", name: "Synthetic Linux", os: "linux", model: "Fixture", os_version: "fixture", is_this_device: false, status: "online", last_seen: 1, machine_key: "synthetic-key", plugins: [] });
  store.devices = [selected]; store.identityID = "synthetic-account"; store.hasIdentity = true; store.isConnected = true;
  internals.transport = { request: (method: string, params: Record<string, unknown>) => { calls.push({ method, params }); return new Promise((resolve, fail) => { finish = resolve; reject = fail; }); } };
  const dispose = render(() => <><SkillDiscoverySheet dismiss={() => {}} /><PageMenuHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  const button = (text: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === text);
  const click = (text: string) => { const b = button(text); assert(b && !b.disabled, `Missing/enabled button: ${text}`); b!.click(); flush(); };
  const summary = { name: "FIXTURE_COPY", description: "Inactive", license: null, findings: ["Resources remain inactive"], file_count: 2, total_bytes: 123 };
  const token = "a".repeat(32), original = "b".repeat(32), copy = "c".repeat(32), digest = "d".repeat(64);
  const source = "/synthetic-selected";
  const preview = { version: 1, runner_id: selected.id, token, expires_in_seconds: 300, source, destination: "/synthetic-owned/skill-library", summary };
  const imported = { version: 1, runner_id: selected.id, managed_id: original, summary, durability_warning: false };
  const installed = async () => {
    click("Preview copy"); finish(preview); await settle(); click("Confirm import"); finish(imported); await settle();
    click("Preview copy"); finish(preview); await settle();
  };
  try {
    flush(); await settle(); assert(calls.length === 0, "Mount automatically requested KeepBoth");
    const chooser = button("Select a Runner")!;
    chooser.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); flush(); await settle();
    const option = [...document.querySelectorAll("[role=menuitem], [role=option], button")].find((b) => b.textContent === "Synthetic Linux");
    assert(option, "Runner option absent"); (option as HTMLElement).click(); await settle();
    const input = root.querySelector("input")!;
    input.value = source; input.dispatchEvent(new Event("input", { bubbles: true })); flush();
    await installed();
    assert(root.textContent?.includes(original), "Managed copy absent before KeepBoth");
    assert(button("Keep both copies…"), "KeepBoth action absent for an installed copy");
    const beforeConfirm = calls.length; click("Keep both copies…");
    assert(calls.length === beforeConfirm, "KeepBoth published without explicit confirmation");
    assert(root.textContent?.includes("Original copy is never removed") && root.textContent?.includes(original), "KeepBoth confirmation omitted original-copy safety");
    click("Cancel KeepBoth"); assert(calls.length === beforeConfirm && !button("Confirm KeepBoth"), "Cancel kept both copies");
    await installed();
    const beforeApproval = calls.length; click("Keep both copies…"); click("Confirm KeepBoth");
    const request = calls.at(-1)!;
    assert(calls.length === beforeApproval + 1 && request.method === "skills.library.keep_both" && request.params.token === token && request.params.managed_id === original && request.params.confirmed === true && !("root" in request.params) && !("destination" in request.params), "KeepBoth not an explicit exact-ID confirmation");
    finish({ version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "selected-source", digest, summary, durability_warning: false }); await settle();
    const shown = root.textContent ?? "";
    assert(shown.includes(original) && shown.includes(copy), "KeepBoth did not render both copies");
    assert(shown.includes("selected-source") && shown.includes(digest.slice(0, 12)) && !shown.includes(digest), "KeepBoth omitted bounded label or short digest");
    assert(root.textContent?.includes("Not authenticated provenance"), "KeepBoth implied authenticated provenance");
    assert(button("Remove managed copy…"), "KeepBoth removed the original copy");
    await installed();
    // Bounded reply parameters: the longest allowed label, and a durability warning that
    // arrives with an already published copy rather than replacing it with a failure.
    const longLabel = "l".repeat(256);
    click("Keep both copies…"); click("Confirm KeepBoth");
    finish({ version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: longLabel, digest, summary, durability_warning: true }); await settle();
    assert(root.textContent?.includes(longLabel), "Bounded 256-byte label not rendered");
    assert(root.querySelector("[role=alert]")?.textContent?.includes("durability or private cleanup needs attention"), "Durability warning not reported with the committed copy");
    assert(root.textContent?.includes("Not authenticated provenance"), "Durability warning dropped the outcome");
    // A scan over the same sheet must not leave the previous copy, confirmation or outcome up.
    click("Scan");
    assert(!root.textContent?.includes(original) && !root.textContent?.includes(copy) && !button("Confirm KeepBoth"), "Scan left a stale copy, confirmation or outcome");
    finish({ version: 1, status: "scanned", skills: [], diagnostics: [] }); await settle();
    // Chinese reorders the two IDs and the label; positional slots must keep each value in its own place.
    setLanguage("zh-Hans");
    const outcome = L("Kept both copies. Original %@, new copy %@ from %@, digest %@.", original, copy, "selected-source", digest.slice(0, 12));
    assert(outcome.includes(original) && outcome.includes(copy) && outcome.includes("selected-source") && outcome.includes(digest.slice(0, 12)), "zh placeholders lost or swapped values");
    assert(outcome.indexOf(original) < outcome.indexOf("selected-source") && outcome.indexOf("selected-source") < outcome.indexOf(copy), "zh positional order wrong");
    setLanguage("en");
    await installed();
    const beforeFailure = calls.length; click("Keep both copies…"); click("Confirm KeepBoth");
    reject(new Error("skill_library_source_changed")); await settle();
    assert(calls.length === beforeFailure + 1 && root.querySelector("[role=alert]")?.textContent?.includes("Preview again"), "KeepBoth failure did not render error state");
    assert(!root.textContent?.includes("Not authenticated provenance"), "Failed KeepBoth published an outcome");
    await installed();
    for (const transition of ["account", "key"] as const) {
      await installed();
      click("Keep both copies…"); click("Confirm KeepBoth");
      if (transition === "account") { store.identityID = "other-account"; internals.emit({ kind: "identityChanged" }); store.identityID = "synthetic-account"; internals.emit({ kind: "identityChanged" }); }
      else { store.devices = [{ ...selected, machineKey: "temporary-key" }]; internals.emit({ kind: "rosterChanged" }); store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); }
      flush(); finish({ version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "PRIVATE_ABA", digest, summary, durability_warning: false }); await settle();
      assert(!root.textContent?.includes("PRIVATE_ABA") && !root.textContent?.includes("Not authenticated provenance"), `Late ${transition} ABA KeepBoth reply rendered`);
    }
    await installed();
    const invalid = [null, { version: 1, runner_id: selected.id, original_managed_id: original, managed_id: original, source_label: "selected-source", digest, summary, durability_warning: false },
      { version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "", digest, summary, durability_warning: false },
      { version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "selected-source", digest: "D".repeat(64), summary, durability_warning: false },
      { version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "selected-source", digest, summary, durability_warning: false, body: "PRIVATE" },
      { version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "selected-source\n", digest, summary, durability_warning: false }];
    for (const reply of invalid) {
      const pending = store.keepBothSkillLibrary(selected.id, token, original).then(() => false, () => true);
      finish(reply); assert(await pending, "Malformed KeepBoth reply accepted");
    }
    const uncorrelated = store.keepBothSkillLibrary(selected.id, token, "e".repeat(32)).then(() => false, () => true);
    finish({ version: 1, runner_id: selected.id, original_managed_id: original, managed_id: copy, source_label: "selected-source", digest, summary, durability_warning: false });
    assert(await uncorrelated, "Uncorrelated KeepBoth reply accepted");
    return { mounted: true, automaticRequests: 0, confirmation: "explicit", originalPreserved: true, duplicateDistinguishable: true, provenanceClaim: "absent", failureState: "visible", accountABA: "fenced", runnerKeyABA: "fenced", boundedLabelBytes: longLabel.length, durabilityWarning: "committed-with-warning", scanClearsCopy: true, zhPlaceholders: "positional", malformedRejected: invalid.length, uncorrelated: "rejected" };
  } finally { dispose(); root.remove(); Object.assign(store, saved); internals.transport = transport; }
}

export async function checkSkillCompareConsumer() {
  const root = document.createElement("div"); document.body.append(root);
  const saved = { devices: store.devices, identityID: store.identityID, hasIdentity: store.hasIdentity, isConnected: store.isConnected };
  const internals = store as unknown as { transport: unknown; emit: (event: { kind: "identityChanged" | "rosterChanged" | "snapshotReplaced" | "connectionChanged" }) => void };
  const transport = internals.transport;
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let finish: (value: unknown) => void = () => {};
  let reject: (error: unknown) => void = () => {};
  const selected = toDevice({ id: "synthetic-runner", name: "Synthetic Linux", os: "linux", model: "Fixture", os_version: "fixture", is_this_device: false, status: "online", last_seen: 1, machine_key: "synthetic-key", plugins: [] });
  store.devices = [selected]; store.identityID = "synthetic-account"; store.hasIdentity = true; store.isConnected = true;
  internals.transport = { request: (method: string, params: Record<string, unknown>) => { calls.push({ method, params }); return new Promise((resolve, fail) => { finish = resolve; reject = fail; }); } };
  const dispose = render(() => <><SkillDiscoverySheet dismiss={() => {}} /><PageMenuHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  const button = (text: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === text);
  const click = (text: string) => { const b = button(text); assert(b && !b.disabled, `Missing/enabled button: ${text}`); b!.click(); flush(); };
  const summary = { name: "FIXTURE_COPY", description: "Inactive", license: null, findings: [], file_count: 2, total_bytes: 123 };
  const token = "a".repeat(32), original = "b".repeat(32);
  const source = "/synthetic-selected";
  const preview = { version: 1, runner_id: selected.id, token, expires_in_seconds: 300, source, destination: "/synthetic-owned/skill-library", summary };
  const installed = async () => {
    click("Preview copy"); finish(preview); await settle(); click("Confirm import");
    finish({ version: 1, runner_id: selected.id, managed_id: original, summary, durability_warning: false }); await settle();
  };
  try {
    flush(); await settle(); assert(calls.length === 0, "Mount automatically compared");
    const chooser = button("Select a Runner")!;
    chooser.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); flush(); await settle();
    const option = [...document.querySelectorAll("[role=menuitem], [role=option], button")].find((b) => b.textContent === "Synthetic Linux");
    assert(option, "Runner option absent"); (option as HTMLElement).click(); await settle();
    const input = root.querySelector("input")!;
    input.value = source; input.dispatchEvent(new Event("input", { bubbles: true })); flush();
    await installed();
    // Bounded read-only reply: nested labels at the five-segment limit, on all three sides.
    const added = ["a/b/c/d/e", "resources/added"];
    const removed = ["resources/removed"];
    const changed = ["SKILL.md", "resources/changed"];
    const beforeCompare = calls.length; click("Compare source with copy…");
    const request = calls.at(-1)!;
    assert(calls.length === beforeCompare + 1 && request.method === "skills.library.compare" && request.params.root === source && request.params.managed_id === original && !("token" in request.params) && !("confirmed" in request.params), "Compare not a plain read-only exact-ID request");
    finish({ version: 1, runner_id: selected.id, managed_id: original, added, removed, changed }); await settle();
    const shown = root.textContent ?? "";
    for (const label of [...added, ...removed, ...changed]) assert(shown.includes(label), `Compare omitted ${label}`);
    assert(shown.includes("1 only in the source.") === false && shown.includes("2 only in the source."), "Compare did not count added labels");
    assert(shown.includes("Read-only.") && shown.includes("Nothing was changed and nothing is enabled here."), "Compare implied a mutation or an enable");
    assert(!button("Confirm import") && !button("Confirm KeepBoth") && button("Remove managed copy…"), "Compare invented a confirmation");
    // Nothing on the managed copy changed: the same copy and its removal stay exactly as they were.
    await installed();
    const beforeFailure = calls.length; click("Compare source with copy…");
    reject(new Error("skill_library_failed")); await settle();
    assert(calls.length === beforeFailure + 1 && root.querySelector("[role=alert]")?.textContent?.includes("Preview again"), "Compare failure did not render an error state");
    assert(!root.textContent?.includes("only in the source."), "Failed compare published a difference");
    for (const transition of ["account", "key"] as const) {
      await installed();
      click("Compare source with copy…");
      if (transition === "account") { store.identityID = "other-account"; internals.emit({ kind: "identityChanged" }); store.identityID = "synthetic-account"; internals.emit({ kind: "identityChanged" }); }
      else { store.devices = [{ ...selected, machineKey: "temporary-key" }]; internals.emit({ kind: "rosterChanged" }); store.devices = [selected]; internals.emit({ kind: "snapshotReplaced" }); }
      flush(); finish({ version: 1, runner_id: selected.id, managed_id: original, added: ["PRIVATE_ABA"], removed: [], changed: [] }); await settle();
      assert(!root.textContent?.includes("PRIVATE_ABA") && !root.textContent?.includes("only in the source."), `Late ${transition} ABA compare rendered`);
    }
    await installed();
    click("Compare source with copy…");
    store.isConnected = false; internals.emit({ kind: "connectionChanged" }); store.isConnected = true; internals.emit({ kind: "connectionChanged" });
    flush(); finish({ version: 1, runner_id: selected.id, managed_id: original, added: ["PRIVATE_CONNECTION"], removed: [], changed: [] }); await settle();
    assert(!root.textContent?.includes("PRIVATE_CONNECTION"), "Late connection ABA compare rendered");
    await installed();
    const invalid = [null,
      { version: 1, runner_id: selected.id, managed_id: original, added: ["../escape"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["/absolute"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["back\\slash"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["a/b/c/d/e/f"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["same"], removed: ["same"], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["b", "a"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: ["dup", "dup"], removed: [], changed: [] },
      { version: 1, runner_id: selected.id, managed_id: original, added: [], removed: [], changed: [], body: "PRIVATE" },
      { version: 1, runner_id: selected.id, managed_id: original, added: Array.from({ length: 257 }, (_, i) => `f/${i}`), removed: [], changed: [] }];
    for (const reply of invalid) {
      const pending = store.compareSkillLibrary(selected.id, source, original).then(() => false, () => true);
      finish(reply); assert(await pending, "Malformed compare reply accepted");
    }
    const uncorrelated = store.compareSkillLibrary(selected.id, source, "e".repeat(32)).then(() => false, () => true);
    finish({ version: 1, runner_id: selected.id, managed_id: original, added: [], removed: [], changed: [] });
    assert(await uncorrelated, "Uncorrelated compare reply accepted");
    // Every compare string must exist in the table, so the shipped zh cannot fall back to English.
    setLanguage("zh-Hans");
    for (const key of ["Compare source with copy…", "Compare on %@, copy %@ against the selected source. Read-only.", "%@ only in the source.", "%@ only in the copy.", "%@ differ between source and copy.", "Nothing was changed and nothing is enabled here. Applying a difference needs its own confirmed action."]) {
      assert(L(key, selected.name, original, 2, 2, 2) !== key, `No zh entry for: ${key}`);
    }
    assert(L("%@ only in the source.", 2).includes("2"), "zh compare count placeholder wrong");
    setLanguage("en");
    return { mounted: true, automaticRequests: 0, mutation: "none", request: "exact-ID read-only", maxSegments: 5, countsRendered: true, inert: true, inventedConfirmation: false, zhCoverage: "complete", failureState: "visible", accountABA: "fenced", runnerKeyABA: "fenced", connectionABA: "fenced", malformedRejected: invalid.length, uncorrelated: "rejected" };
  } finally { dispose(); root.remove(); Object.assign(store, saved); internals.transport = transport; }
}
