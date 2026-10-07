import { useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, connectionActivationReason, type BotPreview, type BotSetupAction, type MemoryConnectionView } from '../../../src/core/memoryService';
import { useBotMap } from '../../../src/core/store';
import { FieldRow, Row, Section, ToggleRow } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemoryOutput, useMemoryTask } from '../../../src/ui/memoryUI';
export default function BotMemorySetupScreen() {
    const { id } = useLocalSearchParams<{
        id: string;
    }>();
    const task = useMemoryTask();
    const bot = useBotMap().get(id);
    const [connection, setConnection] = useState<MemoryConnectionView>();
    const backend = connection?.backend;
    const activationReason = connectionActivationReason(connection);
    const [action, setAction] = useState<BotSetupAction>('pgvector.initialize');
    const [path, setPath] = useState('');
    const [create, setCreate] = useState(false);
    const [preview, setPreview] = useState<BotPreview>();
    const [output, setOutput] = useState<unknown>();
    const [dirty, setDirty] = useState(false);
    useEffect(() => { void task.run(async () => { const [c, p] = await Promise.all([memoryService.listConnections(), memoryService.getPreferences(id)]); const selected = c.connections.find(c => c.id === p.connection_id); setConnection(selected); setAction(selected?.backend === 'lance_db' ? 'lance.binding' : 'pgvector.initialize'); }); }, [id]);
    const actions: BotSetupAction[] = activationReason ? [] : backend === 'pgvector' ? ['pgvector.initialize'] : backend === 'lance_db' ? ['lance.binding', 'lance.export', 'lance.import'] : [];
    function change(fn: () => void) { fn(); setDirty(true); setPreview(undefined); }
    async function makePreview() { setPreview(await memoryService.botPreview(action, id, action === 'lance.binding' ? { directory: path, create } : action === 'lance.export' || action === 'lance.import' ? { path } : undefined)); }
    return <MemoryForm title="Runner Memory Setup" task={task} dirty={dirty}>
    <Section title="Assigned Runner" footer="These are trusted UI-only operations routed to the bot’s assigned Runner. No phone vector database or native runtime. Nothing applies until the exact worker-produced preview is displayed and approved. Tokens expire and are single-use even when apply fails.">
      <Row title="Bot" detail={bot?.name ?? id}/><Row title="Runner" detail={bot?.runner_id ?? 'Unavailable'}/><Row title="Backend" detail={backend ?? 'Not configured'}/>
      {backend === 'lance_db' && <Row title="LanceDB Cloud activation unavailable" subtitle={LANCE_CLOUD_UNAVAILABLE_REASON} subtitleLines={6}/>}
      {actions.length ? <MemoryChoice title="Setup action" value={action} values={actions} onChange={v => change(() => setAction(v))}/> : <Row title="Setup unavailable" subtitle={activationReason ?? 'Choose a pgvector or LanceDB connection for this bot. The Runner reports build/runtime/schema availability through actual RPC errors, never simulated readiness.'} subtitleLines={5}/>}
    </Section>
    {actions.includes(action) && action === 'pgvector.initialize' && <Section footer="Preview inspects the actual connected target and returns the complete non-destructive SQL, extension, tables, indexes and permissions. No DDL applies during preview. Apply accepts only the one-use token; no client SQL overrides."><Row title="Exact pgvector initialization" subtitle="Database/OID/role/address/port/version/session identity and schema SQL must match the preview." subtitleLines={4}/></Section>}
    {actions.includes(action) && action.startsWith('lance.') && <Section title="Runner-local LanceDB only" footer="These actions never activate Cloud or fall back from a failed Cloud connection. Configure the connection explicitly Runner-local first. Enter a path on the assigned Runner, not this phone. Local DB files do not sync through the account. Moving a bot requires explicit export/import or private transfer. Transfers are plaintext and limited to 8 MiB / 1000 documents; exact namespace, fingerprint and generation must validate. Cloud transfer is unsupported.">
      <FieldRow label={action === 'lance.binding' ? 'Directory' : 'Transfer file'} value={path} onChangeText={v => change(() => setPath(v))} autoCapitalize="none" autoCorrect={false}/>
      {action === 'lance.binding' && <ToggleRow title="Approve table creation in preview" value={create} onValueChange={v => change(() => setCreate(v))}/>}
    </Section>}
    <Section><MemoryAction title="Generate Exact Preview" reason={activationReason ?? (!actions.includes(action) ? 'No supported setup action for this connection.' : action.startsWith('lance.') && !path ? 'Enter the Runner-local absolute path.' : undefined)} onPress={() => confirmMemory('Inspect exact setup target?', `Bot: ${id}\nAssigned Runner: ${bot?.runner_id}\nAction: ${action}\nNo DDL/import/export is applied by preview. Lance previews freeze bounded plaintext transfer bytes on the Runner.`, () => void task.run(makePreview))}/></Section>
    {!activationReason && preview && <>
      <MemoryOutput title="Exact target / schema / SQL / transfer" value={{ action: preview.action, runner_id: preview.runner_id, bot_id: preview.bot_id, connection_revision: preview.connection_revision, expires_at: preview.expires_at, details: preview.details }}/>
      <Section><MemoryAction title="Apply This Exact Preview" onPress={() => confirmMemory('Apply the exact displayed preview?', `Bot: ${preview.bot_id}\nRunner: ${preview.runner_id}\nAction: ${preview.action}\nApprove only the displayed target/schema/SQL or source/target/namespace/fingerprint. No override, fallback or overwrite is permitted. Transfer files contain plaintext and must be handled privately.`, () => void task.run(async () => { setPreview(undefined); setOutput(await memoryService.applyBotPreview(preview, true)); setDirty(false); }))}/></Section>
    </>}
    {output !== undefined && <MemoryOutput title="Setup result" value={output}/>}
  </MemoryForm>;
}
