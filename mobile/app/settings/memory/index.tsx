import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, type MemoryConnectionsView } from '../../../src/core/memoryService';
import { Row, Section } from '../../../src/ui/forms';
import { MemoryForm, useMemoryTask, MemoryAction } from '../../../src/ui/memoryUI';
export default function MemoryConnectionsScreen() {
    const router = useRouter(), task = useMemoryTask();
    const [view, setView] = useState<MemoryConnectionsView>();
    async function refresh() { setView(await memoryService.listConnections()); }
    useFocusEffect(useCallback(() => { void task.run(refresh); }, [task]));
    return <MemoryForm title="Memory Connections" task={task} dirty={false}>
    <Section title="Account connections" footer="Encrypted configuration syncs to paired Devices. Service work runs on each bot’s assigned Runner. Saving a connection does not initialize a schema, download assets, or enable capture.">
      {view?.connections.map(c => <Row key={c.id} title={c.name} subtitle={c.availability === 'blocked' ? `Activation blocked · ${c.reason ?? 'Connection is blocked.'}` : `${c.backend} · Eligible configuration, not readiness · Setup checked per bot`} chevron onPress={() => router.push({ pathname: '/settings/memory/connection', params: { id: c.id } })}/>)}
      {view && !view.connections.length && <Row title="No connections" subtitle="Add a connection, then choose it in a bot’s Memory page."/>}
      <Row title="Add Connection" onPress={() => router.push('/settings/memory/connection')}/>
      <Row title="LanceDB Cloud activation unavailable" subtitle={LANCE_CLOUD_UNAVAILABLE_REASON} subtitleLines={6}/>
    </Section>
    <Section title="Embedding profiles" footer="pgvector and LanceDB require an exact model/revision/preprocessing vector space. Changing it requires a validated replacement index; dimensions alone are not compatible.">
      {view?.embeddings.map(e => <Row key={e.id} title={e.model} subtitle={`${e.model_revision} · ${e.dimensions} dimensions`} chevron onPress={() => router.push({ pathname: '/settings/memory/embedding', params: { id: e.id } })}/>)}
      <Row title="Add Embedding Profile" onPress={() => router.push('/settings/memory/embedding')}/>
      <Row title="Local Assets on a Runner" subtitle="Explicit preview and approval; no phone paths or runtime." chevron onPress={() => router.push('/settings/memory/local-setup')}/>
    </Section>
    <Section><MemoryAction title="Refresh Configuration" onPress={() => void task.run(refresh)}/></Section>
  </MemoryForm>;
}
