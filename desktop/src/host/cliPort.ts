import { L } from "../l10n";

/** Saved/native preferences must carry the matching numeric build port. */
export function requireCLIPort(value: unknown, expected: number): number {
  if ((expected !== 4874 && expected !== 4875) || value !== expected) {
    throw new Error(L("This build connects only to its isolated Beans CLI port %@.", String(expected)));
  }
  return expected;
}

/** Query/UI text is accepted only when its numeric port matches this build. */
export function parseCLIPort(text: string, expected: number): number {
  return requireCLIPort(Number(text), expected);
}

export function browserCLIPort(saved: unknown, query: URLSearchParams, expected: number): number {
  requireCLIPort(saved, expected);
  for (const explicit of query.getAll("port")) parseCLIPort(explicit, expected);
  return expected;
}
