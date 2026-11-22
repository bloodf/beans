import { afterEach, expect, test, vi } from "vitest";
vi.mock("expo-localization", () => ({ getLocales: () => [{ languageCode: "en" }] }));
vi.mock("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
const { setAppLanguage } = await import("../i18n");
const { alwaysAllowNote } = await import("./alwaysAllow");
afterEach(() => setAppLanguage("en"));

test("a secret's grant is worded as a grant, never as an Auto-review rule", () => {
  const grant = { rule: "always run this exact command here with GITHUB_TOKEN", grant: true };
  expect(alwaysAllowNote(grant, true)).toBe("Always allow grants this from now on: “always run this exact command here with GITHUB_TOKEN”.");
  expect(alwaysAllowNote(grant, false, "always")).toBe("Allowed always. Manage it with the secrets tool.");
  expect(alwaysAllowNote(grant, false, "allowed")).toBeUndefined();
  setAppLanguage("zh");
  expect(alwaysAllowNote(grant, false, "always")).toBe("已始终允许。可用密钥工具管理。");
});

test("an Auto-review rule, and a card from an older Runner, read as before", () => {
  const rule = { rule: "Allow cargo test in this project" };
  expect(alwaysAllowNote(rule, true)).toBe("Always allow adds the rule “Allow cargo test in this project”.");
  expect(alwaysAllowNote(rule, false, "always")).toBe("Added the rule “Allow cargo test in this project” to Auto-review.");
  expect(alwaysAllowNote({ rule: "x", grant: false }, false, "denied")).toBeUndefined();
  expect(alwaysAllowNote({}, true)).toBeUndefined();
});
