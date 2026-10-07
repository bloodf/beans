import { createSignal, For, onCleanup, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { L } from "../../l10n";
import { store, errorText } from "../../model/store";
import { MEMORY_ADVANCED_ACTIONS, MemoryPreferencesDraft, MemoryServiceAPI, MemoryServiceViewState, memoryVectorEditBody, secretPatch, supportsMemoryAction, supportsMemoryAdvancedAction, type MemoryBackend, type MemoryConnectionEdit, type MemoryConnectionOptions, type MemoryConnectionView, type MemoryConnectionsView, type MemoryHealth, type MemoryOperations, type MemoryPreferencesView, type MemoryAdvancedFeature } from "../../model/memoryService";
import { Button, PopUpButton, Switch, TextArea, TextField } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { MemorySetupControls, presentLocalEmbeddingSetup } from "./memorySetup";
import "./memoryService.css";

export const memoryAPI = new MemoryServiceAPI((method, params) => store.memoryRequest(method, { ...params }));
const backendTitles: Record<MemoryBackend, string> = { hindsight: "Hindsight", open_viking: "OpenViking", pgvector: "pgvector", lance_db: "LanceDB" };
export function presentMemoryConnections(): void { presentSheet((dismiss) => <MemoryConnectionsSheet api={memoryAPI} dismiss={dismiss} />); }
export function presentBotMemoryService(bot: { id: string; name: string }): void { presentSheet((dismiss) => <BotMemoryServiceSheet bot={bot} api={memoryAPI} dismiss={dismiss} />); }
export function MemoryField(props: { label: string; children: JSX.Element; note?: string }) {
  return <div class="memory-form-row"><div class="memory-form-label">{props.label}</div><div class="memory-form-control">{props.children}<Show when={props.note}><div class="memory-note">{props.note}</div></Show></div></div>;
}
export function MemoryToggle(props: { label: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void; note?: string }) {
  return <MemoryField label={props.label} note={props.note}><Switch label={props.label} checked={props.checked} disabled={props.disabled} onChange={props.onChange} /></MemoryField>;
}
function blockedConnectionText(connection: MemoryConnectionView): string {
  return connection.reason === "lancedb_cloud_transport_unavailable"
    ? L("LanceDB Cloud transport is unavailable. Runner-local Lance remains supported.")
    : L("This memory connection is blocked.");
}

export function MemoryConnectionsSheet(props: { api: MemoryServiceAPI; dismiss: () => void }) {
  const [listing, setListing] = createSignal<MemoryConnectionsView | undefined>();
  const [failure, setFailure] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  const load = async () => {
    setBusy(true); setFailure("");
    try { const reply = await props.api.listConnections(); if (alive) setListing(reply); }
    catch (error) { if (alive) setFailure(errorText(error)); }
    finally { if (alive) setBusy(false); }
  };
  onSettled(() => { void load(); });
  const edit = (connection?: MemoryConnectionView) => {
    const current = listing();
    if (!current) return;
    presentSheet((dismiss) => <MemoryConnectionForm connection={connection} listing={current} api={props.api} onSaved={load} dismiss={dismiss} />);
  };
  const disconnect = async (connection: MemoryConnectionView) => {
    if (await alert({ message: L("Disconnect %@?", connection.name), informative: L("Bots stop using this connection. This does not erase its remote memory bank."), buttons: [{ title: L("Disconnect"), destructive: true }, { title: L("Cancel") }] }) !== 0) return;
    setBusy(true);
    try { await props.api.disconnectConnection(connection.id); await load(); }
    catch (error) { setFailure(errorText(error)); setBusy(false); }
  };
  return <Sheet title={L("Memory connections")} subtitle={L("Connections belong to your account. Each bot keeps its own core-bound memory bank.")} width={660} confirm={L("Done")} cancel={null} onConfirm={props.dismiss} onCancel={props.dismiss} class="memory-service-sheet" leading={<Button disabled={busy()} onClick={() => void load()}>{L("Reload")}</Button>}>
    <Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show>
    <Show when={busy()}><div role="status">{L("Loading…")}</div></Show>
    <Show when={listing()}>{(data) => <>
      <section class="memory-section"><h3>{L("Connections")}</h3>
        <Show when={data().connections.length === 0}><p class="memory-note">{L("No memory connections. Local MEMORY.md remains available.")}</p></Show>
        <For each={data().connections}>{(connection) => <div class="memory-list-row"><div><strong>{connection.name}</strong><div class="memory-note">{backendTitles[connection.backend]} · {connection.has_secret ? L("Key saved") : L("No key saved")}</div><Show when={connection.availability === "blocked"}><div class="memory-note">{blockedConnectionText(connection)}</div></Show></div><div class="memory-inline-actions"><Button disabled={busy()} onClick={() => edit(connection)}>{L("Edit…")}</Button><Button disabled={busy()} onClick={() => void disconnect(connection)}>{L("Disconnect…")}</Button></div></div>}</For>
        <Button disabled={busy()} onClick={() => edit()}>{L("Add connection…")}</Button>
      </section>
      <section class="memory-section"><h3>{L("Embedding profiles")}</h3><For each={data().embeddings}>{(profile) => <div class="memory-list-row"><div><strong>{profile.model}</strong><div class="memory-note">{profile.model_revision} · {profile.dimensions} {L("dimensions")}</div></div><Button onClick={() => presentLocalEmbeddingSetup(profile)}>{L("Local setup…")}</Button></div>}</For><Button onClick={() => presentSheet((dismiss) => <MemoryEmbeddingForm onSaved={load} dismiss={dismiss} />)}>{L("Add embedding profile…")}</Button></section>
      <p class="memory-note">{L("Capture is off until a bot is explicitly opted in. Remote services receive approved plaintext; the encrypted relay is not the memory backend.")}</p>
    </>}</Show>
  </Sheet>;
}

interface BindingDraft { botID: string; mode: "user_key" | "trusted_gateway"; account: string; user: string; key: string; secretAction: "keep" | "replace" | "clear"; remove: boolean }
function MemoryConnectionForm(props: { connection?: MemoryConnectionView; listing: MemoryConnectionsView; api: MemoryServiceAPI; onSaved: () => Promise<void>; dismiss: () => void }) {
  const id = props.connection?.id ?? `memory-${crypto.randomUUID()}`;
  const [backend, setBackend] = createSignal<MemoryBackend>(props.connection?.backend ?? "hindsight");
  const [name, setName] = createSignal(props.connection?.name ?? "");
  const [endpointMode, setEndpointMode] = createSignal<"keep" | "replace" | "clear">(props.connection ? "keep" : "replace");
  const [endpoint, setEndpoint] = createSignal("");
  const [keyMode, setKeyMode] = createSignal<"keep" | "replace" | "clear">("keep");
  const [key, setKey] = createSignal("");
  const [embedding, setEmbedding] = createSignal(props.connection?.embedding_profile ?? "");
  const [editOptions, setEditOptions] = createSignal(false);
  const [schema, setSchema] = createSignal("");
  const [role, setRole] = createSignal("");
  const [region, setRegion] = createSignal("");
  const [transportPolicy, setTransportPolicy] = createSignal<"keep" | "secure" | "insecure">("keep");
  const [bindings, setBindings] = createSignal<BindingDraft[]>([]);
  const [replaceAll, setReplaceAll] = createSignal(false);
  const [busy, setBusy] = createSignal(false), [failure, setFailure] = createSignal("");
  onCleanup(() => { setKey(""); setBindings([]); });
  const save = async () => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try {
      const edit: MemoryConnectionEdit = { id, backend: backend(), name: name(), secret: secretPatch(keyMode(), key()), embedding_profile: embedding() || null };
      if (endpointMode() === "clear") edit.endpoint = null;
      else if (endpointMode() === "replace") edit.endpoint = endpoint() || null;
      if (transportPolicy() === "insecure") {
        if (await alert({ message: L("Allow unencrypted HTTP?"), informative: L("This connection can send approved memory text and credentials without transport encryption."), buttons: [{ title: L("Allow HTTP") }, { title: L("Cancel") }] }) !== 0) { setBusy(false); return; }
        edit.allow_insecure_http = true;
      } else if (transportPolicy() === "secure") edit.allow_insecure_http = false;
      if (editOptions()) {
        let options: MemoryConnectionOptions;
        switch (backend()) {
          case "hindsight": options = { backend: "hindsight" }; break;
          case "pgvector": options = { backend: "pgvector", schema: schema(), role: role() || null }; break;
          case "lance_db": options = { backend: "lance_db", region: region() || null }; break;
          case "open_viking": {
            const bound: Extract<MemoryConnectionOptions, { backend: "open_viking" }>["bindings"] = Object.create(null);
            for (const row of bindings()) {
              if (!row.botID || Object.hasOwn(bound, row.botID)) throw new Error(L("Choose a distinct bot for each binding."));
              bound[row.botID] = row.remove ? null : { mode: row.mode, account_id: row.account, user_id: row.user, secret: secretPatch(row.secretAction, row.key) };
            }
            options = { backend: "open_viking", bindings: bound };
            if (replaceAll()) {
              if (await alert({ message: L("Replace all OpenViking bindings?"), informative: L("Existing bindings not included here are removed. This replaces the entire binding set, not a partial patch."), buttons: [{ title: L("Replace all bindings") }, { title: L("Cancel") }] }) !== 0) { setBusy(false); return; }
              options.replace_all = true; options.confirm_replace_all = true;
            }
            break;
          }
        }
        edit.options = options;
      }
      await props.api.setConnection(edit); setKey(""); setBindings([]); await props.onSaved(); props.dismiss();
    } catch (error) { setFailure(errorText(error)); setBusy(false); }
  };
  return <Sheet title={props.connection ? L("Edit memory connection") : L("Add memory connection")} width={620} confirm={L("Save connection")} confirmDisabled={busy() || !name()} onConfirm={() => void save()} onCancel={() => { if (!busy()) props.dismiss(); }} class="memory-service-sheet">
    <Show when={failure()}><div class="memory-error" role="alert">{failure()}</div></Show>
    <fieldset disabled={busy()} class="memory-form-fields">
    <MemoryField label={L("Name")}><TextField value={name()} label={L("Connection name")} autofocus disabled={busy()} onInput={setName} /></MemoryField>
    <MemoryField label={L("Backend")}><PopUpButton value={backend()} disabled={busy() || !!props.connection} label={L("Memory backend")} options={(Object.keys(backendTitles) as MemoryBackend[]).map((value) => ({ value, label: backendTitles[value] }))} onChange={setBackend} /></MemoryField>
    <MemoryField label={L("Endpoint")} note={L("Saved endpoints are never read back. Keep preserves the current value; local Lance storage is bound on its Runner.")}><PopUpButton value={endpointMode()} label={L("Endpoint change")} options={[{ value: "keep", label: L("Keep") }, { value: "replace", label: L("Replace") }, { value: "clear", label: L("Clear") }]} onChange={(value) => setEndpointMode(value as "keep" | "replace" | "clear")} /><Show when={endpointMode() === "replace"}><TextField value={endpoint()} label={L("Exact memory endpoint")} disabled={busy()} onInput={setEndpoint} /></Show></MemoryField>
    <MemoryField label={L("Secret")} note={props.connection?.has_secret ? L("A key is saved. Its value is never shown.") : L("No key is returned to this form.")}><PopUpButton value={keyMode()} label={L("Secret change")} options={[{ value: "keep", label: L("Keep") }, { value: "replace", label: L("Replace") }, { value: "clear", label: L("Clear") }]} onChange={(value) => { setKeyMode(value as "keep" | "replace" | "clear"); setKey(""); }} /><Show when={keyMode() === "replace"}><TextField secure value={key()} label={L("Replacement memory secret")} disabled={busy()} onInput={setKey} /></Show></MemoryField>
    <MemoryField label={L("Embedding profile")}><PopUpButton value={embedding()} label={L("Embedding profile")} options={[{ value: "", label: L("None") }, ...props.listing.embeddings.map((e) => ({ value: e.id, label: `${e.model} · ${e.dimensions}` }))]} onChange={setEmbedding} /></MemoryField>
    <MemoryField label={L("Transport policy")}><PopUpButton value={transportPolicy()} label={L("Memory transport policy")} options={[{ value: "keep", label: L("Keep saved policy") }, { value: "secure", label: L("Require secure transport") }, { value: "insecure", label: L("Allow insecure HTTP…") }]} onChange={(value) => setTransportPolicy(value as "keep" | "secure" | "insecure")} /></MemoryField>
    <MemoryToggle label={L("Replace backend settings")} checked={editOptions()} onChange={setEditOptions} disabled={busy()} note={L("Off preserves saved settings. No unknown backend JSON is accepted.")} />
    <Show when={editOptions() && backend() === "pgvector"}><MemoryField label={L("Schema")}><TextField value={schema()} label={L("PostgreSQL schema")} onInput={setSchema} /></MemoryField></Show>
    <Show when={editOptions() && backend() === "pgvector"}><MemoryField label={L("Database role")} note={L("Optional role; password stays in the separate secret patch.")}><TextField value={role()} label={L("PostgreSQL role")} onInput={setRole} /></MemoryField></Show>
    <Show when={editOptions() && backend() === "lance_db"}><MemoryField label={L("Cloud region")} note={L("Leave empty for no portable region. Local directories are approved separately on the Runner.")}><TextField value={region()} label={L("Lance region")} onInput={setRegion} /></MemoryField></Show>
    <Show when={editOptions() && backend() === "open_viking"}><section class="memory-section">
      <h3>{L("Bot identity bindings")}</h3>
      <p class="memory-note">{L("Omitted bots keep their existing bindings. Removing a binding is explicit. Clearing a key preserves its identity but requires setup before use.")}</p>
      <MemoryToggle label={L("Replace all bindings…")} checked={replaceAll()} onChange={setReplaceAll} note={L("Off applies only the rows below. On requires separate confirmation to remove omitted bindings.")} />
      <For each={bindings()} keyed={false}>{(value, index) => {
        const row = () => value();
        const patch = (change: Partial<BindingDraft>) => setBindings(bindings().map((item, i) => i === index ? { ...item, ...change } : item));
        return <fieldset class="memory-binding">
          <legend>{L("Bot binding")}</legend>
          <PopUpButton label={L("Binding bot")} value={row().botID} options={[{ value: "", label: L("Choose a bot") }, ...store.bots.map((bot) => ({ value: bot.id, label: bot.name }))]} onChange={(botID) => patch({ botID })} />
          <MemoryToggle label={L("Remove this bot binding")} checked={row().remove} onChange={(remove) => patch({ remove })} />
          <Show when={!row().remove}>
            <PopUpButton label={L("Authentication mode")} value={row().mode} options={[{ value: "user_key", label: L("Per-user key") }, { value: "trusted_gateway", label: L("Trusted gateway") }]} onChange={(mode) => patch({ mode: mode as BindingDraft["mode"] })} />
            <TextField value={row().account} label={L("Account ID")} placeholder={L("Account ID")} onInput={(account) => patch({ account })} />
            <TextField value={row().user} label={L("User ID")} placeholder={L("User ID")} onInput={(user) => patch({ user })} />
            <PopUpButton label={L("Binding secret change")} value={row().secretAction} options={[{ value: "keep", label: L("Keep") }, { value: "replace", label: L("Replace") }, { value: "clear", label: L("Clear") }]} onChange={(action) => patch({ secretAction: action as BindingDraft["secretAction"], key: "" })} />
            <Show when={row().secretAction === "replace"}><TextField secure value={row().key} label={L("New binding key")} onInput={(key) => patch({ key })} /></Show>
          </Show>
          <Button onClick={() => setBindings(bindings().filter((_, i) => i !== index))}>{L("Discard draft row")}</Button>
        </fieldset>;
      }}</For>
      <Button onClick={() => setBindings([...bindings(), { botID: "", mode: "user_key", account: "", user: "", key: "", secretAction: "replace", remove: false }])}>{L("Add bot binding")}</Button>
    </section></Show>
    </fieldset>
  </Sheet>;
}

function MemoryEmbeddingForm(props: { onSaved: () => Promise<void>; dismiss: () => void }) {
  const [mode, setMode] = createSignal<"api" | "local_cpu">("api");
  const [values, setValues] = createSignal<Record<string, string>>({ normalization: "l2", distance: "cosine", pooling: "mean", max_tokens: "512", pad_id: "0", pad_type_id: "0", pad_token: "[PAD]" });
  const [failure, setFailure] = createSignal(""), [busy, setBusy] = createSignal(false);
  const set = (key: string, value: string) => setValues({ ...values(), [key]: value });
  const field = (key: string, label: string, secure = false) => <MemoryField label={L(label)}><TextField value={values()[key] ?? ""} label={L(label)} secure={secure} onInput={(value) => set(key, value)} /></MemoryField>;
  const save = async () => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try {
      const v = values();
      const profile = { model: v.model ?? "", revision: v.revision ?? "", dimensions: Number(v.dimensions), normalization: v.normalization, distance: v.distance, document_prefix: v.document_prefix ?? "", query_prefix: v.query_prefix ?? "", mode: mode(), endpoint: mode() === "api" ? v.endpoint : null, local: mode() === "local_cpu" ? { model_sha256: v.model_sha256, tokenizer_sha256: v.tokenizer_sha256, max_tokens: Number(v.max_tokens), pooling: v.pooling, tensors: { input_ids: v.input_ids, attention_mask: v.attention_mask, token_type_ids: v.token_type_ids || null, output: v.output }, add_special_tokens: v.add_special_tokens !== "false", pad_id: Number(v.pad_id), pad_type_id: Number(v.pad_type_id), pad_token: v.pad_token } : null };
      await store.memoryRequest("memory.embeddings.set", { id: v.id, profile, secret: mode() === "api" && v.key ? secretPatch("replace", v.key) : secretPatch("clear") });
      set("key", ""); await props.onSaved(); props.dismiss();
    } catch (error) { setFailure(errorText(error)); setBusy(false); }
  };
  onCleanup(() => set("key", ""));
  return <Sheet title={L("Embedding profile")} subtitle={L("Pin the complete vector space. Changing it requires a validated replacement index, not a dimensions-only match.")} width={620} confirm={L("Save profile")} confirmDisabled={busy()} onConfirm={() => void save()} onCancel={() => { if (!busy()) props.dismiss(); }} class="memory-service-sheet"><Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show>
    <fieldset disabled={busy()} class="memory-form-fields">
    {field("id", "Profile ID")}{field("model", "Exact model")}{field("revision", "Immutable model revision")}{field("dimensions", "Dimensions")}
    <MemoryField label={L("Execution")}><PopUpButton value={mode()} label={L("Embedding execution")} options={[{ value: "api", label: L("API") }, { value: "local_cpu", label: L("Local CPU") }]} onChange={(value) => { setMode(value as "api" | "local_cpu"); set("key", ""); }} /></MemoryField>
    <MemoryField label={L("Normalization")}><PopUpButton value={values().normalization ?? "l2"} label={L("Normalization")} options={[{ value: "none", label: L("None") }, { value: "l2", label: "L2" }]} onChange={(v) => set("normalization", v)} /></MemoryField>
    <MemoryField label={L("Distance")}><PopUpButton value={values().distance ?? "cosine"} label={L("Distance")} options={[{ value: "cosine", label: L("Cosine") }, { value: "dot", label: L("Dot") }, { value: "euclidean", label: L("Euclidean") }]} onChange={(v) => set("distance", v)} /></MemoryField>
    {field("document_prefix", "Exact document prefix")}{field("query_prefix", "Exact query prefix")}
    <Show when={mode() === "api"}>{field("endpoint", "Exact /embeddings endpoint")}{field("key", "Embedding key", true)}</Show>
    <Show when={mode() === "local_cpu"}>{field("model_sha256", "Model SHA-256")}{field("tokenizer_sha256", "Tokenizer SHA-256")}{field("max_tokens", "Maximum tokens")}{field("input_ids", "Input IDs tensor")}{field("attention_mask", "Attention mask tensor")}{field("token_type_ids", "Token type tensor (optional)")}{field("output", "Output tensor")}<MemoryField label={L("Pooling")}><PopUpButton value={values().pooling ?? "mean"} label={L("Pooling")} options={["mean", "cls", "pooled"].map((value) => ({ value, label: value }))} onChange={(v) => set("pooling", v)} /></MemoryField>{field("pad_id", "Padding ID")}{field("pad_type_id", "Padding type ID")}{field("pad_token", "Padding token")}<MemoryToggle label={L("Add special tokens")} checked={values().add_special_tokens !== "false"} onChange={(v) => set("add_special_tokens", String(v))} /></Show>
    </fieldset>
  </Sheet>;
}

export function BotMemoryServiceSheet(props: { bot: { id: string; name: string }; api: MemoryServiceAPI; dismiss: () => void }) {
  const [listing, setListing] = createSignal<MemoryConnectionsView>(), [saved, setSaved] = createSignal<MemoryPreferencesView>();
  const [draft, setDraft] = createSignal<MemoryPreferencesDraft>(), [health, setHealth] = createSignal<MemoryHealth>(), [operations, setOperations] = createSignal<MemoryOperations>();
  const [failure, setFailure] = createSignal(""), [busy, setBusy] = createSignal(false), [epoch, setEpoch] = createSignal(0);
  let viewState: MemoryServiceViewState | undefined;
  let alive = true;
  onCleanup(() => { alive = false; });
  const load = async (resetDraft = false) => {
    setBusy(true); setFailure("");
    try {
      const [connections, preferences] = await Promise.all([props.api.listConnections(), props.api.getPreferences(props.bot.id)]);
      if (!alive) return;
      viewState?.refreshPreferences(preferences);
      if (!viewState || resetDraft) viewState = new MemoryServiceViewState(props.bot.id, preferences);
      setListing(connections); setSaved(viewState.preferences); setDraft(viewState.draft); setEpoch(epoch() + 1);
      setHealth(undefined);
      try { const delivery = await props.api.operations(props.bot.id); if (alive) setOperations(delivery); }
      catch (error) { if (alive) { setOperations(undefined); setFailure(errorText(error)); } }
    } catch (error) { if (alive) setFailure(errorText(error)); }
    finally { if (alive) setBusy(false); }
  };
  const refreshStatus = async () => {
    const [connections, preferences] = await Promise.all([props.api.listConnections(), props.api.getPreferences(props.bot.id)]);
    if (!alive) return;
    const previous = saved()?.connection_id;
    viewState?.refreshPreferences(preferences);
    setListing(connections); setSaved(preferences); setEpoch(epoch() + 1);
    if (previous !== preferences.connection_id) setHealth(undefined);
    const delivery = await props.api.operations(props.bot.id);
    if (alive) setOperations(delivery);
    if (health()) {
      try { const status = await props.api.health(props.bot.id); if (alive) setHealth(status); }
      catch (error) { if (alive) setHealth(undefined); throw error; }
    }
  };
  const refreshAuthoritative = async () => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try { await refreshStatus(); }
    catch (error) { if (alive) setFailure(errorText(error)); }
    finally { if (alive) setBusy(false); }
  };
  const deletionReady = () => { epoch(); return !!viewState && !viewState.deletionLocked; };
  const beginDeletion = () => {
    if (!viewState) throw new Error("preferences_refresh_required");
    const deletionEpoch = viewState.beginDeletion(); setEpoch(epoch() + 1); return deletionEpoch;
  };
  const recordDeletionEpoch = (value: number) => { viewState?.recordDeletionEpoch(value); };
  const checkService = async () => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try { const status = await props.api.health(props.bot.id); if (alive) setHealth(status); }
    catch (error) { if (alive) { setHealth(undefined); setFailure(errorText(error)); } }
    finally { if (alive) setBusy(false); }
  };
  const operationAction = async (id: string, method: "status" | "cancel") => {
    if (busy()) return;
    setBusy(true); setFailure("");
    try {
      if (method === "cancel" && await alert({ message: L("Cancel this operation?"), informative: L("Cancellation may not prevent late writes; deletion remains pending until verified."), buttons: [{ title: L("Cancel operation") }, { title: L("Keep operation") }], escape: 1 }) !== 0) return;
      await store.memoryRequest(`memory.operations.${method}`, { bot_id: props.bot.id, id });
      const delivery = await props.api.operations(props.bot.id); if (alive) setOperations(delivery);
    } catch (error) { if (alive) setFailure(errorText(error)); }
    finally { if (alive) setBusy(false); }
  };
  onSettled(() => { void load(); });
  const changed = (mutate: (value: MemoryPreferencesDraft) => void) => { const d = draft(); if (d) { mutate(d); setEpoch(epoch() + 1); } };
  const draftConnectionBlocked = () => { epoch(); return listing()?.connections.find((c) => c.id === draft()?.connectionID)?.availability === "blocked"; };
  const save = async () => {
    const d = draft(); if (!d || busy()) return;
    if (draftConnectionBlocked()) { setFailure(L("This memory connection is blocked.")); return; }
    setBusy(true); setFailure("");
    try {
      try { d.request(); } catch (error) {
        if (!(error instanceof Error)) throw error;
        if (error.message === "plaintext_consent_required") {
          const connection = listing()?.connections.find((c) => c.id === d.connectionID);
          if (await alert({ message: L("Send future plaintext to %@?", connection?.name ?? ""), informative: L("Bot: %@. Remote queries and opted-in completed conversation text go to this service. Capture never includes historical messages. Up to %d capture deliveries per turn can incur service costs.", props.bot.name, d.maxCaptureDeliveriesPerTurn), buttons: [{ title: L("Approve future use") }, { title: L("Cancel") }] }) !== 0) { setBusy(false); return; }
          d.approveRemotePlaintext();
        } else if (error.message !== "group_consent_required") throw error;
        if (d.captureGroupText) {
          if (await alert({ message: L("Capture future group text for %@?", props.bot.name), informative: L("This is separate consent to send other participants' eligible visible group messages to the selected memory service."), buttons: [{ title: L("Approve group capture") }, { title: L("Cancel") }] }) !== 0) { setBusy(false); return; }
          d.approveGroupCapture();
        }
      }
      await props.api.savePreferences(d); await load(true);
    } catch (error) { setFailure(errorText(error)); setBusy(false); }
  };
  const connection = () => listing()?.connections.find((c) => c.id === saved()?.connection_id);
  return <Sheet title={L("%@'s memory service", props.bot.name)} subtitle={L("Local MEMORY.md stays independent. This bot's bank namespace is computed by core, never entered here.")} width={720} confirm={L("Save preferences")} confirmDisabled={busy() || !draft() || draftConnectionBlocked()} onConfirm={() => void save()} onCancel={() => { if (!busy()) props.dismiss(); }} class="memory-service-sheet" leading={<Button disabled={busy()} onClick={() => void load()}>{L("Reload")}</Button>}>
    <Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show>
    <Show when={busy()}><div role="status">{L("Loading…")}</div></Show>
    <Button disabled={busy() || !saved()?.connection_id} onClick={() => void checkService()}>{L("Check service and capabilities")}</Button>
    <Show when={!health()}><p class="memory-note">{L("Service readiness has not been checked. Checking is explicit; opening this form never calls remote Health, retain, reflect or initialization.")}</p></Show>
    <Show when={draft()}>{(value) => { const d = () => { epoch(); return value(); }; return <fieldset disabled={busy()} class="memory-form-fields">
      <MemoryField label={L("Connection")}><PopUpButton value={d().connectionID ?? ""} disabled={busy()} label={L("Bot memory connection")} options={[{ value: "", label: L("Off") }, ...(listing()?.connections ?? []).filter((c) => c.availability === "supported" || c.id === d().connectionID).map((c) => ({ value: c.id, label: c.availability === "blocked" ? L("%@ (blocked)", c.name) : c.name }))]} onChange={(v) => { if (listing()?.connections.find((c) => c.id === v)?.availability === "blocked") return; changed((draft) => { draft.connectionID = v || null; if (!v) { draft.autoRecall = false; draft.captureConversation = false; draft.captureGroupText = false; } }); }} /></MemoryField>
      <For each={(listing()?.connections ?? []).filter((c) => c.availability === "blocked")}>{(c) => <div class="memory-note">{c.name}: {blockedConnectionText(c)}</div>}</For>
      <MemoryToggle label={L("Recall during turns")} checked={d().autoRecall} disabled={busy() || !d().connectionID} onChange={(v) => changed((draft) => { draft.autoRecall = v; })} />
      <MemoryToggle label={L("Capture future conversations")} checked={d().captureConversation} disabled={busy() || !d().connectionID} onChange={(v) => changed((draft) => { draft.captureConversation = v; if (!v) draft.captureGroupText = false; })} />
      <MemoryToggle label={L("Capture group text separately")} checked={d().captureGroupText} disabled={busy() || !d().captureConversation} onChange={(v) => changed((draft) => { draft.captureGroupText = v; })} />
      <MemoryField label={L("Capture deliveries per turn")}><PopUpButton value={d().maxCaptureDeliveriesPerTurn} label={L("Capture delivery cap")} options={[0, 1, 2, 3, 4].map((value) => ({ value, label: String(value) }))} onChange={(v) => changed((draft) => { draft.maxCaptureDeliveriesPerTurn = v; })} /></MemoryField>
      <p class="memory-note">{L("Unattended capture is unavailable: there is no enforceable service-side total-cost policy. No background spending consent is assumed.")}</p>
      <section class="memory-section"><h3>{L("Recall limits")}</h3><For each={([ ["timeout_ms", "Timeout (ms)"], ["max_bytes", "Response bytes"], ["max_results", "Results"], ["max_context_chars", "Context characters"] ] as const)}>{([key, label]) => <MemoryField label={L(label)}><TextField value={String(d().recallBudget[key])} label={L(label)} onInput={(v) => changed((draft) => { draft.recallBudget[key] = Number(v); })} /></MemoryField>}</For></section>
    </fieldset>; }}</Show>
    <Show when={health()}>{(h) => <section class="memory-section"><h3>{L("Saved service status")}</h3><div class="memory-note">{connection()?.name}</div><div role="status">{h().status}{h().reason ? ` · ${h().reason}` : ""}{h().deletion_pending ? ` · ${L("Deletion pending—not verified erased")}` : ""}</div><MemoryServiceActions bot={props.bot} health={h()} connection={connection()} preferences={saved()} onChanged={refreshStatus} onBusy={(value) => { if (alive) setBusy(value); }} working={busy()} deletionReady={deletionReady()} beginDeletion={beginDeletion} recordDeletionEpoch={recordDeletionEpoch} /></section>}</Show>
    <Show when={operations()}>{(delivery) => <section class="memory-section"><h3>{L("Deliveries")}</h3><For each={delivery().operations}>{(op) => <div class="memory-list-row"><div><span>{op.document_id}</span><div class="memory-note">{op.state === "completed" ? L("Stored") : op.state}{op.error_code ? ` · ${op.error_code}` : ""}</div></div><div class="memory-inline-actions"><Show when={health()?.capabilities.operation_status}><Button disabled={busy()} onClick={() => void operationAction(op.id, "status")}>{L("Check")}</Button></Show><Show when={health()?.capabilities.cancel_operation && op.state !== "completed" && op.state !== "failed"}><Button disabled={busy()} onClick={() => void operationAction(op.id, "cancel")}>{L("Cancel…")}</Button></Show></div></div>}</For><Show when={delivery().deletion?.pending}><p class="memory-note">{L("Cleanup remains pending. Backups and Lance history may retain data.")}</p></Show></section>}</Show>
    <Show when={!deletionReady() && draft()}><div role="status" class="memory-note">{L("Deletion actions are locked until authoritative preferences refresh succeeds.")}</div><Button disabled={busy()} onClick={() => void refreshAuthoritative()}>{L("Refresh authoritative status")}</Button></Show>
    <Show when={connection()}>{(c) => <MemorySetupControls bot={props.bot} connection={c()} onChanged={refreshStatus} />}</Show>
  </Sheet>;
}

function MemoryServiceActions(props: { bot: { id: string; name: string }; health: MemoryHealth; connection?: MemoryConnectionView; preferences?: MemoryPreferencesView; onChanged: () => Promise<void>; onBusy: (value: boolean) => void; working: boolean; deletionReady: boolean; beginDeletion: () => number; recordDeletionEpoch: (epoch: number) => void }) {
  const [query, setQuery] = createSignal(""), [text, setText] = createSignal(""), [documentID, setDocumentID] = createSignal("");
  const [result, setResult] = createSignal(""), [failure, setFailure] = createSignal(""), [busy, setBusy] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  const call = async (method: string, body: Record<string, unknown>, mutating = false) => {
    if (busy() || props.working) return;
    setBusy(true); props.onBusy(true); setFailure("");
    try {
      if (mutating && await alert({ message: L("Send this memory request?"), informative: L("Bot: %@. This sends approved plaintext to its configured service and may incur service costs. No guaranteed exactly-once billing is implied.", props.bot.name), buttons: [{ title: L("Send request") }, { title: L("Cancel") }] }) !== 0) return;
      const reply = await store.memoryRequest(method, { bot_id: props.bot.id, ...body });
      if (alive) setResult(JSON.stringify(reply, null, 2)); if (mutating) await props.onChanged();
    } catch (error) { if (alive) setFailure(errorText(error)); }
    finally { if (alive) setBusy(false); props.onBusy(false); }
  };
  const remove = async (all: boolean) => {
    if (busy() || props.working || !props.deletionReady || !props.connection || !props.preferences) return;
    const document_id = documentID(), connection_revision = { ...props.connection.revision };
    setBusy(true); props.onBusy(true); setFailure("");
    let admitted = false;
    try {
      if (await alert({ message: all ? L("Clear %@'s remote bank?", props.bot.name) : L("Delete document %@?", document_id), informative: L("This targets the current connection revision and deletion epoch. The result may remain pending; backups/history can retain data."), buttons: [{ title: L("Confirm deletion"), destructive: true }, { title: L("Cancel") }] }) !== 0) return;
      const deletion_epoch = props.beginDeletion(); admitted = true;
      const reply = await store.memoryRequest("memory.service.delete", { bot_id: props.bot.id, ...(all ? {} : { document_id }), confirm: true, connection_revision, deletion_epoch });
      if (reply && typeof reply === "object" && "deletion_epoch" in reply && typeof reply.deletion_epoch === "number") props.recordDeletionEpoch(reply.deletion_epoch);
      if (alive) setResult(JSON.stringify(reply, null, 2));
    } catch (error) { if (alive) setFailure(errorText(error)); }
    finally {
      if (admitted) {
        try { await props.onChanged(); }
        catch (error) { if (alive) setFailure(errorText(error)); }
      }
      if (alive) setBusy(false); props.onBusy(false);
    }
  };
  return <fieldset disabled={busy() || props.working} class="memory-form-fields">
    <Show when={failure()}><div role="alert" class="memory-error">{failure()}</div></Show>
    <Show when={supportsMemoryAction(props.health.capabilities, "recall") || supportsMemoryAction(props.health.capabilities, "reflect")}><MemoryField label={L("Query")}><TextField label={L("Memory query")} value={query()} onInput={setQuery} /></MemoryField><div class="memory-inline-actions"><Show when={supportsMemoryAction(props.health.capabilities, "recall")}><Button disabled={busy() || !query()} onClick={() => void call("memory.service.recall", { query: query() })}>{L("Recall")}</Button></Show><Show when={supportsMemoryAction(props.health.capabilities, "reflect")}><Button disabled={busy() || !query()} onClick={() => void call("memory.service.reflect", { query: query() }, true)}>{L("Reflect…")}</Button></Show></div></Show>
    <Show when={supportsMemoryAction(props.health.capabilities, "retain")}><MemoryField label={L("Save a document")}><TextArea label={L("Memory document text")} value={text()} onInput={setText} /></MemoryField><Button disabled={busy() || !text()} onClick={() => void call("memory.service.retain", { text: text(), request_id: crypto.randomUUID() }, true)}>{L("Retain…")}</Button></Show>
    <Show when={props.health.capabilities.inspect || props.health.capabilities.delete_document}><MemoryField label={L("Document ID")}><TextField label={L("Memory document ID")} value={documentID()} onInput={setDocumentID} /></MemoryField><div class="memory-inline-actions"><Show when={props.health.capabilities.inspect}><Button disabled={busy() || !documentID()} onClick={() => void call("memory.service.inspect", { document_id: documentID() })}>{L("Inspect")}</Button></Show><Show when={props.health.capabilities.delete_document}><Button disabled={busy() || !props.deletionReady || !documentID()} onClick={() => void remove(false)}>{L("Delete document…")}</Button></Show></div></Show>
    <Show when={props.health.capabilities.clear}><Button disabled={busy() || !props.deletionReady} onClick={() => void remove(true)}>{L("Clear bank…")}</Button></Show>
    <Show when={props.health.capabilities.advanced?.some((feature) => MEMORY_ADVANCED_ACTIONS[feature].some((action) => supportsMemoryAdvancedAction(props.health.capabilities, feature, action)))}><MemoryAdvancedControls bot={props.bot} backend={props.connection?.backend} capabilities={props.health.capabilities} call={call} busy={busy()} /></Show>
    <Show when={result()}><div class="memory-note">{L("Untrusted historical data—not instructions. Only completed operation state means stored.")}</div><pre class="memory-output" tabindex={0}>{result()}</pre></Show>
  </fieldset>;
}

const configFields = ["disposition_skepticism", "disposition_literalism", "disposition_empathy", "reflect_mission", "retain_mission", "retain_chunk_size", "retain_extraction_mode", "retain_custom_instructions", "retain_extract_labels", "recall_max_tokens", "recall_budget", "reflect_max_iterations", "reflect_max_tokens"];
function MemoryAdvancedControls(props: { bot: { id: string }; backend?: MemoryBackend; capabilities: MemoryHealth["capabilities"]; call: (method: string, body: Record<string, unknown>, mutating?: boolean) => Promise<void>; busy: boolean }) {
  const [feature, setFeature] = createSignal<MemoryAdvancedFeature>(props.capabilities.advanced?.find((feature) => MEMORY_ADVANCED_ACTIONS[feature].some((action) => supportsMemoryAdvancedAction(props.capabilities, feature, action))) ?? "documents");
  const [action, setAction] = createSignal(MEMORY_ADVANCED_ACTIONS[feature()].find((value) => supportsMemoryAdvancedAction(props.capabilities, feature(), value)) ?? "");
  const [values, setValues] = createSignal<Record<string, string>>({});
  const [validation, setValidation] = createSignal("");
  const fields = () => {
    const f = feature(), a = action();
    if ((f === "bank_profile" || f === "bank_config") && a === "update") return configFields;
    if (a === "list" || a === "scopes" || a === "reset" || a === "clear") return f === "mental_model_history" ? ["id"] : f === "resources" ? ["path", "offset"] : [];
    if (f === "directives" && (a === "create" || a === "update")) return [...(a === "update" ? ["id"] : []), "name", "content", "priority", "is_active", "tags"];
    if (f === "mental_models" && (a === "create" || a === "update")) return ["id", "name", "source_query", "tags", "max_tokens"];
    if (f === "memory_edit" && a === "edit") return ["document_id", "text"];
    if (f === "memory_edit" && a === "update") return ["id", "text", "context", "occurred_start", "occurred_end", "tags"];
    if ((f === "memory_invalidate" || f === "memory_restore") && a === "update") return ["id", "reason"];
    if (f === "sessions" && a === "add_message") return ["id", "role", "content"];
    if (f === "resources") return a === "add" ? ["id", "source_url"] : ["path"];
    return ["id"];
  };
  const actions = () => MEMORY_ADVANCED_ACTIONS[feature()].filter((action) => supportsMemoryAdvancedAction(props.capabilities, feature(), action));
  const run = async () => {
    if (props.busy) return;
    setValidation("");
    if (!supportsMemoryAdvancedAction(props.capabilities, feature(), action())) { setValidation(L("This action is not available for the negotiated capabilities.")); return; }
    const body: Record<string, unknown> = {};
    for (const key of fields()) {
      const value = values()[key];
      if (!value) continue;
      if (["priority", "offset", "max_tokens", "retain_chunk_size", "recall_max_tokens", "reflect_max_iterations", "reflect_max_tokens", "disposition_skepticism", "disposition_literalism", "disposition_empathy"].includes(key)) {
        const number = Number(value);
        if (!Number.isSafeInteger(number)) { setValidation(L("Enter a whole number for %@.", key)); return; }
        body[key] = number;
      }
      else if (key === "tags") body[key] = value.split(",").map((t) => t.trim()).filter(Boolean);
      else if (key === "is_active" || key === "retain_extract_labels") body[key] = value === "true";
      else body[key] = value;
    }
    if (feature() === "memory_edit" && action() === "edit") {
      try { Object.assign(body, memoryVectorEditBody(values().document_id ?? "", values().text ?? "")); }
      catch (error) { setValidation(errorText(error)); return; }
    }
    await props.call("memory.service.advanced", { feature: feature(), action: action(), body: (feature() === "bank_profile" || feature() === "bank_config") && action() === "update" ? { updates: body } : body }, !["get", "list", "scopes", "history", "chunks", "read"].includes(action()));
  };
  return <section class="memory-section">
    <h3>{L("Advanced service controls")}</h3>
    <p class="memory-note">{L("Only negotiated features are shown. Core verifies each concrete action against the service's supported routes; no background refresh is enabled.")}</p>
    <Show when={validation()}><div role="alert" class="memory-error">{validation()}</div></Show>
    <MemoryField label={L("Feature")}><PopUpButton label={L("Advanced memory feature")} value={feature()} options={(props.capabilities.advanced ?? []).filter((feature) => MEMORY_ADVANCED_ACTIONS[feature].some((action) => supportsMemoryAdvancedAction(props.capabilities, feature, action))).map((value) => ({ value, label: value.replaceAll("_", " ") }))} onChange={(v) => { setFeature(v); setAction(MEMORY_ADVANCED_ACTIONS[v].find((action) => supportsMemoryAdvancedAction(props.capabilities, v, action)) ?? ""); setValues({}); setValidation(""); }} /></MemoryField>
    <MemoryField label={L("Action")}><PopUpButton label={L("Advanced memory action")} value={action()} options={actions().map((value) => ({ value, label: value.replaceAll("_", " ") }))} onChange={(v) => { setAction(v); setValues({}); setValidation(""); }} /></MemoryField>
    <For each={fields()}>{(key) => <MemoryField label={key.replaceAll("_", " ")}>
      {key === "is_active" || key === "retain_extract_labels"
        ? <PopUpButton label={key.replaceAll("_", " ")} value={values()[key] ?? ""} options={[{ value: "", label: L("Unchanged") }, { value: "true", label: L("Yes") }, { value: "false", label: L("No") }]} onChange={(value) => setValues({ ...values(), [key]: value })} />
        : key === "role"
        ? <PopUpButton label={L("Message speaker")} value={values()[key] ?? ""} options={[{ value: "", label: L("Choose speaker") }, { value: "user", label: L("User") }, { value: "assistant", label: L("Assistant") }]} onChange={(value) => setValues({ ...values(), [key]: value })} />
        : ["content", "text", "context", "source_query", "reflect_mission", "retain_mission", "retain_custom_instructions", "reason"].includes(key)
        ? <TextArea label={key.replaceAll("_", " ")} value={values()[key] ?? ""} onInput={(value) => setValues({ ...values(), [key]: value })} />
        : <TextField label={key.replaceAll("_", " ")} value={values()[key] ?? ""} onInput={(value) => setValues({ ...values(), [key]: value })} />}
    </MemoryField>}</For>
    <Button disabled={props.busy || !actions().includes(action())} onClick={() => void run()}>{L("Run action…")}</Button>
  </section>;
}
