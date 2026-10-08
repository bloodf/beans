import * as Clipboard from 'expo-clipboard';
import { Stack, useFocusEffect } from 'expo-router';
import { useCallback, useRef, useState } from 'react';
import { AppState, Button, ScrollView, Share, Text, View } from 'react-native';
import { request } from '../../modules/beans-core';
import { decodeDiagnosticsReport, DiagnosticsReportError } from '../../src/core/diagnosticsReport';
import { useStore } from '../../src/core/store';
import { t, useLanguage } from '../../src/i18n';
import { Section } from '../../src/ui/forms';
import { usePalette } from '../../src/ui/theme';

function authority() {
  const s = useStore.getState();
  return [s.ready, s.paired, s.identityId, s.deviceId, s.relayUrl, s.relayConnected, s.relayUpdateRequired] as const;
}
function sameAuthority(a: ReturnType<typeof authority>, b: ReturnType<typeof authority>) {
  return a.every((value, index) => value === b[index]);
}
type Review = { json: string; host: string | null; authority: ReturnType<typeof authority> };

export default function DiagnosticsScreen() {
  useLanguage();
  const p = usePalette();
  const [review, setReview] = useState<Review>();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const focused = useRef(false), generation = useRef(0), pending = useRef(false);
  const revoke = useCallback(() => {
    generation.current++;
    pending.current = false;
    setReview(undefined);
    setBusy(false);
    setStatus('');
  }, []);
  useFocusEffect(useCallback(() => {
    focused.current = true;
    revoke();
    let observed = authority();
    const unsubscribe = useStore.subscribe(() => {
      const next = authority();
      if (!sameAuthority(observed, next)) { observed = next; revoke(); }
    });
    const lifecycle = AppState.addEventListener('change', state => { if (state !== 'active') revoke(); });
    return () => { focused.current = false; unsubscribe(); lifecycle.remove(); revoke(); };
  }, [revoke]));

  function valid(token: number, captured: ReturnType<typeof authority>) {
    return focused.current && AppState.currentState === 'active' && generation.current === token
      && sameAuthority(captured, authority());
  }
  async function generate() {
    if (pending.current || !focused.current || AppState.currentState !== 'active') return;
    const captured = authority();
    if (!captured[0] || !captured[1] || !captured[2] || !captured[3]) return;
    const token = ++generation.current;
    pending.current = true;
    setBusy(true); setReview(undefined); setStatus('Generating report…');
    try {
      const report = decodeDiagnosticsReport(await request('diagnostics.report', {}));
      if (!valid(token, captured)) return;
      setReview({ json: JSON.stringify(report, null, 2), host: report.relay.host, authority: captured });
      setStatus('Review this report before sharing.');
    } catch (error) {
      if (valid(token, captured)) setStatus(error instanceof DiagnosticsReportError
        ? error.code === 'unsupported_schema' ? 'This report schema is not supported. Update Beans.' : 'The diagnostics report is invalid.'
        : 'Could not generate the diagnostics report.');
    } finally {
      if (valid(token, captured)) { pending.current = false; setBusy(false); }
    }
  }
  async function share(copy: boolean) {
    if (!review || pending.current || !valid(generation.current, review.authority)) return;
    const token = generation.current, captured = review.authority, json = review.json;
    pending.current = true; setBusy(true);
    try {
      // Both actions use exactly the bytes displayed, admitted under captured account authority.
      if (copy) await Clipboard.setStringAsync(json);
      else await Share.share({ message: json });
      if (valid(token, captured)) setStatus(copy ? 'Report copied.' : 'Review this report before sharing.');
    } catch {
      if (valid(token, captured)) setStatus(copy ? 'Could not copy the report.' : 'Could not share the report.');
    } finally {
      if (valid(token, captured)) { pending.current = false; setBusy(false); }
    }
  }
  return <>
    <Stack.Screen options={{ title: t('Diagnostics') }} />
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
      <Section footer={t('Generate requests only the local core report. The core may check public relay health. Nothing is uploaded automatically.')}>
        <Button title={t('Generate Report')} disabled={busy} onPress={() => void generate()} />
      </Section>
      <Section footer={t('Relay host and port are disclosed, including private hosts. Providers are configured-only, plugins are cached setup-only, and Runners show presence, not work readiness. A busy port does not prove service failure.')}>
        <View style={{ padding: 16 }}>
          <Text accessibilityLiveRegion="polite" style={{ color: p.label }}>{status ? t(status) : t('Generate a report to review sanitized setup evidence.')}</Text>
          {review && <Text style={{ color: p.secondaryLabel, marginTop: 12 }}>{review.host
            ? t('Disclosed relay host/port: {host}', { host: review.host }) : t('No relay host is disclosed.')}</Text>}
        </View>
      </Section>
      {review && <Section title={t('Report Review')}>
        <Text selectable accessibilityLabel={t('Sanitized diagnostics report')} style={{ color: p.label, padding: 16, fontFamily: 'monospace' }}>{review.json}</Text>
        <Button title={t('Copy Report')} disabled={busy} onPress={() => void share(true)} />
        <Button title={t('Share Report')} disabled={busy} onPress={() => void share(false)} />
      </Section>}
    </ScrollView>
  </>;
}
