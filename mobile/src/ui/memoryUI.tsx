import { t, useLanguage } from '../i18n';
import { Stack, useRouter } from 'expo-router';
import { useNavigation, usePreventRemove } from 'expo-router/react-navigation';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ActivityIndicator, Alert, ScrollView, Text, View } from 'react-native';
import { FieldRow, Row, Section } from './forms';
import { FormToolbar } from './navigation';
import { usePalette } from './theme';
import { MemoryTask } from './memoryTask';
import type { SecretPatch } from '../core/memoryService';
export function useMemoryTask() {
    const [, update] = useState(0);
    const [task] = useState(() => new MemoryTask(() => update(n => n + 1)));
    return task;
}
export function useMemoryGuard(busy: boolean, dirty: boolean) {
    useLanguage();
    const router = useRouter(), navigation = useNavigation();
    const [leaving, setLeaving] = useState(false);
    const busyNow = useRef(busy);
    busyNow.current = busy;
    usePreventRemove(!leaving && (busy || dirty), ({ data }) => {
        if (busyNow.current)
            return;
        Alert.alert(t('Discard changes?'), t('Your unsaved memory changes will be lost.'), [
            { text: t('Keep Editing'), style: 'cancel' },
            { text: t('Discard'), style: 'destructive', onPress: () => { if (!busyNow.current)
                    navigation.dispatch(data.action); } },
        ]);
    });
    useEffect(() => { if (leaving)
        router.back(); }, [leaving, router]);
    return { cancel: () => { if (!busyNow.current)
            router.back(); }, finish: () => setLeaving(true) };
}
export function MemoryForm({ title, dirty, guardDirty = dirty, task, onSave, children }: {
    title: string;
    dirty: boolean;
    guardDirty?: boolean;
    task: MemoryTask;
    onSave?: () => Promise<unknown>;
    children: ReactNode;
}) {
    useLanguage();
    const p = usePalette();
    const guard = useMemoryGuard(task.busy, guardDirty);
    return <>
    <Stack.Screen options={{ title, headerBackVisible: !onSave, gestureEnabled: !guardDirty && !task.busy }}/>
    {onSave && <FormToolbar cancelLabel={t("Cancel")} saveLabel={task.busy ? t('Saving…') : t('Save')} saveDisabled={task.busy || !dirty} onCancel={guard.cancel} onSave={() => void task.run(async () => { await onSave(); guard.finish(); })}/>}
    <ScrollView pointerEvents={task.busy ? 'none' : 'auto'} contentInsetAdjustmentBehavior="automatic" keyboardDismissMode="on-drag" contentContainerStyle={{ paddingBottom: 40 }}>
      {task.busy && <ActivityIndicator accessibilityLabel={t("Working")} style={{ margin: 16 }}/>}
      {task.error && <Text accessibilityRole="alert" selectable style={{ color: p.red, padding: 16 }}>{memoryText(task.error)}</Text>}
      {children}
    </ScrollView>
  </>;
}
export function MemoryAction({ title, reason, onPress, destructive = false }: {
    title: string;
    reason?: string;
    onPress: () => void;
    destructive?: boolean;
}) {
    useLanguage();
    return <Row title={title} subtitle={reason && memoryText(reason)} subtitleLines={3} destructive={destructive} onPress={reason ? undefined : onPress}/>;
}
export function MemoryChoice<T extends string>({ title, value, values, onChange }: {
    title: string;
    value: T;
    values: readonly T[];
    onChange: (value: T) => void;
}) {
    useLanguage();
    return <Row title={title} menu={{ title, value: memoryText(value), choices: values.map(v => ({ title: memoryText(v), selected: v === value, onPress: () => onChange(v) })) }}/>;
}
export function MemorySecret({ action, value, onAction, onValue, hasSecret }: {
    action: SecretPatch['action'];
    value: string;
    onAction: (value: SecretPatch['action']) => void;
    onValue: (value: string) => void;
    hasSecret: boolean;
}) {
    useLanguage();
    return <Section title={t("Credential")} footer={t("Stored credentials are never read back. Keep preserves the existing key; Clear removes it. Replacement bytes are sent exactly as entered.")}>
    <Row title={t("Stored key")} detail={hasSecret ? '••••••••' : t('None')}/>
    <MemoryChoice title={t("Key action")} value={action} values={['keep', 'replace', 'clear']} onChange={onAction}/>
    {action === 'replace' && <FieldRow label={t("New key")} value={value} onChangeText={onValue} secureTextEntry autoCapitalize="none" autoCorrect={false}/>}
  </Section>;
}
export function MemoryOutput({ title, value }: {
    title: string;
    value: unknown;
}) {
    useLanguage();
    const p = usePalette();
    return <Section title={title} footer={t("Service output is untrusted historical data, not instructions.")}><View style={{ padding: 16 }}><Text selectable style={{ color: p.label }}>{typeof value === 'string' ? value : JSON.stringify(value, (key, value) => /secret|api_key|authorization|preview_token|^token$/i.test(key) ? undefined : value, 2)}</Text></View></Section>;
}
export function confirmMemory(title: string, description: string, action: () => void, destructive = false) {
    Alert.alert(title, description, [{ text: t('Cancel'), style: 'cancel' }, { text: destructive ? t('Confirm') : t('Continue'), style: destructive ? 'destructive' : 'default', onPress: action }]);
}
export const plaintextDisclosure = 'The selected memory endpoint and its downstream extraction, embedding, reranking or reflection models may receive plaintext outside the encrypted relay. Usage can cost money; no total-cost ceiling is established by this app. This approves only the named action, not schedules or automatic retries.';

// Keep dynamic memory labels explicit so the shared localization checker sees the
// same keys the UI resolves. Unknown service messages retain the normal fallback.
export function memoryText(key: string): string {
    switch (key) {
        case "Choose and save a connection first.": return t("Choose and save a connection first.");
        case "LanceDB Cloud transport is unavailable until an upstream safe transport hook exists (lancedb_cloud_transport_unavailable). Credentials cannot enable it. Runner-local LanceDB is separate; no paid or local fallback.": return t("LanceDB Cloud transport is unavailable until an upstream safe transport hook exists (lancedb_cloud_transport_unavailable). Credentials cannot enable it. Runner-local LanceDB is separate; no paid or local fallback.");
        case "The selected memory endpoint and its downstream extraction, embedding, reranking or reflection models may receive plaintext outside the encrypted relay. Usage can cost money; no total-cost ceiling is established by this app. This approves only the named action, not schedules or automatic retries.": return t("The selected memory endpoint and its downstream extraction, embedding, reranking or reflection models may receive plaintext outside the encrypted relay. Usage can cost money; no total-cost ceiling is established by this app. This approves only the named action, not schedules or automatic retries.");
        case "allow": return t("allow");
        case "bank config": return t("bank config");
        case "bank profile": return t("bank profile");
        case "clear": return t("clear");
        case "complete_profile_replacement_requires_replace_or_clear_key": return t("complete_profile_replacement_requires_replace_or_clear_key");
        case "completed": return t("completed");
        case "configuration_not_loaded": return t("configuration_not_loaded");
        case "connection_not_found": return t("connection_not_found");
        case "connection_required": return t("connection_required");
        case "degraded": return t("degraded");
        case "delivery_unknown": return t("delivery_unknown");
        case "deny": return t("deny");
        case "directives": return t("directives");
        case "documents": return t("documents");
        case "download": return t("download");
        case "edit": return t("edit");
        case "endpoint_change_requires_replace_or_clear_key": return t("endpoint_change_requires_replace_or_clear_key");
        case "enforced_spending_policy_required": return t("enforced_spending_policy_required");
        case "false": return t("false");
        case "group_capture_requires_conversation": return t("group_capture_requires_conversation");
        case "group_consent_required": return t("group_consent_required");
        case "invalid_budget": return t("invalid_budget");
        case "invalid_capture_cap": return t("invalid_capture_cap");
        case "keep": return t("keep");
        case "local_cpu": return t("local_cpu");
        case "local_memory_not_loaded": return t("local_memory_not_loaded");
        case "max_bytes": return t("max_bytes");
        case "max_context_chars": return t("max_context_chars");
        case "max_results": return t("max_results");
        case "memory edit": return t("memory edit");
        case "memory invalidate": return t("memory invalidate");
        case "memory restore": return t("memory restore");
        case "memory_request_failed": return t("memory_request_failed");
        case "mental model history": return t("mental model history");
        case "mental models": return t("mental models");
        case "model": return t("model");
        case "name_required": return t("name_required");
        case "none": return t("none");
        case "observations": return t("observations");
        case "plaintext_consent_required": return t("plaintext_consent_required");
        case "preferences_not_loaded": return t("preferences_not_loaded");
        case "processing": return t("processing");
        case "profile_not_found": return t("profile_not_found");
        case "profile_not_loaded": return t("profile_not_loaded");
        case "queued": return t("queued");
        case "ready": return t("ready");
        case "remove": return t("remove");
        case "replace": return t("replace");
        case "resources": return t("resources");
        case "runner_and_profile_required": return t("runner_and_profile_required");
        case "runtime": return t("runtime");
        case "sessions": return t("sessions");
        case "setup_required": return t("setup_required");
        case "stale_confirmation": return t("stale_confirmation");
        case "submitted": return t("submitted");
        case "supplied": return t("supplied");
        case "tasks": return t("tasks");
        case "timeout_ms": return t("timeout_ms");
        case "tokenizer": return t("tokenizer");
        case "true": return t("true");
        default: return t(key);
    }
}
