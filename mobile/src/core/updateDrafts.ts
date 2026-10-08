import type { PickedFile } from "./engine";
import type { Bot } from "./model";

export interface ComposerDraft {
  text: string;
  attachments: PickedFile[];
  mentions: Bot[];
  listening: boolean;
}
const empty: ComposerDraft = { text: "", attachments: [], mentions: [], listening: false };
// Stable account/relay/chat ownership retains the content, not an orphaned dirty flag.
const drafts = new Map<string, ComposerDraft>();
const listeners = new Set<() => void>();
export function composerDraftKey(identity: string | null, relay: string | null, chat: string): string {
  return JSON.stringify([identity, relay, chat]);
}
export function readComposerDraft(key: string): ComposerDraft { return drafts.get(key) ?? empty; }
export function writeComposerDraft(key: string, patch: Partial<ComposerDraft>): void {
  const next = { ...readComposerDraft(key), ...patch };
  if (next.text.length || next.attachments.length || next.mentions.length || next.listening) drafts.set(key, next);
  else drafts.delete(key);
  listeners.forEach(listener => listener());
}
export function subscribeComposerDrafts(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function hasUpdateDrafts(): boolean {
  return [...drafts.values()].some(draft => !!draft.text.trim() || draft.attachments.length > 0 || draft.listening);
}
