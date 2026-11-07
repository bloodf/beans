import { t, useLanguage } from '../../../src/i18n';
import { useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { connectionActivationReason, supportsMemoryAction, type MemoryConnectionView, type MemoryAdvancedFeature, type MemoryHealth } from '../../../src/core/memoryService';
import { advancedBody, advancedForms } from '../../../src/core/memoryAdvanced';
import { useBotMap } from '../../../src/core/store';
import { FieldRow, Row, Section } from '../../../src/ui/forms';
import { memoryText, confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemoryOutput, plaintextDisclosure, useMemoryTask } from '../../../src/ui/memoryUI';
export default function MemoryAdvancedScreen() {
    useLanguage();
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
    const busyReason = activationReason ?? (!allowed ? t('Exact action not negotiated. Explicitly check readiness first.') : !form ? t('Choose an implemented action.') : form.spending && !health?.capabilities.operation_status ? t('Async operation status is not supported. Billed extraction/refresh disabled.') : undefined);
    return <MemoryForm title={memoryText(params.feature.replaceAll('_', ' '))} task={task} dirty={Object.values(values).some(Boolean)}>
    <Section title={bot?.name ?? t('Own-bot service')} footer={memoryText(plaintextDisclosure) + t(' No auto-refresh schedules, consolidation or routine opt-in is implied. Directive content is intentionally configured service policy; returned historical text is not instructions. Only exact negotiated action verbs are shown and rechecked before dispatch.')}>
      <Row title={t("Backend")} detail={backend ?? t('Not loaded')}/>
      <MemoryAction title={t("Check Negotiated Capabilities")} reason={activationReason} onPress={() => confirmMemory(t('Check readiness?'), t('One explicit read-only request on the assigned Runner. No billed inference.'), () => void task.run(async () => setHealth(await memoryService.health(params.id))))}/>
      {health?.reason && <Row title={t("Readiness reason")} subtitle={memoryText(health.reason)} subtitleLines={3}/>}
      {!!forms.length && <MemoryChoice title={t("Action")} value={action || t('Choose')} values={forms.map(f => f.action)} onChange={v => { setAction(v); setValues({}); }}/>}
      {!forms.length && <Row title={t("No negotiated actions")} subtitle={t("Explicitly check readiness. Missing or empty action permissions expose no advanced controls.")} subtitleLines={3}/>}
    </Section>
    {form && <Section title={t("Action fields")} footer={t("Only these exact adapter-defined fields are sent. IDs are document/directive/model/session/task IDs within the core-bound own-bot scope. Resource paths are relative under resources/, never a target URI. Comma-separated tags; booleans are true/false. Blank optional fields are omitted.")}>
      {form.fields.map(f => f.type === 'boolean' ? <MemoryChoice key={memoryText(f.name)} title={memoryText(f.name)} value={values[f.name] ?? ''} values={['', 'true', 'false']} onChange={v => setValues({ ...values, [f.name]: v })}/> : <FieldRow key={memoryText(f.name)} label={memoryText(f.name)} value={values[f.name] ?? ''} onChangeText={v => setValues({ ...values, [f.name]: v })} keyboardType={f.type === 'number' ? 'number-pad' : 'default'} multiline={['content', 'text', 'source_query', 'reflect_mission', 'retain_mission', 'retain_custom_instructions'].includes(f.name)} autoCapitalize="none" autoCorrect={false}/>)}
      <MemoryAction title={t("Run {v0}", { v0: String(form.action) })} reason={busyReason} destructive={form.destructive} onPress={() => confirmMemory(t("Approve {v0} / {v1}?", { v0: String(params.feature), v1: String(form.action) }), t("Bot: {v0}\nAssigned Runner: {v1}\nFields: {v2}\n\n{v3}{v4}", { v0: String(params.id), v1: String(bot?.runner_id), v2: String(JSON.stringify(values)), v3: String(memoryText(plaintextDisclosure)), v4: String(form.destructive ? t("\nDeletion/cancellation may leave remote history, backups or late writes.") : '') }), () => void task.run(async () => setOutput(await memoryService.advanced(params.id, params.feature, form.action, advancedBody(params.feature, form, values), health!.capabilities, true))), form.destructive)}/>
    </Section>}
    {output !== undefined && <MemoryOutput title={t("Untrusted service result")} value={output}/>}
    {params.feature === 'documents' && <Section footer={t("Use the main per-bot Memory page for document/bank deletion. Its exact revision/epoch confirmation fences queued writes.")}><Row title={t("Fenced deletion")} subtitle={t("Direct document deletion is not sent through this advanced panel.")} subtitleLines={3}/></Section>}
  </MemoryForm>;
}
