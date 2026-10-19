// These are facts claimed by a code signature, not Apple/profile authorization.
// A consistent identity must still fail the unsupported Apple trust gate.
export type IpaSignedIdentity = Readonly<{
  teamId: string;
  applicationIdentifier: string;
  applicationIdentifierPrefix: string;
  bundleIdentifier: string;
}>;

export function inspectIpaSignedClaims(json: string, bundleIdentifier: string, teamId: string): IpaSignedIdentity {
  function fail(): never { throw new Error("IPA signed entitlement claims are malformed or differ from release identity"); }
  if (!["ai.amoena.beans", "ai.amoena.beans.notify"].includes(bundleIdentifier) || !/^[A-Z0-9]{10}$/.test(teamId) || Buffer.byteLength(json) > 64 * 1024) fail();
  let value: unknown;
  try { value = JSON.parse(json); } catch { fail(); }
  // Bound the decoded representation as well as the subprocess output. Unknown
  // entitlement names are not approved by this structural/identity inspection.
  let nodes = 0;
  function inspect(value: unknown, depth: number): void {
    if (++nodes > 4096 || depth > 8) fail();
    if (typeof value === "string") {
      if (Buffer.byteLength(value) > 4096 || /[\x00-\x1f\x7f]/.test(value)) fail();
    } else if (typeof value === "number") {
      if (!Number.isSafeInteger(value)) fail();
    } else if (typeof value === "boolean") {
      return;
    } else if (Array.isArray(value)) {
      if (value.length > 256) fail();
      for (const item of value) inspect(item, depth + 1);
    } else if (value && typeof value === "object") {
      const entries = Object.entries(value);
      if (entries.length > 256) fail();
      for (const [key, item] of entries) {
        if (!key || Buffer.byteLength(key) > 256 || /[\x00-\x1f\x7f]/.test(key)) fail();
        inspect(item, depth + 1);
      }
    } else fail();
  }
  inspect(value, 0);
  if (!value || typeof value !== "object" || Array.isArray(value)) fail();
  const claims = value as Record<string, unknown>;
  if (claims["com.apple.developer.team-identifier"] !== teamId) fail();
  const applicationIdentifier = claims["application-identifier"];
  if (typeof applicationIdentifier !== "string") fail();
  const suffix = `.${bundleIdentifier}`;
  if (!applicationIdentifier.endsWith(suffix)) fail();
  const prefix = applicationIdentifier.slice(0, -suffix.length);
  // An App ID prefix may differ from the Team ID. Its approval belongs to the
  // future authoritative profile/owner-policy validator, not this comparison.
  if (!/^[A-Z0-9]{10}$/.test(prefix)) fail();
  for (const name of ["keychain-access-groups", "com.apple.security.application-groups"]) {
    if (!Object.hasOwn(claims, name)) continue;
    const groups = claims[name];
    if (!Array.isArray(groups) || groups.some(group => typeof group !== "string" || !group || group.includes("*")) || new Set(groups).size !== groups.length) fail();
  }
  // This store-IPA inspection refuses an explicit debugger allowance. Absence
  // is not distribution authorization; the authoritative profile gate is open work.
  if (Object.hasOwn(claims, "get-task-allow") && claims["get-task-allow"] !== false) fail();
  return Object.freeze({ teamId, applicationIdentifier, applicationIdentifierPrefix: prefix, bundleIdentifier });
}
