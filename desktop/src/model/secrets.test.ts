// Issue #560: a Runner's secrets and grants as the Secrets pane words them.

import { expect, test } from "vitest";
import { grantDetail, grantTitle, secretSummary, validSecretName } from "./secrets";

test("secrets read as their length, source, note and saver, never a value", () => {
  expect(secretSummary({ name: "K", length: 40, source: "clipboard", note: "Google OAuth client", saved_by: "Chef", updated_at: 1 })).toBe("40 characters · clipboard · Google OAuth client · saved by Chef");
  expect(secretSummary({ name: "K", length: 8, source: "typed in Beans on Studio", note: "", saved_by: "", updated_at: 1 })).toBe("8 characters · typed in Beans on Studio");
});

test("grants read as what they let through and where", () => {
  expect(grantTitle({ kind: "tool", tool: "create_charge", secrets: ["STRIPE_KEY", "OTHER"], granted_at: 0 })).toBe("create_charge receives STRIPE_KEY, OTHER");
  const command = { kind: "command" as const, folder: "/srv/app", command: "vercel env add K {{secret:K}}", secrets: ["K"], granted_at: 0 };
  expect(grantTitle(command)).toBe("$ vercel env add K {{secret:K}}");
  expect(grantDetail(command)).toBe("in /srv/app with K");
});

test("names are the store's: an environment variable's", () => {
  for (const good of ["STRIPE_KEY", "_X", "a1"]) expect(validSecretName(good)).toBe(true);
  for (const bad of ["", "1A", "A-B", "has space", "A".repeat(65)]) expect(validSecretName(bad)).toBe(false);
});
