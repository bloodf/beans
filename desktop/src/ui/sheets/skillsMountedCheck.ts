// Mounted consumer checks for the skills surfaces. Served by `scripts/check-desktop-skills-consumer.ts`,
// which opens this page in a real browser and fails on any error. Each check renders the shipped sheet
// against a synthetic transport: only the mounted production consumer is under test, never a copy of it.
import { checkSkillCompareConsumer, checkSkillKeepBothConsumer, checkSkillLibraryConsumer } from "./skillLibrary.test";
import { checkSkillDiscoveryConsumer } from "./skillDiscovery.test";

declare global { interface Window { __result?: unknown; __error?: string } }

const CHECKS: [string, () => Promise<unknown>][] = [
  ["discovery", checkSkillDiscoveryConsumer],
  ["library", checkSkillLibraryConsumer],
  ["keepBoth", checkSkillKeepBothConsumer],
  ["compare", checkSkillCompareConsumer],
];

void (async () => {
  const results: Record<string, unknown> = {};
  for (const [name, check] of CHECKS) {
    try { results[name] = await check(); } catch (error) { window.__error = `${name}: ${String((error as Error)?.message ?? error)}`; return; }
  }
  window.__result = results;
})();