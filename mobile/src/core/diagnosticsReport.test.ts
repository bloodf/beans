import { expect, test } from 'bun:test';
import { decodeDiagnosticsReport, DiagnosticsReportError } from './diagnosticsReport';

test('production diagnostics decoder preserves schema payload and rejects private or invalid evidence', () => {
  const full = {
    schema_version: 1, versions: { core: '0.1.2', relay_protocol: 5 },
    this_device: { os: 'android', is_runner: false, has_identity: true }, home: { exists: true }, port: { free: false },
    relay: { configured: true, host: '[::1]:4874', reachable: true, reason: 'http_error', http_status: 503, protocol: null },
    providers: { built_in: ['deepseek', 'anthropic', 'opencode', 'opencode-go', 'chatgpt', 'grok'].map(kind => ({ kind, configured: true })), custom_configured: 2, health: 'not_checked' },
    plugins: { basis: 'cached_setup_state', entries: ['ready', 'needs_setup', 'needs_auth', 'connecting', 'error', 'unknown'].map((state, i) => ({ slot: i + 1, state })) },
    mcp_json: { servers: 3, problems: 1, file_error: true },
    runners: [{ is_this_device: false, os: 'linux', presence: 'offline', version: '1.2.3' }],
  };
  expect(JSON.parse(JSON.stringify(decodeDiagnosticsReport(full)))).toEqual(full);
  const empty = structuredClone(full);
  Object.assign(empty.relay, { configured: false, host: null, reachable: null, reason: 'not_configured', http_status: null, protocol: null });
  Object.assign(empty.runners[0]!, { version: null });
  expect(JSON.parse(JSON.stringify(decodeDiagnosticsReport(empty)))).toEqual(empty);
  const secret = 'PRIVATE_SECRET_SENTINEL';
  function extend(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(extend);
    if (value && typeof value === 'object') return { ...Object.fromEntries(Object.entries(value).map(([k, v]) => [k, extend(v)])), secret };
    return value;
  }
  const sanitized = decodeDiagnosticsReport(extend(full));
  expect(sanitized).toEqual(full);
  expect(JSON.stringify(sanitized)).not.toContain(secret);
  function reject(change: (value: any) => void, code = 'invalid_report') {
    const input = structuredClone(full); change(input);
    try { decodeDiagnosticsReport(input); throw new Error('accepted invalid report'); }
    catch (error) {
      expect(error).toBeInstanceOf(DiagnosticsReportError);
      expect((error as DiagnosticsReportError).code).toBe(code);
      expect(String(error)).not.toContain(secret);
    }
  }
  reject(v => { v.schema_version = 2; }, 'unsupported_schema');
  for (const field of Object.keys(full)) {
    reject(v => { delete v[field]; });
    reject(v => { v[field] = null; });
  }
  for (const url of [`https://relay.test/${secret}`, `user:${secret}@relay.test`, `relay.test/${secret}`, `relay.test?key=${secret}`, `relay.test#${secret}`, `relay.test\\${secret}`, `relay.test\n${secret}`, 'relay.test:99999']) reject(v => { v.relay.host = url; });
  for (const host of ['relay.test', '192.168.1.2:4874', '[2001:db8::1]:4874']) {
    const input = structuredClone(full); input.relay.host = host;
    expect(decodeDiagnosticsReport(input).relay.host).toBe(host);
  }
  for (const reason of ['none', 'not_configured', 'invalid_url', 'unreachable', 'update_required', 'http_error']) {
    const input = structuredClone(full); input.relay.reason = reason;
    expect(decodeDiagnosticsReport(input).relay.reason).toBe(reason);
  }
  for (const change of [
    (v: any) => { v.relay.reason = secret; }, (v: any) => { v.this_device.os = secret; },
    (v: any) => { v.providers.built_in[0].kind = secret; }, (v: any) => { v.providers.health = secret; },
    (v: any) => { v.plugins.basis = secret; }, (v: any) => { v.plugins.entries[0].state = secret; },
    (v: any) => { v.runners[0].presence = secret; }, (v: any) => { v.runners[0].os = 'ios'; },
    (v: any) => { v.versions.core = secret; }, (v: any) => { v.runners[0].version = `1.2.3-${secret}`; },
    (v: any) => { v.providers.custom_configured = -1; }, (v: any) => { v.mcp_json.servers = 1.5; },
    (v: any) => { v.plugins.entries[0].slot = 0; }, (v: any) => { v.relay.reachable = secret; },
    (v: any) => { v.home.exists = null; }, (v: any) => { v.providers.built_in[0].configured = null; },
    (v: any) => { v.relay.protocol = Number.MAX_SAFE_INTEGER + 1; },
    (v: any) => { v.versions.core = '1.2.3\n'; }, (v: any) => { v.relay.host = 'relay.test\n'; },
  ]) reject(change);
  expect(() => decodeDiagnosticsReport({ error: { message: secret } })).toThrow(DiagnosticsReportError);
});
