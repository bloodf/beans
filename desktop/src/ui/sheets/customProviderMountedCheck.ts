// Mounted consumer checks for the custom provider sheet's Refresh Models path. Served by
// `scripts/check-desktop-provider-refresh-consumer.py`, which opens this page in a real Chromium and
// fails on any error. The check renders the shipped sheet against a synthetic transport: the mounted
// production consumer is under test, never a copy of it, and no Runner or provider is contacted.
import { checkCustomProviderRefresh } from "./customProvider.test";

declare global { interface Window { __result?: unknown; __error?: string } }

const CHECKS: [string, () => Promise<unknown>][] = [["refresh", checkCustomProviderRefresh]];

void (async () => {
  const results: Record<string, unknown> = {};
  for (const [name, check] of CHECKS) {
    try { results[name] = await check(); } catch (error) { window.__error = `${name}: ${String((error as Error)?.message ?? error)}`; return; }
  }
  window.__result = results;
})();