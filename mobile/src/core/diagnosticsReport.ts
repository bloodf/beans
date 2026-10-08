// Decode into a new recursive allowlist; never share the native response object.
export class DiagnosticsReportError extends Error {
  constructor(readonly code: 'unsupported_schema' | 'invalid_report') { super(code); }
}
function invalid(): never { throw new DiagnosticsReportError('invalid_report'); }
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}
function bool(value: unknown): boolean { return typeof value === 'boolean' ? value : invalid(); }
function count(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : invalid();
}
function choice<T extends string>(value: unknown, values: readonly T[]): T {
  return typeof value === 'string' && values.includes(value as T) ? value as T : invalid();
}
function release(value: unknown): string {
  return typeof value === 'string' && !/\s/.test(value) && /^[0-9]{1,10}\.[0-9]{1,10}\.[0-9]{1,10}$/.test(value) ? value : invalid();
}
function nullable<T>(value: unknown, decode: (value: unknown) => T): T | null {
  return value == null ? null : decode(value);
}
function array<T>(value: unknown, decode: (value: unknown) => T): T[] {
  return Array.isArray(value) ? value.map(decode) : invalid();
}
function host(value: unknown): string {
  if (typeof value !== 'string' || /\s/.test(value) || !/^(?:[A-Za-z0-9._-]+|\[[0-9A-Fa-f:]+\])(?::[0-9]{1,5})?$/.test(value)) return invalid();
  try {
    const url = new URL(`https://${value}`);
    if (!url.hostname || url.username || url.password || url.pathname !== '/' || url.search || url.hash) return invalid();
  } catch { return invalid(); }
  return value;
}
const os = ['macos', 'linux', 'windows', 'ios', 'ipados', 'android', 'unknown'] as const;
export function decodeDiagnosticsReport(value: unknown) {
  const root = object(value);
  const schema = count(root.schema_version);
  if (schema !== 1) throw new DiagnosticsReportError('unsupported_schema');
  const versions = object(root.versions), device = object(root.this_device);
  const relay = object(root.relay), providers = object(root.providers);
  const plugins = object(root.plugins), mcp = object(root.mcp_json);
  return {
    schema_version: 1,
    versions: { core: release(versions.core), relay_protocol: count(versions.relay_protocol) },
    this_device: { os: choice(device.os, os), is_runner: bool(device.is_runner), has_identity: bool(device.has_identity) },
    home: { exists: bool(object(root.home).exists) },
    port: { free: bool(object(root.port).free) },
    relay: {
      configured: bool(relay.configured), host: nullable(relay.host, host), reachable: nullable(relay.reachable, bool),
      reason: choice(relay.reason, ['none', 'not_configured', 'invalid_url', 'unreachable', 'update_required', 'http_error']),
      http_status: nullable(relay.http_status, count), protocol: nullable(relay.protocol, count),
    },
    providers: {
      built_in: array(providers.built_in, value => {
        const row = object(value);
        return { kind: choice(row.kind, ['deepseek', 'anthropic', 'opencode', 'opencode-go', 'chatgpt', 'grok']), configured: bool(row.configured) };
      }),
      custom_configured: count(providers.custom_configured), health: choice(providers.health, ['not_checked']),
    },
    plugins: {
      basis: choice(plugins.basis, ['cached_setup_state']),
      entries: array(plugins.entries, value => {
        const row = object(value), slot = count(row.slot);
        if (!slot) return invalid();
        return { slot, state: choice(row.state, ['ready', 'needs_setup', 'needs_auth', 'connecting', 'error', 'unknown']) };
      }),
    },
    mcp_json: { servers: count(mcp.servers), problems: count(mcp.problems), file_error: bool(mcp.file_error) },
    runners: array(root.runners, value => {
      const row = object(value);
      return { is_this_device: bool(row.is_this_device), os: choice(row.os, ['macos', 'linux', 'windows']),
        presence: choice(row.presence, ['online', 'offline']), version: nullable(row.version, release) };
    }),
  };
}
