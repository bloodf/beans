import { expect, test } from 'vitest';
import { MemoryServiceAPI, type MemoryCapabilities } from './memoryService';

// The existing phone controls share this adapter with the native request wrapper.
const revision = { counter: 7, device_id: 'fixture' };
const controls = [
    { capability: 'recall', method: 'memory.service.recall', params: { bot_id: 'bot', query: 'query' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.recall('bot', 'query', caps, true) },
    { capability: 'reflect', method: 'memory.service.reflect', params: { bot_id: 'bot', query: 'query' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.reflect('bot', 'query', caps, true) },
    { capability: 'inspect', method: 'memory.service.inspect', params: { bot_id: 'bot', document_id: 'doc' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.inspect('bot', 'doc', caps) },
    { capability: 'retain', method: 'memory.service.retain', params: { bot_id: 'bot', text: 'note', request_id: 'request' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.retain('bot', 'note', 'request', caps, true) },
    { capability: 'operation_status', method: 'memory.operations.status', params: { bot_id: 'bot', id: 'op' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.operation('bot', 'op', 'status', caps, true) },
    { capability: 'cancel_operation', method: 'memory.operations.cancel', params: { bot_id: 'bot', id: 'op' }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.operation('bot', 'op', 'cancel', caps, true) },
    { capability: 'delete_document', method: 'memory.service.delete', params: { bot_id: 'bot', document_id: 'doc', connection_revision: revision, deletion_epoch: 4, confirm: true }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.delete('bot', 'doc', revision, 4, caps, true) },
    { capability: 'clear', method: 'memory.service.delete', params: { bot_id: 'bot', connection_revision: revision, deletion_epoch: 4, confirm: true }, run: (api: MemoryServiceAPI, caps: MemoryCapabilities) => api.delete('bot', undefined, revision, 4, caps, true) },
] as const;

for (const control of controls) {
    test(`${control.capability} requires its negotiated capability before transport`, async () => {
        const calls: { method: string; params: unknown }[] = [];
        const api = new MemoryServiceAPI(async (method, params) => {
            calls.push({ method, params });
            if (method === 'memory.service.delete') return { pending: true, deletion_epoch: 5, operation_id: 'remote' };
            if (method === 'memory.service.retain' || method.startsWith('memory.operations.'))
                return { id: 'op', document_id: 'doc', state: 'delivery_unknown', operation_id: 'remote', error_code: null };
            return { evidence: [], document: null, operation: null, data: {} };
        });
        await expect(control.run(api, {})).rejects.toThrow('unsupported_operation');
        await expect(control.run(api, { [control.capability]: false })).rejects.toThrow('unsupported_operation');
        expect(calls).toEqual([]);
        await control.run(api, { [control.capability]: true });
        expect(calls).toEqual([{ method: control.method, params: control.params }]);
    });
}
