// A Runner's secret store as the Secrets pane shows it (#560): each secret by name with its
// length and where it came from, and the uses the user allowed always. The Runner never sends a
// value; this only words and checks what it sends.

import { L } from "../l10n";
import * as Format from "./format";

export interface WireStoredSecret {
  name: string;
  length: number;
  source: string;
  note: string;
  saved_by: string;
  updated_at: number;
}

export type WireSecretGrant =
  | { kind: "command"; folder: string; command: string; secrets: string[]; granted_at: number }
  | { kind: "tool"; tool: string; secrets: string[]; granted_at: number };

export interface WireSecrets {
  secrets: WireStoredSecret[];
  grants: WireSecretGrant[];
}

/** A secret's name: an environment variable's, as the Runner's store takes it. */
export function validSecretName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name);
}

/** A secret's row: its length, where it came from, and what it is for. */
export function secretSummary(secret: WireStoredSecret): string {
  const parts = [L("%d characters", secret.length), secret.source];
  if (secret.note) parts.push(secret.note);
  if (secret.saved_by) parts.push(L("saved by %@", secret.saved_by));
  return parts.filter(Boolean).join(" · ");
}

/** What a grant lets through, in one line. */
export function grantTitle(grant: WireSecretGrant): string {
  return grant.kind === "command" ? `$ ${grant.command}` : L("%@ receives %@", grant.tool, grant.secrets.join(", "));
}

/** Where and since when a grant applies. */
export function grantDetail(grant: WireSecretGrant): string {
  const when = grant.granted_at > 0 ? L("since %@", Format.stamp(grant.granted_at * 1000)) : "";
  const where = grant.kind === "command" ? L("in %@ with %@", grant.folder, grant.secrets.join(", ")) : "";
  return [where, when].filter(Boolean).join(" · ");
}
