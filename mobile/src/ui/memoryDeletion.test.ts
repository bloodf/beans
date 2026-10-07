import { expect, test } from 'bun:test';
import { MemoryDeletionState } from './memoryTask';
import { MemoryServiceAPI, type MemoryPreferencesView } from '../core/memoryService';

const preferences = (epoch: number): MemoryPreferencesView => ({
    connection_id: 'connection', auto_recall: false, capture_conversation: false,
    capture_group_text: false, unattended_capture: false, max_capture_deliveries_per_turn: 1,
    recall_budget: { timeout_ms: 2000, max_bytes: 16384, max_results: 8, max_context_chars: 4000 },
    consent_revision: { counter: 1, device_id: 'fixture' }, deletion_epoch: epoch,
});

for (const outcome of ['completed', 'pending', 'failed'] as const) {
    test(`two document deletes refresh the fence after ${outcome} result`, async () => {
        let epoch = 0;
        const confirmations: number[] = [];
        const api = new MemoryServiceAPI(async (method, params) => {
            if (method === 'memory.preferences.get') return preferences(epoch);
            if (method === 'memory.service.delete') {
                if (!params || typeof params !== 'object' || !('deletion_epoch' in params)) throw new Error('invalid_fixture');
                confirmations.push(Number(params.deletion_epoch));
                if (params.deletion_epoch !== epoch) throw new Error('stale_confirmation');
                epoch++;
                if (outcome === 'failed' && confirmations.length === 1) throw new Error('remote_delete_failed_after_fence');
                return { pending: outcome !== 'completed', deletion_epoch: epoch, operation_id: null };
            }
            throw new Error('unexpected_fixture_method');
        });
        const state = new MemoryDeletionState();
        state.preferences = await api.getPreferences('bot');
        for (const document of ['first', 'second']) {
            const approved = state.preferences!;
            const attempt = state.run(approved,
                () => api.delete('bot', document, { counter: 1, device_id: 'fixture' }, approved.deletion_epoch, { delete_document: true }, true),
                () => api.getPreferences('bot'));
            if (outcome === 'failed' && document === 'first') await expect(attempt).rejects.toThrow('remote_delete_failed_after_fence');
            else expect((await attempt).pending).toBe(outcome !== 'completed');
        }
        expect(confirmations).toEqual([0, 1]);
        expect(state.preferences?.deletion_epoch).toBe(2);
    });
}

test('refresh failure locks deletion until an explicit successful refresh', async () => {
    const state = new MemoryDeletionState();
    const initial = preferences(0);
    state.preferences = initial;
    let deletes = 0;
    await expect(state.run(initial, async () => { deletes++; return { pending: true }; }, async () => { throw new Error('refresh_failed'); })).rejects.toThrow('refresh_failed');
    expect(state.preferences).toBeUndefined();
    await expect(state.run(initial, async () => { deletes++; }, async () => preferences(1))).rejects.toThrow('stale_confirmation');
    expect(deletes).toBe(1);
    state.preferences = preferences(1);
    const approved = state.preferences;
    await state.run(approved, async () => { deletes++; }, async () => preferences(2));
    expect(deletes).toBe(2);
});

test('an old confirmation cannot be reused after a refreshed fence', async () => {
    const state = new MemoryDeletionState();
    const initial = preferences(0);
    state.preferences = initial;
    await state.run(initial, async () => ({}), async () => preferences(1));
    let called = false;
    await expect(state.run(initial, async () => { called = true; }, async () => preferences(2))).rejects.toThrow('stale_confirmation');
    expect(called).toBe(false);
});
