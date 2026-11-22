// What a permission or command card says under Always allow. An Auto-review rule is added to
// Auto-review; a secret's grant (`grant`) is kept in the Runner's secret store and managed with
// the secrets tool, so it is never worded as a rule (issue #550).

import { t } from "../i18n";

export function alwaysAllowNote(offer: { rule?: string; grant?: boolean }, pending: boolean, decision?: string): string | undefined {
  if (!offer.rule) return undefined;
  if (offer.grant) {
    if (pending) return t("Always allow grants this from now on: “{rule}”.", { rule: offer.rule });
    return decision === "always" ? t("Allowed always. Manage it with the secrets tool.") : undefined;
  }
  if (pending) return t("Always allow adds the rule “{rule}”.", { rule: offer.rule });
  return decision === "always" ? t("Added the rule “{rule}” to Auto-review.", { rule: offer.rule }) : undefined;
}
