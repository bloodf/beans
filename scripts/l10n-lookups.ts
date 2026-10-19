// Finite string maps used by the desktop memory presentation layer. Only a map
// passed through L(map[index]) contributes keys; unrelated constants do not.
function executable(source: string) {
  return source.replace(/\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g,
    value => ' '.repeat(value.length));
}
export function memoryLookupKeys(model: string, view: string): Set<string> {
  const used = new Set([...executable(view).matchAll(/\bL\(\s*(MEMORY_(?:STATUS|OPERATION)_LABELS)\s*\[/g)].map(match => match[1]));
  const keys = new Set<string>();
  const code = executable(model);
  for (const match of model.matchAll(/export const (MEMORY_(?:STATUS|OPERATION)_LABELS): Record<\w+, string> = \{([^}]+)\};/g)) {
    if (!used.has(match[1]) || !code.slice(match.index, match.index + 12).startsWith('export const')) continue;
    // Reject non-literal or incomplete objects instead of guessing runtime values.
    const body = match[2]!;
    const entries = [...body.matchAll(/\s*\w+\s*:\s*("(?:[^"\\]|\\.)*")\s*(?:,|$)/g)];
    if (entries.map(entry => entry[0]).join('') !== body) continue;
    for (const entry of entries) keys.add(JSON.parse(entry[1]!));
  }
  return keys;
}
