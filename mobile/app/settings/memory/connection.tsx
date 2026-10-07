import { useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import * as Crypto from 'expo-crypto';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, secretPatch, type MemoryBackend, type MemoryConnectionView, type MemoryConnectionOptions, type MemoryConnectionsView, type SecretPatch } from '../../../src/core/memoryService';
import { useStore } from '../../../src/core/store';
import { FieldRow, Row, Section, ToggleRow } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemorySecret, useMemoryTask } from '../../../src/ui/memoryUI';
type Binding = NonNullable<Extract<MemoryConnectionOptions, {
    backend: 'open_viking';
}>['bindings'][string]>;
export default function MemoryConnectionScreen() {
    const params = useLocalSearchParams<{
        id?: string;
    }>();
    const [id] = useState(params.id || Crypto.randomUUID());
    const task = useMemoryTask();
    const bots = useStore(s => s.bots);
    const [view, setView] = useState<MemoryConnectionsView>();
    const [saved, setSaved] = useState<MemoryConnectionView>();
    const [name, setName] = useState('');
    const [backend, setBackend] = useState<MemoryBackend>('hindsight');
    const [endpointAction, setEndpointAction] = useState<'keep' | 'replace' | 'clear'>(params.id ? 'keep' : 'replace');
    const [endpoint, setEndpoint] = useState('');
    const [secretAction, setSecretAction] = useState<SecretPatch['action']>('keep');
    const [key, setKey] = useState('');
    const [embedding, setEmbedding] = useState<string>('none');
    const [insecure, setInsecure] = useState<'keep' | 'allow' | 'deny'>(params.id ? 'keep' : 'deny');
    const [replaceOptions, setReplaceOptions] = useState(!params.id);
    const [schema, setSchema] = useState('');
    const [bindings, setBindings] = useState<Record<string, Binding | null>>({});
    const [dirty, setDirty] = useState(!params.id);
    const [loaded, setLoaded] = useState(!params.id);
    useEffect(() => { void task.run(async () => { const v = await memoryService.listConnections(); setView(v); const c = v.connections.find(c => c.id === id); if (params.id && !c)
        throw new Error('connection_not_found'); setSaved(c); if (c) {
        setName(c.name);
        setBackend(c.backend);
        setEmbedding(c.embedding_profile ?? 'none');
    } setLoaded(true); }); }, []);
    const edit = (action: () => void) => { action(); setDirty(true); };
    function useRunnerLocal() {
        edit(() => {
            setEndpointAction('clear');
            setEndpoint('');
            setSecretAction('clear');
            setKey('');
            setReplaceOptions(true);
        });
    }
    function bindingChange(botID: string, value: 'keep' | 'edit' | 'remove') {
        edit(() => setBindings(current => {
            const next = { ...current };
            if (value === 'keep')
                delete next[botID];
            else
                next[botID] = value === 'remove' ? null : { mode: 'user_key', account_id: '', user_id: '', secret: { action: 'replace', value: '' } };
            return next;
        }));
    }
    async function save() {
        if (!loaded)
            throw new Error('configuration_not_loaded');
        if (!name.trim())
            throw new Error('name_required');
        if (saved?.has_secret && endpointAction !== 'keep' && secretAction === 'keep')
            throw new Error('endpoint_change_requires_replace_or_clear_key');
        let options: MemoryConnectionOptions | undefined;
        if (backend === 'open_viking') {
            if (!saved || Object.keys(bindings).length > 0)
                options = { backend, bindings };
        }
        else if (replaceOptions) {
            switch (backend) {
                case 'hindsight':
                    options = { backend };
                    break;
                case 'pgvector':
                    options = { backend, schema };
                    break;
                case 'lance_db':
                    options = { backend, region: null };
                    break;
            }
        }
        await memoryService.setConnection({ id, backend, name: name.trim(), secret: secretPatch(secretAction, key),
            ...(endpointAction === 'keep' ? {} : { endpoint: endpointAction === 'clear' ? null : endpoint }),
            embedding_profile: embedding === 'none' ? null : embedding,
            ...(insecure === 'keep' ? {} : { allow_insecure_http: insecure === 'allow' }), options });
    }
    return <MemoryForm title={params.id ? 'Edit Memory Connection' : 'Add Memory Connection'} task={task} dirty={dirty} onSave={save}>
    <Section title="Connection" footer="Saved endpoints/options are private and are not read back. Keep preserves them. To change an endpoint with a stored credential, explicitly replace or clear its key in the same save.">
      <FieldRow label="Name" value={name} onChangeText={v => edit(() => setName(v))}/>
      {saved && <Row title="Configuration eligibility" detail={saved.availability === 'supported' ? 'Eligible — not initialized/ready' : 'Activation blocked'} subtitle={saved.reason ?? undefined} subtitleLines={3}/>}
      {saved ? <Row title="Backend" detail={backend}/> : <MemoryChoice title="Backend" value={backend} values={['hindsight', 'open_viking', 'pgvector', 'lance_db']} onChange={v => { edit(() => setBackend(v)); if (v === 'lance_db') useRunnerLocal(); }}/>}
      {backend === 'lance_db' ? <>
        <Row title="LanceDB Cloud activation unavailable" subtitle={LANCE_CLOUD_UNAVAILABLE_REASON} subtitleLines={6}/>
        <Row title="LanceDB storage target" menu={{ title: 'LanceDB storage target', value: endpointAction === 'clear' ? 'Runner-local' : 'Keep private target', choices: [
            ...(saved ? [{ title: 'Keep private target', selected: endpointAction === 'keep', onPress: () => edit(() => { setEndpointAction('keep'); setEndpoint(''); setSecretAction('keep'); setKey(''); setReplaceOptions(false); }) }] : []),
            { title: 'Runner-local (explicit)', selected: endpointAction === 'clear', onPress: () => saved
                ? confirmMemory('Configure Runner-local LanceDB?', 'This explicitly clears the private endpoint and connection credential; it is not a fallback after a Cloud failure. Remote data remains untouched. Save this choice, then explicitly configure the bot’s private database on its assigned Runner.', useRunnerLocal, true)
                : useRunnerLocal() },
        ] }}/>
      </> : <>
        <MemoryChoice title="Endpoint" value={endpointAction} values={['keep', 'replace', 'clear']} onChange={v => edit(() => setEndpointAction(v))}/>
        {endpointAction === 'replace' && <FieldRow label="Runner URL" value={endpoint} onChangeText={v => edit(() => setEndpoint(v))} autoCapitalize="none" autoCorrect={false} placeholder={backend === 'pgvector' ? 'postgresql://host/database' : 'https://…'}/>}
      </>}
      <Row title="Embedding profile" menu={{ title: 'Embedding profile', value: embedding, choices: [{ title: 'None', selected: embedding === 'none', onPress: () => edit(() => setEmbedding('none')) }, ...(view?.embeddings.map(e => ({ title: e.model, selected: embedding === e.id, onPress: () => edit(() => setEmbedding(e.id)) })) ?? [])] }}/>
      {backend !== 'lance_db' && <Row title="HTTP policy" menu={{ title: 'HTTP policy', value: insecure, choices: ['keep', 'allow', 'deny'].map(value => ({ title: value, selected: insecure === value, onPress: () => value === 'allow' ? confirmMemory('Allow unencrypted HTTP?', 'Plaintext and credentials may be exposed on the network. Approve only a trusted private endpoint.', () => edit(() => setInsecure('allow'))) : edit(() => setInsecure(value as 'keep' | 'deny')) })) }}/>}
    </Section>
    {backend !== 'lance_db' && <MemorySecret action={secretAction} value={key} hasSecret={saved?.has_secret ?? false} onAction={v => edit(() => { setSecretAction(v); setKey(''); })} onValue={v => edit(() => setKey(v))}/>}
    <Section title="Backend options" footer={backend === 'open_viking' ? 'Per-bot patches preserve every omitted private binding/key. Keep sends no edit; Remove sends an explicit null for that bot only. No whole-map replacement is offered. Stored identities and keys are never read back.' : 'Known options are an atomic replacement. Keep preserves private options.'}>
      {backend !== 'open_viking' && backend !== 'lance_db' && <ToggleRow title="Replace known options" value={replaceOptions} onValueChange={v => edit(() => setReplaceOptions(v))}/>}
      {replaceOptions && backend === 'pgvector' && <FieldRow label="Schema" value={schema} onChangeText={v => edit(() => setSchema(v))} autoCapitalize="none" autoCorrect={false}/>}
      {backend === 'open_viking' && <Row title="Per-bot authorization" subtitle="USER keys must match the expected account/user. A trusted gateway is explicitly configured; a shared USER key plus identity headers is not isolation." subtitleLines={5}/>}
    </Section>
    {backend === 'open_viking' && bots.map(bot => {
            const b = bindings[bot.id];
            const change = (next: Binding) => edit(() => setBindings(current => ({ ...current, [bot.id]: next })));
            return <Section key={bot.id} title={bot.name} footer="Keep key is valid only for the exact saved bot/mode/account/user identity. Identity changes discard entered keys and require a fresh replacement; core rechecks this. Remove disconnects only this binding, not its remote data.">
        <MemoryChoice title="Binding change" value={b === null ? 'remove' : b ? 'edit' : 'keep'} values={['keep', 'edit', 'remove']} onChange={v => v === 'remove'
            ? confirmMemory('Remove this bot binding?', `Bot: ${bot.name} (${bot.id})\nOnly this bot’s private binding/key is removed from the account connection. Other bot bindings remain unchanged. Remote data is not deleted.`, () => bindingChange(bot.id, 'remove'), true)
            : bindingChange(bot.id, v)}/>
        {b && <>
          <MemoryChoice title="Authorization" value={b.mode} values={['user_key', 'trusted_gateway']} onChange={mode => change({ ...b, mode, secret: { action: 'replace', value: '' } })}/>
          <FieldRow label="Account" value={b.account_id} onChangeText={account_id => change({ ...b, account_id, secret: { action: 'replace', value: '' } })} autoCapitalize="none"/>
          <FieldRow label="User" value={b.user_id} onChangeText={user_id => change({ ...b, user_id, secret: { action: 'replace', value: '' } })} autoCapitalize="none"/>
          <MemoryChoice title="Key action" value={b.secret.action} values={['keep', 'replace', 'clear']} onChange={action => change({ ...b, secret: action === 'replace' ? { action, value: '' } : { action } })}/>
          {b.secret.action === 'replace' && <FieldRow label="Bot key" secureTextEntry value={b.secret.value} onChangeText={value => change({ ...b, secret: { action: 'replace', value } })} autoCapitalize="none" autoCorrect={false}/>}
        </>}
      </Section>;
        })}
    {saved && <Section footer="Disconnect invalidates queued use but never deletes service data."><MemoryAction title="Disconnect Connection" destructive onPress={() => confirmMemory('Disconnect this connection?', saved.name + ' is removed from the encrypted account configuration. Remote data remains.', () => void task.run(async () => { await memoryService.disconnectConnection(id); setDirty(false); setLoaded(false); }), true)}/></Section>}
  </MemoryForm>;
}
