import { expect, test } from "bun:test";
import { inspectIpaSignedClaims } from "./release-ipa-claims.ts";
import { preflightMobileTrust, preflightReleaseTrust } from "./release-build-inputs.ts";

const teamId = "TEAMONLY00";
const prefix = "PREFIX0000";
const identity = (bundle = "ai.amoena.beans") => ({
  "application-identifier": `${prefix}.${bundle}`,
  "com.apple.developer.team-identifier": teamId,
  "keychain-access-groups": [`${prefix}.${bundle}`],
  "com.apple.security.application-groups": ["group.ai.amoena.beans"],
  "get-task-allow": false,
});
const inspect = (value: unknown, bundle = "ai.amoena.beans") => inspectIpaSignedClaims(JSON.stringify(value), bundle, teamId);

test("signed app and extension claims bind exact native identity without equating App ID prefix and Team ID", () => {
  for (const bundle of ["ai.amoena.beans", "ai.amoena.beans.notify"]) {
    const result = inspect(identity(bundle), bundle);
    expect(result).toEqual({ teamId, applicationIdentifier: `${prefix}.${bundle}`, applicationIdentifierPrefix: prefix, bundleIdentifier: bundle });
    expect(Object.isFrozen(result)).toBe(true);
  }
  expect(() => inspect(identity("ai.amoena.beans.notify"))).toThrow("signed entitlement claims");
  expect(() => inspect(identity(), "ai.amoena.beans.notify")).toThrow("signed entitlement claims");
});

test("absent, malformed and mismatched signed identities refuse without echoing candidate content", () => {
  const cases: unknown[] = [null, [], true, "claims", {},
    { ...identity(), "com.apple.developer.team-identifier": "OTHERTEAM0" },
    { ...identity(), "com.apple.developer.team-identifier": [teamId] },
    { ...identity(), "application-identifier": null },
    { ...identity(), "application-identifier": `${prefix}.ai.amoena.beans.evil` },
    { ...identity(), "application-identifier": `${prefix}.*` },
    { ...identity(), "application-identifier": `*.ai.amoena.beans` },
    { ...identity(), "application-identifier": `EXTRA.${prefix}.ai.amoena.beans` },
    { ...identity(), "application-identifier": `${prefix.toLowerCase()}.ai.amoena.beans` },
    { ...identity(), "get-task-allow": "false" },
    { ...identity(), "get-task-allow": true },
  ];
  for (const value of cases) expect(() => inspect(value)).toThrow("IPA signed entitlement claims are malformed or differ from release identity");
  expect(() => inspectIpaSignedClaims("{broken", "ai.amoena.beans", teamId)).toThrow("signed entitlement claims");
  expect(() => inspectIpaSignedClaims(JSON.stringify(identity()), "other.app", teamId)).toThrow("signed entitlement claims");
  expect(() => inspectIpaSignedClaims(JSON.stringify(identity()), "ai.amoena.beans", "wrong")).toThrow("signed entitlement claims");
});

test("wildcards are refused in signed group claims, independently of profile allowlist matching", () => {
  for (const name of ["keychain-access-groups", "com.apple.security.application-groups"]) {
    for (const value of ["group", [null], [""], ["group.*"], ["*"], ["group", "group"]]) {
      expect(() => inspect({ ...identity(), [name]: value })).toThrow("signed entitlement claims");
    }
  }
  // A concrete unrelated group is still only a claim: owner/profile approval is
  // not invented from this parser or from the candidate's own signature.
  expect(inspect({ ...identity(), "keychain-access-groups": ["OTHER00000.unapproved"] }).teamId).toBe(teamId);
});

test("signed entitlement decoding has byte, depth, collection, string and integer bounds", () => {
  expect(() => inspectIpaSignedClaims(" ".repeat(65537), "ai.amoena.beans", teamId)).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), large: "é".repeat(2049) })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), list: Array(257).fill(false) })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), fractional: 1.5 })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), unsafe: Number.MAX_SAFE_INTEGER + 1 })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), control: "line\nfeed" })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), absent: null })).toThrow("signed entitlement claims");
  let nested: unknown = false;
  for (let i = 0; i < 9; i++) nested = [nested];
  expect(() => inspect({ ...identity(), nested })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), nodes: Array.from({ length: 17 }, () => Array(256).fill(false)) })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), ["k".repeat(257)]: false })).toThrow("signed entitlement claims");
  expect(() => inspect({ ...identity(), ...Object.fromEntries(Array.from({ length: 257 }, (_, i) => [`key${i}`, false])) })).toThrow("signed entitlement claims");
});

test("consistent synthetic claims never authorize Apple binaries or all-platform preflight", () => {
  const claims = inspect({ ...identity(), "unknown.entitlement": { requested: true } });
  expect(claims.applicationIdentifierPrefix).toBe(prefix);
  const policy = { profile: "testflight", teamId, certificateSha256: ["0".repeat(64)] } as const;
  expect(() => preflightMobileTrust("testflight", policy)).toThrow("IPA cryptographic trust verification unsupported");
  expect(() => preflightReleaseTrust("all", {
    github: { profile: "github", signerSha256: ["0".repeat(64)], rotation: "none" },
    production: { profile: "production", uploadSignerSha256: "0".repeat(64), rotation: "none" },
    testflight: { ...policy, certificateSha256: [...policy.certificateSha256] },
  })).toThrow("IPA cryptographic trust verification unsupported");
});
