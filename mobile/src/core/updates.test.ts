import { expect, test } from "bun:test";
import { initializeUpdates, updateCandidates, updateCheckDue, updateIsSkipped, useUpdates } from "./updates";

const release = (version: string) => ({ tag_name: `beans-v${version}`, draft: false, prerelease: false });

test("stable discovery preserves every newer candidate in numeric order", () => {
  expect(updateCandidates([
    release("1.9.0"), release("1.10.0"), release("2.0.0"), release("1.10.0"),
    release("1.8.0"), release("01.11.0"), release("1.11.0-beta.1"),
    { ...release("3.0.0"), draft: true }, { ...release("4.0.0"), prerelease: true },
    { ...release("5.0.0"), tag_name: "mobile-v5.0.0" },
  ], "1.8.0")).toEqual(["2.0.0", "1.10.0", "1.9.0"]);
  expect(() => updateCandidates([], "unknown")).toThrow("Invalid installed Beans version");
});

test("daily check policy handles opt-out, exact boundary and clock rollback", () => {
  const day = 86_400_000;
  expect(updateCheckDue({}, day)).toBe(true);
  expect(updateCheckDue({ update_checks: false }, day)).toBe(false);
  expect(updateCheckDue({ update_checked_at: 10 }, day + 9)).toBe(false);
  expect(updateCheckDue({ update_checked_at: 10 }, day + 10)).toBe(true);
  expect(updateCheckDue({ update_checked_at: day + 1 }, day)).toBe(true);
  expect(updateCheckDue({ update_checked_at: NaN }, day)).toBe(true);
  expect(updateCheckDue({}, NaN)).toBe(false);
});

test("skip suppresses only exact automatic offer, never explicit manual check", () => {
  const prefs = { update_skipped_version: "1.10.0" };
  expect(updateIsSkipped(prefs, "1.10.0", false)).toBe(true);
  expect(updateIsSkipped(prefs, "1.10.1", false)).toBe(false);
  expect(updateIsSkipped(prefs, "1.10.0", true)).toBe(false);
});

test("production id cannot authorize installation without native verifier or channel evidence", () => {
  initializeUpdates("android", "ai.amoena.beans");
  expect(useUpdates.getState()).toEqual({ status: "blocked", reason: "native_verifier_unavailable" });
  initializeUpdates("android", null);
  expect(useUpdates.getState().reason).toBe("native_verifier_unavailable");
  initializeUpdates("android", "ai.amoena.beans.dev");
  expect(useUpdates.getState().reason).toBe("development");
  initializeUpdates("ios", "ai.amoena.beans");
  expect(useUpdates.getState().reason).toBe("store_managed");
});
