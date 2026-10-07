import { createSignal, For, Show } from "solid-js";
import { L } from "../../l10n";
import { store, errorText } from "../../model/store";
import { isRunner } from "../../model/models";
import type { MemoryConnectionView, MemoryEmbeddingView } from "../../model/memoryService";
import { BotSetupApproval, LocalAssetApproval, MemorySetupAPI, type AssetKind, type AssetRequest, type BotPreviewMethod, type BotSetupPreview, type LocalAssetPreview, type SetupPreviewRequest } from "../../model/memorySetup";
import { Button, PopUpButton, TextField } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { MemoryField, MemoryToggle } from "./memoryService";

export const memorySetupAPI = new MemorySetupAPI((method, params) => store.memoryRequest(method, { ...params }));
export function presentLocalEmbeddingSetup(profile: MemoryEmbeddingView): void {
  presentSheet((dismiss) => <LocalEmbeddingSetupSheet profile={profile} api={memorySetupAPI} dismiss={dismiss} />);
}
export function MemorySetupControls(props: { bot: { id: string; name: string }; connection: MemoryConnectionView; onChanged: () => Promise<void> }) {
  const open = (method: BotPreviewMethod) => { if (props.connection.availability === "supported") presentSheet((dismiss) => <BotSetupSheet bot={props.bot} connection={props.connection} method={method} api={memorySetupAPI} onChanged={props.onChanged} dismiss={dismiss} />); };
  return <Show when={props.connection.availability === "supported" && (props.connection.backend === "pgvector" || props.connection.backend === "lance_db")}><section class="memory-section"><h3>{L("Runner setup")}</h3>
    <Show when={props.connection.backend === "pgvector"}><p class="memory-note">{L("Initialization previews the actual database, role, server session, schema and non-destructive SQL. Nothing is initialized by Health.")}</p><Button onClick={() => open("memory.pgvector.initialize.preview")}>{L("Preview database initialization…")}</Button></Show>
    <Show when={props.connection.backend === "lance_db"}><p class="memory-note">{L("Local Lance directories stay on the assigned Runner. Export/import is private plaintext transfer, not database-file sync or dimensions-only migration.")}</p><div class="memory-inline-actions"><Button onClick={() => open("memory.lance.binding.preview")}>{L("Bind local directory…")}</Button><Button onClick={() => open("memory.lance.export.preview")}>{L("Export…")}</Button><Button onClick={() => open("memory.lance.import.preview")}>{L("Import…")}</Button></div></Show>
  </section></Show>;
}
const setupTitles: Record<BotPreviewMethod, string> = { "memory.pgvector.initialize.preview": "Initialize pgvector", "memory.lance.binding.preview": "Bind local Lance storage", "memory.lance.export.preview": "Export Lance documents", "memory.lance.import.preview": "Import Lance documents" };

export function BotSetupSheet(props: { bot: { id: string; name: string }; connection: MemoryConnectionView; method: BotPreviewMethod; api: MemorySetupAPI; onChanged: () => Promise<void>; dismiss: () => void }) {
  const [path, setPath] = createSignal(""), [create, setCreate] = createSignal(false);
  const [approval, setApproval] = createSignal<BotSetupApproval>(), [busy, setBusy] = createSignal(false), [failure, setFailure] = createSignal(""), [result, setResult] = createSignal("");
  const changedPath = (value: string) => { setPath(value); setApproval(undefined); setResult(""); };
  const preview = async () => {
    if (busy()) return;
    setBusy(true); setFailure(""); setApproval(undefined); setResult("");
    try {
      let request: SetupPreviewRequest;
      switch (props.method) {
        case "memory.pgvector.initialize.preview": request = { method: props.method, params: { bot_id: props.bot.id } }; break;
        case "memory.lance.binding.preview": request = { method: props.method, params: { bot_id: props.bot.id, directory: path(), create: create() } }; break;
        case "memory.lance.export.preview": case "memory.lance.import.preview": request = { method: props.method, params: { bot_id: props.bot.id, path: path() } }; break;
      }
      const reply = await props.api.preview(request);
      if (!("action" in reply)) throw new Error("invalid_setup_reply");
      setApproval(new BotSetupApproval(reply));
    } catch (error) { setFailure(errorText(error)); }
    finally { setBusy(false); }
  };
  const apply = async () => {
    const approved = approval(); if (!approved || busy()) return;
    setBusy(true); setFailure("");
    try {
      if (await alert({ message: L("Approve this exact preview?"), informative: L("Bot: %@. Runner: %@. Only the displayed target/path, SQL or vector space is approved. This token is one-use, including failed apply; changes require a new preview.", props.bot.name, approved.preview.runner_id), buttons: [{ title: L("Apply exact preview") }, { title: L("Cancel") }] }) !== 0) return;
      const currentBot = store.bots.find((b) => b.id === props.bot.id);
      if (!currentBot) throw new Error("approval_stale");
      const result = await props.api.applyBot(approved, true, { bot_id: props.bot.id, runner_id: currentBot.runnerID, connection_revision: props.connection.revision });
      if ("initialized" in result) setResult(L("Database initialization completed."));
      else if ("exported" in result) setResult(L("Exported %d bytes to the approved create-new destination.", result.bytes));
      else if ("imported" in result) setResult(L("Validated documents imported atomically into the approved vector space."));
      else if ("status" in result) setResult(result.status);
      await props.onChanged();
    } catch (error) { setFailure(errorText(error)); }
    finally { setApproval(undefined); setBusy(false); }
  };
  return <Sheet title={L(setupTitles[props.method])} subtitle={L("%@'s assigned Runner owns this operation. Preview is separate from approval.", props.bot.name)} width={700} confirm={approval() ? L("Approve exact preview…") : L("Preview")} confirmDisabled={busy() || (props.method !== "memory.pgvector.initialize.preview" && !path())} onConfirm={() => void (approval() ? apply() : preview())} onCancel={() => { if (!busy()) props.dismiss(); }} class="memory-service-sheet">
    <Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show><Show when={result()}><div role="status">{result()}</div></Show>
    <fieldset disabled={busy()} class="memory-form-fields"><Show when={props.method !== "memory.pgvector.initialize.preview"}><MemoryField label={props.method === "memory.lance.binding.preview" ? L("Absolute Runner directory") : props.method === "memory.lance.export.preview" ? L("Absolute destination file") : L("Absolute source file")}><TextField label={L("Exact Runner path")} value={path()} autofocus onInput={changedPath} /></MemoryField></Show>
      <Show when={props.method === "memory.lance.binding.preview"}><MemoryToggle label={L("Create the exact previewed table")} checked={create()} onChange={(value) => { setCreate(value); setApproval(undefined); }} note={L("Off opens existing storage only. No unrelated tables or directories are overwritten.")} /></Show>
    </fieldset>
    <Show when={props.method === "memory.lance.export.preview" || props.method === "memory.lance.import.preview"}><p class="memory-note">{L("Transfer files are plaintext; keep them private. The complete vector-space fingerprint and namespace must match. Limits: 8 MiB and 1,000 documents. Export never overwrites an existing file.")}</p></Show>
    <Show when={approval()}>{(approved) => <BotSetupPreviewView preview={approved().preview} />}</Show>
  </Sheet>;
}

function BotSetupPreviewView(props: { preview: BotSetupApproval["preview"] }) {
  const renderDetails = () => {
    const p = props.preview;
    switch (p.action) {
      case "memory.pgvector.initialize.preview": {
        const target = p.details.target;
        const rows = [
          ["Database", target.database], ["Database OID", String(target.database_oid)], ["Role", target.role],
          ["Server address", target.server_address ?? L("Local socket")],
          ["Server port", target.server_port === null ? L("Local socket") : String(target.server_port)],
          ["Server version", target.server_version], ["Session PID", String(target.session_pid)], ["Schema", p.details.schema],
          ["Ready", String(p.details.readiness.ready)],
          ["Extension version", p.details.readiness.extension_version ?? L("Not installed")],
          ["Extension schema", p.details.readiness.extension_schema ?? L("Not installed")],
          ["Schema exists", String(p.details.readiness.schema_exists)],
          ["Can create schema", String(p.details.readiness.can_create_schema)],
          ["Can create tables", String(p.details.readiness.can_create_tables)],
          ["Can install extension", String(p.details.readiness.can_install_extension)],
        ] as const;
        return <><For each={rows}>{([label, value]) => <MemoryField label={L(label)}><span class="memory-preview-value">{value}</span></MemoryField>}</For><div class="memory-note">{L("Non-destructive SQL, read-only preview")}</div><pre class="memory-output" tabindex={0}>{p.details.sql}</pre></>;
      }
      case "memory.lance.binding.preview":
        return <><MemoryField label={L("Directory")}><span class="memory-preview-value">{p.details.directory}</span></MemoryField><MemoryField label={L("Create table")}><span>{p.details.create ? L("Yes") : L("No")}</span></MemoryField><MemoryField label={L("Table")}><span>{p.details.table}</span></MemoryField><MemoryField label={L("Core namespace")}><span class="memory-preview-value">{p.details.namespace}</span></MemoryField></>;
      case "memory.lance.export.preview":
      case "memory.lance.import.preview": {
        const rows = [
          ["Path", p.details.path], ["Core namespace", p.details.namespace], ["Vector-space fingerprint", p.details.space.fingerprint],
          ["Dimensions", String(p.details.space.dimensions)], ["Distance", p.details.space.distance], ["Generation", String(p.details.space.generation)],
          ["Documents", String(p.details.documents)], ["Transfer SHA-256", p.details.sha256],
        ] as const;
        return <For each={rows}>{([label, value]) => <MemoryField label={L(label)}><span class="memory-preview-value">{value}</span></MemoryField>}</For>;
      }
    }
  };
  return <section class="memory-section">
    <h3>{L("Exact approved target")}</h3>
    <MemoryField label={L("Bot")}><span class="memory-preview-value">{props.preview.bot_id}</span></MemoryField>
    <MemoryField label={L("Runner")}><span class="memory-preview-value">{props.preview.runner_id}</span></MemoryField>
    <MemoryField label={L("Expires")}><span class="memory-preview-value">{new Date(props.preview.expires_at * 1000).toLocaleString()}</span></MemoryField>
    {renderDetails()}
  </section>;
}

interface AssetSetupDraft { kind: AssetKind; source: "supplied" | "download"; location: string; license: string; bytes: string; sha256: string }
export function LocalEmbeddingSetupSheet(props: { profile: MemoryEmbeddingView; api: MemorySetupAPI; dismiss: () => void }) {
  const runners = store.devices.filter(isRunner);
  const [runner, setRunner] = createSignal(runners[0]?.id ?? "");
  const [assets, setAssets] = createSignal<AssetSetupDraft[]>(["runtime", "model", "tokenizer"].map((kind) => ({ kind: kind as AssetKind, source: "supplied", location: "", license: "", bytes: "", sha256: "" })));
  const [approval, setApproval] = createSignal<LocalAssetApproval>(), [busy, setBusy] = createSignal(false), [failure, setFailure] = createSignal(""), [status, setStatus] = createSignal("");
  const target = () => ({ runner_id: runner(), profile_id: props.profile.id, profile_revision: props.profile.revision });
  const patch = (kind: AssetKind, change: Partial<AssetSetupDraft>) => { setAssets(assets().map((asset) => asset.kind === kind ? { ...asset, ...change } : asset)); setApproval(undefined); setStatus(""); };
  const preview = async () => {
    if (busy()) return;
    setBusy(true); setFailure(""); setApproval(undefined); setStatus("");
    try {
      const plan = { assets: assets().map((a): AssetRequest => ({ kind: a.kind, source: a.source === "supplied" ? { kind: "supplied", path: a.location } : { kind: "download", url: a.location }, license: a.license, bytes: Number(a.bytes), sha256: a.sha256 })) };
      const reply = await props.api.preview({ method: "memory.embeddings.local.preview", params: { ...target(), plan } });
      if (!("preview_token" in reply)) throw new Error("invalid_setup_reply");
      setApproval(new LocalAssetApproval(reply));
    } catch (error) { setFailure(errorText(error)); }
    finally { setBusy(false); }
  };
  const apply = async () => {
    const approved = approval(); if (!approved || busy()) return;
    setBusy(true); setFailure("");
    try {
      if (await alert({ message: L("Approve these exact assets?"), informative: L("Runner: %@. Profile: %@. Approve the displayed source, license, size and SHA-256 for runtime, model and tokenizer. HTTPS downloads happen only for an approved immutable plan; runtime binding is private to this Runner. No inference or paid fallback is called.", runner(), props.profile.id), buttons: [{ title: L("Approve and install") }, { title: L("Cancel") }] }) !== 0) return;
      const result = await props.api.applyLocal(approved, true, target());
      if (!("installed" in result)) throw new Error("invalid_setup_reply");
      setStatus(result.status === "ready" ? L("Approved local assets installed; runtime and model loaded successfully. No inference was called.") : L("Assets installed, but the local runtime is unavailable. No API fallback is used."));
    } catch (error) { setFailure(errorText(error)); }
    finally { setApproval(undefined); setBusy(false); }
  };
  const checkStatus = async () => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try { setStatus((await props.api.localStatus(runner(), props.profile.id)).status); }
    catch (error) { setFailure(errorText(error)); }
    finally { setBusy(false); }
  };
  return <Sheet title={L("Local embedding assets")} subtitle={L("Requires a saved local CPU profile. Core validates model/tokenizer hashes and runtime compatibility; unavailable assets never produce synthetic vectors.")} width={720} confirm={approval() ? L("Approve exact assets…") : L("Preview assets")} confirmDisabled={busy() || !runner()} onConfirm={() => void (approval() ? apply() : preview())} onCancel={() => { if (!busy()) props.dismiss(); }} class="memory-service-sheet" leading={<Button disabled={busy() || !runner()} onClick={() => void checkStatus()}>{L("Check approved runtime")}</Button>}>
    <Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show><Show when={status()}><div role="status">{status()}</div></Show>
    <fieldset disabled={busy()} class="memory-form-fields"><MemoryField label={L("Runner")}><PopUpButton label={L("Embedding Runner")} value={runner()} options={runners.map((device) => ({ value: device.id, label: device.name }))} onChange={(value) => { setRunner(value); setApproval(undefined); setStatus(""); }} /></MemoryField><MemoryField label={L("Profile")}><span>{props.profile.model} · {props.profile.model_revision} · {props.profile.dimensions}</span></MemoryField>
      <Show when={!approval()}><For each={assets()} keyed={false}>{(value) => { const asset = () => value(); return <section class="memory-section"><h3>{L(asset().kind)}</h3><MemoryField label={L("Source")}><PopUpButton value={asset().source} label={`${asset().kind} ${L("source")}`} options={[{ value: "supplied", label: L("Supplied Runner file") }, { value: "download", label: L("Exact HTTPS artifact") }]} onChange={(value) => patch(asset().kind, { source: value as "supplied" | "download", location: "" })} /></MemoryField><MemoryField label={asset().source === "supplied" ? L("Absolute Runner path") : L("Exact HTTPS URL")}><TextField label={`${asset().kind} ${L("location")}`} value={asset().location} onInput={(location) => patch(asset().kind, { location })} /></MemoryField><MemoryField label={L("License")}><TextField label={`${asset().kind} ${L("license")}`} value={asset().license} onInput={(license) => patch(asset().kind, { license })} /></MemoryField><MemoryField label={L("Exact bytes")}><TextField label={`${asset().kind} ${L("bytes")}`} value={asset().bytes} onInput={(bytes) => patch(asset().kind, { bytes })} /></MemoryField><MemoryField label={L("SHA-256")}><TextField label={`${asset().kind} SHA-256`} value={asset().sha256} onInput={(sha256) => patch(asset().kind, { sha256 })} /></MemoryField></section>; }}</For></Show>
    </fieldset>
    <Show when={approval()}>{(a) => <section class="memory-section"><h3>{L("Frozen asset preview")}</h3><MemoryField label={L("Total bytes")}><span>{a().preview.total_bytes}</span></MemoryField><For each={a().preview.assets}>{(asset) => <section class="memory-section"><h3>{L(asset.kind)}</h3><MemoryField label={L("Approved source")}><span class="memory-preview-value">{asset.source.kind === "supplied" ? asset.source.path : asset.source.url}</span></MemoryField><MemoryField label={L("License")}><span class="memory-preview-value">{asset.license}</span></MemoryField><MemoryField label={L("Exact bytes")}><span>{asset.bytes}</span></MemoryField><MemoryField label={L("SHA-256")}><span class="memory-preview-value">{asset.sha256}</span></MemoryField></section>}</For><div class="memory-note">{L("This preview expires in ten minutes. Sources and hashes cannot be changed at apply. Administrator download allowlists are not supplied by this form.")}</div><Button disabled={busy()} onClick={() => setApproval(undefined)}>{L("Discard preview and edit")}</Button></section>}</Show>
  </Sheet>;
}
