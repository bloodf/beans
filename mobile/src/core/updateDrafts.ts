// Keep unsent draft blockers after navigation: a hidden composer must not authorize replacement.
const drafts = new Set<object>();
export function markUpdateDraft(owner: object, dirty: boolean): void {
  if (dirty) drafts.add(owner); else drafts.delete(owner);
}
export function hasUpdateDrafts(): boolean { return drafts.size > 0; }
