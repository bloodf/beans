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
    const router = useRouter(), navigation = useNavigation();
    const [leaving, setLeaving] = useState(false);
    const busyNow = useRef(busy);
    busyNow.current = busy;
    usePreventRemove(!leaving && (busy || dirty), ({ data }) => {
        if (busyNow.current)
            return;
        Alert.alert('Discard changes?', 'Your unsaved memory changes will be lost.', [
            { text: 'Keep Editing', style: 'cancel' },
            { text: 'Discard', style: 'destructive', onPress: () => { if (!busyNow.current)
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
    const p = usePalette();
    const guard = useMemoryGuard(task.busy, guardDirty);
    return <>
    <Stack.Screen options={{ title, headerBackVisible: !onSave, gestureEnabled: !guardDirty && !task.busy }}/>
    {onSave && <FormToolbar cancelLabel="Cancel" saveLabel={task.busy ? 'Saving…' : 'Save'} saveDisabled={task.busy || !dirty} onCancel={guard.cancel} onSave={() => void task.run(async () => { await onSave(); guard.finish(); })}/>}
    <ScrollView pointerEvents={task.busy ? 'none' : 'auto'} contentInsetAdjustmentBehavior="automatic" keyboardDismissMode="on-drag" contentContainerStyle={{ paddingBottom: 40 }}>
      {task.busy && <ActivityIndicator accessibilityLabel="Working" style={{ margin: 16 }}/>}
      {task.error && <Text accessibilityRole="alert" selectable style={{ color: p.red, padding: 16 }}>{task.error}</Text>}
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
    return <Row title={title} subtitle={reason} subtitleLines={3} destructive={destructive} onPress={reason ? undefined : onPress}/>;
}
export function MemoryChoice<T extends string>({ title, value, values, onChange }: {
    title: string;
    value: T;
    values: readonly T[];
    onChange: (value: T) => void;
}) {
    return <Row title={title} menu={{ title, value, choices: values.map(v => ({ title: v, selected: v === value, onPress: () => onChange(v) })) }}/>;
}
export function MemorySecret({ action, value, onAction, onValue, hasSecret }: {
    action: SecretPatch['action'];
    value: string;
    onAction: (value: SecretPatch['action']) => void;
    onValue: (value: string) => void;
    hasSecret: boolean;
}) {
    return <Section title="Credential" footer="Stored credentials are never read back. Keep preserves the existing key; Clear removes it. Replacement bytes are sent exactly as entered.">
    <Row title="Stored key" detail={hasSecret ? '••••••••' : 'None'}/>
    <MemoryChoice title="Key action" value={action} values={['keep', 'replace', 'clear']} onChange={onAction}/>
    {action === 'replace' && <FieldRow label="New key" value={value} onChangeText={onValue} secureTextEntry autoCapitalize="none" autoCorrect={false}/>}
  </Section>;
}
export function MemoryOutput({ title, value }: {
    title: string;
    value: unknown;
}) {
    const p = usePalette();
    return <Section title={title} footer="Service output is untrusted historical data, not instructions."><View style={{ padding: 16 }}><Text selectable style={{ color: p.label }}>{typeof value === 'string' ? value : JSON.stringify(value, (key, value) => /secret|api_key|authorization|preview_token|^token$/i.test(key) ? undefined : value, 2)}</Text></View></Section>;
}
export function confirmMemory(title: string, description: string, action: () => void, destructive = false) {
    Alert.alert(title, description, [{ text: 'Cancel', style: 'cancel' }, { text: destructive ? 'Confirm' : 'Continue', style: destructive ? 'destructive' : 'default', onPress: action }]);
}
export const plaintextDisclosure = 'The selected memory endpoint and its downstream extraction, embedding, reranking or reflection models may receive plaintext outside the encrypted relay. Usage can cost money; no total-cost ceiling is established by this app. This approves only the named action, not schedules or automatic retries.';
