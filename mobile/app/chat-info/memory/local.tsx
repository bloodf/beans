import { useLocalSearchParams } from 'expo-router';
import { useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { useBotMap } from '../../../src/core/store';
import type { LocalMemoryView } from '../../../src/core/memoryService';
import { FieldRow, Row, Section } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryForm, useMemoryTask } from '../../../src/ui/memoryUI';
export default function LocalMemoryScreen() {
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
    return <MemoryForm title="Local MEMORY.md" task={task} dirty={dirty} onSave={save}>
    <Section title={bot?.name ?? 'Bot'} footer="This editor reads/writes only the assigned Runner’s local MEMORY.md through the existing core RPC. It does not retain content in a memory service, enable capture or merge historical conversations. Hash-based conflict protection refuses stale writes; a failed save keeps your draft.">
      <Row title="Assigned Runner" detail={bot?.runner_id ?? 'Unavailable'}/>
      <MemoryAction title={saved ? 'Reload Local Memory' : 'Load Local Memory'} onPress={() => dirty ? confirmMemory('Discard draft and reload?', 'The latest Runner file replaces this local draft.', () => void task.run(load), true) : void task.run(load)}/>
      {saved && <><Row title="Loaded budget" detail={`${saved.lines}/${saved.max_lines} lines · ${saved.bytes}/${saved.max_bytes} bytes`}/>{saved.truncated && <Row title="Context budget exceeded" subtitle="Only a bounded portion is loaded into bot context. The editor retains the full file." subtitleLines={3}/>}<FieldRow multiline value={text} onChangeText={setText} placeholder="Bot memory" style={{ minHeight: 280 }}/></>}
    </Section>
  </MemoryForm>;
}
