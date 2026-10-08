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
let reachable = new Set<string>();

/** Authoritative account/roster changes discard drafts that can no longer be opened. */
export function reconcileComposerDrafts(identity: string | null, relay: string | null, chats: readonly { id: string }[]): void {
  reachable = new Set(identity === null ? [] : chats.map(chat => composerDraftKey(identity, relay, chat.id)));
  let changed = false;
  for (const key of drafts.keys()) {
    if (!reachable.has(key)) {
      drafts.delete(key);
      changed = true;
    }
  }
  if (changed) listeners.forEach(listener => listener());
}
export function composerDraftKey(identity: string | null, relay: string | null, chat: string): string {
  return JSON.stringify([identity, relay, chat]);
}
export function readComposerDraft(key: string): ComposerDraft { return drafts.get(key) ?? empty; }
export function writeComposerDraft(key: string, patch: Partial<ComposerDraft>): void {
  // Late picker/speech callbacks must not resurrect a deleted chat or forgotten account.
  if (!reachable.has(key)) return;
  const next = { ...readComposerDraft(key), ...patch };
  if (!next.text.trim()) next.mentions = [];
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
