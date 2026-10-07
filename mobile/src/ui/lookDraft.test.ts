import { describe, expect, test } from "bun:test";
import type { PickedFile } from "../core/engine";
import { emptyLook, setField, type BotLook } from "../core/look";
import { isDirty, resetScope, restoreDefaults, savePayload, shuffle, startDraft } from "./lookDraft";

const saved: BotLook = { version: 1, base: { shape: "sun", expression: "happy" }, states: { error: { expression: "sad" } } };
const photo: PickedFile = { uri: "file:///a.jpg", name: "a.jpg", mime: "image/jpeg", width: 512, height: 512 };

describe("Save and Cancel", () => {
  test("a fresh draft is clean and saves nothing", () => {
    const draft = startDraft(saved);
    expect(isDirty(draft, saved)).toBe(false);
    expect(savePayload(draft, saved)).toEqual({});
  });

  test("an unsaved bot starts from the seeded default", () => {
    const draft = startDraft(undefined);
    expect(draft.look).toEqual(emptyLook());
    expect(isDirty(draft, undefined)).toBe(false);
  });

  test("an edit is dirty and saves the whole look once, leaving the photo alone", () => {
    const draft = { ...startDraft(saved), look: setField(saved, "base", "shape", "cloud") };
    expect(isDirty(draft, saved)).toBe(true);
    const payload = savePayload(draft, saved);
    expect(payload).toEqual({ look: { ...saved, base: { shape: "cloud", expression: "happy" } } });
    expect("photo" in payload).toBe(false);
  });

  test("editing back to what is saved is clean again", () => {
    const edited = setField(saved, "base", "shape", "cloud");
    const back = setField(edited, "base", "shape", "sun");
    expect(isDirty({ ...startDraft(saved), look: back }, saved)).toBe(false);
  });

  test("Cancel is not saving: the draft never touches the saved look", () => {
    const draft = shuffle(startDraft(saved), () => 0.5);
    expect(saved).toEqual({ version: 1, base: { shape: "sun", expression: "happy" }, states: { error: { expression: "sad" } } });
    expect(isDirty(draft, saved)).toBe(true);
  });
});

describe("photo changes wait for Save with the look", () => {
  test("a photo change alone is dirty and saves only the photo", () => {
    const draft = { ...startDraft(saved), photo: { kind: "replace" as const, file: photo } };
    expect(isDirty(draft, saved)).toBe(true);
    expect(savePayload(draft, saved)).toEqual({ photo });
  });

  test("removing the photo saves null and keeps the generated look", () => {
    const draft = { ...startDraft(saved), photo: { kind: "remove" as const } };
    expect(savePayload(draft, saved)).toEqual({ photo: null });
  });

  test("a look edit with a photo change goes in one payload", () => {
    const draft = { ...startDraft(saved), look: setField(saved, "base", "shape", "boxy"), photo: { kind: "replace" as const, file: photo } };
    expect(Object.keys(savePayload(draft, saved)).sort()).toEqual(["look", "photo"]);
  });
});

describe("Shuffle and Reset", () => {
  test("shuffle changes the look, keeps scope and the photo change", () => {
    const draft = { ...startDraft(saved), scope: "error" as const, photo: { kind: "remove" as const } };
    const next = shuffle(draft, () => 0.3);
    expect(next.scope).toBe("error");
    expect(next.photo).toEqual({ kind: "remove" });
    expect(next.look.base.expression).toBe("happy");
    expect(next.look.base.shape).not.toBe(undefined);
  });

  test("restore defaults saves a reset (null) when a look was saved, nothing when none was", () => {
    expect(savePayload(restoreDefaults(startDraft(saved)), saved)).toEqual({ look: null });
    expect(savePayload(restoreDefaults(startDraft(undefined)), undefined)).toEqual({});
  });

  test("reset on a state drops only that state; on the base it restores defaults", () => {
    const inState = resetScope({ ...startDraft(saved), scope: "error" });
    expect(inState.look).toEqual({ version: 1, base: { shape: "sun", expression: "happy" } });
    expect(resetScope(startDraft(saved)).look).toEqual(emptyLook());
  });

  test("restore defaults keeps a pending photo change", () => {
    const draft = restoreDefaults({ ...startDraft(saved), photo: { kind: "replace", file: photo } });
    expect(draft.photo).toEqual({ kind: "replace", file: photo });
  });
});
