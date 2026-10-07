import { useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { connectionActivationReason, supportsMemoryAction, type MemoryConnectionView, type MemoryAdvancedFeature, type MemoryHealth } from '../../../src/core/memoryService';
import { advancedBody, advancedForms } from '../../../src/core/memoryAdvanced';
import { useBotMap } from '../../../src/core/store';
import { FieldRow, Row, Section } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemoryOutput, plaintextDisclosure, useMemoryTask } from '../../../src/ui/memoryUI';
export default function MemoryAdvancedScreen() {
    const params = useLocalSearchParams<{
        id: string;
        feature: MemoryAdvancedFeature;
    }>();
    const task = useMemoryTask();
    const bot = useBotMap().get(params.id);
    const [connection, setConnection] = useState<MemoryConnectionView>();
    const backend = connection?.backend;
    const activationReason = connectionActivationReason(connection);
    const [health, setHealth] = useState<MemoryHealth>();
    const [action, setAction] = useState('');
    const [values, setValues] = useState<Record<string, string>>({});
    const [output, setOutput] = useState<unknown>();
    useEffect(() => { void task.run(async () => { const [config, p] = await Promise.all([memoryService.listConnections(), memoryService.getPreferences(params.id)]); setConnection(config.connections.find(c => c.id === p.connection_id)); }); }, [params.id]);
    const forms = backend && !activationReason ? advancedForms(backend, params.feature, health?.capabilities) : [];
    const form = forms.find(f => f.action === action);
    const allowed = supportsMemoryAction(health?.capabilities, params.feature, action);
    const busyReason = activationReason ?? (!allowed ? 'Exact action not negotiated. Explicitly check readiness first.' : !form ? 'Choose an implemented action.' : form.spending && !health?.capabilities.operation_status ? 'Async operation status is not supported. Billed extraction/refresh disabled.' : undefined);
    return <MemoryForm title={params.feature.replaceAll('_', ' ')} task={task} dirty={Object.values(values).some(Boolean)}>
    <Section title={bot?.name ?? 'Own-bot service'} footer={plaintextDisclosure + ' No auto-refresh schedules, consolidation or routine opt-in is implied. Directive content is intentionally configured service policy; returned historical text is not instructions. Only exact negotiated action verbs are shown and rechecked before dispatch.'}>
      <Row title="Backend" detail={backend ?? 'Not loaded'}/>
      <MemoryAction title="Check Negotiated Capabilities" reason={activationReason} onPress={() => confirmMemory('Check readiness?', 'One explicit read-only request on the assigned Runner. No billed inference.', () => void task.run(async () => setHealth(await memoryService.health(params.id))))}/>
      {health?.reason && <Row title="Readiness reason" subtitle={health.reason} subtitleLines={3}/>}
      {!!forms.length && <MemoryChoice title="Action" value={action || 'Choose'} values={forms.map(f => f.action)} onChange={v => { setAction(v); setValues({}); }}/>}
      {!forms.length && <Row title="No negotiated actions" subtitle="Explicitly check readiness. Missing or empty action permissions expose no advanced controls." subtitleLines={3}/>}
    </Section>
    {form && <Section title="Action fields" footer="Only these exact adapter-defined fields are sent. IDs are document/directive/model/session/task IDs within the core-bound own-bot scope. Resource paths are relative under resources/, never a target URI. Comma-separated tags; booleans are true/false. Blank optional fields are omitted.">
      {form.fields.map(f => f.type === 'boolean' ? <MemoryChoice key={f.name} title={f.name} value={values[f.name] ?? ''} values={['', 'true', 'false']} onChange={v => setValues({ ...values, [f.name]: v })}/> : <FieldRow key={f.name} label={f.name} value={values[f.name] ?? ''} onChangeText={v => setValues({ ...values, [f.name]: v })} keyboardType={f.type === 'number' ? 'number-pad' : 'default'} multiline={['content', 'text', 'source_query', 'reflect_mission', 'retain_mission', 'retain_custom_instructions'].includes(f.name)} autoCapitalize="none" autoCorrect={false}/>)}
      <MemoryAction title={`Run ${form.action}`} reason={busyReason} destructive={form.destructive} onPress={() => confirmMemory(`Approve ${params.feature} / ${form.action}?`, `Bot: ${params.id}\nAssigned Runner: ${bot?.runner_id}\nFields: ${JSON.stringify(values)}\n\n${plaintextDisclosure}${form.destructive ? '\nDeletion/cancellation may leave remote history, backups or late writes.' : ''}`, () => void task.run(async () => setOutput(await memoryService.advanced(params.id, params.feature, form.action, advancedBody(params.feature, form, values), health!.capabilities, true))), form.destructive)}/>
    </Section>}
    {output !== undefined && <MemoryOutput title="Untrusted service result" value={output}/>}
    {params.feature === 'documents' && <Section footer="Use the main per-bot Memory page for document/bank deletion. Its exact revision/epoch confirmation fences queued writes."><Row title="Fenced deletion" subtitle="Direct document deletion is not sent through this advanced panel." subtitleLines={3}/></Section>}
  </MemoryForm>;
}
