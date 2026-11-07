import { t, useLanguage } from '../../../src/i18n';
import { useLocalSearchParams } from 'expo-router';
import { useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { useBotMap } from '../../../src/core/store';
import type { LocalMemoryView } from '../../../src/core/memoryService';
import { FieldRow, Row, Section } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryForm, useMemoryTask } from '../../../src/ui/memoryUI';
export default function LocalMemoryScreen() {
    useLanguage();
    const { id } = useLocalSearchParams<{
        id: string;
    }>();
    const bot = useBotMap().get(id), task = useMemoryTask();
    const [saved, setSaved] = useState<LocalMemoryView>();
    const [text, setText] = useState('');
    const dirty = !!saved && text !== saved.text;
    async function load() { const value = await memoryService.readLocal(id); setSaved(value); setText(value.text); }
    async function save() { if (!saved)
        throw new Error('local_memory_not_loaded'); await memoryService.saveLocal(id, text, saved.hash); }
    return <MemoryForm title={t("Local MEMORY.md")} task={task} dirty={dirty} onSave={save}>
    <Section title={bot?.name ?? t('Bot')} footer={t("This editor reads/writes only the assigned Runner’s local MEMORY.md through the existing core RPC. It does not retain content in a memory service, enable capture or merge historical conversations. Hash-based conflict protection refuses stale writes; a failed save keeps your draft.")}>
      <Row title={t("Assigned Runner")} detail={bot?.runner_id ?? t('Unavailable')}/>
      <MemoryAction title={saved ? t('Reload Local Memory') : t('Load Local Memory')} onPress={() => dirty ? confirmMemory(t('Discard draft and reload?'), t('The latest Runner file replaces this local draft.'), () => void task.run(load), true) : void task.run(load)}/>
      {saved && <><Row title={t("Loaded budget")} detail={t("{v0}/{v1} lines · {v2}/{v3} bytes", { v0: String(saved.lines), v1: String(saved.max_lines), v2: String(saved.bytes), v3: String(saved.max_bytes) })}/>{saved.truncated && <Row title={t("Context budget exceeded")} subtitle={t("Only a bounded portion is loaded into bot context. The editor retains the full file.")} subtitleLines={3}/>}<FieldRow multiline value={text} onChangeText={setText} placeholder={t("Bot memory")} style={{ minHeight: 280 }}/></>}
    </Section>
  </MemoryForm>;
}
