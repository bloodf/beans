import { afterEach, expect, test, vi } from "vitest";
vi.mock("expo-localization", () => ({ getLocales: () => [{ languageCode: "en" }] }));
vi.mock("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
const { setAppLanguage } = await import("../i18n");
const { grantSubtitle, grantTitle, secretSummary } = await import("./secretsText");
afterEach(() => setAppLanguage("en"));

// Issue #560: the Secrets screen's words, never a value.
test("secrets and grants read in English and Chinese", () => {
  const secret = { name: "K", length: 40, source: "clipboard", note: "Google OAuth client", saved_by: "Chef", updated_at: 1 };
  expect(secretSummary(secret)).toBe("40 characters · clipboard · Google OAuth client · saved by Chef");
  const command = { kind: "command" as const, folder: "/srv/app", command: "vercel env add K {{secret:K}}", secrets: ["K"], granted_at: 0 };
  expect(grantTitle(command)).toBe("$ vercel env add K {{secret:K}}");
  expect(grantSubtitle(command)).toBe("in /srv/app with K");
  expect(grantTitle({ kind: "tool", tool: "create_charge", secrets: ["A", "B"], granted_at: 0 })).toBe("create_charge receives A, B");
  setAppLanguage("zh");
  expect(secretSummary({ ...secret, note: "", saved_by: "" })).toBe("40 个字符 · clipboard");
});
