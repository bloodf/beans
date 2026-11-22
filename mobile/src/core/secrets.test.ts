import { expect, test } from "vitest";
import { readSecretsListing, validSecretName } from "./secrets";

// Issue #560: what the Runner answers is read only in the shape the screen expects.
test("a listing keeps well-formed secrets and grants and drops the rest", () => {
  const listing = readSecretsListing({
    secrets: [{ name: "STRIPE_KEY", length: 35, source: "clipboard", note: "", saved_by: "Chef", updated_at: 1 }, { name: 3 }, null],
    grants: [{ kind: "tool", tool: "create_charge", secrets: ["STRIPE_KEY"], granted_at: 1 }, { kind: "other", secrets: [] }, { kind: "command" }],
  });
  expect(listing.secrets.map((secret) => secret.name)).toEqual(["STRIPE_KEY"]);
  expect(listing.grants).toHaveLength(1);
  expect(readSecretsListing(null)).toEqual({ secrets: [], grants: [] });
  expect(readSecretsListing({ secrets: "nope" })).toEqual({ secrets: [], grants: [] });
});

test("secret names are environment variable names", () => {
  for (const good of ["STRIPE_KEY", "_X", "a1"]) expect(validSecretName(good)).toBe(true);
  for (const bad of ["", "1A", "A-B", "has space", "A".repeat(65)]) expect(validSecretName(bad)).toBe(false);
});
