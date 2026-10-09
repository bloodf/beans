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
  const [preview, setPreview] = createSignal<Awaited<ReturnType<typeof store.previewSkillLibrary>>>();
  const [managed, setManaged] = createSignal<Awaited<ReturnType<typeof store.importSkillLibrary>>>();
  const [removeConfirmation, setRemoveConfirmation] = createSignal(false);
  const [warning, setWarning] = createSignal(false);
  let previewExpires = 0;
  let alive = true, revision = 0;
  let selectedBinding: { id: string; os: string; key: string } | undefined;
  const unsubscribe = store.subscribe((event) => {
    const selected = store.runners.find((device) => device.id === runnerID());
    const runnerChanged = (event.kind === "rosterChanged" || event.kind === "snapshotReplaced") && selectedBinding !== undefined
      && (!selected || selected.id !== selectedBinding.id || selected.os !== selectedBinding.os || selected.machineKey !== selectedBinding.key);
    if (event.kind === "identityChanged" || event.kind === "connectionChanged" || runnerChanged) {
      revision++; setResult(undefined); setPreview(undefined); setManaged(undefined); setRemoveConfirmation(false); setWarning(false); setBusy(false); setFailure("");
    }
  });
  onCleanup(() => { alive = false; revision++; unsubscribe(); });
  const runners = () => { track.roster(); return store.runners; };
  const runner = () => runners().find((d) => d.id === runnerID());
  const change = () => { revision++; setResult(undefined); setPreview(undefined); setManaged(undefined); setRemoveConfirmation(false); setWarning(false); setFailure(""); };
  const scan = async () => {
    if (busy() || !runner() || !root()) return;
    const request = ++revision;
    const selected = runner()!;
    selectedBinding = { id: selected.id, os: selected.os, key: selected.machineKey };
    setBusy(true); setResult(undefined); setPreview(undefined); setFailure("");
    try {
      const next = await store.discoverSkills(runnerID(), root());
      if (alive && request === revision) setResult(next);
    } catch {
      if (alive && request === revision) setFailure(L("Skill discovery failed. Check Runner, path and account, then scan again."));
    } finally { if (alive && request === revision) setBusy(false); }
  };
  const libraryAction = async (action: "preview" | "import" | "uninstall") => {
    if (busy() || !runner() || !root()) return;
    const approved = preview(), installed = managed();
    if (action === "import" && (!approved || approved.runner_id !== runnerID() || approved.source !== root() || Date.now() >= previewExpires)) {
      setPreview(undefined); setFailure(L("Skill library approval expired. Preview again.")); return;
    }
    if (action === "uninstall" && (!installed || !removeConfirmation())) return;
    const request = ++revision, selected = runner()!;
    selectedBinding = { id: selected.id, os: selected.os, key: selected.machineKey };
    setBusy(true); setFailure(""); setWarning(false);
    // Confirmation is spent even when request fails. Never automatically retry a mutation.
    if (action === "import") setPreview(undefined);
    if (action === "uninstall") setRemoveConfirmation(false);
    try {
      if (action === "preview") {
        setPreview(undefined);
        const next = await store.previewSkillLibrary(runnerID(), root());
        if (alive && request === revision) { previewExpires = Date.now() + next.expires_in_seconds * 1000; setPreview(next); }
      } else if (action === "import") {
        const next = await store.importSkillLibrary(runnerID(), approved!.token);
        if (alive && request === revision) { setManaged(next); setWarning(next.durability_warning); }
      } else {
        const next = await store.uninstallSkillLibrary(runnerID(), installed!.managed_id);
        if (alive && request === revision) { setManaged(undefined); setWarning(next.cleanup_warning || next.durability_warning); }
      }
    } catch { if (alive && request === revision) setFailure(L("Skill library request failed. Preview again before importing.")); }
    finally { if (alive && request === revision) setBusy(false); }
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
    <section aria-label={L("Managed skill copies")}>
      <p>{L("Explicit copy only. No execution, grants, hooks or activation. Preview reads bounded resources on the selected Runner.")}</p>
      <button disabled={busy() || !runner() || !root()} onClick={() => void libraryAction("preview")}>{L("Preview copy")}</button>
      <Show when={preview()}>{(p) => <div aria-live="polite">
        <p>{L("Copy on %@ from %@ to %@", runner()?.name ?? "—", p().source, p().destination)}</p>
        <p>{p().summary.name}</p><p>{p().summary.description}</p>
        <p>{L("Declared license: %@", p().summary.license ?? L("Unknown license"))}</p>
        <p>{L("%@ files, %@ bytes. Approval expires in %@ seconds.", p().summary.file_count, p().summary.total_bytes, p().expires_in_seconds)}</p>
        <For each={p().summary.findings} keyed={(finding) => finding}>{(finding) => <p>{finding()}</p>}</For>
        <p>{L("Confirm this exact preview. Changed source bytes are rejected; no different copy is published.")}</p>
        <button disabled={busy()} onClick={() => void libraryAction("import")}>{L("Confirm import")}</button>
        <button disabled={busy()} onClick={() => setPreview(undefined)}>{L("Cancel import")}</button>
      </div>}</Show>
      <Show when={managed()}>{(m) => <div aria-live="polite">
        <p>{L("Managed copy on %@: %@", runner()?.name ?? "—", m().managed_id)}</p>
        <button disabled={busy()} onClick={() => setRemoveConfirmation(true)}>{L("Remove managed copy…")}</button>
        <Show when={removeConfirmation()}>
          <p>{L("Remove only managed copy %@? Original source remains untouched.", m().managed_id)}</p>
          <button disabled={busy()} onClick={() => void libraryAction("uninstall")}>{L("Confirm removal")}</button>
          <button disabled={busy()} onClick={() => setRemoveConfirmation(false)}>{L("Cancel removal")}</button>
        </Show>
      </div>}</Show>
      <Show when={warning()}><p role="alert">{L("Copy visibility changed, but durability or private cleanup needs attention. Do not automatically retry.")}</p></Show>
    </section>
    <Show when={result()}>{(value) => <div aria-live="polite">
      <Show when={value().status === "unsupported"}><p>{L("Secure discovery unavailable on selected Runner or root. No fallback.")}</p></Show>
      <Show when={value().status === "scanned" && value().skills.length === 0}><p>{L("No skill metadata found within scan bounds.")}</p></Show>
      <For each={value().skills} keyed={(skill) => skill.path}>{(skill) => <article><strong>{skill().name ?? skill().path}</strong><p>{skill().path}</p><p>{skill().description}</p><p>{skill().license}</p></article>}</For>
      <For each={value().diagnostics} keyed={(diagnostic) => diagnostic}>{(diagnostic) => <p>{diagnostic().path ?? "—"}: {diagnostic().code}</p>}</For>
    </div>}</Show>
  </Sheet>;
}
