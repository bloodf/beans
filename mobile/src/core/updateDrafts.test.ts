import { expect, test } from "bun:test";
import { hasUpdateDrafts, markUpdateDraft } from "./updateDrafts";

test("one cleared composer cannot clear another unsent draft", () => {
  const a = {}, b = {};
  markUpdateDraft(a, true); markUpdateDraft(b, true);
  markUpdateDraft(a, false);
  expect(hasUpdateDrafts()).toBe(true);
  markUpdateDraft(b, false);
  expect(hasUpdateDrafts()).toBe(false);
});
