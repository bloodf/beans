import type { MemoryPreferencesView } from '../core/memoryService';

// Single-flight mutation state. Errors never replace the caller's editable draft.
export class MemoryTask {
    busy = false;
    error: string | null = null;
    constructor(private readonly changed: () => void = () => { }) { }
    async run(action: () => Promise<unknown>): Promise<boolean> {
        if (this.busy)
            return false;
        this.busy = true;
        this.error = null;
        this.changed();
        try {
            await action();
            return true;
        }
        catch (error) {
            this.error = error instanceof Error ? error.message : 'memory_request_failed';
            return false;
        }
        finally {
            this.busy = false;
            this.changed();
        }
    }
}

// A deletion can advance the fence even when its remote result fails or remains pending.
// Confirmation is usable only until an attempt starts, and refresh failure keeps it unavailable.
export class MemoryDeletionState {
    preferences: MemoryPreferencesView | undefined;
    async run<T>(confirmed: MemoryPreferencesView, remove: () => Promise<T>, refresh: () => Promise<MemoryPreferencesView>): Promise<T> {
        if (this.preferences !== confirmed) throw new Error('stale_confirmation');
        this.preferences = undefined;
        try {
            return await remove();
        } finally {
            this.preferences = await refresh();
        }
    }
}
