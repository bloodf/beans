import { t, useLanguage } from '../../../src/i18n';
import { useEffect, useState } from 'react';
import { memoryService } from '../../../src/core/memoryServiceNative';
import type { AssetRequest, LocalPreview, MemoryEmbeddingView } from '../../../src/core/memoryService';
import { useStore } from '../../../src/core/store';
import { FieldRow, Row, Section } from '../../../src/ui/forms';
import { confirmMemory, MemoryAction, MemoryChoice, MemoryForm, MemoryOutput, useMemoryTask } from '../../../src/ui/memoryUI';
export default function LocalEmbeddingSetupScreen() {
    useLanguage();
    const task = useMemoryTask(), devices = useStore(s => s.devices);
    const runners = devices.filter(d => ['macos', 'linux', 'windows'].includes(d.os));
    const [runner, setRunner] = useState('');
    const [profiles, setProfiles] = useState<MemoryEmbeddingView[]>([]);
    const [profileID, setProfileID] = useState('');
    const [assets, setAssets] = useState<AssetRequest[]>(() => ['runtime', 'model', 'tokenizer'].map(kind => ({ kind: kind as AssetRequest['kind'], source: { kind: 'supplied', path: '' }, license: '', bytes: 0, sha256: '' })));
    const [preview, setPreview] = useState<LocalPreview>();
    const [result, setResult] = useState<unknown>();
    const [dirty, setDirty] = useState(false);
    useEffect(() => { void task.run(async () => { setProfiles((await memoryService.listConnections()).embeddings); }); }, []);
    function change(action: () => void) { action(); setPreview(undefined); setDirty(true); }
    function asset(index: number, next: AssetRequest) { change(() => setAssets(assets.map((a, i) => i === index ? next : a))); }
    const profile = profiles.find(p => p.id === profileID);
    async function makePreview() { if (!runner || !profile)
        throw new Error('runner_and_profile_required'); setPreview(await memoryService.localPreview(runner, profile.id, profile.revision, { assets })); }
    return <MemoryForm title={t("Runner Embedding Assets")} task={task} dirty={dirty}>
    <Section title={t("Selected Runner")} footer={t("Every source path is on the selected Runner, never on this phone. Preview performs no acquisition or inference. Downloads require an administrator-approved HTTPS origin. Applying separately approves exact source/license/bytes/SHA-256 and runtime load; no fallback.")}>
      <Row title={t("Runner")} menu={{ title: t('Runner'), value: runners.find(r => r.id === runner)?.name ?? t('Choose'), choices: runners.map(r => ({ title: r.name, selected: r.id === runner, onPress: () => change(() => setRunner(r.id)) })) }}/>
      <Row title={t("Profile")} menu={{ title: t('Profile'), value: profile?.model ?? t('Choose saved local_cpu profile'), choices: profiles.map(p => ({ title: p.model, selected: p.id === profileID, onPress: () => change(() => setProfileID(p.id)) })) }}/>
      {!runners.length && <Row title={t("No paired Runner")} subtitle={t("Pair a desktop Runner before installing local assets.")}/>}
    </Section>
    {assets.map((a, i) => <Section key={t(a.kind)} title={t(a.kind)}>
      <MemoryChoice title={t("Source")} value={a.source.kind} values={['supplied', 'download']} onChange={kind => asset(i, { ...a, source: kind === 'supplied' ? { kind, path: '' } : { kind, url: '' } })}/>
      <FieldRow label={a.source.kind === 'supplied' ? t('Runner path') : t('HTTPS URL')} value={a.source.kind === 'supplied' ? a.source.path : a.source.url} onChangeText={v => asset(i, { ...a, source: a.source.kind === 'supplied' ? { kind: 'supplied', path: v } : { kind: 'download', url: v } })} autoCapitalize="none" autoCorrect={false}/>
      <FieldRow label={t("License")} value={a.license} onChangeText={v => asset(i, { ...a, license: v })}/>
      <FieldRow label={t("Exact bytes")} value={a.bytes ? String(a.bytes) : ''} keyboardType="number-pad" onChangeText={v => asset(i, { ...a, bytes: Number(v) })}/>
      <FieldRow label={t("SHA-256")} value={a.sha256} onChangeText={v => asset(i, { ...a, sha256: v })} autoCapitalize="none" autoCorrect={false}/>
    </Section>)}
    <Section><MemoryAction title={t("Preview Exact Asset Plan")} reason={!runner || !profile ? t('Choose a Runner and saved local_cpu profile.') : undefined} onPress={() => void task.run(makePreview)}/></Section>
    {preview && <>
      <MemoryOutput title={t("Exact approval")} value={{ runner_id: preview.runner_id, profile_id: preview.profile_id, profile_revision: preview.profile_revision, assets: preview.assets, total_bytes: preview.total_bytes, preview_digest: preview.preview_digest, expires_in_seconds: preview.expires_in_seconds }}/>
      <Section><MemoryAction title={t("Approve Install and Runtime Load")} onPress={() => confirmMemory(t('Install these exact assets on this Runner?'), t('Approve only the displayed sources, licenses, byte counts and hashes, and explicit runtime/model load on ') + preview.runner_id + t('. Downloads may transfer bytes; no inference is invoked.'), () => void task.run(async () => { setPreview(undefined); setResult(await memoryService.applyLocalPreview(preview, true)); setDirty(false); }))}/></Section>
    </>}
    <Section><MemoryAction title={t("Check Approved Local Runtime")} reason={!runner || !profile ? t('Choose a Runner and profile.') : undefined} onPress={() => confirmMemory(t('Check local runtime?'), t('This explicit status request can load a previously approved native runtime on the selected Runner. It does not infer or download.'), () => void task.run(async () => setResult(await memoryService.localStatus(runner, profileID))))}/></Section>
    {result !== undefined && <MemoryOutput title={t("Runner result")} value={result}/>}
  </MemoryForm>;
}
