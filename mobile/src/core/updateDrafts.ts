import { composerDraft, composerDrafts, editComposerDraft, clearComposerDrafts, removeComposerDraft, subscribeAllComposerDrafts } from "./composerDraft";
import type { PickedFile } from "./engine";
import type { Bot } from "./model";

export interface ComposerDraft {
  text: string;
  attachments: PickedFile[];
  mentions: Bot[];
  listening: boolean;
}
const empty: ComposerDraft = { text: "", attachments: [], mentions: [], listening: false };

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
  const [identity, relay, chat] = JSON.parse(key);
  removeComposerDraft(identity, relay, chat);
}
/** Only an authoritative forget clears every source retained in this process. */
export function forgetComposerDrafts(): void {
  owners.clear();
  clearComposerDrafts();
}
export function retainedComposerDrafts(): readonly [string, ComposerDraft][] { return [...composerDrafts.entries()].filter(([, draft]) => draft.text || draft.attachments.length || draft.reply || draft.pending || draft.recording).map(([key, draft]) => [key, { text: draft.text, attachments: draft.attachments, mentions: draft.mentions, listening: !!draft.recording }]); }
export function readComposerDraft(key: string): ComposerDraft { const draft = composerDrafts.get(key); return draft ? { text: draft.text, attachments: draft.attachments, mentions: draft.mentions, listening: !!draft.recording } : empty; }
export function writeComposerDraft(key: string, patch: Partial<ComposerDraft>, generation = composerDraftGeneration(key)): void {
  if (generation === null || owners.get(key) !== generation) return;
  const next = { ...readComposerDraft(key), ...patch };
  if (!next.text.trim()) next.mentions = [];
  const [identity, relay, chat] = JSON.parse(key);
  const draft = composerDraft(identity, relay, chat);
  draft.recording = next.listening ? draft.recording ?? {} : null;
  editComposerDraft(draft, { text: next.text, attachments: next.attachments, mentions: next.mentions });
}
export function subscribeComposerDrafts(listener: () => void): () => void {
  return subscribeAllComposerDrafts(listener);
}
export function hasUpdateDrafts(): boolean {
  for (const draft of composerDrafts.values()) if (draft.text.trim() || draft.attachments.length || draft.reply || draft.pending || draft.recording || draft.permission) return true;
  return false;
}
