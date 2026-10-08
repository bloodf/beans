import { expect, mock, test } from "bun:test";
import type { Prefs } from "./prefs";

let saved: Prefs = { app_lang: "zh", update_checks: false, update_checked_at: 123, update_skipped_version: "1.10.0" };
mock.module("./prefs", () => ({
  loadPrefs: () => ({ ...saved }),
  savePrefs: (prefs: Prefs) => { saved = { ...prefs }; },
}));
const { mutate, useStore } = await import("./store");

test("dictation changes preserve app language and update opt-out/history across preference reload", () => {
  mutate(() => ({ dictation_lang: "en-US" }));
  expect(useStore.getState().dictation_lang).toBe("en-US");
  expect(saved).toEqual({ app_lang: "zh", dictation_lang: "en-US", update_checks: false, update_checked_at: 123, update_skipped_version: "1.10.0" });
  mutate(() => ({ dictation_lang: undefined }));
  expect(saved.update_checks).toBe(false);
  expect(saved.update_skipped_version).toBe("1.10.0");
  expect(saved.app_lang).toBe("zh");
});
