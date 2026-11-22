// The words of the Secrets screen (#560), kept apart from the screen so tests cover them.

import type { SecretGrant, StoredSecret } from "../core/secrets";
import { t } from "../i18n";

export function secretSummary(secret: StoredSecret): string {
  const parts = [t("{count} characters", { count: secret.length }), secret.source];
  if (secret.note) parts.push(secret.note);
  if (secret.saved_by) parts.push(t("saved by {name}", { name: secret.saved_by }));
  return parts.filter(Boolean).join(" · ");
}

export function grantTitle(grant: SecretGrant): string {
  return grant.kind === "command" ? `$ ${grant.command}` : t("{tool} receives {secrets}", { tool: grant.tool, secrets: grant.secrets.join(", ") });
}

export function grantSubtitle(grant: SecretGrant): string | undefined {
  return grant.kind === "command" ? t("in {folder} with {secrets}", { folder: grant.folder, secrets: grant.secrets.join(", ") }) : undefined;
}
