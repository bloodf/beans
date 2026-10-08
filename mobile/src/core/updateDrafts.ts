import type { PickedFile } from "./engine";
import type { Bot } from "./model";

export interface ComposerDraft {
  text: string;
  attachments: PickedFile[];
  mentions: Bot[];
  listening: boolean;
}
const empty: ComposerDraft = { text: "", attachments: [], mentions: [], listening: false };
const drafts = new Map<string, ComposerDraft>();
const listeners = new Set<() => void>();
// A new object on re-admission fences callbacks captured before deletion/forget.
const owners = new Map<string, object>();
export function composerDraftKey(identity: string | null, relay: string | null, chat: string): string {
  return JSON.stringify([identity, relay, chat]);
}
/** Snapshots admit ownership, never authorize destruction of source-keyed content. */
export function reconcileComposerDrafts(identity: string | null, relay: string | null, chats: readonly { id: string }[]): void {
  if (identity === null) return;
  for (const chat of chats) {
    const key = composerDraftKey(identity, relay, chat.id);
    if (!owners.has(key)) owners.set(key, {});
  }
}
export function composerDraftGeneration(key: string): object | null { return owners.get(key) ?? null; }
export function discardComposerDraft(key: string): void {
  owners.delete(key);
  if (drafts.delete(key)) listeners.forEach(listener => listener());
}
/** Only an authoritative forget clears every source retained in this process. */
export function forgetComposerDrafts(): void {
  owners.clear();
  drafts.clear();
  listeners.forEach(listener => listener());
}
export function retainedComposerDrafts(): readonly [string, ComposerDraft][] { return [...drafts.entries()]; }
export function readComposerDraft(key: string): ComposerDraft { return drafts.get(key) ?? empty; }
export function writeComposerDraft(key: string, patch: Partial<ComposerDraft>, generation = composerDraftGeneration(key)): void {
  if (generation === null || owners.get(key) !== generation) return;
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
  for (const draft of drafts.values()) if (draft.text.trim() || draft.attachments.length || draft.listening) return true;
  return false;
}
