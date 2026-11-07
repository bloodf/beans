import { t, useLanguage } from '../../../src/i18n';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useEffect, useState } from 'react';
import * as Crypto from 'expo-crypto';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, connectionActivationReason, MemoryPreferencesDraft, supportsMemoryAction, type MemoryConnectionsView, type MemoryHealth, type MemoryOperations, type MemoryPreferencesView, type MemoryAdvancedFeature } from '../../../src/core/memoryService';
import { advancedForms } from '../../../src/core/memoryAdvanced';
import { useBotMap, useStore } from '../../../src/core/store';
import { FieldRow, Row, Section, ToggleRow } from '../../../src/ui/forms';
import { memoryText, confirmMemory, MemoryAction, MemoryForm, MemoryOutput, plaintextDisclosure, useMemoryTask } from '../../../src/ui/memoryUI';
import { MemoryDeletionState } from '../../../src/ui/memoryTask';
const advanced: MemoryAdvancedFeature[] = ['bank_profile', 'bank_config', 'directives', 'mental_models', 'mental_model_history', 'observations', 'memory_edit', 'memory_invalidate', 'memory_restore', 'documents', 'sessions', 'resources', 'tasks'];
export default function BotMemoryScreen() {
    useLanguage();
    const { id } = useLocalSearchParams<{
        id: string;
    }>(), router = useRouter(), task = useMemoryTask();
    const bot = useBotMap().get(id);
    const devices = useStore(s => s.devices);
    const [config, setConfig] = useState<MemoryConnectionsView>();
    const [saved, setSaved] = useState<MemoryPreferencesView>();
    const [deletion] = useState(() => new MemoryDeletionState());
    const [draft, setDraft] = useState<MemoryPreferencesDraft>();
    const [dirty, setDirty] = useState(false);
    const [, update] = useState(0);
    const [health, setHealth] = useState<MemoryHealth>();
    const [operations, setOperations] = useState<MemoryOperations>();
    const [query, setQuery] = useState('');
    const [text, setText] = useState('');
    const [requestID, setRequestID] = useState(() => Crypto.randomUUID());
    const [documentID, setDocumentID] = useState('');
    const [output, setOutput] = useState<unknown>();
    useEffect(() => { void task.run(async () => { const [c, p] = await Promise.all([memoryService.listConnections(), memoryService.getPreferences(id)]); setConfig(c); setSaved(p); deletion.preferences = p; setDraft(new MemoryPreferencesDraft(id, p)); }); }, [id]);
    const runner = devices.find(d => d.id === bot?.runner_id);
    const connection = config?.connections.find(c => c.id === saved?.connection_id);
    const caps = health?.capabilities ?? {};
    const activationReason = connectionActivationReason(connection);
    const reason = (action: Parameters<typeof supportsMemoryAction>[1]) => activationReason ?? (dirty ? t('Save or cancel preference changes first.') : !supportsMemoryAction(caps, action) ? t('Not negotiated by this Runner/backend. Check readiness explicitly.') : undefined);
    function edit(action: () => void) { action(); setDirty(true); update(n => n + 1); }
    async function save() { if (!draft || !config)
        throw new Error('preferences_not_loaded'); await memoryService.savePreferences(draft, config); setSaved(await memoryService.getPreferences(id)); }
    async function refreshOperations() { setOperations(await memoryService.operations(id)); }
    async function refreshDeletionPreferences() {
        deletion.preferences = undefined;
        const current = await memoryService.getPreferences(id);
        setSaved(current);
        deletion.preferences = current;
        return current;
    }
    const deletionPreferences = deletion.preferences;
    const deletionReason = deletionPreferences ? undefined : t('Deletion confirmation unavailable. Refresh preferences before another attempt.');
    async function deleteMemory(target: string | undefined, confirmed: MemoryPreferencesView) {
        setOutput(await deletion.run(confirmed,
            () => memoryService.delete(id, target, connection!.revision, confirmed.deletion_epoch, caps, true),
            refreshDeletionPreferences));
    }
    return <MemoryForm title={bot ? t("{v0} Memory", { v0: String(bot.name) }) : t('Bot Memory')} task={task} dirty={dirty} guardDirty={dirty || !!text || !!query || !!documentID} onSave={save}>
    <Section title={t("Own-bot scope")} footer={t("The core derives this bot’s account namespace and routes service operations to its assigned Runner. Rename/model changes do not change the namespace. Namespaces are logical isolation, not server authorization. Pausing, Runner reassignment, consent and connection revisions are rechecked by core.")}>
      <Row title={t("Bot")} detail={bot?.name ?? t('Unavailable')}/><Row title={t("Assigned Runner")} detail={runner?.name ?? bot?.runner_id ?? t('Unavailable')}/>
      <Row title={t("Local MEMORY.md")} subtitle={t("Independent local editor; never uploads to the memory service.")} subtitleLines={3} chevron onPress={() => router.push({ pathname: '/chat-info/memory/local', params: { id } })}/>
      <Row title={t("Service state")} detail={health ? memoryText(health.status) : t('Not checked')}/>
      <Row title={t("Configuration eligibility")} detail={connection?.availability === 'supported' ? t('Eligible — readiness not checked by configuration') : t('Blocked / unavailable')} subtitle={activationReason && memoryText(activationReason)} subtitleLines={3}/>
      {health?.reason && <Row title={t("Readiness reason")} subtitle={memoryText(health.reason)} subtitleLines={3}/>}
      {connection?.backend === 'lance_db' && <Row title={t("LanceDB Cloud activation unavailable")} subtitle={memoryText(LANCE_CLOUD_UNAVAILABLE_REASON)} subtitleLines={6}/>}
      {health?.deletion_pending && <Row title={t("Deletion pending")} subtitle={t("Remote erasure is not verified. Backups and Lance history can retain data.")} subtitleLines={3}/>}
      <MemoryAction title={t("Check Service Readiness")} reason={activationReason ?? (dirty ? t('Save preferences first.') : undefined)} onPress={() => confirmMemory(t('Check this bot’s memory endpoint?'), t("Read-only readiness on {v0}. No retain, reflect, DDL, asset download or inference.", { v0: String(runner?.name ?? bot?.runner_id) }), () => void task.run(async () => { setHealth(await memoryService.health(id)); }))}/>
      <Row title={t("Account Connections")} chevron onPress={() => router.push('/settings/memory')}/>
    </Section>
    {draft && <>
      <Section title={t("Recall and connection")} footer={t("Automatic recall sends task text once per eligible normal turn. It injects bounded evidence as untrusted historical data. Outages leave local memory usable.")}>
        <Row title={t("Connection")} menu={{ title: t('Connection'), value: config?.connections.find(c => c.id === draft.connectionID)?.name ?? t('Off'), choices: [{ title: t('Off'), selected: !draft.connectionID, onPress: () => edit(() => { draft.connectionID = null; }) }, ...(config?.connections.filter(c => c.availability === 'supported').map(c => ({ title: c.name, selected: c.id === draft.connectionID, onPress: () => edit(() => { draft.connectionID = c.id; }) })) ?? [])] }}/>
        {config?.connections.filter(c => c.availability === 'blocked').map(c => <Row key={c.id} title={t("{v0} — activation blocked", { v0: String(c.name) })} subtitle={c.reason ? memoryText(c.reason) : t('Connection is blocked.')} subtitleLines={3}/>)}
        <ToggleRow title={t("Automatic recall")} value={draft.autoRecall} onValueChange={v => edit(() => { draft.autoRecall = v; })}/>
        {(['timeout_ms', 'max_bytes', 'max_results', 'max_context_chars'] as const).map(k => <FieldRow key={memoryText(k)} label={memoryText(k)} value={String(draft.recallBudget[k])} keyboardType="number-pad" onChangeText={v => edit(() => { draft.recallBudget = { ...draft.recallBudget, [k]: Number(v) }; })}/>)}
      </Section>
      <Section title={t("Future conversation capture")} footer={t("Off by default. Only future eligible completed visible user/assistant text is captured after admission and completion consent checks. No history backfill, attachments, hidden thinking, secrets or tool output. Scrubbing is best effort, not DLP.")}>
        <ToggleRow title={t("Capture conversation")} value={draft.captureConversation} onValueChange={v => edit(() => { draft.captureConversation = v; if (!v)
            draft.captureGroupText = false; })}/>
        <FieldRow label={t("Per-turn cap")} value={String(draft.maxCaptureDeliveriesPerTurn)} keyboardType="number-pad" onChangeText={v => edit(() => { draft.maxCaptureDeliveriesPerTurn = Number(v); })}/>
        <Row title={t("Unattended background capture")} subtitle={t("Disabled: an enforced downstream total-cost policy is required. Turn-completion delivery is separately capped at 0–4.")} subtitleLines={4}/>
      </Section>
      <Section title={t("Separate group-text consent")} footer={t("Group context never grants another bot’s bank access. This may send other participants’ visible conversation text to the configured memory endpoint.")}>
        <ToggleRow title={t("Capture group text")} value={draft.captureGroupText} onValueChange={v => edit(() => { draft.captureGroupText = v; })}/>
        <MemoryAction title={t("Approve Group Capture Choices")} reason={!draft.captureGroupText ? t('Enable group capture and conversation capture first.') : undefined} onPress={() => confirmMemory(t('Approve future group-text capture?'), t("Bot: {v0}\nConnection: {v1}\nCapture conversation: {v2}\nPer-turn cap: {v3}\n\nOther participants’ text may be processed outside relay encryption. No historical backfill.", { v0: String(bot?.name ?? id), v1: String(draft.connectionID), v2: String(draft.captureConversation), v3: String(draft.maxCaptureDeliveriesPerTurn) }), () => { draft.approveGroupCapture(); update(n => n + 1); })}/>
      </Section>
      <Section title={t("Plaintext approval")} footer={memoryText(plaintextDisclosure)}>
        <MemoryAction title={t("Approve Exact Recall/Capture Choices")} reason={!draft.connectionID ? t('Select a connection first.') : undefined} onPress={() => confirmMemory(t('Approve this bot’s remote plaintext choices?'), t("Bot: {v0}\nConnection: {v1}\nAutomatic recall: {v2}\nFuture capture: {v3}\nGroup text: {v4}\nPer-turn cap: {v5}\n\n{v6}", { v0: String(bot?.name ?? id), v1: String(draft.connectionID), v2: String(draft.autoRecall), v3: String(draft.captureConversation), v4: String(draft.captureGroupText), v5: String(draft.maxCaptureDeliveriesPerTurn), v6: String(memoryText(plaintextDisclosure)) }), () => { draft.approveRemotePlaintext(); update(n => n + 1); })}/>
      </Section>
    </>}
    <Section title={t("Explicit save / search")} footer={t("Every action requires its own approval. Only completed means stored. An uncertain delivery can have charged the service; retries are not exactly-once billing.")}>
      <FieldRow label={t("Text to save")} multiline value={text} onChangeText={v => { setText(v); setRequestID(Crypto.randomUUID()); }}/>
      <MemoryAction title={t("Save Text to Service")} reason={reason('retain') ?? (!text.trim() ? t('Enter text first.') : undefined)} onPress={() => confirmMemory(t('Send and retain this text?'), memoryText(plaintextDisclosure), () => void task.run(async () => { setOutput(await memoryService.retain(id, text, requestID, caps, true)); await refreshOperations(); }))}/>
      <FieldRow label={t("Query")} multiline value={query} onChangeText={setQuery}/>
      <MemoryAction title={t("Search Own-bot Memory")} reason={reason('recall') ?? (!query.trim() ? t('Enter a query first.') : undefined)} onPress={() => confirmMemory(t('Send this search query?'), memoryText(plaintextDisclosure), () => void task.run(async () => setOutput(await memoryService.recall(id, query, caps, true))))}/>
      <MemoryAction title={t("Native Hindsight Reflect")} reason={reason('reflect') ?? (!query.trim() ? t('Enter a query first.') : undefined)} onPress={() => confirmMemory(t('Run native reflection?'), memoryText(plaintextDisclosure) + t(' This is service-native reflect, not similarity search or Beans synthesis.'), () => void task.run(async () => setOutput(await memoryService.reflect(id, query, caps, true))))}/>
      <FieldRow label={t("Document ID")} value={documentID} onChangeText={setDocumentID} autoCapitalize="none"/>
      <MemoryAction title={t("Inspect Document / Provenance")} reason={reason('inspect') ?? (!documentID ? t('Enter a document ID.') : undefined)} onPress={() => confirmMemory(t('Inspect this document?'), t('Fetch own-bot document and provenance from the assigned Runner’s memory service.'), () => void task.run(async () => setOutput(await memoryService.inspect(id, documentID, caps))))}/>
      <MemoryAction title={t("Refresh Deletion Confirmation")} onPress={() => void task.run(refreshDeletionPreferences)}/>
      <MemoryAction title={t("Delete Document")} destructive reason={deletionReason ?? reason('delete_document') ?? (!documentID ? t('Enter the exact document ID.') : undefined)} onPress={() => confirmMemory(t('Delete this document?'), t("{v0}\nBot: {v1}\nConnection revision: {v2}\nEpoch: {v3}\nPending does not mean erased. Backups/history may remain.", { v0: String(documentID), v1: String(id), v2: String(connection?.revision.counter), v3: String(deletionPreferences?.deletion_epoch) }), () => void task.run(() => deleteMemory(documentID, deletionPreferences!)), true)}/>
      <MemoryAction title={t("Clear Entire Own-bot Bank")} destructive reason={deletionReason ?? reason('clear')} onPress={() => confirmMemory(t('Clear this bot’s entire bank?'), t("Bot: {v0}\nConnection: {v1}\nRevision: {v2}\nEpoch: {v3}\nQueued writes are fenced. Pending cleanup is not verified erasure; backups/history can remain.", { v0: String(id), v1: String(connection?.name), v2: String(connection?.revision.counter), v3: String(deletionPreferences?.deletion_epoch) }), () => void task.run(() => deleteMemory(undefined, deletionPreferences!)), true)}/>
    </Section>
    <Section title={t("Delivery operations")}>
      <MemoryAction title={t("Refresh Delivery Queue")} reason={activationReason ?? (dirty ? t('Save preferences first.') : undefined)} onPress={() => void task.run(refreshOperations)}/>
      {operations?.deletion?.pending && <Row title={t("Pending cleanup")} detail={t("Epoch {v0}", { v0: String(operations.deletion.deletion_epoch) })}/>}
    </Section>
      {operations?.operations.map(op => <Section key={op.id} title={op.document_id}>
        <Row title={op.state === 'completed' ? t('Stored') : op.state === 'delivery_unknown' ? t('Delivery unknown — not confirmed stored') : memoryText(op.state)} subtitle={[op.id, op.error_code].filter(Boolean).join(' · ')} subtitleLines={3}/>
        <MemoryAction title={t("Check Operation")} reason={reason('operation_status')} onPress={() => confirmMemory(t('Poll this operation?'), t('One explicit status call; no automatic polling or retries.'), () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'status', caps, true)); await refreshOperations(); }))}/>
        <MemoryAction title={t("Retry Delivery")} reason={reason('retain') ?? (!['queued', 'failed', 'delivery_unknown'].includes(op.state) ? t('This state is not retryable.') : op.state === 'delivery_unknown' && !caps.idempotent_retain ? t('Uncertain delivery has no verified idempotency; retry disabled.') : undefined)} onPress={() => confirmMemory(t('Retry this delivery?'), memoryText(plaintextDisclosure) + t(' Stable IDs do not guarantee exactly-once billing.'), () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'retry', caps, true)); await refreshOperations(); }))}/>
        <MemoryAction title={t("Cancel Operation")} reason={reason('cancel_operation')} onPress={() => confirmMemory(t('Cancel this operation?'), t('Cooperative cancellation does not guarantee no late writes or refunds.'), () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'cancel', caps, true)); await refreshOperations(); }))}/>
      </Section>)}
    <Section title={t("Backend-specific capabilities")} footer={t("Controls use negotiated capabilities, never backend-name parity. OpenViking sessions/resources/tasks are distinct. Vector stores do not offer native reflect or mental models.")}>
      {advanced.filter(feature => !activationReason && connection && advancedForms(connection.backend, feature, caps).length > 0).map(feature => <MemoryAction key={feature} title={memoryText(feature.replaceAll('_', ' '))} reason={reason(feature)} onPress={() => router.push({ pathname: '/chat-info/memory/advanced', params: { id, feature } })}/>)}
      <Row title={t("Runner Setup / Lance Transfer")} chevron onPress={() => router.push({ pathname: '/chat-info/memory/setup', params: { id } })}/>
    </Section>
    {output !== undefined && <MemoryOutput title={t("Result")} value={output}/>}
  </MemoryForm>;
}
