import { blobatar as upstreamBlobatar, type BlobatarOptions } from "./vendor/core/blobatar";

/** Bot IDs are stable across names, devices, and relay synchronization. */
export function botAvatarSeed(botID: string): string {
  return botID;
}

/** Deterministic, self-contained SVG; no fetch or filesystem access at render time. */
export function blobatar(botID: string, options?: BlobatarOptions): string {
  return upstreamBlobatar(botAvatarSeed(botID), options);
}

export type { BlobatarOptions } from "./vendor/core/blobatar";
