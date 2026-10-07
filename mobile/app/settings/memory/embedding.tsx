import { useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import * as Crypto from 'expo-crypto';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { secretPatch, type EmbeddingProfileEdit, type LocalModelConfig, type MemoryEmbeddingView, type SecretPatch } from '../../../src/core/memoryService';
import { FieldRow, Row, Section, ToggleRow } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemorySecret, useMemoryTask } from '../../../src/ui/memoryUI';
const localStart: LocalModelConfig = { model_sha256: '', tokenizer_sha256: '', max_tokens: 512, pooling: 'mean', tensors: { input_ids: 'input_ids', attention_mask: 'attention_mask', output: 'last_hidden_state', token_type_ids: null }, add_special_tokens: true, pad_id: 0, pad_type_id: 0, pad_token: '[PAD]' };
export default function EmbeddingScreen() {
    const params = useLocalSearchParams<{
        id?: string;
    }>();
    const [id] = useState(params.id || Crypto.randomUUID());
    const task = useMemoryTask();
    const [saved, setSaved] = useState<MemoryEmbeddingView>();
    const [dirty, setDirty] = useState(!params.id);
    const [profile, setProfile] = useState<EmbeddingProfileEdit>({ model: '', revision: '', dimensions: 0, normalization: 'none', distance: 'cosine', document_prefix: '', query_prefix: '', endpoint: '', mode: 'api', local: localStart });
    const [action, setAction] = useState<SecretPatch['action']>('keep');
    const [key, setKey] = useState('');
    useEffect(() => { if (params.id)
        void task.run(async () => { const v = await memoryService.listConnections(); const e = v.embeddings.find(e => e.id === id); if (!e)
            throw new Error('profile_not_found'); setSaved(e); setProfile(p => ({ ...p, model: e.model, revision: e.model_revision, dimensions: e.dimensions })); }); }, []);
    const field = <K extends keyof EmbeddingProfileEdit>(k: K, v: EmbeddingProfileEdit[K]) => { setProfile(p => ({ ...p, [k]: v })); setDirty(true); };
    const local = <K extends keyof LocalModelConfig>(k: K, v: LocalModelConfig[K]) => field('local', { ...(profile.local ?? localStart), [k]: v });
    const l = profile.local ?? localStart;
    async function save() { if (params.id && !saved)
        throw new Error('profile_not_loaded'); if (saved?.has_secret && profile.mode === 'api' && action === 'keep')
        throw new Error('complete_profile_replacement_requires_replace_or_clear_key'); await memoryService.setEmbedding(id, profile, secretPatch(action, key)); }
    return <MemoryForm title={params.id ? 'Replace Embedding Profile' : 'Embedding Profile'} task={task} dirty={dirty} onSave={save}>
    <Section title="Exact vector space" footer="Only masked model/revision/dimensions are read back. Enter the complete intended profile. Model/revision, source hashes and all preprocessing participate in the fingerprint. Existing vector indexes refuse mismatches; saving is not reindexing and does not call inference.">
      <FieldRow label="Model" value={profile.model} onChangeText={v => field('model', v)} autoCapitalize="none"/>
      <FieldRow label="Revision" value={profile.revision} onChangeText={v => field('revision', v)} autoCapitalize="none"/>
      <FieldRow label="Dimensions" value={profile.dimensions ? String(profile.dimensions) : ''} keyboardType="number-pad" onChangeText={v => field('dimensions', Number(v))}/>
      <MemoryChoice title="Mode" value={profile.mode} values={['api', 'local_cpu']} onChange={v => field('mode', v)}/>
      <MemoryChoice title="Normalization" value={profile.normalization} values={['none', 'l2']} onChange={v => field('normalization', v)}/>
      <MemoryChoice title="Distance" value={profile.distance} values={['cosine', 'dot', 'euclidean']} onChange={v => field('distance', v)}/>
      <FieldRow label="Doc prefix" value={profile.document_prefix} onChangeText={v => field('document_prefix', v)} autoCapitalize="none"/>
      <FieldRow label="Query prefix" value={profile.query_prefix} onChangeText={v => field('query_prefix', v)} autoCapitalize="none"/>
      {profile.mode === 'api' && <FieldRow label="Exact URL" value={profile.endpoint ?? ''} onChangeText={v => field('endpoint', v)} placeholder="https://host/embeddings" autoCapitalize="none" autoCorrect={false}/>}
    </Section>
    {profile.mode === 'api' && <MemorySecret action={action} value={key} hasSecret={saved?.has_secret ?? false} onAction={v => { setAction(v); setDirty(true); }} onValue={v => { setKey(v); setDirty(true); }}/>}
    {profile.mode === 'local_cpu' && <>
      <Section title="Portable local model contract" footer="No paths belong in this profile. Install the exact runtime/model/tokenizer on a selected Runner through a separate preview and approval. CPU only; no phone runtime or automatic download/fallback.">
        <FieldRow label="Model SHA" value={l.model_sha256} onChangeText={v => local('model_sha256', v)} autoCapitalize="none" autoCorrect={false}/>
        <FieldRow label="Tokenizer SHA" value={l.tokenizer_sha256} onChangeText={v => local('tokenizer_sha256', v)} autoCapitalize="none" autoCorrect={false}/>
        <FieldRow label="Max tokens" value={String(l.max_tokens)} keyboardType="number-pad" onChangeText={v => local('max_tokens', Number(v))}/>
        <MemoryChoice title="Pooling" value={l.pooling} values={['mean', 'cls', 'pooled']} onChange={v => local('pooling', v)}/>
        <ToggleRow title="Add special tokens" value={l.add_special_tokens} onValueChange={v => local('add_special_tokens', v)}/>
        <FieldRow label="Pad ID" value={String(l.pad_id)} keyboardType="number-pad" onChangeText={v => local('pad_id', Number(v))}/>
        <FieldRow label="Pad type ID" value={String(l.pad_type_id)} keyboardType="number-pad" onChangeText={v => local('pad_type_id', Number(v))}/>
        <FieldRow label="Pad token" value={l.pad_token} onChangeText={v => local('pad_token', v)}/>
      </Section>
      <Section title="Tensor names">{(['input_ids', 'attention_mask', 'output', 'token_type_ids'] as const).map(k => <FieldRow key={k} label={k} value={l.tensors[k] ?? ''} onChangeText={v => local('tensors', { ...l.tensors, [k]: k === 'token_type_ids' ? (v || null) : v })} autoCapitalize="none" autoCorrect={false}/>)}</Section>
    </>}
    {saved && <Section footer="Removing a profile does not migrate or erase indexes."><MemoryAction title="Remove Profile" destructive onPress={() => confirmMemory('Remove profile?', saved.model, () => void task.run(async () => { await memoryService.removeEmbedding(id); setDirty(false); setSaved(undefined); }), true)}/></Section>}
  </MemoryForm>;
}
