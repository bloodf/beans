import { describe, expect, test } from 'bun:test';
import { MemoryServiceAPI, MemoryPreferencesDraft, readMemoryConnections, readMemoryHealth, connectionActivationReason, connectionRequest, secretPatch, type AssetPlan, type MemoryCapabilities, type MemoryConnectionEdit } from './memoryService';
import { MemoryTask } from '../ui/memoryTask';
const revision = { counter: 1, device_id: 'phone' };
const caps: MemoryCapabilities = { retain: true, recall: true, reflect: true, advanced: ['directives', 'mental_models'] };
describe('phone memory consent', () => {
    test('connection changes invalidate prior plaintext and group approval', () => {
        const draft = new MemoryPreferencesDraft('bot');
        draft.connectionID = 'one';
        draft.captureConversation = true;
        draft.captureGroupText = true;
        expect(() => draft.request()).toThrow('plaintext_consent_required');
        draft.approveRemotePlaintext();
        expect(() => draft.request()).toThrow('group_consent_required');
        draft.approveGroupCapture();
        expect(draft.request().params.capture_group_text).toBe(true);
        draft.connectionID = 'two';
        draft.connectionID = 'one';
        expect(() => draft.request()).toThrow('plaintext_consent_required');
    });
    test('capture is off and background spending cannot be enabled', () => {
        const draft = new MemoryPreferencesDraft('bot');
        expect(draft.request().params.capture_conversation).toBe(false);
        draft.unattendedCapture = true;
        expect(() => draft.request()).toThrow('enforced_spending_policy_required');
    });
});
describe('masked edits and operations', () => {
    test('masked reader discards credential and endpoint fields', () => {
        const view = readMemoryConnections({ schema_version: 1, connections: [{ id: 'c', revision, backend: 'hindsight', name: 'C', has_secret: true, embedding_profile: null, availability: 'supported', reason: null, secret: 'private', endpoint: 'private', path: 'private' }], embeddings: [], bots: [] });
        expect(JSON.stringify(view)).not.toContain('private');
    });
    test('required masked eligibility never fabricates readiness or reveals private target data', () => {
        const connection = { id: 'lance', revision, backend: 'lance_db', name: 'Lance', has_secret: true, embedding_profile: null, availability: 'supported', reason: null };
        const reply = (c: unknown) => ({ schema_version: 1, connections: [c], embeddings: [], bots: [] });
        for (const availability of [undefined, 'ready', 'unknown'])
            expect(() => readMemoryConnections(reply({ ...connection, availability }))).toThrow('invalid_memory_reply');
        expect(() => readMemoryConnections(reply({ ...connection, reason: undefined }))).toThrow('invalid_memory_reply');
        const eligible = readMemoryConnections(reply(connection)).connections[0]!;
        expect(connectionActivationReason(eligible)).toBeUndefined();
        expect(eligible).not.toHaveProperty('status');
        const blocked = readMemoryConnections(reply({ ...connection, availability: 'blocked', reason: 'lancedb_cloud_transport_unavailable', endpoint: 'private', path: 'private', api_key: 'private' })).connections[0]!;
        expect(connectionActivationReason(blocked)).toBe('lancedb_cloud_transport_unavailable');
        expect(JSON.stringify(blocked)).not.toContain('private');
        const health = readMemoryHealth({ status: 'setup_required', reason: 'lancedb_cloud_transport_unavailable', deletion_pending: false, capabilities: {} });
        expect(health.status).toBe('setup_required');
        expect(health.reason).toBe('lancedb_cloud_transport_unavailable');
    });
    test('blocked activation sends no preferences mutation; explicit disable and eligible local binding remain available', async () => {
        const config = readMemoryConnections({ schema_version: 1, connections: [
            { id: 'cloud', revision, backend: 'lance_db', name: 'Cloud', has_secret: true, embedding_profile: null, availability: 'blocked', reason: 'lancedb_cloud_transport_unavailable' },
            { id: 'local', revision, backend: 'lance_db', name: 'Local', has_secret: false, embedding_profile: null, availability: 'supported', reason: null },
        ], embeddings: [], bots: [] });
        const bindings: (string | null)[] = [];
        const api = new MemoryServiceAPI(async (method, params) => {
            if (method !== 'memory.preferences.set' || !params || typeof params !== 'object' || !('connection_id' in params)) throw new Error('unexpected_fixture_request');
            bindings.push(params.connection_id as string | null);
            return {};
        });
        const draft = new MemoryPreferencesDraft('bot');
        draft.connectionID = 'cloud';
        await expect(api.savePreferences(draft, config)).rejects.toThrow('lancedb_cloud_transport_unavailable');
        expect(bindings).toEqual([]);
        draft.connectionID = null;
        await api.savePreferences(draft, config);
        draft.connectionID = 'local';
        await api.savePreferences(draft, config);
        expect(bindings).toEqual([null, 'local']);
    });
    test('secret bytes preserved; omitted endpoint/options keep them private', () => {
        const request = connectionRequest({ id: 'c', backend: 'hindsight', name: 'C', secret: secretPatch('replace', ' x ') });
        expect(request.params.secret).toEqual({ action: 'replace', value: ' x ' });
        expect(request.params).not.toHaveProperty('options');
        expect(request.params).not.toHaveProperty('endpoint');
    });
    test('Cloud activation fails as missing transport before dispatch or any fallback', async () => {
        const calls: string[] = [];
        const api = new MemoryServiceAPI(async method => { calls.push(method); return { saved: true }; });
        const local: MemoryConnectionEdit = { id: 'lance', name: 'Local', backend: 'lance_db', endpoint: null, secret: { action: 'clear' }, options: { backend: 'lance_db', region: null } };
        for (const edit of [
            { ...local, endpoint: 'db://cloud' },
            { ...local, secret: { action: 'replace', value: '' } as const },
            { ...local, options: { backend: 'lance_db', region: 'region' } as const },
        ])
            expect(() => api.setConnection(edit)).toThrow('lancedb_cloud_transport_unavailable');
        expect(calls).toEqual([]);
        const metadata = connectionRequest({ id: 'lance', name: 'Renamed', backend: 'lance_db', secret: { action: 'keep' } }).params;
        expect(metadata).not.toHaveProperty('endpoint');
        expect(metadata).not.toHaveProperty('options');
        const configured = connectionRequest(local).params;
        expect(configured.endpoint).toBeNull();
        expect(configured.secret).toEqual({ action: 'clear' });
        await api.setConnection(local);
        expect(calls).toEqual(['memory.connections.set']);
    });
    test('unsupported and unapproved calls never dispatch', async () => {
        const calls: unknown[] = [];
        const api = new MemoryServiceAPI(async (...args) => { calls.push(args); return {}; });
        await expect(api.recall('bot', 'hello', {}, true)).rejects.toThrow('unsupported_operation');
        await expect(api.reflect('bot', 'hello', caps, false)).rejects.toThrow('plaintext_consent_required');
        expect(calls).toEqual([]);
    });
    test('explicit retain reports pending rather than stored and preserves retry request ID', async () => {
        const api = new MemoryServiceAPI(async () => ({ id: 'd', document_id: 'doc', state: 'delivery_unknown', operation_id: null, error_code: null }));
        const operation = await api.retain('bot', 'text', 'same-request', caps, true);
        expect(operation.state).toBe('delivery_unknown');
    });
    test('exact preview can only apply once even after failure', async () => {
        const calls: {
            method: string;
            params: unknown;
        }[] = [];
        const api = new MemoryServiceAPI(async (method, params) => { calls.push({ method, params }); if (method.endsWith('preview'))
            return { token: 'opaque', expires_at: Date.now() / 1000 + 300, action: method, runner_id: 'runner', bot_id: 'bot', profile_id: null, connection_revision: revision, profile_revision: null, details: { target: { database: 'fixture', database_oid: 1, role: 'fixture', server_address: null, server_port: null, server_version: '17', session_pid: 1 }, schema: 'beans', sql: 'EXACT SQL' } }; throw new Error('fixture apply failure'); });
        const preview = await api.botPreview('pgvector.initialize', 'bot');
        await expect(api.applyBotPreview(preview, true)).rejects.toThrow('fixture apply failure');
        await expect(api.applyBotPreview(preview, true)).rejects.toThrow('approval_not_found');
        expect(calls[1]?.params).toEqual({ bot_id: 'bot', token: 'opaque', confirm: true });
    });
    test('asset preview binds selected Runner/profile without applying or loading', async () => {
        const plan: AssetPlan = { assets: ['runtime', 'model', 'tokenizer'].map(kind => ({ kind: kind as 'runtime' | 'model' | 'tokenizer', license: 'fixture', bytes: 3, sha256: 'a'.repeat(64), source: { kind: 'supplied', path: '/runner/' + kind } })) };
        let called: unknown;
        const api = new MemoryServiceAPI(async (method, params) => { called = { method, params }; return { preview_token: 'token', preview_digest: 'b'.repeat(64), expires_in_seconds: 600, runner_id: 'runner', profile_id: 'profile', profile_revision: revision, total_bytes: 9, assets: plan.assets }; });
        const preview = await api.localPreview('runner', 'profile', revision, plan);
        expect(preview.runner_id).toBe('runner');
        expect(called).toEqual({ method: 'memory.embeddings.local.preview', params: { runner_id: 'runner', profile_id: 'profile', profile_revision: revision, plan } });
    });
});
test('async failure retains editable draft and blocks overlapping work', async () => {
    const task = new MemoryTask();
    const draft = { text: 'keep this' };
    const { promise, reject } = Promise.withResolvers<void>();
    const pending = task.run(() => promise);
    expect(task.busy).toBe(true);
    expect(await task.run(async () => { draft.text = 'lost'; })).toBe(false);
    reject(new Error('fixture'));
    expect(await pending).toBe(false);
    expect(draft.text).toBe('keep this');
    expect(task.busy).toBe(false);
    expect(task.error).toBe('fixture');
});
