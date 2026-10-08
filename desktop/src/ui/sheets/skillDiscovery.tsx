import { createSignal, For, onCleanup, Show } from "solid-js";
import { L } from "../../l10n";
import { track } from "../../model/reactive";
import { store } from "../../model/store";
import { PopUpButton, TextField } from "../controls";
import { presentSheet, Sheet } from "../overlay";

export function presentSkillDiscovery(): void {
  presentSheet((dismiss) => <SkillDiscoverySheet dismiss={dismiss} />);
}

export function SkillDiscoverySheet(props: { dismiss: () => void }) {
  const [runnerID, setRunnerID] = createSignal("");
  const [root, setRoot] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal("");
  const [result, setResult] = createSignal<Awaited<ReturnType<typeof store.discoverSkills>>>();
  let alive = true, revision = 0;
  let selectedBinding: { id: string; os: string; key: string } | undefined;
  const unsubscribe = store.subscribe((event) => {
    const selected = store.runners.find((device) => device.id === runnerID());
    const runnerChanged = (event.kind === "rosterChanged" || event.kind === "snapshotReplaced") && selectedBinding !== undefined
      && (!selected || selected.id !== selectedBinding.id || selected.os !== selectedBinding.os || selected.machineKey !== selectedBinding.key);
    if (event.kind === "identityChanged" || event.kind === "connectionChanged" || runnerChanged) {
      revision++; setResult(undefined); setBusy(false); setFailure("");
    }
  });
  onCleanup(() => { alive = false; revision++; unsubscribe(); });
  const runners = () => { track.roster(); return store.runners; };
  const runner = () => runners().find((d) => d.id === runnerID());
  const change = () => { revision++; setResult(undefined); setFailure(""); };
  const scan = async () => {
    if (busy() || !runner() || !root()) return;
    const request = ++revision;
    const selected = runner()!;
    selectedBinding = { id: selected.id, os: selected.os, key: selected.machineKey };
    setBusy(true); setResult(undefined); setFailure("");
    try {
      const next = await store.discoverSkills(runnerID(), root());
      if (alive && request === revision) setResult(next);
    } catch {
      if (alive && request === revision) setFailure(L("Skill discovery failed. Check Runner, path and account, then scan again."));
    } finally { if (alive && request === revision) setBusy(false); }
  };
  return <Sheet title={L("Skill discovery")} width={540} confirm={L("Scan")} cancel={L("Close")}
    confirmDisabled={busy() || !runner() || !root()} onConfirm={() => void scan()} onCancel={props.dismiss} returnInContent>
    <p>{L("Read-only metadata. No import, resource loading or skill execution.")}</p>
    <label>{L("Runner")}<PopUpButton label={L("Runner")} value={runnerID()} disabled={busy()}
      options={[{ value: "", label: L("Select a Runner") }, ...runners().map((d) => ({ value: d.id, label: d.name }))]}
      onChange={(id) => { change(); setRunnerID(id); setRoot(""); }} /></label>
    <label>{runner()?.isThisDevice ? L("Absolute path on this Runner") : L("Absolute path on remote Runner")}
      <TextField value={root()} disabled={busy()} onInput={(value) => { change(); setRoot(value); }} /></label>
    <p>{L("Scan runs on %@ at %@. Only Linux with secure openat2 is supported; no fallback.", runner()?.name ?? "—", root() || "—")}</p>
    <Show when={failure()}><p role="alert">{failure()}</p></Show>
    <Show when={result()}>{(value) => <div aria-live="polite">
      <Show when={value().status === "unsupported"}><p>{L("Secure discovery unavailable on selected Runner or root. No fallback.")}</p></Show>
      <Show when={value().status === "scanned" && value().skills.length === 0}><p>{L("No skill metadata found within scan bounds.")}</p></Show>
      <For each={value().skills} keyed={(skill) => skill.path}>{(skill) => <article><strong>{skill().name ?? skill().path}</strong><p>{skill().path}</p><p>{skill().description}</p><p>{skill().license}</p></article>}</For>
      <For each={value().diagnostics} keyed={(diagnostic) => diagnostic}>{(diagnostic) => <p>{diagnostic().path ?? "—"}: {diagnostic().code}</p>}</For>
    </div>}</Show>
  </Sheet>;
}
