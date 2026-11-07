import { t, useLanguage } from '../../../src/i18n';
import { useFocusEffect, useRouter } from 'expo-router';
import { useCallback, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import { LANCE_CLOUD_UNAVAILABLE_REASON, type MemoryConnectionsView } from '../../../src/core/memoryService';
import { Row, Section } from '../../../src/ui/forms';
import { MemoryForm, useMemoryTask, MemoryAction } from '../../../src/ui/memoryUI';
export default function MemoryConnectionsScreen() {
    useLanguage();
    const router = useRouter(), task = useMemoryTask();
    const [view, setView] = useState<MemoryConnectionsView>();
    async function refresh() { setView(await memoryService.listConnections()); }
    useFocusEffect(useCallback(() => { void task.run(refresh); }, [task]));
    return <MemoryForm title={t("Memory Connections")} task={task} dirty={false}>
    <Section title={t("Account connections")} footer={t("Encrypted configuration syncs to paired Devices. Service work runs on each bot’s assigned Runner. Saving a connection does not initialize a schema, download assets, or enable capture.")}>
      {view?.connections.map(c => <Row key={c.id} title={c.name} subtitle={c.availability === 'blocked' ? t("Activation blocked · {v0}", { v0: String(c.reason ?? t('Connection is blocked.')) }) : t("{v0} · Eligible configuration, not readiness · Setup checked per bot", { v0: String(c.backend) })} chevron onPress={() => router.push({ pathname: '/settings/memory/connection', params: { id: c.id } })}/>)}
      {view && !view.connections.length && <Row title={t("No connections")} subtitle={t("Add a connection, then choose it in a bot’s Memory page.")}/>}
      <Row title={t("Add Connection")} onPress={() => router.push('/settings/memory/connection')}/>
      <Row title={t("LanceDB Cloud activation unavailable")} subtitle={t(LANCE_CLOUD_UNAVAILABLE_REASON)} subtitleLines={6}/>
    </Section>
    <Section title={t("Embedding profiles")} footer={t("pgvector and LanceDB require an exact model/revision/preprocessing vector space. Changing it requires a validated replacement index; dimensions alone are not compatible.")}>
      {view?.embeddings.map(e => <Row key={e.id} title={e.model} subtitle={t("{v0} · {v1} dimensions", { v0: String(e.model_revision), v1: String(e.dimensions) })} chevron onPress={() => router.push({ pathname: '/settings/memory/embedding', params: { id: e.id } })}/>)}
      <Row title={t("Add Embedding Profile")} onPress={() => router.push('/settings/memory/embedding')}/>
      <Row title={t("Local Assets on a Runner")} subtitle={t("Explicit preview and approval; no phone paths or runtime.")} chevron onPress={() => router.push('/settings/memory/local-setup')}/>
    </Section>
    <Section><MemoryAction title={t("Refresh Configuration")} onPress={() => void task.run(refresh)}/></Section>
  </MemoryForm>;
}
