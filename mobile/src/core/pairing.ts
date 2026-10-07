// The pairing string an existing Device shows (as a QR code or text):
// `beans://pair?v=2&relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`. The core does
// the pairing itself; this only checks a scanned or pasted string before it is handed over.

import { t } from "../i18n";

export interface PairingTarget {
  relay: string;
  id: string;
  ek: string;
  nonce: string;
}

export function parsePairingString(text: string): PairingTarget {
  const trimmed = text.trim();
  const prefix = "beans://pair?";
  if (!trimmed.startsWith(prefix)) {
    throw new Error(t("That is not a Beans pairing string"));
  }
  const fields = new Map<string, string>();
  for (const pair of trimmed.slice(prefix.length).split("&")) {
    const eq = pair.indexOf("=");
    const key = pair.slice(0, eq);
    if (eq < 0 || !["v", "relay", "id", "ek", "n"].includes(key) || fields.has(key)) {
      throw new Error(t("Pairing string has an invalid or duplicate field"));
    }
    try {
      fields.set(key, decodeURIComponent(pair.slice(eq + 1)));
    } catch {
      throw new Error(t("Pairing string has an invalid escape"));
    }
  }
  if (fields.get("v") !== "2") {
    throw new Error(t("Beans v2 pairing code required"));
  }
  const relay = fields.get("relay");
  const id = fields.get("id");
  const ek = fields.get("ek");
  const n = fields.get("n");
  if (!relay || !id || !ek || !n) throw new Error(t("Pairing string is missing a field"));
  let url: URL;
  try {
    url = new URL(relay);
  } catch {
    throw new Error(t("Pairing string has an invalid relay URL"));
  }
  if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password || relay.includes("?") || relay.includes("#")) {
    throw new Error(t("Pairing string has an invalid relay URL"));
  }
  // Unpadded base64url encodes 32 bytes in 43 characters, with two zero trailing bits.
  const publicKey = /^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/;
  if (id.length !== 43 || ek.length !== 43 || !publicKey.test(id) || !publicKey.test(ek) || n.length > 256 || /[^A-Za-z0-9_-]/.test(n)) {
    throw new Error(t("Pairing string has an invalid key or nonce"));
  }
  return { relay: relay.replace(/\/+$/, ""), id, ek, nonce: n };
}
