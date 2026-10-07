import { expect, test } from 'bun:test';
import { advancedBody, advancedForms } from './memoryAdvanced';
import { MemoryPreferencesDraft, MemoryServiceAPI, embeddingRequest, readMemoryHealth, supportsMemoryAction, connectionRequest, validateAssetPlan, type AssetPlan, type MemoryCapabilities, type MemoryAdvancedFeature } from './memoryService';
const revision = { counter: 1, device_id: 'fixture' };
const plan: AssetPlan = { assets: ['runtime', 'model', 'tokenizer'].map(kind => ({ kind: kind as 'runtime' | 'model' | 'tokenizer', source: { kind: 'supplied', path: '/fixture/' + kind }, license: 'fixture', bytes: 1, sha256: 'a'.repeat(64) })) };
const formCapabilities: MemoryCapabilities = {
    advanced: ['directives', 'mental_models', 'tasks'],
    advanced_actions: { directives: ['create'], mental_models: ['create'], tasks: ['get'] },
};
test('advanced form drops hostile scope/secret fields rather than passing arbitrary JSON', () => {
    const form = advancedForms('hindsight', 'directives', formCapabilities).find(f => f.action === 'create')!;
    const body = advancedBody('directives', form, { name: 'Policy', content: 'Be concise', priority: '2', is_active: 'false', tags: 'one, two', namespace: 'sibling', api_key: 'secret' });
    expect(body).toEqual({ name: 'Policy', content: 'Be concise', priority: 2, is_active: false, tags: ['one', 'two'] });
});
test('invalid typed advanced values cannot silently become policy changes', () => {
    const form = advancedForms('hindsight', 'directives', formCapabilities).find(f => f.action === 'create')!;
    expect(() => advancedBody('directives', form, { name: 'x', content: 'y', priority: 'Infinity' })).toThrow('invalid_priority');
    expect(() => advancedBody('directives', form, { name: 'x', content: 'y', is_active: 'yes' })).toThrow('invalid_is_active');
});
test('provider capabilities remain distinct and no automatic schedules are selectable', () => {
    expect(advancedForms('open_viking', 'mental_models', formCapabilities)).toEqual([]);
    expect(advancedForms('pgvector', 'mental_models', formCapabilities)).toEqual([]);
    expect(advancedForms('open_viking', 'tasks', formCapabilities).some(f => f.action === 'cancel')).toBe(false);
    const form = advancedForms('hindsight', 'mental_models', formCapabilities).find(f => f.action === 'create')!;
    const body = advancedBody('mental_models', form, { name: 'Facts', source_query: 'Recent facts', trigger: 'automatic' });
    expect(body).not.toHaveProperty('trigger');
});
test('missing or empty exact action maps hide every advanced control and never dispatch', async () => {
    const features: MemoryAdvancedFeature[] = ['bank_profile', 'bank_config', 'directives', 'mental_models', 'mental_model_history', 'observations', 'memory_edit', 'memory_invalidate', 'memory_restore', 'documents', 'sessions', 'resources', 'tasks'];
    let invocations = 0;
    const api = new MemoryServiceAPI(async () => { invocations++; throw new Error('must_not_dispatch'); });
    for (const actions of [undefined, {}, Object.fromEntries(features.map(feature => [feature, []]))]) {
        const capabilities = readMemoryHealth({ status: 'ready', deletion_pending: false, capabilities: { advanced: features, ...(actions === undefined ? {} : { advanced_actions: actions }) } }).capabilities;
        for (const feature of features) {
            for (const backend of ['hindsight', 'open_viking', 'pgvector', 'lance_db'] as const)
                expect(advancedForms(backend, feature, capabilities)).toEqual([]);
            expect(supportsMemoryAction(capabilities, feature)).toBe(false);
            await expect(api.advanced('bot', feature, 'get', {}, capabilities, true)).rejects.toThrow('unsupported_operation');
        }
    }
    expect(invocations).toBe(0);
});
test('one negotiated verb never authorizes sibling verbs or another feature', async () => {
    const capabilities = readMemoryHealth({ status: 'ready', deletion_pending: false, capabilities: { advanced: ['bank_config', 'directives', 'tasks'], advanced_actions: { bank_config: ['get'], directives: ['create'], tasks: [] } } }).capabilities;
    expect(advancedForms('hindsight', 'bank_config', capabilities).map(form => form.action)).toEqual(['get']);
    expect(advancedForms('hindsight', 'directives', capabilities).map(form => form.action)).toEqual(['create']);
    expect(advancedForms('open_viking', 'tasks', capabilities)).toEqual([]);
    const calls: string[] = [];
    const api = new MemoryServiceAPI(async (method) => { calls.push(method); return { evidence: [], document: null, operation: null, data: {} }; });
    for (const action of ['update', 'reset'])
        await expect(api.advanced('bot', 'bank_config', action, {}, capabilities, true)).rejects.toThrow('unsupported_operation');
    await expect(api.advanced('bot', 'bank_config', 'get', {}, capabilities, false)).rejects.toThrow('plaintext_consent_required');
    expect(calls).toEqual([]);
    await api.advanced('bot', 'bank_config', 'get', {}, capabilities, true);
    expect(calls).toEqual(['memory.service.advanced']);
});
test('document delete cannot bypass confirmation and fencing through advanced permissions', async () => {
    const capabilities: MemoryCapabilities = { advanced: ['documents'], advanced_actions: { documents: ['list', 'delete'] }, delete_document: true };
    expect(advancedForms('hindsight', 'documents', capabilities).map(form => form.action)).toEqual(['list']);
    let invocations = 0;
    const api = new MemoryServiceAPI(async () => { invocations++; return {}; });
    await expect(api.advanced('bot', 'documents', 'delete', { id: 'doc' }, capabilities, true)).rejects.toThrow('unsupported_operation');
    await expect(api.delete('bot', 'doc', revision, 0, capabilities, false)).rejects.toThrow('confirmation_required');
    expect(invocations).toBe(0);
});
test('per-bot patches encode explicit removal without enabling whole-map replacement', () => {
    const request = connectionRequest({ id: 'ov', backend: 'open_viking', name: 'OV', secret: { action: 'keep' }, options: {
        backend: 'open_viking', bindings: { botA: null }, replace_all: true, confirm_replace_all: true,
    } as never });
    expect(request.params.options).toEqual({ backend: 'open_viking', bindings: { botA: null } });
});
test('raising capture cap requires renewed exact plaintext approval', () => {
    const draft = new MemoryPreferencesDraft('bot');
    draft.connectionID = 'c';
    draft.captureConversation = true;
    draft.approveRemotePlaintext();
    expect(draft.request().params.max_capture_deliveries_per_turn).toBe(1);
    draft.maxCaptureDeliveriesPerTurn = 2;
    expect(() => draft.request()).toThrow('plaintext_consent_required');
    draft.approveRemotePlaintext();
    expect(draft.request().params.max_capture_deliveries_per_turn).toBe(2);
    draft.recallBudget.max_results = 21;
    expect(() => draft.request()).toThrow('invalid_budget');
});
test('asset contract rejects phone-relative path, wrong hashes and incomplete sets', () => {
    expect(() => validateAssetPlan({ assets: plan.assets.slice(0, 2) })).toThrow('invalid_asset_plan');
    expect(() => validateAssetPlan({ assets: plan.assets.map((a, i) => i ? a : { ...a, source: { kind: 'supplied', path: 'phone-file' } }) })).toThrow('runner_absolute_path_required');
    expect(() => validateAssetPlan({ assets: plan.assets.map((a, i) => i ? a : { ...a, sha256: 'bad' }) })).toThrow('invalid_asset_plan');
});
test('modified selected Runner approval fails before any apply dispatch', async () => {
    let applies = 0;
    const api = new MemoryServiceAPI(async (method) => { if (method.endsWith('apply')) {
        applies++;
        return {};
    } return { preview_token: 't', preview_digest: 'b'.repeat(64), expires_in_seconds: 600, runner_id: 'runner', profile_id: 'p', profile_revision: revision, total_bytes: 3, assets: plan.assets }; });
    const preview = await api.localPreview('runner', 'p', revision, plan);
    preview.runner_id = 'another';
    await expect(api.applyLocalPreview(preview, true)).rejects.toThrow('approval_stale');
    expect(applies).toBe(0);
});
test('cancelled approval remains valid; expired and reused approvals cannot apply', async () => {
    let applies = 0;
    const api = new MemoryServiceAPI(async (method) => { if (method.endsWith('apply')) {
        applies++;
        return { initialized: true };
    } return { token: 't', expires_at: 0, action: method, runner_id: 'runner', bot_id: 'bot', connection_revision: revision, details: { target: { database: 'fixture', database_oid: 1, role: 'fixture', server_address: null, server_port: null, server_version: '17', session_pid: 1 }, schema: 'beans', sql: 'fixture' } }; });
    const preview = await api.botPreview('pgvector.initialize', 'bot');
    await expect(api.applyBotPreview(preview, false)).rejects.toThrow('confirmation_required');
    await expect(api.applyBotPreview(preview, true)).rejects.toThrow('approval_expired');
    await expect(api.applyBotPreview(preview, true)).rejects.toThrow('approval_not_found');
    expect(applies).toBe(0);
});
test('local editor stale hash failure never writes over newer Runner content', async () => {
    const api = new MemoryServiceAPI(async (method, params) => {
        if (method === 'memory.read')
            return { index: { text: 'newer', hash: 'new-hash', lines: 1, bytes: 5, truncated: false, max_lines: 200, max_bytes: 32768 }, path: '/private' };
        if (!params || typeof params !== 'object' || !('expected_hash' in params) || params.expected_hash !== 'new-hash')
            throw new Error('memory_conflict');
        return { hash: 'saved' };
    });
    expect(await api.readLocal('bot')).not.toHaveProperty('path');
    await expect(api.saveLocal('bot', 'draft', 'old-hash')).rejects.toThrow('memory_conflict');
});
test('embedding profile preserves exact prefixes and cannot include Runner paths/private fields', () => {
    const request = embeddingRequest('p', { model: 'm', revision: 'rev', dimensions: 3, normalization: 'l2', distance: 'cosine', document_prefix: ' doc ', query_prefix: ' query ', endpoint: null, mode: 'local_cpu', local: { model_sha256: 'a'.repeat(64), tokenizer_sha256: 'b'.repeat(64), max_tokens: 10, pooling: 'mean', add_special_tokens: true, pad_id: 0, pad_type_id: 0, pad_token: '[PAD]', tensors: { input_ids: 'ids', attention_mask: 'mask', output: 'out', token_type_ids: null, path: '/private' } as never } }, { action: 'keep' });
    expect(request.profile.document_prefix).toBe(' doc ');
    expect(JSON.stringify(request)).not.toContain('/private');
});
test('query limit counts Unicode characters while retain remains byte bounded', async () => {
    let requests = 0;
    const api = new MemoryServiceAPI(async () => {
        requests++;
        return { evidence: [], document: null, operation: null, data: {} };
    });
    await api.recall('bot', '🐈'.repeat(4096), { recall: true }, true);
    await expect(api.recall('bot', '🐈'.repeat(4097), { recall: true }, true)).rejects.toThrow('invalid_memory_query');
    await expect(api.retain('bot', '🐈'.repeat(8193), 'request', { retain: true }, true)).rejects.toThrow('invalid_memory_text');
    expect(requests).toBe(1);
});
test('inspection retains trusted source metadata and rejects impossible speaker values', async () => {
    const document = { id: 'doc', text: 'Historical note', request_id: 'request', content_hash: 'a'.repeat(64), sources: [{ chat_id: 'chat', message_id: 'message', speaker: 'assistant' }] };
    const api = new MemoryServiceAPI(async () => ({ evidence: [], document, operation: null, data: {} }));
    expect((await api.inspect('bot', 'doc', { inspect: true })).document?.sources).toEqual(document.sources);
    document.sources[0]!.speaker = 'system';
    await expect(api.inspect('bot', 'doc', { inspect: true })).rejects.toThrow('invalid_memory_reply');
});
