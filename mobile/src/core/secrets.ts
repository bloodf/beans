// A Runner's secret store as the phone shows it (#560): secrets by name, never by value, and the
// uses the user allowed always. The Runner answers `secrets.*` for the account's own Devices.

export interface StoredSecret {
  name: string;
  length: number;
  source: string;
  note: string;
  saved_by: string;
  updated_at: number;
}

export type SecretGrant =
  | { kind: "command"; folder: string; command: string; secrets: string[]; granted_at: number }
  | { kind: "tool"; tool: string; secrets: string[]; granted_at: number };

export interface SecretsListing {
  secrets: StoredSecret[];
  grants: SecretGrant[];
}

/** A secret's name: an environment variable's, as the Runner's store takes it. */
export function validSecretName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name);
}

/** What the Runner answered, kept only if it has the shape the screen reads. */
export function readSecretsListing(value: unknown): SecretsListing {
  const listing = value as Partial<SecretsListing> | null;
  const secrets = Array.isArray(listing?.secrets) ? listing!.secrets.filter((secret) => typeof secret?.name === "string" && typeof secret.length === "number") : [];
  const grants = Array.isArray(listing?.grants) ? listing!.grants.filter((grant) => (grant?.kind === "command" || grant?.kind === "tool") && Array.isArray(grant.secrets)) : [];
  return { secrets, grants };
}
