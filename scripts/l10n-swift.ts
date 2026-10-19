// Additional finite key expressions used by the native memory form helpers.
// This does not infer arbitrary Swift variables or count every string in a file.
function parts(source: string, delimiter: string): string[] {
  const result: string[] = []; let start = 0, depth = 0, ternary = 0, quoted = false, escaped = false;
  for (let i = 0; i < source.length; i++) {
    const c = source[i]!;
    if (quoted) { if (escaped) escaped = false; else if (c === "\\") escaped = true; else if (c === '"') quoted = false; continue; }
    if (c === '"') quoted = true;
    else if ("([{".includes(c)) depth++;
    else if (c === "?" && source[i + 1] !== "?" && source[i - 1] !== "?" && source[i + 1] !== ".") ternary++;
    else if (c === ":" && ternary) ternary--;
    else if (")]}".includes(c)) depth--;
    if (c === delimiter && depth === 0 && ternary === 0) { result.push(source.slice(start, i)); start = i + 1; }
  }
  result.push(source.slice(start)); return result;
}
function literals(expression: string): string[] {
  const value = expression.trim();
  if (value.startsWith("(") && value.endsWith(")")) return literals(value.slice(1, -1));
  if (/^"(?:[^"\\]|\\.)*"$/.test(value)) {
    if (value.includes("\\(")) return []; // runtime interpolation is not a finite key
    return [value.slice(1, -1).replace(/\\([ntr"\\])/g, (_, c) => (({ n: "\n", t: "\t", r: "\r" } as Record<string, string>)[c] ?? c))];
  }
  // A conditional chooses its result branches, not strings in the condition.
  let depth = 0, quoted = false, escaped = false, question = -1, nested = 0;
  for (let i = 0; i < value.length; i++) {
    const c = value[i]!;
    if (quoted) { if (escaped) escaped = false; else if (c === "\\") escaped = true; else if (c === '"') quoted = false; continue; }
    if (c === '"') quoted = true;
    else if ("([{".includes(c)) depth++;
    else if (")]}".includes(c)) depth--;
    else if (depth === 0 && c === "?" && value[i + 1] !== "?" && value[i - 1] !== "?" && value[i + 1] !== ".") { if (question < 0) question = i; else nested++; }
    else if (depth === 0 && c === ":" && question >= 0) { if (nested) nested--; else return [...literals(value.slice(question + 1, i)), ...literals(value.slice(i + 1))]; }
  }
  return [];
}
export function memorySwiftKeys(source: string, path: string): Set<string> {
  const keys = new Set<string>();
  const memoryForm = new Set([
    "Settings/MemoryFormViewController.swift", "Settings/MemoryConnectionViewController.swift",
    "Settings/MemoryEmbeddingViewController.swift", "Settings/MemoryLocalAssetsViewController.swift",
    "Settings/MemoryBotSetupViewController.swift", "Settings/MemorySettingsViewController.swift",
    "Sheets/MemoryAdvancedViewController.swift", "Sheets/BotMemoryServiceViewController.swift",
  ]).has(path);
  // Strip comments without removing strings, so commented-out calls stay unused.
  source = source.replace(/"(?:[^"\\]|\\.)*"|\/\/[^\n]*|\/\*[\s\S]*?\*\//g, token => token.startsWith('"') ? token : " ".repeat(token.length));
  const call = /\b(L|MemoryActionButton|heading|note|row|text|popup|toggle|confirm|optionRow|addRow|addText|Field)\s*\(/g;
  for (const match of source.matchAll(call)) {
    const name = match[1]!;
    if (name !== "MemoryActionButton" && !memoryForm) continue;
    const start = match.index! + match[0].length;
    let depth = 1, quoted = false, escaped = false, end = start;
    for (; end < source.length && depth; end++) {
      const c = source[end]!;
      if (quoted) { if (escaped) escaped = false; else if (c === "\\") escaped = true; else if (c === '"') quoted = false; continue; }
      if (c === '"') quoted = true; else if (c === "(") depth++; else if (c === ")") depth--;
    }
    if (depth) continue;
    const args = parts(source.slice(start, end - 1), ",");
    const first = name === "Field" ? args[1] : args[0];
    if (name === "L" && args[1]?.trim().startsWith("context:")) {
      const contexts = literals(args[1].replace(/^\s*context\s*:\s*/, ""));
      for (const key of literals(first ?? "")) for (const context of contexts) keys.add(`${key}|${context}`);
    } else if (first) for (const key of literals(first)) keys.add(key);
    if (name === "popup") {
      const choices = args.find(arg => /^\s*choices\s*:/.test(arg))?.replace(/^\s*choices\s*:\s*/, "");
      if (choices?.startsWith("[") && choices.endsWith("]")) for (const option of parts(choices.slice(1, -1), ",")) for (const key of literals(option)) keys.add(key);
    }
    if (name === "text") {
      const placeholder = args.find(arg => /^\s*placeholder\s*:/.test(arg))?.replace(/^\s*placeholder\s*:\s*/, "");
      if (placeholder) for (const key of literals(placeholder)) keys.add(key);
    }
  }
  return keys;
}
