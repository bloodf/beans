import type { PickedFile } from "./engine";
import type { Bot } from "./model";

export interface ComposerReply { messageID: string; name: string; text: string }
export interface ComposerDraft {
  text: string; attachments: PickedFile[]; mentions: Bot[]; reply: ComposerReply | null;
  revision: number; pending: object | null; recording: object | null; uncertain: boolean; invalidated: boolean;
  permission: object | null;
  listeners: Set<() => void>;
}
const drafts = new Map<string, ComposerDraft>();
const key = (identity: string | null, relay: string | null, chat: string) => JSON.stringify([identity, relay, chat]);
export function composerDraft(identity: string | null, relay: string | null, chat: string): ComposerDraft {
  const id = key(identity, relay, chat);
  let draft = drafts.get(id);
  if (!draft) {
    draft = { text: "", attachments: [], mentions: [], reply: null, revision: 0, pending: null, recording: null, permission: null, uncertain: false, invalidated: false, listeners: new Set() };
    drafts.set(id, draft);
  }
  return draft;
}
export function notifyComposerDraft(draft: ComposerDraft) { for (const listener of draft.listeners) listener(); }
export function releaseComposerDraft(draft: ComposerDraft) {
  if (draft.listeners.size || draft.pending || draft.recording || draft.permission || draft.text || draft.attachments.length || draft.reply) return;
  for (const [id, owner] of drafts) if (owner === draft) drafts.delete(id);
}
export function editComposerDraft(draft: ComposerDraft, patch: Partial<Pick<ComposerDraft, "text" | "attachments" | "mentions" | "reply">>) {
  if (draft.invalidated) return;
  Object.assign(draft, patch);
  if (patch.text === "") draft.mentions = [];
  draft.revision++;
  notifyComposerDraft(draft);
  releaseComposerDraft(draft);
}
export function invalidateComposerSends() {
  for (const draft of drafts.values()) if (draft.pending || draft.recording || draft.permission) {
    draft.uncertain = true;
    // Sticky revision change fences source transitions away and back.
    draft.revision++;
    draft.permission = null;
    notifyComposerDraft(draft);
  }
}
function discard(draft: ComposerDraft) {
  draft.invalidated = true;
  draft.text = ""; draft.attachments = []; draft.mentions = []; draft.reply = null;
  draft.permission = null;
  draft.recording = null;
  draft.revision++;
  notifyComposerDraft(draft);
}
export function clearComposerDrafts() {
  for (const draft of drafts.values()) discard(draft);
  drafts.clear();
}
export function removeComposerDraft(identity: string | null, relay: string | null, chat: string) {
  const id = key(identity, relay, chat);
  const draft = drafts.get(id);
  if (draft) discard(draft);
  drafts.delete(id);
}
export async function submitComposerDraft(draft: ComposerDraft, send: (text: string, files: PickedFile[], mentions: string[], replyTo?: string) => Promise<boolean>) {
  if (draft.invalidated || draft.pending || draft.recording || draft.permission || (!draft.text.trim() && !draft.attachments.length)) return;
  const operation = {};
  const revision = draft.revision;
  const text = draft.text;
  const mentions = draft.mentions.filter(bot => text.toLowerCase().includes(`@${bot.name.toLowerCase()}`)).map(bot => bot.id);
  draft.pending = operation;
  draft.uncertain = false;
  notifyComposerDraft(draft);
  try {
    const acknowledged = await send(text, draft.attachments, mentions, draft.reply?.messageID);
    if (!acknowledged) draft.uncertain = true;
    else if (!draft.invalidated && draft.revision === revision) {
      editComposerDraft(draft, { text: "", attachments: [], mentions: [], reply: null });
    }
  } catch {
    draft.uncertain = true;
  } finally {
    if (draft.pending === operation) draft.pending = null;
    notifyComposerDraft(draft);
    releaseComposerDraft(draft);
  }
}
