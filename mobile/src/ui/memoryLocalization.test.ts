import { afterEach, expect, test, vi } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import ts from 'typescript';
vi.mock('../core/prefs', () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
vi.mock('expo-localization', () => ({ getLocales: () => [{ languageCode: 'en' }] }));
const { t, setAppLanguage } = await import('../i18n');
const { zh } = await import('../i18n/zh');
afterEach(() => setAppLanguage('en'));
const disclosure = 'The selected memory endpoint and its downstream extraction, embedding, reranking or reflection models may receive plaintext outside the encrypted relay. Usage can cost money; no total-cost ceiling is established by this app. This approves only the named action, not schedules or automatic retries.';
test('consent preserves English and names scope and cost limits in Chinese', () => {
  setAppLanguage('en');
  expect(t(disclosure)).toBe(disclosure);
  setAppLanguage('zh');
  expect(t(disclosure)).toBe('所选记忆端点及其下游提取、嵌入、重排序或反思模型可能在加密中继之外接收明文。使用可能产生费用；此应用未设定总费用上限。此批准仅针对指定操作，不包括计划任务或自动重试。');
  expect(t('Clear this bot’s entire bank?')).toBe('清空此机器人的整个记忆库？');
  expect(t('Delete this document?')).toBe('删除此文档？');
  expect(t('Run {v0}', { v0: 'retain' })).toBe('运行 retain');
  setAppLanguage('en');
  expect(t('Run {v0}', { v0: 'retain' })).toBe('Run retain');
});
test('memory presentation keys have Chinese translations with identical placeholders', () => {
  const dirs = ['app/chat-info/memory', 'app/settings/memory'];
  const paths = dirs.flatMap(dir => readdirSync(resolve(dir)).filter(f => f.endsWith('.tsx')).map(f => resolve(dir, f)));
  paths.push(resolve('src/ui/memoryUI.tsx'));
  let checked = 0;
  for (const path of paths) {
    const source = ts.createSourceFile(path, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    function visit(node: ts.Node) {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 't' && node.arguments[0] && ts.isStringLiteral(node.arguments[0])) {
        const key = node.arguments[0].text;
        expect(zh[key], `${path}: ${key}`).toBeTruthy();
        expect(zh[key].match(/\{\w+\}/g)?.sort() ?? []).toEqual(key.match(/\{\w+\}/g)?.sort() ?? []);
        checked++;
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
  expect(checked).toBeGreaterThan(200);
});
