import { readFileSync } from "node:fs";
import { expect, test } from 'vitest';
import { memoryLookupKeys } from './l10n-lookups.ts';
const model = 'export const MEMORY_OPERATION_LABELS: Record<State, string> = { queued: "Queued", completed: "Stored" };';
test('only a finite map translated by its consumer contributes keys', () => {
  expect([...memoryLookupKeys(model, 'L(MEMORY_OPERATION_LABELS[op.state])')]).toEqual(['Queued', 'Stored']);
  expect([...memoryLookupKeys(model, 'show(MEMORY_OPERATION_LABELS[op.state])')]).toEqual([]);
  expect([...memoryLookupKeys(model + '\nconst unused = "Unused";', 'L(MEMORY_OPERATION_LABELS[op.state])')]).not.toContain('Unused');
});
test('comments, source samples and dynamic maps do not satisfy unused translations', () => {
  expect([...memoryLookupKeys(model, '// L(MEMORY_OPERATION_LABELS[state])\nconst sample = "L(MEMORY_OPERATION_LABELS[state])";')]).toEqual([]);
  expect([...memoryLookupKeys('/* ' + model + ' */', 'L(MEMORY_OPERATION_LABELS[state])')]).toEqual([]);
  expect([...memoryLookupKeys(model.replace('"Stored"', 'runtimeLabel'), 'L(MEMORY_OPERATION_LABELS[state])')]).toEqual([]);
});

test('production indexed health labels are discovered through their real consumer', () => {
  const model = readFileSync(new URL('../desktop/src/model/memoryService.ts', import.meta.url), 'utf8');
  const view = readFileSync(new URL('../desktop/src/ui/sheets/memoryService.tsx', import.meta.url), 'utf8');
  const keys = memoryLookupKeys(model, view);
  expect([...keys]).toEqual(['Ready', 'Degraded', 'Setup required', 'Queued, not stored', 'Submitted, not stored', 'Processing, not stored', 'Stored', 'Failed, not stored', 'Delivery unknown, not confirmed stored']);
});
