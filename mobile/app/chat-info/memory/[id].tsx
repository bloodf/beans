import { useLocalSearchParams, useRouter } from 'expo-router';
import { useEffect, useState } from 'react';
import * as Crypto from 'expo-crypto';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, connectionActivationReason, MemoryPreferencesDraft, supportsMemoryAction, type MemoryConnectionsView, type MemoryHealth, type MemoryOperations, type MemoryPreferencesView, type MemoryAdvancedFeature } from '../../../src/core/memoryService';
import { advancedForms } from '../../../src/core/memoryAdvanced';
import { useBotMap, useStore } from '../../../src/core/store';
import { FieldRow, Row, Section, ToggleRow } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryForm, MemoryOutput, plaintextDisclosure, useMemoryTask } from '../../../src/ui/memoryUI';
import { MemoryDeletionState } from '../../../src/ui/memoryTask';
const advanced: MemoryAdvancedFeature[] = ['bank_profile', 'bank_config', 'directives', 'mental_models', 'mental_model_history', 'observations', 'memory_edit', 'memory_invalidate', 'memory_restore', 'documents', 'sessions', 'resources', 'tasks'];
export default function BotMemoryScreen() {
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
    const reason = (action: Parameters<typeof supportsMemoryAction>[1]) => activationReason ?? (dirty ? 'Save or cancel preference changes first.' : !supportsMemoryAction(caps, action) ? 'Not negotiated by this Runner/backend. Check readiness explicitly.' : undefined);
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
    const deletionReason = deletionPreferences ? undefined : 'Deletion confirmation unavailable. Refresh preferences before another attempt.';
    async function deleteMemory(target: string | undefined, confirmed: MemoryPreferencesView) {
        setOutput(await deletion.run(confirmed,
            () => memoryService.delete(id, target, connection!.revision, confirmed.deletion_epoch, caps, true),
            refreshDeletionPreferences));
    }
    return <MemoryForm title={bot ? `${bot.name} Memory` : 'Bot Memory'} task={task} dirty={dirty} guardDirty={dirty || !!text || !!query || !!documentID} onSave={save}>
    <Section title="Own-bot scope" footer="The core derives this bot’s account namespace and routes service operations to its assigned Runner. Rename/model changes do not change the namespace. Namespaces are logical isolation, not server authorization. Pausing, Runner reassignment, consent and connection revisions are rechecked by core.">
      <Row title="Bot" detail={bot?.name ?? 'Unavailable'}/><Row title="Assigned Runner" detail={runner?.name ?? bot?.runner_id ?? 'Unavailable'}/>
      <Row title="Local MEMORY.md" subtitle="Independent local editor; never uploads to the memory service." subtitleLines={3} chevron onPress={() => router.push({ pathname: '/chat-info/memory/local', params: { id } })}/>
      <Row title="Service state" detail={health?.status ?? 'Not checked'}/>
      <Row title="Configuration eligibility" detail={connection?.availability === 'supported' ? 'Eligible — readiness not checked by configuration' : 'Blocked / unavailable'} subtitle={activationReason} subtitleLines={3}/>
      {health?.reason && <Row title="Readiness reason" subtitle={health.reason} subtitleLines={3}/>}
      {connection?.backend === 'lance_db' && <Row title="LanceDB Cloud activation unavailable" subtitle={LANCE_CLOUD_UNAVAILABLE_REASON} subtitleLines={6}/>}
      {health?.deletion_pending && <Row title="Deletion pending" subtitle="Remote erasure is not verified. Backups and Lance history can retain data." subtitleLines={3}/>}
      <MemoryAction title="Check Service Readiness" reason={activationReason ?? (dirty ? 'Save preferences first.' : undefined)} onPress={() => confirmMemory('Check this bot’s memory endpoint?', `Read-only readiness on ${runner?.name ?? bot?.runner_id}. No retain, reflect, DDL, asset download or inference.`, () => void task.run(async () => { setHealth(await memoryService.health(id)); }))}/>
      <Row title="Account Connections" chevron onPress={() => router.push('/settings/memory')}/>
    </Section>
    {draft && <>
      <Section title="Recall and connection" footer="Automatic recall sends task text once per eligible normal turn. It injects bounded evidence as untrusted historical data. Outages leave local memory usable.">
        <Row title="Connection" menu={{ title: 'Connection', value: config?.connections.find(c => c.id === draft.connectionID)?.name ?? 'Off', choices: [{ title: 'Off', selected: !draft.connectionID, onPress: () => edit(() => { draft.connectionID = null; }) }, ...(config?.connections.filter(c => c.availability === 'supported').map(c => ({ title: c.name, selected: c.id === draft.connectionID, onPress: () => edit(() => { draft.connectionID = c.id; }) })) ?? [])] }}/>
        {config?.connections.filter(c => c.availability === 'blocked').map(c => <Row key={c.id} title={`${c.name} — activation blocked`} subtitle={c.reason ?? 'Connection is blocked.'} subtitleLines={3}/>)}
        <ToggleRow title="Automatic recall" value={draft.autoRecall} onValueChange={v => edit(() => { draft.autoRecall = v; })}/>
        {(['timeout_ms', 'max_bytes', 'max_results', 'max_context_chars'] as const).map(k => <FieldRow key={k} label={k} value={String(draft.recallBudget[k])} keyboardType="number-pad" onChangeText={v => edit(() => { draft.recallBudget = { ...draft.recallBudget, [k]: Number(v) }; })}/>)}
      </Section>
      <Section title="Future conversation capture" footer="Off by default. Only future eligible completed visible user/assistant text is captured after admission and completion consent checks. No history backfill, attachments, hidden thinking, secrets or tool output. Scrubbing is best effort, not DLP.">
        <ToggleRow title="Capture conversation" value={draft.captureConversation} onValueChange={v => edit(() => { draft.captureConversation = v; if (!v)
            draft.captureGroupText = false; })}/>
        <FieldRow label="Per-turn cap" value={String(draft.maxCaptureDeliveriesPerTurn)} keyboardType="number-pad" onChangeText={v => edit(() => { draft.maxCaptureDeliveriesPerTurn = Number(v); })}/>
        <Row title="Unattended background capture" subtitle="Disabled: an enforced downstream total-cost policy is required. Turn-completion delivery is separately capped at 0–4." subtitleLines={4}/>
      </Section>
      <Section title="Separate group-text consent" footer="Group context never grants another bot’s bank access. This may send other participants’ visible conversation text to the configured memory endpoint.">
        <ToggleRow title="Capture group text" value={draft.captureGroupText} onValueChange={v => edit(() => { draft.captureGroupText = v; })}/>
        <MemoryAction title="Approve Group Capture Choices" reason={!draft.captureGroupText ? 'Enable group capture and conversation capture first.' : undefined} onPress={() => confirmMemory('Approve future group-text capture?', `Bot: ${bot?.name ?? id}\nConnection: ${draft.connectionID}\nCapture conversation: ${draft.captureConversation}\nPer-turn cap: ${draft.maxCaptureDeliveriesPerTurn}\n\nOther participants’ text may be processed outside relay encryption. No historical backfill.`, () => { draft.approveGroupCapture(); update(n => n + 1); })}/>
      </Section>
      <Section title="Plaintext approval" footer={plaintextDisclosure}>
        <MemoryAction title="Approve Exact Recall/Capture Choices" reason={!draft.connectionID ? 'Select a connection first.' : undefined} onPress={() => confirmMemory('Approve this bot’s remote plaintext choices?', `Bot: ${bot?.name ?? id}\nConnection: ${draft.connectionID}\nAutomatic recall: ${draft.autoRecall}\nFuture capture: ${draft.captureConversation}\nGroup text: ${draft.captureGroupText}\nPer-turn cap: ${draft.maxCaptureDeliveriesPerTurn}\n\n${plaintextDisclosure}`, () => { draft.approveRemotePlaintext(); update(n => n + 1); })}/>
      </Section>
    </>}
    <Section title="Explicit save / search" footer="Every action requires its own approval. Only completed means stored. An uncertain delivery can have charged the service; retries are not exactly-once billing.">
      <FieldRow label="Text to save" multiline value={text} onChangeText={v => { setText(v); setRequestID(Crypto.randomUUID()); }}/>
      <MemoryAction title="Save Text to Service" reason={reason('retain') ?? (!text.trim() ? 'Enter text first.' : undefined)} onPress={() => confirmMemory('Send and retain this text?', plaintextDisclosure, () => void task.run(async () => { setOutput(await memoryService.retain(id, text, requestID, caps, true)); await refreshOperations(); }))}/>
      <FieldRow label="Query" multiline value={query} onChangeText={setQuery}/>
      <MemoryAction title="Search Own-bot Memory" reason={reason('recall') ?? (!query.trim() ? 'Enter a query first.' : undefined)} onPress={() => confirmMemory('Send this search query?', plaintextDisclosure, () => void task.run(async () => setOutput(await memoryService.recall(id, query, caps, true))))}/>
      <MemoryAction title="Native Hindsight Reflect" reason={reason('reflect') ?? (!query.trim() ? 'Enter a query first.' : undefined)} onPress={() => confirmMemory('Run native reflection?', plaintextDisclosure + ' This is service-native reflect, not similarity search or Beans synthesis.', () => void task.run(async () => setOutput(await memoryService.reflect(id, query, caps, true))))}/>
      <FieldRow label="Document ID" value={documentID} onChangeText={setDocumentID} autoCapitalize="none"/>
      <MemoryAction title="Inspect Document / Provenance" reason={reason('inspect') ?? (!documentID ? 'Enter a document ID.' : undefined)} onPress={() => confirmMemory('Inspect this document?', 'Fetch own-bot document and provenance from the assigned Runner’s memory service.', () => void task.run(async () => setOutput(await memoryService.inspect(id, documentID, caps))))}/>
      <MemoryAction title="Refresh Deletion Confirmation" onPress={() => void task.run(refreshDeletionPreferences)}/>
      <MemoryAction title="Delete Document" destructive reason={deletionReason ?? reason('delete_document') ?? (!documentID ? 'Enter the exact document ID.' : undefined)} onPress={() => confirmMemory('Delete this document?', `${documentID}\nBot: ${id}\nConnection revision: ${connection?.revision.counter}\nEpoch: ${deletionPreferences?.deletion_epoch}\nPending does not mean erased. Backups/history may remain.`, () => void task.run(() => deleteMemory(documentID, deletionPreferences!)), true)}/>
      <MemoryAction title="Clear Entire Own-bot Bank" destructive reason={deletionReason ?? reason('clear')} onPress={() => confirmMemory('Clear this bot’s entire bank?', `Bot: ${id}\nConnection: ${connection?.name}\nRevision: ${connection?.revision.counter}\nEpoch: ${deletionPreferences?.deletion_epoch}\nQueued writes are fenced. Pending cleanup is not verified erasure; backups/history can remain.`, () => void task.run(() => deleteMemory(undefined, deletionPreferences!)), true)}/>
    </Section>
    <Section title="Delivery operations">
      <MemoryAction title="Refresh Delivery Queue" reason={activationReason ?? (dirty ? 'Save preferences first.' : undefined)} onPress={() => void task.run(refreshOperations)}/>
      {operations?.deletion?.pending && <Row title="Pending cleanup" detail={`Epoch ${operations.deletion.deletion_epoch}`}/>}
    </Section>
      {operations?.operations.map(op => <Section key={op.id} title={op.document_id}>
        <Row title={op.state === 'completed' ? 'Stored' : op.state === 'delivery_unknown' ? 'Delivery unknown — not confirmed stored' : op.state} subtitle={[op.id, op.error_code].filter(Boolean).join(' · ')} subtitleLines={3}/>
        <MemoryAction title="Check Operation" reason={reason('operation_status')} onPress={() => confirmMemory('Poll this operation?', 'One explicit status call; no automatic polling or retries.', () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'status', caps, true)); await refreshOperations(); }))}/>
        <MemoryAction title="Retry Delivery" reason={reason('retain') ?? (!['queued', 'failed', 'delivery_unknown'].includes(op.state) ? 'This state is not retryable.' : op.state === 'delivery_unknown' && !caps.idempotent_retain ? 'Uncertain delivery has no verified idempotency; retry disabled.' : undefined)} onPress={() => confirmMemory('Retry this delivery?', plaintextDisclosure + ' Stable IDs do not guarantee exactly-once billing.', () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'retry', caps, true)); await refreshOperations(); }))}/>
        <MemoryAction title="Cancel Operation" reason={reason('cancel_operation')} onPress={() => confirmMemory('Cancel this operation?', 'Cooperative cancellation does not guarantee no late writes or refunds.', () => void task.run(async () => { setOutput(await memoryService.operation(id, op.id, 'cancel', caps, true)); await refreshOperations(); }))}/>
      </Section>)}
    <Section title="Backend-specific capabilities" footer="Controls use negotiated capabilities, never backend-name parity. OpenViking sessions/resources/tasks are distinct. Vector stores do not offer native reflect or mental models.">
      {advanced.filter(feature => !activationReason && connection && advancedForms(connection.backend, feature, caps).length > 0).map(feature => <MemoryAction key={feature} title={feature.replaceAll('_', ' ')} reason={reason(feature)} onPress={() => router.push({ pathname: '/chat-info/memory/advanced', params: { id, feature } })}/>)}
      <Row title="Runner Setup / Lance Transfer" chevron onPress={() => router.push({ pathname: '/chat-info/memory/setup', params: { id } })}/>
    </Section>
    {output !== undefined && <MemoryOutput title="Result" value={output}/>}
  </MemoryForm>;
}
