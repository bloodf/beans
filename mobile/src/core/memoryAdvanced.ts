// Fields/actions ported from the concrete Hindsight/OpenViking/vector adapter boundary.
// No arbitrary JSON, service URL, namespace, bank, key or scheduled refresh input.
import { supportsMemoryAction, type MemoryAdvancedFeature, type MemoryBackend, type MemoryCapabilities } from './memoryService';
export interface AdvancedField {
    name: string;
    type: 'text' | 'number' | 'boolean' | 'tags';
    required?: boolean;
}
export interface AdvancedForm {
    action: string;
    fields: AdvancedField[];
    destructive?: boolean;
    spending?: boolean;
}
const id: AdvancedField = { name: 'id', type: 'text', required: true };
const list: AdvancedForm = { action: 'list', fields: [] };
const get: AdvancedForm = { action: 'get', fields: [id] };
const del: AdvancedForm = { action: 'delete', fields: [id], destructive: true };
const configFields: AdvancedField[] = [...['disposition_skepticism', 'disposition_literalism', 'disposition_empathy', 'retain_chunk_size', 'recall_max_tokens', 'reflect_max_iterations', 'reflect_max_tokens'].map(name => ({ name, type: 'number' as const })), ...['reflect_mission', 'retain_mission', 'retain_extraction_mode', 'retain_custom_instructions', 'recall_budget'].map(name => ({ name, type: 'text' as const })), { name: 'retain_extract_labels', type: 'boolean' }];
export function advancedForms(backend: MemoryBackend, feature: MemoryAdvancedFeature, capabilities?: MemoryCapabilities): AdvancedForm[] {
    if (!supportsMemoryAction(capabilities, feature))
        return [];
    return implementedForms(backend, feature).filter(form => supportsMemoryAction(capabilities, feature, form.action));
}
function implementedForms(backend: MemoryBackend, feature: MemoryAdvancedFeature): AdvancedForm[] {
    if (backend === 'pgvector' || backend === 'lance_db')
        return feature === 'memory_edit' ? [{ action: 'edit', fields: [{ name: 'document_id', type: 'text', required: true }, { name: 'text', type: 'text', required: true }] }] : [];
    if (backend === 'open_viking')
        switch (feature) {
            case 'sessions': return [list, { action: 'create', fields: [id] }, get, del, { action: 'add_message', fields: [id, { name: 'role', type: 'text', required: true }, { name: 'content', type: 'text', required: true }] }, { action: 'commit', fields: [id], spending: true }];
            case 'resources': return [{ action: 'list', fields: [{ name: 'path', type: 'text' }, { name: 'offset', type: 'number' }] }, { action: 'add', fields: [id, { name: 'source_url', type: 'text', required: true }], spending: true }, { action: 'read', fields: [{ name: 'path', type: 'text', required: true }] }, { action: 'delete', fields: [{ name: 'path', type: 'text', required: true }], destructive: true }];
            case 'tasks': return [list, get, { action: 'cancel', fields: [id], destructive: true }];
            default: return [];
        }
    switch (feature) {
        case 'bank_profile':
        case 'bank_config': return [{ action: 'get', fields: [] }, { action: 'update', fields: configFields }, { action: 'reset', fields: [], destructive: true }];
        case 'directives': {
            const fields: AdvancedField[] = [{ name: 'name', type: 'text', required: true }, { name: 'content', type: 'text', required: true }, { name: 'priority', type: 'number' }, { name: 'is_active', type: 'boolean' }, { name: 'tags', type: 'tags' }];
            return [list, get, { action: 'create', fields }, { action: 'update', fields: [id, ...fields.map(f => ({ ...f, required: false }))] }, del];
        }
        case 'mental_models': {
            const fields: AdvancedField[] = [{ name: 'name', type: 'text', required: true }, { name: 'source_query', type: 'text', required: true }, { name: 'tags', type: 'tags' }, { name: 'max_tokens', type: 'number' }];
            return [list, get, { action: 'create', fields, spending: true }, { action: 'update', fields: [id, ...fields.map(f => ({ ...f, required: false }))] }, del, { action: 'refresh', fields: [id], spending: true }];
        }
        case 'mental_model_history': return [{ action: 'list', fields: [id] }, get];
        case 'observations': return [list, { action: 'scopes', fields: [] }, { action: 'clear', fields: [], destructive: true }, { action: 'clear_derived', fields: [id], destructive: true }];
        case 'memory_edit': return [get, { action: 'history', fields: [id] }, { action: 'update', fields: [id, { name: 'text', type: 'text', required: true }, { name: 'context', type: 'text' }, { name: 'occurred_start', type: 'text' }, { name: 'occurred_end', type: 'text' }, { name: 'tags', type: 'tags' }] }];
        case 'memory_invalidate':
        case 'memory_restore': return [get, { action: 'history', fields: [id] }, { action: 'update', fields: [id, { name: 'reason', type: 'text' }], destructive: feature === 'memory_invalidate' }];
        case 'documents': return [list, get, { action: 'chunks', fields: [id] }];
        case 'tasks': return [list, get, { action: 'cancel', fields: [id], destructive: true }, { action: 'delete_record', fields: [id], destructive: true }];
        default: return [];
    }
}
export function advancedBody(feature: MemoryAdvancedFeature, form: AdvancedForm, values: Record<string, string>): Record<string, unknown> {
    const body: Record<string, unknown> = {};
    for (const field of form.fields) {
        const value = values[field.name] ?? '';
        if (!value) {
            if (field.required)
                throw new Error(`${field.name}_required`);
            continue;
        }
        if (field.type === 'number') {
            const n = Number(value);
            if (!Number.isSafeInteger(n) || n < 0)
                throw new Error(`invalid_${field.name}`);
            body[field.name] = n;
        }
        else if (field.type === 'boolean') {
            if (value !== 'true' && value !== 'false')
                throw new Error(`invalid_${field.name}`);
            body[field.name] = value === 'true';
        }
        else if (field.type === 'tags')
            body[field.name] = value.split(',').map(v => v.trim()).filter(Boolean);
        else
            body[field.name] = value;
    }
    if (body.role && !['user', 'assistant'].includes(String(body.role)))
        throw new Error('invalid_role');
    return (feature === 'bank_profile' || feature === 'bank_config') && form.action === 'update' ? { updates: body } : body;
}
