import { expect, test } from 'bun:test';
import { memoryLookupKeys } from './l10n-lookups';
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
