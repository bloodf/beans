// App-global update policy. GitHub release metadata identifies candidates, never install authority.
import { create } from "zustand";

const DAY = 24 * 60 * 60 * 1000;
export const UPDATE_REPOSITORY = "bloodf/beans";

export interface UpdatePreferences {
  update_checks?: boolean;
  update_checked_at?: number;
  update_skipped_version?: string;
}

export interface ReleaseCandidate {
  tag_name: string;
  draft: boolean;
  prerelease: boolean;
}

function versionParts(version: string): number[] | null {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) return null;
  const parts = version.split(".").map(Number);
  return parts.every(Number.isSafeInteger) ? parts : null;
}

function compare(a: number[], b: number[]): number {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i]! > b[i]! ? 1 : -1;
  return 0;
}

// Return every newer stable candidate in numeric order. A newer server-only release must
// not hide an older Android-ready release. Native signed readiness must authorize each one.
export function updateCandidates(releases: readonly ReleaseCandidate[], installed: string): string[] {
  const current = versionParts(installed);
  if (!current) throw new Error("Invalid installed Beans version");
  const versions = new Map<string, number[]>();
  for (const release of releases) {
    if (release.draft !== false || release.prerelease !== false || !release.tag_name.startsWith("beans-v")) continue;
    const version = release.tag_name.slice(7);
    const parts = versionParts(version);
    if (parts && compare(parts, current) > 0) versions.set(version, parts);
  }
  return [...versions.keys()].sort((a, b) => compare(versions.get(b)!, versions.get(a)!));
}

export function updateCheckDue(prefs: UpdatePreferences, now: number): boolean {
  if (prefs.update_checks === false || !Number.isSafeInteger(now) || now < 0) return false;
  const last = prefs.update_checked_at;
  return last === undefined || !Number.isSafeInteger(last) || last < 0 || last > now || now - last >= DAY;
}

export function updateIsSkipped(prefs: UpdatePreferences, version: string, manual: boolean): boolean {
  return !manual && prefs.update_skipped_version === version;
}

export type UpdateBlock = "store_managed" | "development" | "native_verifier_unavailable";

export function updateBlock(platform: string, applicationId: string | null): UpdateBlock {
  if (applicationId === "ai.amoena.beans.dev") return "development";
  if (platform !== "android") return "store_managed";
  // Current Android bridge exposes neither pinned readiness verification nor distribution
  // evidence. Production package identity alone cannot authorize GitHub APK installation.
  return "native_verifier_unavailable";
}

// No network, download, consent token or install state exists while native trust is absent.
export const useUpdates = create<{ status: "blocked"; reason: UpdateBlock }>()(() => ({
  status: "blocked",
  reason: "native_verifier_unavailable",
}));

export function initializeUpdates(platform: string, applicationId: string | null): void {
  useUpdates.setState({ status: "blocked", reason: updateBlock(platform, applicationId) });
}
