// The Look editor's draft: the look being edited, which scope (base or one activity state) the
// controls address, and a pending photo change. Nothing reaches the account until Save; Cancel
// drops the draft. Pure, so the Save/Cancel/Shuffle/Reset rules run under bun test.

import type { PickedFile } from "../core/engine";
import { emptyLook, lookToWire, resetState, sameLook, sanitizeLook, shuffleLook, type BotLook, type Scope } from "../core/look";

/// Photo changes wait for Save like everything else: keep, remove, or replace with a picked file.
export type PhotoChange = { kind: "keep" } | { kind: "remove" } | { kind: "replace"; file: PickedFile };

export interface LookDraft {
  look: BotLook;
  scope: Scope;
  photo: PhotoChange;
}

/// A draft from the bot as saved. Cancel is simply not saving this.
export function startDraft(saved: BotLook | undefined): LookDraft {
  return { look: sanitizeLook(saved) ?? emptyLook(), scope: "base", photo: { kind: "keep" } };
}

/// Changed from what is saved: the look means something different, or a photo change is pending.
export function isDirty(draft: LookDraft, saved: BotLook | undefined): boolean {
  return draft.photo.kind !== "keep" || !sameLook(draft.look, saved);
}

/// Shuffle: new shape, hue, and tone in the base; the photo change and scope stay.
export function shuffle(draft: LookDraft, random?: () => number): LookDraft {
  return { ...draft, look: shuffleLook(draft.look, random) };
}

/// Restore seeded defaults: no customization at all. The photo change stays as it is, since a
/// photo is independent of the generated look.
export function restoreDefaults(draft: LookDraft): LookDraft {
  return { ...draft, look: emptyLook() };
}

/// Back to inheriting the base: the open state's overrides go. The base has nothing to inherit,
/// so there this is Restore.
export function resetScope(draft: LookDraft): LookDraft {
  return draft.scope === "base" ? restoreDefaults(draft) : { ...draft, look: resetState(draft.look, draft.scope) };
}

/// What `saveBotAppearance` takes: only what changed. The look goes as null when it means the
/// seeded default, which resets generated customization and leaves the photo alone.
export function savePayload(draft: LookDraft, saved: BotLook | undefined): { look?: BotLook | null; photo?: PickedFile | null } {
  const out: { look?: BotLook | null; photo?: PickedFile | null } = {};
  if (!sameLook(draft.look, saved)) out.look = lookToWire(draft.look);
  if (draft.photo.kind === "remove") out.photo = null;
  else if (draft.photo.kind === "replace") out.photo = draft.photo.file;
  return out;
}
